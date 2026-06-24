// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Gas metering operations. Monetary accounting and sequence advancement are
// deliberately owned by kanari-core so every transaction is charged exactly
// once, regardless of whether it uses a native fast path or MoveVM.
use crate::changeset::ChangeSet;
use anyhow::Result;
use kanari_types::{GasConfig, GasMeter, GasOperation};
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::ModuleId;
use move_core_types::resolver::{ModuleResolver, ResourceResolver};

impl super::MoveRuntime {
    /// Apply resource metering to a ChangeSet.
    ///
    /// This method validates the signed gas price, enforces the signed gas
    /// limit, and records actual resource usage. It must not debit balances,
    /// credit the DAO, or advance sequence numbers; kanari-core performs those
    /// state transitions once after merging the runtime ChangeSet.
    pub(crate) fn apply_gas_info(
        &self,
        cs: &mut ChangeSet,
        _sender: Option<AccountAddress>,
        gas_limit: u64,
        gas_price: u64,
        gas_op: GasOperation,
        vm_gas_used: u64,
        storage_written: u64,
        storage_deleted: u64,
    ) -> Result<()> {
        let mut meter = GasMeter::new(gas_limit, gas_price);
        let config = GasConfig::default();
        config.validate_price(gas_price)?;

        // Charge at least the deterministic admission cost and otherwise the
        // actual MoveVM instruction/native work observed during execution.
        meter.consume(gas_op.gas_units().max(vm_gas_used))?;

        // Storage and event output are converted into resource units. This
        // remains enforced in zero-fee mode and therefore prevents an attacker
        // from creating an unbounded write set with a zero monetary gas price.
        // Deletions still require bounded processing, but receive a lower unit
        // weight than newly written bytes.
        let storage_units = storage_written
            .checked_add(storage_deleted / 2)
            .ok_or_else(|| anyhow::anyhow!("Storage gas overflow"))?;
        meter.consume(storage_units)?;

        // Preserve byte counters for diagnostics/estimation. Monetary charging
        // is intentionally deferred to core and is based on the final metered
        // gas usage, avoiding duplicate balance and DAO mutations.
        meter.charge_storage(storage_written, &config)?;
        meter.rebate_storage(storage_deleted);

        cs.set_gas_used(meter.gas_used);
        Ok(())
    }

    /// Calculate storage bytes written from Move VM changeset and Kanari ChangeSet
    pub(crate) fn calculate_storage_impact(
        &self,
        move_cs: &move_core_types::effects::ChangeSet,
        kanari_cs: &ChangeSet,
    ) -> (u64, u64) {
        let mut written = 0;
        let mut deleted = 0;

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
                        if let Ok(Some(bytes)) = self.resolver.get_module(&module_id) {
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
                        if let Ok(Some(bytes)) = self.resolver.get_resource(addr, tag) {
                            deleted += bytes.len() as u64;
                        }
                    }
                }
            }
        }

        // 2. Kanari Objects (Created/Modified)
        for (_id, obj) in &kanari_cs.created_objects {
            written += obj.data.len() as u64;
            // Add some overhead for type name and owner
            written += obj.type_.len() as u64 + 32;
        }

        // 3. Kanari Objects (Deleted)
        // We look up the object in storage to find its size for the rebate.
        for obj_id in &kanari_cs.deleted_objects {
            if let Some(obj) = self.object_storage.get_object(obj_id) {
                deleted += obj.data.len() as u64;
                deleted += obj.type_name.len() as u64 + 32;
            }
        }

        // 4. Events (Events consume storage/log space)
        for event in &kanari_cs.events {
            written += event.event_data.len() as u64;
            written += event.type_tag.len() as u64;
        }

        (written, deleted)
    }
}
