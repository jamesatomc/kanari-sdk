from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "move-execution/v1/kanari-move-runtime-v1/src/changeset.rs"
    text = read(path)
    text = text.replace(
        "    pub token_balance_sets: Vec<(AccountAddress, String, BalanceRecord)>,",
        "    pub token_balance_sets: Vec<(AccountAddress, String, BalanceRecord)>,\n"
        "    pub move_modules: Vec<(AccountAddress, String, Option<Vec<u8>>)>,\n"
        "    pub move_resources: Vec<(AccountAddress, String, Option<Vec<u8>>)>,",
        1,
    )
    text = text.replace(
        "            token_balance_sets: Vec::new(),\n            created_objects: Vec::new(),",
        "            token_balance_sets: Vec::new(),\n"
        "            move_modules: Vec::new(),\n"
        "            move_resources: Vec::new(),\n"
        "            created_objects: Vec::new(),",
        1,
    )
    text = text.replace(
        "            && self.token_balance_sets.is_empty()\n            && self.created_objects.is_empty()",
        "            && self.token_balance_sets.is_empty()\n"
        "            && self.move_modules.is_empty()\n"
        "            && self.move_resources.is_empty()\n"
        "            && self.created_objects.is_empty()",
        1,
    )
    text = text.replace(
        "        self.created_objects.extend(other.created_objects);",
        "        self.move_modules.append(&mut other.move_modules);\n"
        "        self.move_resources.append(&mut other.move_resources);\n"
        "        self.created_objects.extend(other.created_objects);",
        1,
    )
    marker = "    pub fn add_event(&mut self, event: Event) {\n        self.events.push(event);\n    }"
    methods = '''    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn write_move_module(&mut self, address: AccountAddress, name: String, bytes: Vec<u8>) {
        self.move_modules.push((address, name, Some(bytes)));
    }

    pub fn delete_move_module(&mut self, address: AccountAddress, name: String) {
        self.move_modules.push((address, name, None));
    }

    pub fn write_move_resource(&mut self, address: AccountAddress, tag: String, bytes: Vec<u8>) {
        self.move_resources.push((address, tag, Some(bytes)));
    }

    pub fn delete_move_resource(&mut self, address: AccountAddress, tag: String) {
        self.move_resources.push((address, tag, None));
    }'''
    if marker not in text:
        raise RuntimeError("ChangeSet insertion point not found")
    write(path, text.replace(marker, methods, 1))
