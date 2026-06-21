#!/usr/bin/env python3
from pathlib import Path

path = Path('move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs')
text = path.read_text(encoding='utf-8')
anchor = 'mod parsers;\n'
if text.count(anchor) != 1:
    raise SystemExit('module anchor mismatch')
path.write_text(text.replace(anchor, anchor + 'mod safe_view;\n'), encoding='utf-8')
