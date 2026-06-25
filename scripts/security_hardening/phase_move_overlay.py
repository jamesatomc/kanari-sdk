from .common import read, write


def apply():
    path = "move-execution/v1/kanari-move-runtime-v1/src/state.rs"
    text = read(path)
    marker = "        self.add_many_to_index_list(ACCOUNT_INDEX_KEY, account_index_additions)?;\n\n        // Update total supply"
    replacement = '''        self.add_many_to_index_list(ACCOUNT_INDEX_KEY, account_index_additions)?;

        for (address, name, bytes) in &changeset.move_modules {
            let key = format!("module:{}:{}", address.to_hex_literal(), name);
            if let Some(bytes) = bytes {
                self.save_internal(key.as_bytes(), bytes)?;
                self.add_to_index_list(b"module_index", key)?;
            } else {
                self.delete_internal(key.as_bytes());
                self.remove_from_index_list(b"module_index", &key)?;
            }
        }
        for (address, tag, bytes) in &changeset.move_resources {
            let key = format!("resource:{}:{}", address.to_hex_literal(), tag);
            if let Some(bytes) = bytes {
                self.save_internal(key.as_bytes(), bytes)?;
            } else {
                self.delete_internal(key.as_bytes());
            }
        }

        // Update total supply'''
    if marker not in text:
        raise RuntimeError("state overlay marker not found")
    write(path, text.replace(marker, replacement, 1))
