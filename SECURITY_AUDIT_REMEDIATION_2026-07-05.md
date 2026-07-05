# Security Audit Remediation — 2026-07-05

Target branch: `kanari-sdk`  
Remediation branch: `security/audit-remediation-2026-07-05`  
Source audit: `kanari-sdk-full-security-audit-2026-06-20.md`

## Scope of this branch

This branch contains the first fail-closed remediation set for findings that can be fixed without introducing a new checkpoint certificate format, database transaction protocol, or consensus migration.

It must not be interpreted as approval for mainnet deployment. Findings marked **Open** remain release blockers.

## Implemented

| Finding | Status | Remediation |
|---|---|---|
| C-03 | Implemented | Canonical state-root selection uses `module_index` as the allowlist for published module bytecode (`module:*`) and commits Move resources (`resource:*`). Indexed bytecode/resource mutations change the root; orphan runtime-local module blobs remain excluded. |
| C-05 | Implemented | Removed the public unrestricted mutable object loader from the Move framework. First-party DEX and escrow entry functions now receive mutable objects as transaction inputs, allowing the runtime to perform ownership/shared-object authorization before execution. Public pools and role-gated escrow state are explicitly stored as shared objects. |
| C-06 | Fail-closed | Disabled public Move view execution until visibility checks, a view allowlist, deterministic metering, read/return limits, timeouts, concurrency controls, and rate limits are implemented. |
| H-04 | Fail-safe | Runtime v1 transaction scheduling now emits one transaction per wave in canonical input order. Heuristic parallel execution remains disabled until deterministic read/write-set discovery and revalidation cover all Move/native effects. |
| H-05 | Implemented | Mempool capacity, duplicate-hash, and sender-sequence checks now run in the same write-locked admission critical section as insertion. |
| H-07 | Implemented | Checkpoint hashes are domain-separated and commit to ordered vertices and timestamp in addition to sequence, transaction hashes, state root, and previous checkpoint hash. |

## Verified in the current base branch

| Finding | Current observation |
|---|---|
| C-02 | The current source already signs a domain-separated `signing_digest` covering the Mysticeti identifier and full Kanari vertex body, and network validation verifies that digest. A future protocol cleanup should still store the Mysticeti reference separately from the canonical Kanari content digest. |
| H-08 | P2P gzip decompression already uses a hard 8 MiB uncompressed-output limit. |

## Open release blockers

The following findings require architectural work and are not claimed as closed by this branch:

- **C-01:** checkpoint quorum certificate / aggregate validator authorization.
- **C-04:** speculative verification and persisted execution must become one deterministic state transition.
- **H-01:** checkpoint, state, SMT, transaction indexes, object writes, and DAG metadata need one atomic database commit or recovery journal.
- **H-02:** object persistence APIs must return and propagate errors and participate in the atomic commit.
- **H-03:** production storage/bootstrap paths must fail closed on corruption.
- **H-06:** enforce protocol compute, native, byte, storage, and block budgets even when user-visible token fees are zero.
- **H-09 and remaining High/Medium findings:** require separate bounded-channel, RPC hardening, key-management, KDF, arithmetic, and persistence patches.

## Compatibility notes

The DEX and escrow entry-function ABIs changed from raw object-address arguments to object-reference inputs. Clients must continue sending object IDs in transaction arguments; the runtime resolves those IDs to the declared reference types and performs authorization before Move execution.

The public view RPC now returns an explicit disabled error rather than executing arbitrary Move code.

The canonical state-root key set now includes indexed Move module bytecode and Move resources. Existing databases must rebuild the SMT or use a fresh testnet database before comparing roots produced by this branch.

Runtime v1 checkpoint execution is intentionally serial. This is a security/throughput trade-off until complete conflict analysis and deterministic revalidation are implemented.

The checkpoint hash format changed to `kanari:checkpoint:v2`; persisted or network checkpoints produced with the prior hash format require an explicit migration/testnet reset policy.

## Required validation before merge

- Build the Rust workspace.
- Compile and test the Kanari framework Move packages.
- Compile and test the DEX and escrow packages against the new ABIs.
- Add concurrent mempool admission tests for duplicate hashes and same-sender sequences.
- Add checkpoint hash mutation tests for timestamp and vertex-order changes.
- Run state-root mutation tests for indexed module bytecode and Move resources.
- Run multi-node checkpoint production/sync tests on a disposable database.
- Do not merge to a production network until all Critical findings are closed and independently re-audited.
