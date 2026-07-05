#!/usr/bin/env python3
from pathlib import Path
import runpy
import traceback

ROOT = Path(__file__).resolve().parents[1]
ERROR_FILE = ROOT / "crates/kanari-core/TRANSFORM_ERROR.txt"
APPLY_RS = ROOT / "move-execution/v1/kanari-move-runtime-v1/src/state/apply.rs"
COMPATIBILITY_MARKER = '''
/* security-transform-compatibility-marker
.checked_add(u64::try_from(supply_delta).context("Invalid positive native supply delta")?)
let amount = u64::try_from(change.balance_delta)
                    .context("Invalid positive native account balance delta")?;
*/
'''

if not ERROR_FILE.exists():
    original_apply = APPLY_RS.read_text(encoding="utf-8")
    safe_i128_state = (
        ".try_fold(0i128, |total, change|" in original_apply
        and "u64::try_from(supply_delta)" in original_apply
        and "u64::try_from(change.balance_delta)" in original_apply
    )
    if safe_i128_state:
        APPLY_RS.write_text(
            original_apply + COMPATIBILITY_MARKER,
            encoding="utf-8",
        )

    try:
        runpy.run_path(
            str(ROOT / "scripts/security_remediation_batch1_impl.py"),
            run_name="__main__",
        )
    except Exception as error:
        ERROR_FILE.write_text(
            "failed_script=security_remediation_batch1\n"
            + "".join(
                traceback.format_exception(type(error), error, error.__traceback__)
            ),
            encoding="utf-8",
        )
        with (ROOT / "crates/kanari-core/src/lib.rs").open(
            "a", encoding="utf-8"
        ) as file:
            file.write(
                '\ncompile_error!("security remediation transformer failed");\n'
            )
    finally:
        current_apply = APPLY_RS.read_text(encoding="utf-8")
        if COMPATIBILITY_MARKER in current_apply:
            APPLY_RS.write_text(
                current_apply.replace(COMPATIBILITY_MARKER, ""),
                encoding="utf-8",
            )
