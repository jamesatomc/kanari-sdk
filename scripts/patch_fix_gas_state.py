#!/usr/bin/env python3
from pathlib import Path
import shutil

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


def copy_template(template: str, destination: str) -> None:
    shutil.copyfile(ROOT / template, ROOT / destination)


copy_template(
    "scripts/fix-gas-templates/scheduler.rs",
    "move-execution/v1/kanari-move-runtime-v1/src/scheduler.rs",
)
copy_template(
    "scripts/fix-gas-templates/scheduler_tests.rs",
    "move-execution/v1/kanari-move-runtime-v1/tests/unit/scheduler_tests.rs",
)

replace_once(
    "crates/kanari-types/src/transaction.rs",
    """    /// Get conflict keys for this transaction.\n    /// Transactions with overlapping conflict keys must be executed sequentially.\n    pub fn get_conflict_keys(&self) -> Vec<String> {\n""",
    """    /// Returns true only when the complete mutable access set can be\n    /// derived before Move bytecode execution. Arbitrary Move calls may touch\n    /// globals and dynamic fields that are not represented in their arguments.\n    pub fn has_complete_conflict_set(&self) -> bool {\n        matches!(self, Transaction::ExecuteFunction { .. }) && self.native_call().is_some()\n    }\n\n    /// Get conflict keys for this transaction.\n    /// Transactions with overlapping conflict keys must be executed sequentially.\n    pub fn get_conflict_keys(&self) -> Vec<String> {\n""",
)

replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/changeset.rs",
    "use move_core_types::account_address::AccountAddress;\n",
    "use move_core_types::account_address::AccountAddress;\nuse move_core_types::language_storage::{ModuleId, StructTag};\n",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/changeset.rs",
    """    pub removed_dynamic_fields: Vec<(String, Vec<u8>)>,\n    pub gas_used: u64,\n""",
    """    pub removed_dynamic_fields: Vec<(String, Vec<u8>)>,\n    /// Canonical Move module writes captured from the VM session.\n    pub module_writes: Vec<(ModuleId, Vec<u8>)>,\n    /// Canonical Move resource writes. None represents deletion.\n    pub resource_writes: Vec<(AccountAddress, StructTag, Option<Vec<u8>>)>,\n    pub gas_used: u64,\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/changeset.rs",
    """            added_dynamic_fields: Vec::new(),\n            removed_dynamic_fields: Vec::new(),\n            gas_used,\n""",
    """            added_dynamic_fields: Vec::new(),\n            removed_dynamic_fields: Vec::new(),\n            module_writes: Vec::new(),\n            resource_writes: Vec::new(),\n            gas_used,\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/changeset.rs",
    """            && self.removed_dynamic_fields.is_empty()\n            && self.gas_used == 0\n""",
    """            && self.removed_dynamic_fields.is_empty()\n            && self.module_writes.is_empty()\n            && self.resource_writes.is_empty()\n            && self.gas_used == 0\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/changeset.rs",
    """        self.removed_dynamic_fields\n            .append(&mut other.removed_dynamic_fields);\n\n        self.gas_used += other.gas_used;\n""",
    """        self.removed_dynamic_fields\n            .append(&mut other.removed_dynamic_fields);\n        self.module_writes.append(&mut other.module_writes);\n        self.resource_writes.append(&mut other.resource_writes);\n\n        self.gas_used = self.gas_used.saturating_add(other.gas_used);\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/changeset.rs",
    """    pub fn add_event(&mut self, event: Event) {\n        self.events.push(event);\n    }\n\n""",
    """    pub fn add_event(&mut self, event: Event) {\n        self.events.push(event);\n    }\n\n    pub fn add_module_write(&mut self, module_id: ModuleId, bytes: Vec<u8>) {\n        self.module_writes.retain(|(existing, _)| existing != &module_id);\n        self.module_writes.push((module_id, bytes));\n    }\n\n    pub fn add_resource_write(\n        &mut self,\n        address: AccountAddress,\n        tag: StructTag,\n        bytes: Option<Vec<u8>>,\n    ) {\n        self.resource_writes\n            .retain(|(existing_address, existing_tag, _)| {\n                existing_address != &address || existing_tag != &tag\n            });\n        self.resource_writes.push((address, tag, bytes));\n    }\n\n""",
)

replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/parsers.rs",
    "use move_core_types::effects::Op as MoveOp;\n",
    "use move_core_types::effects::Op as MoveOp;\nuse move_core_types::language_storage::ModuleId;\n",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/parsers.rs",
    """            for (module_name, op) in account_changes.modules() {\n                if matches!(op, MoveOp::New(_) | MoveOp::Modify(_)) {\n                    kanari_cs.publish_module(*addr, module_name.to_string());\n                }\n            }\n\n            for (struct_tag, op) in account_changes.resources() {\n                match op {\n""",
    """            for (module_name, op) in account_changes.modules() {\n                match op {\n                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {\n                        kanari_cs.publish_module(*addr, module_name.to_string());\n                        kanari_cs.add_module_write(\n                            ModuleId::new(*addr, module_name.clone()),\n                            bytes.to_vec(),\n                        );\n                    }\n                    MoveOp::Delete => {}\n                }\n            }\n\n            for (struct_tag, op) in account_changes.resources() {\n                kanari_cs.add_resource_write(\n                    *addr,\n                    struct_tag.clone(),\n                    match op {\n                        MoveOp::New(bytes) | MoveOp::Modify(bytes) => Some(bytes.to_vec()),\n                        MoveOp::Delete => None,\n                    },\n                );\n                match op {\n""",
)

replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/state.rs",
    """    pub(crate) fn delete_internal(&mut self, key: &[u8]) {\n        self.overlay.insert(key.to_vec(), None);\n        self.update_canonical_root_cache(key, None);\n    }\n\n""",
    """    pub(crate) fn delete_internal(&mut self, key: &[u8]) {\n        self.overlay.insert(key.to_vec(), None);\n        self.update_canonical_root_cache(key, None);\n    }\n\n    /// Stage auxiliary metadata in the same atomic store batch as state.\n    /// Non-canonical keys are deliberately excluded from the state root.\n    pub fn stage_value<T: Serialize + ?Sized>(&mut self, key: &[u8], value: &T) -> Result<()> {\n        self.save_internal(key, value)\n    }\n\n""",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/state.rs",
    """        let mut owners_to_recompute = self.owners_requiring_balance_recompute(changeset)?;\n\n        for (address, change) in &changeset.account_changes {\n""",
    """        let mut owners_to_recompute = self.owners_requiring_balance_recompute(changeset)?;\n\n        for (module_id, bytes) in &changeset.module_writes {\n            let module_key = format!(\n                \"module:{}:{}\",\n                module_id.address().to_hex_literal(),\n                module_id.name().as_str()\n            );\n            self.save_internal(module_key.as_bytes(), bytes)?;\n            self.add_to_index_list(b\"module_index\", module_key)?;\n        }\n\n        for (address, tag, bytes) in &changeset.resource_writes {\n            let resource_key = format!(\"resource:{}:{}\", address.to_hex_literal(), tag);\n            match bytes {\n                Some(value) => self.save_internal(resource_key.as_bytes(), value)?,\n                None => self.delete_internal(resource_key.as_bytes()),\n            }\n        }\n\n        for (address, change) in &changeset.account_changes {\n""",
)

print("patched scheduler and canonical state writes")
