#![allow(clippy::print_stdout)]
// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Example demonstrating indexer with simulated blockchain data

use anyhow::Result;
use kanari_indexer::{Indexer, IndexerConfig};
use kanari_types::block::Block;
use kanari_types::transaction::{SignedTransaction, Transaction};
use std::path::PathBuf;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== Kanari Indexer Integration Example ===\n");

    // Create indexer
    let config = IndexerConfig {
        db_path: PathBuf::from("integration_test.db"),
        in_memory: true,
        batch_size: 5,
    };

    let indexer = Indexer::new(config)?;
    println!("✓ Indexer initialized\n");

    // Create genesis block
    let genesis = Block::genesis();
    println!("Created genesis block at height {}", genesis.header.height);

    indexer.index_block(&genesis)?;
    println!("✓ Genesis block indexed\n");

    // Create a sample transaction (for demonstration)
    let _tx = SignedTransaction::new(Transaction::new_transfer(
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string(),
        "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890".to_string(),
        1000,
        0,
    ));
    println!("Created sample transfer transaction");

    // Show available operations
    println!("\n=== Available Query Operations ===");
    println!("1. Block Queries:");
    println!("   - get_block_by_height(height)");
    println!("   - get_latest_height()");
    println!("   - get_block_count()");
    println!();
    println!("2. Transaction Queries:");
    println!("   - get_transaction_by_hash(tx_hash)");
    println!("   - get_transactions_by_block(height)");
    println!("   - get_transactions_by_sender(addr, limit)");
    println!();
    println!("3. Event Queries:");
    println!("   - get_events_by_transaction(tx_hash)");
    println!("   - get_events_by_key(event_key, limit)");
    println!();
    println!("4. Coin & Balance Queries:");
    println!("   - get_coin_by_id(coin_id)");
    println!("   - get_coins_by_owner(owner)");
    println!("   - get_account_balance(addr, coin_type)");
    println!("   - get_all_balances(addr)");
    println!();
    println!("5. Analytics:");
    println!("   - get_transaction_count()");
    println!("   - get_event_count()");
    println!("   - get_transaction_stats()");
    println!("   - get_top_addresses(limit)");
    println!();

    // Show current statistics
    let stats = indexer.get_statistics()?;
    println!("=== Current Statistics ===");
    println!("{}", stats);

    println!("\n✓ Integration example completed successfully!");
    println!("\nTo use with real blockchain data:");
    println!("1. Connect to a Kanari node");
    println!("2. Fetch blocks using the node's API");
    println!("3. Call indexer.index_block(&block) for each block");
    println!("4. Use query methods to retrieve indexed data");

    Ok(())
}
