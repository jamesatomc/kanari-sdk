from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/parsers.rs"
    text = read(path)
    old = '''            for (module_name, op) in account_changes.modules() {
                if matches!(op, MoveOp::New(_) | MoveOp::Modify(_)) {
                    kanari_cs.publish_module(*addr, module_name.to_string());
                }
            }'''
    new = '''            for (module_name, op) in account_changes.modules() {
                match op {
                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {
                        kanari_cs.publish_module(*addr, module_name.to_string());
                        kanari_cs.write_move_module(*addr, module_name.to_string(), bytes.to_vec());
                    }
                    MoveOp::Delete => {
                        kanari_cs.delete_move_module(*addr, module_name.to_string());
                    }
                }
            }'''
    if old not in text:
        raise RuntimeError("Move module parser block not found")
    text = text.replace(old, new, 1)
    text = text.replace(
        "                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {\n                        // Extract UID",
        "                    MoveOp::New(bytes) | MoveOp::Modify(bytes) => {\n"
        "                        kanari_cs.write_move_resource(*addr, struct_tag.to_string(), bytes.to_vec());\n"
        "                        // Extract UID",
        1,
    )
    old = '''                    MoveOp::Delete => {
                        debug!(
                            "[PARSER] skipping delete without concrete object id: addr={} type={}",
                            addr.to_hex_literal(),
                            struct_tag
                        );
                    }'''
    new = '''                    MoveOp::Delete => {
                        kanari_cs.delete_move_resource(*addr, struct_tag.to_string());
                        debug!(
                            "[PARSER] recorded Move resource deletion: addr={} type={}",
                            addr.to_hex_literal(),
                            struct_tag
                        );
                    }'''
    if old not in text:
        raise RuntimeError("Move resource delete block not found")
    write(path, text.replace(old, new, 1))
