# Kanari Move Runtime v1: Object and State Management

This document describes the current implementation in this crate. It replaces the older “object management improvements” note, which described an intermediate preload/borrow design and contained stale test counts and method locations.

## Scope

The runtime has four distinct responsibilities:

1. Execute Move bytecode and collect deterministic effects.
2. Validate object references, ownership, versions, gas, and supply effects.
3. Convert effects into a canonical `ChangeSet` and deterministic access manifest.
4. Apply the accepted changeset to the state overlay, SMT, indexes, and persistent store.

Object loading, Move VM resolution, object policy, state application, and persistence are related but are not the same layer.

## Current execution flow

```text
Signed transaction
  -> MoveRuntime session and resolver
  -> Move VM execution
  -> native object/gas/event extensions
  -> ChangeSet (objects, modules, balances, fields, events, gas)
  -> deterministic StateAccessSet
  -> scheduler conflict decision
  -> StateManager validation
  -> apply changeset and supply checks
  -> incremental SMT/root update
  -> checkpoint/persistent commit
```

`ChangeSet` is the canonical boundary between VM execution and state mutation. A VM success is not a commit: the state manager still validates references, duplicate mutable inputs, ownership, gas overlap, arithmetic, and persistence invariants.

## Object representation

Objects created by Move execution are represented by `CreatedObject` and may carry:

- owner and `ObjectOwnerKind`;
- optional `UIDRecord` for UID-shaped resources;
- optional `IDRecord` for copyable IDs used by DeFi protocols;
- Move type string;
- serialized object data;
- object version and derived digest/object reference.

Deleted objects, explicit object changes, dynamic-field additions/removals, shared inputs, immutable inputs, gas references, and ordinary input objects are represented separately in the changeset. This separation prevents an index/query projection from being confused with canonical object state.

## Ownership and authorization

The runtime enforces infrastructure-level object safety: object references must be valid, versions/digests must match, mutable inputs must not be duplicated, and native KANARI transfer inputs cannot overlap the gas object. Coin ownership is checked by the runtime policy.

Generic DeFi objects may be passed according to the protocol's object policy. That does not grant business authority. Move entry functions must independently check buyer, seller, admin, owner, lifecycle state, amount, and replay conditions. Escrow modules must test create, delivery, release, dispute, refund, and unauthorized cross-role calls.

## Deterministic access scheduling

`StateAccessSet` contains deterministic byte keys for reads and writes. It is derived from resolver reads and canonical effects, including objects, owner state, Move writes, dynamic-field fences, gas, and other state keys.

Two transactions may execute in parallel only when no write/read or write/write conflict exists:

```text
Mutable(T_i) INTERSECT Mutable(T_j) = EMPTY_SET
```

The manifest is intentionally conservative. An unknown or unclassified write is fenced rather than treated as independent. This protects deterministic replay at the cost of possible parallelism.

## ChangeSet and failure atomicity

The current `ChangeSet` carries owner deltas, native gas credits, events, treasury/NFT capability updates, absolute token balance sets, input/shared/immutable/gas references, created/deleted objects, explicit object changes, dynamic fields, Move writes, resolver-read metadata, gas usage, success, and an optional error.

State application is staged. Validation must complete before canonical state is exposed. Failed execution or failed validation must not leave partial object writes, owner indexes, supply updates, or state-root changes.

## State root and persistence

The state manager maintains an overlay for speculative/buffered writes and applies canonical updates through the persistent store. The SMT update is incremental: changed leaves and affected paths are batched, then the effective root is checked before commit. Full root audits remain available for verification and recovery.

The runtime persists schema/version markers, object/index state, supply metadata, and checkpoint-related data through `PersistentStore`/RocksDB. Recovery compares checkpoint height, canonical root, object/index consistency, and native supply. Derived caches may be rebuilt; they must not silently redefine canonical state.

## Supply invariant

Native operations must preserve:

```text
total_supply
  = circulating_supply
  + object_locked_supply
  + untracked_supply
```

Correctly tracked mint, burn, transfer, split, merge, gas, and escrow operations finish with `untracked_supply = 0`. Overflow, underflow, stale object state, malformed indexes, or inconsistent treasury data fail closed.

## Gas

`KanariGasMeter` charges runtime work such as bytecode, serialization, vectors, native calls, and value-size-dependent operations. The meter is a computation/DoS boundary; payment-object selection and native supply accounting are validated separately. Gas price or zero-price development configuration does not remove execution, storage, or recovery cost.

## Testing map

Tests are organized by behavior rather than by the former preload implementation:

- `tests/unit/state_tests.rs`: state application, roots, supply, and recovery behavior;
- `tests/unit/changeset_tests.rs`: changeset and access-manifest behavior;
- `tests/unit/object_ops_tests.rs`: object operation and policy cases;
- `tests/unit/object_storage_tests.rs`: object persistence and lookup;
- `tests/unit/scheduler_tests.rs`: deterministic conflict scheduling;
- `tests/state_persistence.rs`: persistent state/restart behavior;
- `tests/state_root_module_commitment.rs`: root/module commitment behavior;
- `tests/repro_inflation.rs` and `tests/test_mint_consolidation.rs`: supply regressions;
- dynamic-field/object-field and publish/upgrade tests: cross-transaction and module lifecycle behavior.

Run the crate's current tests instead of relying on a hard-coded historical test count:

```text
cargo test -p kanari-move-runtime-v1 --tests -- --test-threads=2
```

## Known boundaries

The runtime cannot infer application-level authorization for every arbitrary Move type. Streaming backup encryption, long-running fuzzing, multi-day persistent chaos, and independent security review remain operational/research work. Performance depends on object contention, signature scheme, storage backend, checkpoint policy, and workload shape.
