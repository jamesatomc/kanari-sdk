# Kanari Indexer Integration with Node

This document describes how the blockchain indexer is integrated with the kanari-node.

## Overview

The kanari-indexer is automatically integrated into the kanari-node to provide real-time indexing of blockchain data as blocks are synced and committed.

## Architecture

```md
kanari-node
├── src/
│   ├── main.rs          # Node entry point, initializes indexer
│   ├── sync.rs          # Block synchronization, triggers indexing
│   └── indexer.rs       # Indexer wrapper and initialization
```

## How It Works

### 1. Initialization

When the node starts, the indexer is automatically initialized in `main.rs`:

```rust
// Initialize blockchain indexer
let indexer = match NodeIndexer::new(data_dir.clone()) {
    Ok(idx) => {
        tracing::info!("Blockchain indexer initialized");
        Some(idx)
    }
    Err(e) => {
        tracing::warn!("Failed to initialize indexer: {}", e);
        None
    }
};
```

The indexer database is created at `<data_dir>/indexer.db`.

### 2. Integration with SyncManager

The indexer is passed to the `SyncManager` during initialization:

```rust
let sync_manager = Arc::new(SyncManager::new(
    engine.clone(),
    network_tx.clone(),
    indexer.clone(),  // Optional Arc<Mutex<Indexer>>
    peer_id.clone(),
));
```

### 3. Automatic Indexing

When a block is successfully synced from the network, it's automatically indexed:

```rust
// In sync.rs - handle_block_response
match self.engine.sync_full_block_from_data(&block) {
    Ok(_) => {
        // Convert FullBlockData to Block
        let kanari_block = kanari_types::block::Block::new(...);
        
        // Index the block
        if let Some(ref indexer) = self.indexer {
            let idx = indexer.lock().map_err(|e| anyhow::anyhow!(
                "failed to acquire indexer lock: {}",
                e
            ))?;
            idx.index_block(&kanari_block)?;
        }
    }
}
```

## Thread Safety

The `Indexer` uses `rusqlite::Connection` which contains `RefCell` and is not `Sync`. To safely share it across threads in the async runtime, we wrap it with `std::sync::Mutex`:

```rust
pub struct NodeIndexer {
    indexer: Arc<Mutex<Indexer>>,
}
```

This follows Rust best practices for non-thread-safe resources in async contexts.

## Database Location

The indexer database is stored alongside the blockchain data:

```md
~/.kanari/kanari-db/
├── kanari_db/         # Blockchain state (RocksDB)
└── indexer.db         # Indexer database (SQLite)
```

## Querying Indexed Data

You can query the indexed data using the kanari-indexer API directly or through RPC endpoints (future enhancement).

### Example Usage

```rust
use kanari_indexer::{Indexer, IndexerConfig};

let config = IndexerConfig {
    db_path: PathBuf::from("~/.kanari/kanari-db/indexer.db"),
    in_memory: false,
    batch_size: 100,
};

let indexer = Indexer::new(config)?;

// Query transactions by sender
let txs = indexer.db().get_transactions_by_sender(addr, 10)?;

// Get account balances
let balances = indexer.db().get_all_balances(addr)?;

// Get statistics
let stats = indexer.get_statistics()?;
```

## Performance Considerations

- **Synchronous Indexing**: Blocks are indexed synchronously during sync to ensure consistency
- **Minimal Overhead**: SQLite operations are fast (~1-5ms per block)
- **WAL Mode**: Enabled for better concurrent read performance
- **Batch Operations**: Can be optimized further if needed

## Error Handling

If indexing fails, the error is logged but doesn't prevent block synchronization:

```rust
match indexer.lock() {
    Ok(idx) => match idx.index_block(&kanari_block) {
        Ok(_) => info!("[INDEXER] Indexed block #{}", height),
        Err(e) => error!("[INDEXER] Failed to index block #{}: {}", height, e),
    },
    Err(e) => error!("[INDEXER] Failed to acquire indexer lock: {}", e),
}
```

This ensures the node continues operating even if there are temporary indexing issues.

## Future Enhancements

1. **RPC Endpoints**: Expose indexer queries via RPC API
2. **Async Indexing**: Move indexing to background task for better performance
3. **Indexing Status**: Track and report indexing progress
4. **Reindexing Command**: CLI command to reindex from scratch
5. **Custom Event Handlers**: Plugin system for custom event processing

## Troubleshooting

### Indexer Not Initializing

Check logs for:

```md
Node indexer initialized at <path>
```

If you see:

```md
Failed to initialize indexer: <error>
```

Common causes:

- Insufficient disk space
- Permission issues on data directory
- Corrupted database file

### Indexing Errors

Look for:

```md
[INDEXER] Failed to index block #<height>: <error>
```

The node will continue syncing blocks even if indexing fails.

### Database Corruption

If the indexer database becomes corrupted:

1. Stop the node
2. Delete `indexer.db`
3. Restart the node (it will reindex from genesis)

Or use the reindexing command (when implemented):

```bash
kanari-node reindex --from-height 0
```

## Testing

To test the indexer integration:

1. Start a node:

```bash
cargo run -p kanari-node -- start --data-dir ./test-data
```

1. Let it sync some blocks

2. Query the indexer:

```rust
// In a separate program
let indexer = Indexer::new(config)?;
let stats = indexer.get_statistics()?;
println!("{:?}", stats);
```

## Configuration

Currently, the indexer is always enabled when the node starts. Future versions may add:

- `--enable-indexer` flag (default: true)
- `--indexer-db-path` option
- `--indexer-batch-size` tuning parameter
