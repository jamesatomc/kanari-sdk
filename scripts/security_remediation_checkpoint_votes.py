#!/usr/bin/env python3
from pathlib import Path
import runpy

ROOT = Path(__file__).resolve().parents[1]
P2P_RS = ROOT / "crates/kanari-node/src/p2p.rs"

if "fn claimed_identity_matches" not in P2P_RS.read_text(encoding="utf-8"):
    runpy.run_path(
        str(ROOT / "scripts/security_remediation_node.py"),
        run_name="__main__",
    )

runpy.run_path(
    str(ROOT / "scripts/security_remediation_checkpoint_votes_v2.py"),
    run_name="__main__",
)
