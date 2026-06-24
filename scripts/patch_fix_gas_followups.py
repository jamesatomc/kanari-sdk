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


replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/genesis.rs",
    "runtime.persist_created_objects(&init_changeset);",
    "runtime.persist_created_objects(&init_changeset)?;",
)
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/genesis.rs",
    "runtime.persist_deleted_objects(&init_changeset);",
    "runtime.persist_deleted_objects(&init_changeset)?;",
)

replace_once(
    "crates/kanari-core/src/engine/queries.rs",
    """        let (computed_root, verified_state, to_execute, receipts) =\n            self.prepare_checkpoint_state(&checkpoint_to_apply)?;\n""",
    """        let (computed_root, verified_state, receipts) =\n            self.prepare_checkpoint_state(&checkpoint_to_apply)?;\n""",
)
replace_once(
    "crates/kanari-core/src/engine/queries.rs",
    """        self.apply_prepared_checkpoint(\n            checkpoint_to_apply,\n            verified_state,\n            to_execute,\n            receipts,\n            true,\n        )?;\n""",
    """        self.apply_prepared_checkpoint(\n            checkpoint_to_apply,\n            verified_state,\n            receipts,\n            true,\n        )?;\n""",
)

print("patched checkpoint sync and genesis persistence call sites")
