from __future__ import annotations

from .common import read, write


def apply() -> None:
    workflow = ".github/workflows/kanari.yml"
    text = read(workflow)
    text = text.replace("continue-on-error: true", "continue-on-error: false")
    text = text.replace(
        "cargo test --workspace --lib --bins",
        "cargo test --workspace --all-targets --all-features",
    )
    text = text.replace(
        "cargo clippy --workspace --lib --bins",
        "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    )
    write(workflow, text)

    write(
        "SECURITY_RELEASE_FREEZE.md",
        '''# Security release freeze

The `fix-gas` lineage must not be deployed with real assets until the security
hardening PR is merged and its full workspace, integration, multi-node and
failure-injection test suites pass.

Multi-validator checkpoint production is deliberately fail-closed. The current
Kanari-to-Mysticeti adapter does not yet ingest authenticated remote Mysticeti
blocks and therefore cannot produce a real committed sub-DAG certificate. A
single-validator development committee may self-certify because quorum is one;
this exception is not a public-network deployment mode.

Release gates:

1. no account-ledger KANARI transfer shortcut; `Coin<KANARI>` objects are canonical;
2. every non-genesis checkpoint carries a verified quorum certificate;
3. producer and verifier execute the same strict deterministic order;
4. replay markers and account sequence checks are permanent;
5. checkpoint gas schedule digest matches the protocol schedule;
6. transaction, block, P2P and sync-buffer resource limits are enforced;
7. pending checkpoint journals are empty after clean shutdown/restart;
8. custodial secret payloads are Argon2id/AES-GCM v2.
''',
    )
