# Kanari Node Indexer Integration

This document matches the current checkpoint-based sync path in `kanari-node`.

## Overview

`kanari-node` creates an optional SQLite indexer at:

```text
<data_dir>/indexer.db
```

The indexer is initialized in `src/app.rs`, not in `main.rs`. Failure to initialize the indexer is logged and the validator continues without indexing.

## Runtime Wiring

The current constructor order is:

```rust
let sync_manager = Arc::new(SyncManager::new(
    engine.clone(),
    network_tx.clone(),
    peer_id.clone(),
    node_indexer.as_ref().map(|idx| idx.indexer().clone()),
));
```

The underlying `Indexer` is wrapped in `Arc<Mutex<Indexer>>` because its SQLite connection is not `Sync`.

## Checkpoint Indexing Flow

The node syncs `CheckpointSyncData`, which contains:

- the checkpoint;
- an optional checkpoint certificate.

For non-empty inbound checkpoints, the engine requires and verifies the certificate before applying state. Verification covers chain ID, epoch, protocol version, checkpoint/previous hash, state root, committee digest, unique committee signatures and quorum voting power.

After a buffered checkpoint is successfully applied in sequence, `SyncManager` calls the indexer path:

1. request the materialized checkpoint view with `engine.get_full_block(sequence)`;
2. convert the checkpoint-backed view into the block representation expected by `kanari-indexer`;
3. write it through the shared indexer mutex;
4. log any indexing failure without rolling back the already committed checkpoint.

The indexer therefore follows certified checkpoint commit order. It is not an authority for consensus or state-root validation.

## Data Layout

A validator data directory may contain:

```text
<data_dir>/
  indexer.db
  ... blockchain/state/checkpoint/object-store data ...
```

The exact storage subdirectories are internal implementation details. Back up the complete validator data directory while the node is stopped; do not copy only `indexer.db` as a complete validator backup.

## Querying

Indexer queries are currently available through the Rust `kanari-indexer` API. Dedicated indexer JSON-RPC endpoints are not yet part of the node API.

Example:

```rust
use kanari_indexer::{Indexer, IndexerConfig};
use std::path::PathBuf;

let indexer = Indexer::new(IndexerConfig {
    db_path: PathBuf::from("C:/kanari/devnet/node1/indexer.db"),
    in_memory: false,
    batch_size: 100,
})?;

let stats = indexer.get_statistics()?;
println!("{stats}");
```

Open the database only according to SQLite locking rules. Prefer querying a stopped node, a read-only replica, or a supported application API rather than attaching arbitrary writers to the live database.

## Error Handling

Indexer initialization failure:

```text
Failed to initialize indexer: ... Indexing will be disabled.
```

Checkpoint indexing failure:

```text
[INDEXER] Failed to index checkpoint #<sequence>: ...
```

These errors do not make an uncertified checkpoint valid and do not bypass consensus verification. They mean the index may lag the committed chain.

## Recovery

There is currently no `kanari-node reindex` CLI command. Do not document or automate one until it exists.

For a corrupted indexer:

1. stop the validator;
2. take a verified full data-directory backup;
3. preserve the corrupted `indexer.db` for diagnosis;
4. restore a known-good full backup, or rebuild the index through a reviewed recovery tool;
5. start the validator with the same committee manifest and key files;
6. compare height, supply and state root with the committee.

Deleting only `indexer.db` does not currently guarantee automatic reindexing from genesis.

## Monitoring

Use node JSON-RPC for canonical validator health:

- `kanari_health`
- `kanari_getStats`
- `kanari_getNetworkStatus`

The current health response does not expose indexer progress. Add an explicit indexer status API before relying on the indexer for production readiness checks.

## Testing

1. Start a fresh multi-validator committee with `setup-multi-node.ps1`.
2. Submit reviewed transactions.
3. Wait for certified checkpoint convergence.
4. Verify equal height, supply and state root with `monitor-cluster-health.ps1`.
5. Inspect indexer statistics through the Rust API.
6. Restart a follower and verify checkpoint catch-up plus indexer continuation.

The checkpoint/state database is canonical. The indexer is a derived query layer.
