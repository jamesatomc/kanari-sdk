#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(relative: str, old: str, new: str) -> None:
    path = ROOT / relative
    text = path.read_text()
    if new in text and old not in text:
        return
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{relative}: expected one source match, found {count}\n{old}")
    path.write_text(text.replace(old, new, 1))


def replace_all(relative: str, old: str, new: str, minimum: int = 1) -> None:
    path = ROOT / relative
    text = path.read_text()
    if old not in text and new in text:
        return
    count = text.count(old)
    if count < minimum:
        raise RuntimeError(f"{relative}: expected at least {minimum} matches, found {count}: {old}")
    path.write_text(text.replace(old, new))


replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/kanari_gas_meter.rs",
    """    pub fn new(gas_limit: u64) -> Self {\n        Self {\n            gas_used: 0,\n            gas_limit,\n        }\n    }\n\n""",
    """    pub fn new(gas_limit: u64) -> Self {\n        Self {\n            gas_used: 0,\n            gas_limit,\n        }\n    }\n\n    pub fn gas_used(&self) -> u64 {\n        self.gas_used\n    }\n\n""",
)

replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/gas_ops.rs",
    """        gas_op: GasOperation,\n        storage_written: u64,\n        storage_deleted: u64,\n""",
    """        gas_op: GasOperation,\n        vm_gas_used: u64,\n        storage_written: u64,\n        storage_deleted: u64,\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/gas_ops.rs",
    """        // Charge execution gas\n        meter.consume(gas_op.gas_units())?;\n\n        // Charge storage gas\n        meter.charge_storage(storage_written, &config)?;\n        meter.rebate_storage(storage_deleted);\n""",
    """        // Charge at least the static admission cost and otherwise the actual\n        // MoveVM instruction/native work observed during execution.\n        meter.consume(gas_op.gas_units().max(vm_gas_used))?;\n\n        // Storage is free monetarily, but write-set size is still resource\n        // metered so a zero-price transaction cannot emit unbounded state.\n        let storage_units = storage_written\n            .checked_add(storage_deleted / 2)\n            .ok_or_else(|| anyhow::anyhow!(\"Storage gas overflow\"))?;\n        meter.consume(storage_units)?;\n        meter.charge_storage(storage_written, &config)?;\n        meter.rebate_storage(storage_deleted);\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/gas_ops.rs",
    """        // Use the gas units from GasOperation (calculated by our GasMeter), not from KanariGasMeter\n        // KanariGasMeter is only for DoS protection during VM execution\n        cs.set_gas_used(gas_op.gas_units());\n""",
    """        cs.set_gas_used(meter.gas_used);\n""",
)

replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    """        let (move_changeset, events) = {\n            // Separate lock into a variable first to prevent it from being dropped immediately\n            let vm_guard = self.read_vm();\n            let mut session = self.create_session_with_storage_ext(&vm_guard);\n\n            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);\n            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);\n\n            session\n                .publish_module(module_bytes.clone(), sender, &mut metered_gas)\n                .map_err(|e| anyhow::anyhow!(\"{:?}\", e))?;\n\n            session.finish().0.map_err(|e| anyhow::anyhow!(\"{:?}\", e))?\n        };\n""",
    """        let (move_changeset, events, vm_gas_used) = {\n            let vm_guard = self.read_vm();\n            let mut session = self.create_session_with_storage_ext(&vm_guard);\n\n            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);\n            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);\n\n            session\n                .publish_module(module_bytes.clone(), sender, &mut metered_gas)\n                .map_err(|e| anyhow::anyhow!(\"{:?}\", e))?;\n            let vm_gas_used = metered_gas.gas_used();\n            let (changeset, events) =\n                session.finish().0.map_err(|e| anyhow::anyhow!(\"{:?}\", e))?;\n            (changeset, events, vm_gas_used)\n        };\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    """                gas_op,\n                written,\n                deleted,\n""",
    """                gas_op,\n                vm_gas_used,\n                written,\n                deleted,\n""",
)

replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    """        let execution_result = if bypass_entry_check {\n            let mut unmetered_gas = UnmeteredGasMeter;\n            session.execute_function_bypass_visibility(\n                module_id,\n                ident,\n                ty_args_loaded,\n                final_args,\n                &mut unmetered_gas,\n            )\n        } else {\n            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);\n            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);\n            session.execute_entry_function(\n                module_id,\n                ident,\n                ty_args_loaded,\n                final_args,\n                &mut metered_gas,\n            )\n        };\n""",
    """        let (execution_result, vm_gas_used) = if bypass_entry_check {\n            let mut unmetered_gas = UnmeteredGasMeter;\n            (\n                session.execute_function_bypass_visibility(\n                    module_id,\n                    ident,\n                    ty_args_loaded,\n                    final_args,\n                    &mut unmetered_gas,\n                ),\n                0,\n            )\n        } else {\n            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);\n            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);\n            let result = session.execute_entry_function(\n                module_id,\n                ident,\n                ty_args_loaded,\n                final_args,\n                &mut metered_gas,\n            );\n            (result, metered_gas.gas_used())\n        };\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    """                    self.apply_gas_info(\n                        &mut cs, sender, gas_limit, gas_price, gas_op, written, deleted,\n                    )?;\n""",
    """                    self.apply_gas_info(\n                        &mut cs,\n                        sender,\n                        gas_limit,\n                        gas_price,\n                        gas_op,\n                        vm_gas_used,\n                        written,\n                        deleted,\n                    )?;\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    """                        GasOperation::ExecuteFunction {\n                            complexity: penalty_complexity,\n                        },\n                        0,\n                        0,\n""",
    """                        GasOperation::ExecuteFunction {\n                            complexity: penalty_complexity,\n                        },\n                        vm_gas_used,\n                        0,\n                        0,\n""",
)

replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    """    pub fn persist_created_objects(&self, cs: &ChangeSet) {\n        for (id, created) in &cs.created_objects {\n            let stored = StoredObject {\n                id: id.clone(),\n                owner: created.owner,\n                type_name: created.type_.clone(),\n                data: created.data.clone(),\n                version: created.version,\n            };\n            let _ = self.object_storage.store_object(stored);\n        }\n    }\n\n    pub fn persist_deleted_objects(&self, cs: &ChangeSet) {\n        for obj_id in &cs.deleted_objects {\n            let _ = self.object_storage.delete_object(obj_id);\n        }\n    }\n""",
    """    pub fn persist_created_objects(&self, cs: &ChangeSet) -> Result<()> {\n        for (id, created) in &cs.created_objects {\n            let stored = StoredObject {\n                id: id.clone(),\n                owner: created.owner,\n                type_name: created.type_.clone(),\n                data: created.data.clone(),\n                version: created.version,\n            };\n            self.object_storage\n                .store_object(stored)\n                .map_err(|error| anyhow::anyhow!(\"Failed to persist object {id}: {error}\"))?;\n        }\n        Ok(())\n    }\n\n    pub fn persist_deleted_objects(&self, cs: &ChangeSet) -> Result<()> {\n        for obj_id in &cs.deleted_objects {\n            self.object_storage\n                .delete_object(obj_id)\n                .map_err(|error| anyhow::anyhow!(\"Failed to delete object {obj_id}: {error}\"))?;\n        }\n        Ok(())\n    }\n""",
)
replace_all(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    "self.persist_created_objects(&cs);",
    "self.persist_created_objects(&cs)?;",
    minimum=2,
)
replace_all(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    "self.persist_deleted_objects(&cs);",
    "self.persist_deleted_objects(&cs)?;",
    minimum=1,
)

replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs",
    """    pub fn reload_vm_cache(&self) -> Result<()> {\n        let new_vm = MoveVM::new(self.all_natives.as_ref().clone())\n""",
    """    pub fn reload_vm_cache(&self) -> Result<()> {\n        let refreshed_modules: HashSet<ModuleId> =\n            self.state.get_all_module_ids()?.into_iter().collect();\n        *self\n            .published_modules\n            .write()\n            .unwrap_or_else(|poisoned| poisoned.into_inner()) = refreshed_modules;\n\n        let new_vm = MoveVM::new(self.all_natives.as_ref().clone())\n""",
)

print("patched runtime metering and checked persistence")
