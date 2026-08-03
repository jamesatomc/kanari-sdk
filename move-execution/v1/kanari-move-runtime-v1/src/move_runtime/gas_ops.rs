// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Gas metering and accounting operations
use crate::changeset::ChangeSet;
use anyhow::Result;
use kanari_types::GasConfig;
use kanari_types::gas::{GasMeter, GasOperation};
use move_core_types::language_storage::ModuleId;

impl super::MoveRuntime {
    /// Helper to apply gas accounting to a ChangeSet. Handles sender debit + sequence increment
    /// metadata only. Monetary gas charging is owned by the engine layer so every
    /// execution path debits exactly once.
    pub(crate) fn apply_gas_info(
        &self,
        cs: &mut ChangeSet,
        gas_limit: u64,
        gas_price: u64,
        gas_op: GasOperation,
        storage_written: u64,
        storage_deleted: u64,
    ) -> Result<()> {
        let mut meter = GasMeter::new(gas_limit, gas_price);
        let config = GasConfig::default();

        // Charge execution gas
        meter.consume(gas_op.gas_units())?;

        // Charge storage gas
        meter.charge_storage(storage_written, &config)?;
        meter.rebate_storage(storage_deleted);

        // Report gas units derived from GasOperation. The VM-internal KanariGasMeter
        // is still used as an execution cap, but monetary debit/credit happens later
        // in kanari-core so publish/call/immediate/mempool paths stay consistent.
        cs.set_gas_used(gas_op.gas_units());

        Ok(())
    }

    /// Calculate storage bytes written from Move VM changeset and Kanari ChangeSet
    pub(crate) fn calculate_storage_impact(
        &self,
        move_cs: &move_core_types::effects::ChangeSet,
        kanari_cs: &ChangeSet,
        state_overlay: crate::StateOverlayView<'_>,
    ) -> Result<(u64, u64)> {
        let mut written = 0;
        let mut deleted = 0;
        let overlay_state = self.state.with_overlay(state_overlay.cloned());

        // 1. Move VM Changes (Modules & Resources)
        for (addr, changes) in move_cs.accounts() {
            for (module_name, op) in changes.modules() {
                match op {
                    move_core_types::effects::Op::New(bytes)
                    | move_core_types::effects::Op::Modify(bytes) => {
                        written += bytes.len() as u64;
                    }
                    move_core_types::effects::Op::Delete => {
                        let module_id = ModuleId::new(*addr, module_name.clone());
                        if let Some(bytes) = overlay_state.try_get_module(&module_id).map_err(|error| {
                            anyhow::anyhow!(
                                "Failed to read deleted module {module_id} for gas accounting: {error:?}"
                            )
                        })? {
                            deleted += bytes.len() as u64;
                        }
                    }
                }
            }
            for (tag, op) in changes.resources() {
                match op {
                    move_core_types::effects::Op::New(bytes)
                    | move_core_types::effects::Op::Modify(bytes) => {
                        written += bytes.len() as u64;
                    }
                    move_core_types::effects::Op::Delete => {
                        if let Some(bytes) =
                            overlay_state.try_get_resource(addr, tag).map_err(|error| {
                                anyhow::anyhow!(
                                    "Failed to read deleted resource {addr}::{tag} for gas accounting: {error:?}"
                                )
                            })?
                        {
                            deleted += bytes.len() as u64;
                        }
                    }
                }
            }
        }

        // 2. Kanari Objects (Created/Modified)
        for (_, obj) in &kanari_cs.created_objects {
            written += obj.data.len() as u64;
            // Add some overhead for type name and owner
            written += obj.type_.len() as u64 + 32;
        }

        // 3. Kanari Objects (Deleted)
        // We look up the object in storage to find its size for the rebate.
        for obj_id in &kanari_cs.deleted_objects {
            if let Some(obj) =
                self.get_object_for_execution(obj_id, state_overlay.map(std::sync::Arc::as_ref))?
            {
                deleted += obj.data.len() as u64;
                deleted += obj.type_name.len() as u64 + 32;
            }
        }

        // 4. Events (Events consume storage/log space)
        for event in &kanari_cs.events {
            written += event.event_data.len() as u64;
            written += event.type_tag.len() as u64;
        }

        Ok((written, deleted))
    }
}
