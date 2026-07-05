// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Gas metering and accounting operations
use crate::changeset::ChangeSet;
use anyhow::Result;
use kanari_types::GasConfig;
use kanari_types::address::Address as KanariAddress;
use kanari_types::gas::{GasMeter, GasOperation};
use move_core_types::account_address::AccountAddress;

use move_core_types::language_storage::ModuleId;
use move_core_types::resolver::{ModuleResolver, ResourceResolver};

impl super::MoveRuntime {
    /// Helper to apply gas accounting to a ChangeSet. Handles sender debit + sequence increment
    /// and credits gas to DAO. `sender` may be `None` for system-level calls.
    pub(crate) fn apply_gas_info(
        &self,
        cs: &mut ChangeSet,
        sender: Option<AccountAddress>,
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

        // Calculate total cost: execution (in Mist) + net storage fee (in Mist)
        // execution cost = meter.total_cost()
        // net storage fee = meter.net_storage_fee(&config)
        let execution_cost = meter.total_cost();
        let storage_fee = meter.net_storage_fee(&config);

        // Total cost can't be negative overall (though storage rebate could exceed storage cost)
        // But execution cost should usually cover it. If total < 0, we cap at 0.
        let total_cost_signed = (execution_cost as i128) + (storage_fee as i128);
        let total_cost = if total_cost_signed < 0 {
            0
        } else {
            total_cost_signed as u64
        };

        if let Some(saddr) = sender {
            let sender_change = cs.get_or_create_change(saddr);
            sender_change.increment_sequence();
            sender_change.debit(total_cost);
        }

        let dao_addr = AccountAddress::from_hex_literal(KanariAddress::DAO_ADDRESS)?;
        cs.collect_gas(dao_addr, total_cost);

        // Use the gas units from GasOperation (calculated by our GasMeter), not from KanariGasMeter
        // KanariGasMeter is only for DoS protection during VM execution
        cs.set_gas_used(gas_op.gas_units());

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
