# Kanari Network

## Technical Whitepaper

**Version 2.0 — 12 August 2026**

### Abstract

Kanari is an object-centric programmable payment network implemented in Rust and Move. It combines Mysticeti-style DAG consensus, deterministic Move execution, explicit object ownership, incremental Sparse Merkle Tree roots, RocksDB persistence, and cryptographic agility including post-quantum and hybrid signatures.

This whitepaper documents the implemented design and measured engineering behavior. Performance is workload-dependent; no TPS, latency, or finality number is a protocol guarantee.

### Principles

1. Consensus metadata and committed state are separate layers.
2. Checkpoints advance only for real committed work.
3. State roots and native supply must converge after recovery.
4. Runtime policy and Move contract authorization are separate controls.
5. Security and performance claims require reproducible tests and metrics.

## 1.1 Scope and terminology

In this paper, *submitted* means accepted by an API, *executed* means evaluated by Move, and *committed* means that effects passed validation and were durably applied to canonical state and a checkpoint. Only committed state is externally final. A valid signature therefore does not guarantee execution success.

Kanari is not presented as a permissionless deployment by default. Validator membership, genesis material, peer identity, upgrade policy, and key custody remain deployment decisions. The implementation provides audit points for those decisions; it does not remove operational risk.

## 1.2 Design trade-offs

Object parallelism helps independent transactions but hot shared objects create a dependency lane. Post-quantum signatures improve long-term cryptographic posture at the cost of larger keys, signatures, CPU, and bandwidth. Persistent durability improves recovery guarantees at the cost of write amplification and compaction. These trade-offs are reported explicitly rather than hidden behind one TPS number.

## 1.3 Versioning

State schema, transaction, signature, and backup formats require explicit version identifiers. A node must reject an incompatible format or run a tested migration; it must not guess a legacy interpretation.
