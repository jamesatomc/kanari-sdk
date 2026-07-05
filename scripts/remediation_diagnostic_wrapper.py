from pathlib import Path
import runpy
import subprocess
import tempfile
import traceback

ROOT = Path(__file__).resolve().parents[1]
SENTINEL = ROOT / "scripts/TRANSFORM_ERROR.txt"


def run_blob(blob_sha: str, display_name: str) -> None:
    if SENTINEL.exists():
        return

    try:
        source = subprocess.check_output(
            ["git", "cat-file", "blob", blob_sha],
            cwd=ROOT,
        )
        with tempfile.NamedTemporaryFile(
            mode="wb",
            suffix=f"-{display_name}.py",
            delete=False,
        ) as file:
            file.write(source)
            implementation_path = Path(file.name)
        runpy.run_path(str(implementation_path), run_name="__main__")
    except BaseException as error:
        details = "".join(
            traceback.format_exception(type(error), error, error.__traceback__)
        )
        SENTINEL.write_text(
            f"failed_script={display_name}\nblob_sha={blob_sha}\n{details}",
            encoding="utf-8",
        )
        # The old workflow packages transformed source before rustfmt. Append an
        # intentional parse error so validation fails after the artifact is uploaded
        # and before any generated source can be committed.
        with (ROOT / "crates/kanari-core/src/lib.rs").open(
            "a", encoding="utf-8"
        ) as file:
            file.write("\nthis_is_an_intentional_transform_diagnostic_failure\n")
        print(details)
