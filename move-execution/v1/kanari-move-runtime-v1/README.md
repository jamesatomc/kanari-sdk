# kanari-move-runtime-v1

`kanari-move-runtime-v1` is Kanari's Move VM execution and canonical-state boundary. It executes Move transactions, collects typed effects, validates object/gas/supply rules, and applies accepted changes to an overlay, incremental SMT root, indexes, and persistent storage.

This README describes the current implementation. For deeper diagrams and object details see:

- [Current architecture and ER diagrams](./ER_DIAGRAM.md)
- [Object and state management](./OBJECT_MANAGEMENT_IMPROVEMENTS.md)

## Responsibilities

The crate is responsible for:

- Move VM sessions and Kanari native extensions;
- entry-function and module publish/upgrade execution;
- object loading, borrowing, saving, deleting, transfer, and dynamic fields;
- deterministic `ChangeSet` construction;
- resolver read tracing and `StateAccessSet` generation;
- gas metering and gas-operation accounting;
- state application, supply validation, and state-root updates;
- RocksDB/in-memory persistence and recovery diagnostics;
- deterministic transaction scheduling waves.

Consensus networking, transaction admission, and validator orchestration live in other crates. This runtime does not decide application-level DeFi roles automatically.

## Public crate surface

```rust
use kanari_move_runtime_v1::{ChangeSet, TransactionScheduler};
use kanari_move_runtime_v1::runtime::MoveRuntime;
```

The crate also exposes `state`, `storage`, and `changeset` modules for integration paths that need state or execution metadata. Prefer the grouped `runtime::MoveRuntime` export for new callers.

## Execution model

```text
signed transaction
  -> MoveRuntime session
  -> resolver and native extensions
  -> Move VM execution
  -> ChangeSet + resolver reads
  -> deterministic StateAccessSet
  -> scheduler conflict wave
  -> StateManager validation
  -> overlay/SMT/index updates
  -> PersistentStore commit
```

A successful VM call is not automatically committed. StateManager validates object references, ownership, versions/digests, duplicate mutable inputs, gas overlap, arithmetic, and supply effects before canonical state changes become visible.

## Objects and authorization

Objects carry owner, owner kind, type, serialized data, version, and a digest/object reference. A `ChangeSet` can describe created/deleted objects, explicit object changes, dynamic-field changes, shared and immutable inputs, gas references, and Move writes.

Native KANARI Coin policy is strict:

- the sender cannot mutate another owner's Coin;
- the mutable transfer Coin and gas Coin must be distinct;
- stale versions/digests and duplicate mutable inputs fail;
- failed validation cannot partially write state.

Generic DeFi objects may be passed according to runtime policy, but Move contracts must check buyer, seller, owner, admin, lifecycle, amount, and replay authorization themselves.

## Deterministic scheduling

`TransactionScheduler` creates bounded speculative waves. The current wave limit is `MAX_SPECULATIVE_WAVE_SIZE = 64`; conflict keys from signed transactions prevent unsafe co-location in the same wave. Resolver traces and canonical effects remain the final safety boundary, and the engine can retry conflicts serially.

For mutable access sets, parallel execution requires:

```text
Mutable(T_i) INTERSECT Mutable(T_j) = EMPTY_SET
```

Unknown or conservative access keys are fenced rather than assumed independent. This favors deterministic replay over optimistic throughput.

## State and supply invariants

Canonical state is applied through `StateManager` and `PersistentStore`. Derived indexes and caches may be rebuilt but must not silently redefine canonical state. Incremental SMT updates must agree with full root recomputation.

Native supply must satisfy:

```text
total_supply
  = circulating_supply
  + object_locked_supply
  + untracked_supply
```

Correctly tracked native operations finish with `untracked_supply = 0`. Mint, burn, transfer, split, merge, gas, escrow lock/release, and recovery paths are covered by state/supply tests and must fail closed on invalid arithmetic or metadata.

## Persistence and recovery

`PersistentStore` abstracts RocksDB and in-memory testing storage. Runtime schema markers, object/index data, supply metadata, checkpoint information, and SMT state are validated during startup and recovery. Recovery audits compare checkpoint height, transaction count, state root, object/index consistency, and supply summary.

Use an isolated store per node and do not share a writable database between processes. Test the actual filesystem, WAL, backup, and power-loss behavior of the deployment platform.

## Gas

`KanariGasMeter` bounds Move computation and charges bytecode, serialization, vectors, native calls, and value-size-dependent work. Gas-object selection and native supply accounting are separate validation concerns. A zero gas price in development does not remove CPU, memory, storage, or recovery cost.

## Tests

Run the current crate tests rather than relying on historical counts:

```text
cargo test -p kanari-move-runtime-v1 --tests -- --test-threads=2
```

Important suites include:

- `tests/unit/state_tests.rs`
- `tests/unit/changeset_tests.rs`
- `tests/unit/object_ops_tests.rs`
- `tests/unit/object_storage_tests.rs`
- `tests/unit/scheduler_tests.rs`
- `tests/state_persistence.rs`
- `tests/state_root_module_commitment.rs`
- `tests/repro_inflation.rs`
- `tests/test_mint_consolidation.rs`
- dynamic-field/object-field cross-transaction tests
- module publish/upgrade tests

For production confidence, combine these tests with four-node chaos, persistent-load recovery, RPC adversarial tests, and nightly fuzzing from the repository harness.

## Known boundaries

The runtime cannot infer business authorization for every arbitrary Move type. Multi-day soak, long-running fuzzing, streaming backup encryption, independent PQC review, and complete native/RPC security review remain broader project gates. Performance depends on object contention, signature choice, storage backend, checkpoint policy, and workload shape.

## License

Copyright (c) KanariNetwork, Inc. Licensed under Apache-2.0.
