from __future__ import annotations

import re
from .common import read, write


def apply() -> None:
    path = "move-execution/v1/kanari-move-runtime-v1/src/state.rs"
    text = read(path)
    text = text.replace(
        "let use_incremental_smt = if self.store.get_db().is_some() {\n                true",
        "let use_incremental_smt = if self.store.get_db().is_some() {\n                !self.smt_dirty",
        1,
    )
    write(path, text)
