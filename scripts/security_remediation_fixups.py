#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def rewrite(path: str, old: str, new: str) -> None:
    file = ROOT / path
    text = file.read_text(encoding="utf-8")
    if new in text:
        return
    if old not in text:
        raise RuntimeError(f"missing fixup pattern in {path}: {old}")
    file.write_text(text.replace(old, new), encoding="utf-8")


rewrite(
    "crates/kanari-core/src/engine.rs",
    "bcs::serialized_size(signed)? as usize",
    "bcs::to_bytes(signed)?.len()",
)

print("security remediation fixups applied or already present")
