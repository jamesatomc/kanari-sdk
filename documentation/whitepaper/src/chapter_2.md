# 2. Architecture and Consensus

## 2.1 DAG and authority flow

Authorities receive signed transactions, publish DAG vertices, exchange dependency references, and feed ordered work into execution. A vertex is consensus metadata; it is not itself a committed state transition.

The pipeline is: receive and validate; propagate and order; schedule object access sets; execute deterministic effects; apply and persist the changeset; then advance the checkpoint.

## 2.2 Safety invariants

- duplicate vertices cannot execute twice;
- missing rounds trigger synchronization;
- DAG traffic cannot create empty checkpoints;
- a root mismatch at equal height is an observable divergence;
- honest nodes converge on checkpoint height, root, and supply.

## 2.3 Fault model

The design covers delayed, duplicated, reordered, and missing P2P messages, follower or leader termination, and multi-node restart. Byzantine safety still depends on an honest-validator quorum and deterministic execution.

## 2.4 Components

`kanari-core` orchestrates execution and checkpoints. `kanari-node` provides startup, P2P, synchronization, and service wiring. `kanari-rpc-server` exposes validated APIs. `kanari-move-runtime-v1` applies Move state. `crates/smt` maintains canonical sparse roots.

## 2.5 Protocol phases

The normal path is: client signing; RPC decoding and admission; pending-queue intake; DAG proposal and synchronization; consensus ordering; Move execution; changeset validation and application; durable checkpoint persistence; and committed-result queries. Each phase has a separate error boundary and should expose latency and failure metrics.

## 2.6 Byzantine and failure assumptions

Safety depends on the configured Mysticeti-style quorum and deterministic commit rules. Liveness additionally depends on network delivery, available storage, and responsive authorities. A malicious validator may send conflicting or malformed messages, so author, round, parents, signatures, identity, and replay status must be checked before a vertex is injected.

## 2.7 Limits of the claim

A DAG does not automatically prove fairness, censorship resistance, or a particular finality time. Those properties require protocol-specific proofs and measurements. Kanari reports convergence evidence and treats latency and fairness improvements as engineering goals unless formally specified.

## 2.8 Consensus safety model

For an authority set of size `n` and Byzantine bound `f`, the conventional quorum condition is:

`n >= 3f + 1` and `q = 2f + 1`

Here `q` is the minimum commit support under the configured BFT protocol. This describes the deployment assumption; the exact commit rule remains defined by the implementation and protocol configuration.
