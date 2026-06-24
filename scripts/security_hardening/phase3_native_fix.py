from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "move-execution/v1/kanari-move-runtime-v1/src/kanari_gas_meter.rs"
    text = read(path)
    old = ".saturating_add(amount.get())"
    new = ".saturating_add(u64::from(amount))"
    if text.count(old) != 1:
        raise RuntimeError("native gas amount conversion not found")
    write(path, text.replace(old, new, 1))
