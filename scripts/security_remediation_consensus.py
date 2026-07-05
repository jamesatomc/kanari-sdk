#!/usr/bin/env python3
from pathlib import Path
import runpy
import traceback

ROOT = Path(__file__).resolve().parents[1]
ERROR_FILE = ROOT / "crates/kanari-core/TRANSFORM_ERROR.txt"

if not ERROR_FILE.exists():
    try:
        runpy.run_path(
            str(ROOT / "scripts/security_remediation_consensus_impl.py"),
            run_name="__main__",
        )
    except Exception as error:
        ERROR_FILE.write_text(
            "failed_script=security_remediation_consensus\n"
            + "".join(
                traceback.format_exception(type(error), error, error.__traceback__)
            ),
            encoding="utf-8",
        )
        with (ROOT / "crates/kanari-core/src/lib.rs").open("a", encoding="utf-8") as file:
            file.write('\ncompile_error!("security remediation transformer failed");\n')
