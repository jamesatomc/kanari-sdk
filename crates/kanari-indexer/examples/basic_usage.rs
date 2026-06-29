#![allow(clippy::print_stdout)]
// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Example demonstrating basic indexer usage

use anyhow::Result;
use kanari_indexer::{Indexer, IndexerConfig};
use std::path::PathBuf;

fn main() -> Result<()> {
    // Initialize tracing for logging
    tracing_subscriber::fmt::init();

    println!("=== Kanari Indexer Basic Example ===\n");

    // Create indexer with in-memory database for demonstration
    let config = IndexerConfig {
        db_path: PathBuf::from("example_indexer.db"),
        in_memory: true, // Use in-memory for this example
        batch_size: 10,
    };

    let indexer = Indexer::new(config)?;
    println!("✓ Indexer initialized successfully\n");

    // Show initial statistics
    let stats = indexer.get_statistics()?;
    println!("Initial Statistics:");
    println!("  Blocks: {}", stats.block_count);
    println!("  Transactions: {}", stats.transaction_count);
    println!("  Events: {}", stats.event_count);
    println!();

    // Demonstrate metadata operations
    indexer.db().set_metadata("example_key", "example_value")?;
    if let Some(value) = indexer.db().get_metadata("example_key")? {
        println!("✓ Metadata test: example_key = {}\n", value);
    }

    // Show available query methods
    println!("Available Operations:");
    println!("  - index_block(block): Index a single block");
    println!("  - index_blocks_batch(blocks): Index multiple blocks");
    println!("  - sync_to_latest(fetch_fn): Sync from current to latest");
    println!("  - get_block_by_height(height): Query block by height");
    println!("  - get_transactions_by_sender(addr, limit): Query transactions");
    println!("  - get_coins_by_owner(addr): Query coins owned by address");
    println!("  - get_all_balances(addr): Get all balances for address");
    println!("  - get_events_by_transaction(tx_hash): Get events from TX");
    println!();

    println!("Example completed successfully!");

    Ok(())
}
