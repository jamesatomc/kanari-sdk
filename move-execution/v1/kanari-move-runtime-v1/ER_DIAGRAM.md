# ER Diagram — kanari-move-runtime

This document shows an Entity-Relationship (ER) style diagram describing the main components and data flows inside the `kanari-move-runtime` crate.

The diagram below uses Mermaid's `erDiagram` syntax — GitHub and many editors render this automatically.

```mermaid
erDiagram
    %% Core Execution Flow
    SignedTransaction ||--o{ Transaction : contains
    Transaction }o--|| MoveRuntime : executes
    MoveRuntime ||--|| ChangeSet : produces
    ChangeSet ||--o{ CreatedObject : creates
    ChangeSet ||--o{ Event : emits
    ChangeSet ||--o{ AccountChange : modifies
    
    %% State Management
    StateManager ||--|| PersistentStore : persists_to
    StateManager ||--o{ Account : manages
    Account ||--o{ StoredObject : owns
    StoredObject }o--|| ObjectStorage : stored_in
    
    %% Move VM Integration
    MoveRuntime ||--|| MoveVM : wraps
    MoveRuntime ||--|| KanariMoveResolver : uses
    MoveRuntime ||--|| KanariGasMeter : charges
    MoveRuntime ||--|| NativeFunctionTable : integrates
    
    %% Storage Layer
    ObjectStorage ||--|| PersistentStore : backed_by
    PersistentStore ||--|| RocksDB : implements
    ObjectStorage ||--o{ StoredObject : contains
    MoveVMState ||--|| PersistentStore : persists_to
    
    %% Runtime Extensions
    MoveRuntime ||--|| MoveRuntimeExtensions : extends
    MoveRuntimeExtensions ||--|| parsers : provides
    MoveRuntimeExtensions ||--|| gas_ops : provides
    MoveRuntimeExtensions ||--|| object_ops : provides
    
    %% Parallel Execution
    BlockchainEngine ||--o{ MoveRuntime : pools
    BlockchainEngine ||--|| TransactionScheduler : schedules
    BlockchainEngine ||--|| StateManager : manages
    
    %% Relationships Notes
    CreatedObject }o--|| StoredObject : converts_to
    AccountChange }o--|| Account : applies_to
    Event }o--|| Account : emitted_by
```

## Detailed Component Relationships

### 1. Transaction Execution Pipeline

```mermaid
sequenceDiagram
    participant ST as SignedTransaction
    participant BE as BlockchainEngine
    participant TS as TransactionScheduler
    participant MR as MoveRuntime
    participant VM as MoveVM
    participant CS as ChangeSet
    participant SM as StateManager
    participant PS as PersistentStore

    ST->>BE: submit_transaction()
    BE->>TS: schedule for parallel execution
    TS->>MR: execute_transaction(tx)
    MR->>VM: invoke Move bytecode
    VM-->>MR: VM ChangeSet + events
    MR->>MR: parse_move_changeset()
    MR-->>CS: return kanari ChangeSet
    CS->>SM: apply_changeset()
    SM->>PS: persist state changes
    PS-->>SM: ack persistence
    SM-->>BE: execution complete
```

### 2. Storage Architecture

```mermaid
graph TD
    A[StateManager] --> B[PersistentStore]
    A --> C[ObjectStorage]
    B --> D[RocksDB Backend]
    C --> B
    C --> E[StoredObject Cache]
    B --> F[In-Memory Fallback]
    
    G[MoveVMState] --> B
    H[CreatedObject] --> C
    I[Account] --> B
```

## Types → Source Mapping (with line links)

### Core Execution Types

- **MoveRuntime**: [src/move_runtime/mod.rs#L33](src/move_runtime/mod.rs#L33)
- **SignedTransaction**: [crates/kanari-types/src/transaction.rs](../../crates/kanari-types/src/transaction.rs)
- **Transaction**: [crates/kanari-types/src/transaction.rs](../../crates/kanari-types/src/transaction.rs)
- **ChangeSet**: [src/changeset.rs#L67](src/changeset.rs#L67)
- **CreatedObject**: [src/changeset.rs#L14](src/changeset.rs#L14)
- **Event**: [crates/kanari-types/src/event.rs](../../crates/kanari-types/src/event.rs)
- **AccountChange**: [src/changeset.rs#L26](src/changeset.rs#L26)

### State Management

- **StateManager**: [src/state.rs#L75](src/state.rs#L75)
- **Account**: [src/state.rs#L28](src/state.rs#L28)
- **PersistentStore**: [src/storage/persistent_store.rs#L69](src/storage/persistent_store.rs#L69)
- **ObjectStorage**: [src/storage/object_storage.rs#L106](src/storage/object_storage.rs#L106)
- **StoredObject**: [src/storage/object_storage.rs#L127](src/storage/object_storage.rs#L127)
- **MoveVMState**: [src/storage/move_vm_state.rs#L18](src/storage/move_vm_state.rs#L18)

### Gas & Metering

- **KanariGasMeter**: [src/kanari_gas_meter.rs#L9](src/kanari_gas_meter.rs#L9)
- **GasOperation**: [crates/kanari-types/src/gas.rs](../../crates/kanari-types/src/gas.rs)

### Runtime Extensions

- **MoveRuntimeExtensions**: [src/move_runtime/move_runtime_extensions.rs#L17](src/move_runtime/move_runtime_extensions.rs#L17)
- **parsers** module: [src/move_runtime/parsers.rs#L11](src/move_runtime/parsers.rs#L11)
- **gas_ops** module: [src/move_runtime/gas_ops.rs#L14](src/move_runtime/gas_ops.rs#L14)
- **object_ops** module: [src/move_runtime/object_ops.rs#L11](src/move_runtime/object_ops.rs#L11)
- **RuntimeStats**: [src/move_runtime/move_runtime_extensions.rs#L172](src/move_runtime/move_runtime_extensions.rs#L172)

### Engine & Scheduling

- **BlockchainEngine**: [crates/kanari-core/src/engine.rs#L42](../../crates/kanari-core/src/engine.rs#L42)
- **TransactionScheduler**: [src/scheduler.rs#L12](src/scheduler.rs#L12)
- **KanariMoveResolver**: [src/storage/resolver.rs#L10](src/storage/resolver.rs#L10)

### Key Method Entry Points

#### MoveRuntime Methods

- `new_with_kanari_natives()`: [src/move_runtime/mod.rs#L111](src/move_runtime/mod.rs#L111)
- `publish_module()`: [src/move_runtime/mod.rs#L148](src/move_runtime/mod.rs#L148)
- `publish_module_bundle()`: [src/move_runtime/mod.rs#L243](src/move_runtime/mod.rs#L243)
- `execute_function()`: [src/move_runtime/mod.rs#L350](src/move_runtime/mod.rs#L350)

#### StateManager Methods

- `get_or_create_account()`: [src/state.rs#L125](src/state.rs#L125)
- `apply_changeset()`: [src/state.rs#L200](src/state.rs#L200)
- `compute_state_root()`: [src/state.rs#L450](src/state.rs#L450)
- `commit()`: [src/state.rs#L500](src/state.rs#L500)

#### Storage Methods

- `PersistentStore::open_default()`: [src/storage/persistent_store.rs#L76](src/storage/persistent_store.rs#L76)
- `ObjectStorage::store_object()`: [src/storage/object_storage.rs#L148](src/storage/object_storage.rs#L148)
- `ObjectStorage::get_object()`: [src/storage/object_storage.rs#L198](src/storage/object_storage.rs#L198)
- `ObjectStorage::get_objects_by_owner()`: [src/storage/object_storage.rs#L224](src/storage/object_storage.rs#L224)
- `MoveVMState::save_module()`: [src/storage/move_vm_state.rs#L59](src/storage/move_vm_state.rs#L59)

#### Parsing & Helpers

- `parse_move_changeset()`: [src/move_runtime/parsers.rs#L11](src/move_runtime/parsers.rs#L11)
- `parse_move_events()`: [src/move_runtime/parsers.rs#L142](src/move_runtime/parsers.rs#L142)
- `apply_gas_info()`: [src/move_runtime/gas_ops.rs#L14](src/move_runtime/gas_ops.rs#L14)

## Architecture Notes

### Design Principles

1. **Separation of Concerns**:
   - `MoveRuntime` handles VM execution logic
   - `StateManager` manages application-level state
   - `PersistentStore` provides durable storage abstraction
   - `ObjectStorage` handles Move objects specifically

2. **Parallel Execution Support**:
   - `TransactionScheduler` enables optimistic parallel execution
   - `BlockchainEngine` maintains a pool of `MoveRuntime` instances
   - Conflict detection prevents race conditions

3. **Storage Flexibility**:
   - RocksDB for production persistence
   - In-memory store for testing/Miri
   - Overlay mechanism for speculative execution

4. **Gas Metering**:
   - `KanariGasMeter` acts as step counter (not coin deduction)
   - Prevents DoS attacks and infinite loops
   - Zero-cost transactions (gas paid by infrastructure)

### Data Flow Summary

1. **Transaction Submission**: `SignedTransaction` → `BlockchainEngine` → `TransactionScheduler`
2. **Execution**: `MoveRuntime` → `MoveVM` → `ChangeSet`
3. **State Update**: `ChangeSet` → `StateManager` → `PersistentStore` / `ObjectStorage`
4. **Persistence**: All state changes committed to RocksDB via `PersistentStore`

### Next Steps (Optional Enhancements)

- Add performance metrics tracking for each component
- Document conflict resolution strategies in `TransactionScheduler`
- Expand on the overlay mechanism for speculative execution
- Add detailed error handling flow diagrams
