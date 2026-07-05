#!/usr/bin/env python3
from pathlib import Path
import runpy
import traceback

ROOT = Path(__file__).resolve().parents[1]
ERROR_FILE = ROOT / "crates/kanari-core/TRANSFORM_ERROR.txt"
SESSION_RS = ROOT / "crates/kanari-auth/src/session.rs"

if not ERROR_FILE.exists():
    session_source = SESSION_RS.read_text(encoding="utf-8")
    old_initializer = """            wallet_address,
            private_key,
            curve_type,"""
    new_initializer = """            wallet_address,
            private_key: private_key.map(Zeroizing::new),
            curve_type,"""
    if new_initializer not in session_source:
        if session_source.count(old_initializer) != 1:
            raise RuntimeError(
                "Session private-key initializer is missing or ambiguous"
            )
        SESSION_RS.write_text(
            session_source.replace(old_initializer, new_initializer, 1),
            encoding="utf-8",
        )

    try:
        runpy.run_path(
            str(ROOT / "scripts/security_remediation_auth_impl.py"),
            run_name="__main__",
        )
    except Exception as error:
        ERROR_FILE.write_text(
            "failed_script=security_remediation_auth\n"
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
