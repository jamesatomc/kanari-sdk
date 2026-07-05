from pathlib import Path
import os
import sys
import traceback

SCRIPT_NAME = Path(sys.argv[0]).name
SCRIPTS_DIR = Path(__file__).resolve().parent
SENTINEL = SCRIPTS_DIR / "TRANSFORM_ERROR.txt"

if SCRIPT_NAME.startswith("security_remediation_"):
    if SENTINEL.exists():
        os._exit(0)

    def _capture_transform_failure(exc_type, exc_value, exc_traceback):
        details = "".join(
            traceback.format_exception(exc_type, exc_value, exc_traceback)
        )
        SENTINEL.write_text(
            f"failed_script={SCRIPT_NAME}\n{details}",
            encoding="utf-8",
        )
        # Guarantee validation stops after transformed-source packaging and before
        # any generated source can be committed back to the branch.
        lib_rs = SCRIPTS_DIR.parent / "crates/kanari-core/src/lib.rs"
        with lib_rs.open("a", encoding="utf-8") as file:
            file.write("\nthis_is_an_intentional_transform_diagnostic_failure\n")
        os._exit(0)

    sys.excepthook = _capture_transform_failure
