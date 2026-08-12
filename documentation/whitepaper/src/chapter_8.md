# 8. Verification, Claims, and Reproducibility

## 8.1 What this document claims

This paper is an engineering specification for the implementation in this repository, not a proof that every deployment is secure. Statements are classified as:

| Class | Meaning |
| --- | --- |
| Implemented | Visible in the current source tree and covered by an automated test or invariant check. |
| Measured | Observed in a named benchmark campaign with workload, backend, and environment recorded. |
| Operational requirement | Required from an operator but not enforced by every binary. |
| Research item | A proposed improvement or an uncompleted validation. |

The distinction prevents a benchmark from being mistaken for a protocol guarantee and prevents a library choice from being mistaken for a security certification.

## 8.2 Reproducible experiment record

Every performance or failure campaign should record the commit identifier, operating system, CPU model and core count, RAM, storage medium, Rust toolchain, build profile, validator count, RPC/P2P ports, transaction mix, sender count, object fanout, gas policy, duration, failures, peak memory, and final root/supply audit. Setup work (wallet derivation, faucet or native fanout, and database creation) must be reported separately from execution throughput.

The repository's scripts under `scripts/` are the reference harness. They emit logs and JSON summaries so another operator can distinguish a transaction failure, a node failure, a synchronization delay, and a setup bottleneck. A result without these fields is informative only, not a capacity claim.

## 8.3 Current verification evidence

The latest engineering campaign recorded:

* SMT: 24 tests passed, including parallel and property-oriented cases.
* RPC server: 37 tests passed, including malformed input, object references, and gas/object overlap.
* Move runtime/state: more than 120 unit and persistence tests passed.
* Four-node chaos: duplicate publishes, 200 ms delay, two-node crash/restart, recovery audit, root convergence, supply convergence, and adversarial RPC probes passed.
* Persistent four-node profile: 100/100 transactions succeeded at approximately 47 aggregate lane TPS on the tested Windows host.
* In-memory owned-object benchmark: approximately 13K TPS for the stated deterministic workload; this is not persistent-network TPS.

These results are evidence for the tested scenarios only. They do not establish a universal TPS, latency, validator count, or Byzantine tolerance beyond the configured protocol assumptions.

## 8.4 Safety and liveness obligations

For every committed checkpoint, validators must agree on the checkpoint height, transaction effects, canonical state root, object versions, and native supply summary. For every rejected transaction, no partial canonical mutation may remain. For every restart, recovery must either restore the last complete checkpoint or fail closed; it must not silently invent objects, supply, or ownership.

Liveness is conditional on network synchrony, sufficient honest validators, available storage, and a functioning execution queue. A stalled or partitioned node is not evidence of a safety violation, but an operator must alert on prolonged root divergence, synchronization gaps, write stalls, or unbounded pending work.

## 8.5 Security review boundary

The review boundary includes Move authorization, native functions, transaction decoding, signature and nonce checks, object version/digest checks, gas/input overlap, consensus message validation, peer admission, persistence/recovery, key storage, and dependency advisories. PQC implementations are treated as cryptographic dependencies, not as an independent audit of Kanari itself. Public deployments still require key rotation, secret backup, TLS or a trusted reverse proxy, restricted admin endpoints, rate limits, and incident procedures.

## 8.6 Known limits and acceptance gates

Before a production release, the project should complete a multi-hour or multi-day four-node soak with real wallets, crash/restart during persistent load, larger RocksDB compaction/write-stall profiles, nightly fuzzing for BCS/RPC/object authorization/consensus/SMT, and an external review of native/RPC paths. A release may ship with an explicit limitation, but it must not label an unmeasured property as guaranteed.
## 8.7 Current implementation inventory

The implementation described by this paper is distributed across these repository surfaces:

| Surface | Current responsibility |
| --- | --- |
| `crates/kanari-core` | transaction engine, DAG vertex production, checkpoint orchestration, and state application coordination |
| `crates/kanari-node` | validator process, P2P, synchronization, RPC/service wiring, and operational status |
| `move-execution/v1/kanari-move-runtime-v1` | Move VM integration, object policy, gas, changesets, persistent state, and recovery |
| `crates/smt` | sparse Merkle nodes, overlays, incremental root updates, and root verification |
| `crates/kanari-crypto` | key derivation, wallet/keystore primitives, classical/PQC/hybrid signatures, hashing, and encryption |
| `crates/kanari-rpc-server` | JSON/BCS request validation, transaction submission, query, and adversarial input boundaries |
| `crates/kanari-system-natives` | native cryptographic and system calls exposed to Move |
| `scripts/` | four-node launch, chaos, fanout, benchmark, recovery, and audit harnesses |

The source of truth for behavior is the matching code, tests, migration notes, and release commit. This table is an audit map, not a claim that every path is independently certified.
