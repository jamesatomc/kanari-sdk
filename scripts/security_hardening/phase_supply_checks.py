from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "move-execution/v1/kanari-move-runtime-v1/src/state.rs"
    text = read(path)
    before = '''        if validate_supply && let Err(e) = self.validate_supply_invariants() {
            Self::report_supply_invariant_violation("after apply_changeset", &e);
        }

        Ok(())'''
    after = '''        if validate_supply {
            if let Err(error) = self.validate_supply_invariants() {
                Self::report_supply_invariant_violation("after apply_changeset", &error);
                return Err(error).context("Supply invariant failed after applying changeset");
            }
        }

        Ok(())'''
    if before not in text:
        raise RuntimeError("supply validation block not found")
    write(path, text.replace(before, after, 1))
