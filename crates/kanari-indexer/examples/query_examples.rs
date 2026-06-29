#![allow(clippy::print_stdout)]
// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Example demonstrating various query patterns

use anyhow::Result;
use kanari_indexer::{Indexer, IndexerConfig};
use std::path::PathBuf;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    println!("=== Kanari Indexer Query Examples ===\n");

    // Initialize indexer
    let config = IndexerConfig {
        db_path: PathBuf::from("query_examples.db"),
        in_memory: true,
        batch_size: 10,
    };

    let indexer = Indexer::new(config)?;

    // Example 1: Block Queries
    println!("1. BLOCK QUERIES");
    println!("   Getting latest block height...");
    let latest_height = indexer.db().get_latest_height()?;
    println!("   Latest height: {}\n", latest_height);

    // Example 2: Transaction Statistics
    println!("2. TRANSACTION STATISTICS");
    let tx_count = indexer.db().get_transaction_count()?;
    println!("   Total transactions: {}", tx_count);

    let stats = indexer.db().get_transaction_stats()?;
    if !stats.is_empty() {
        println!("   Transactions by type:");
        for (tx_type, count) in stats {
            println!("     - {}: {}", tx_type, count);
        }
    } else {
        println!("   No transactions indexed yet");
    }
    println!();

    // Example 3: Top Addresses
    println!("3. TOP ADDRESSES");
    let top_addresses = indexer.db().get_top_addresses(5)?;
    if !top_addresses.is_empty() {
        println!("   Most active addresses:");
        for (addr, count) in top_addresses {
            println!("     - {}: {} transactions", addr, count);
        }
    } else {
        println!("   No transaction data available");
    }
    println!();

    // Example 4: Account Balance Queries
    println!("4. ACCOUNT BALANCE QUERIES");
    let sample_address = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    println!("   Checking balances for: {}...", sample_address);

    let balances = indexer.db().get_all_balances(sample_address)?;
    if !balances.is_empty() {
        for balance in balances {
            println!(
                "     - {}: {} coins ({} objects)",
                balance.coin_type, balance.total_balance, balance.coin_count
            );
        }
    } else {
        println!("   No balances found for this address");
    }
    println!();

    // Example 5: Event Queries
    println!("5. EVENT QUERIES");
    let event_count = indexer.db().get_event_count()?;
    println!("   Total events indexed: {}", event_count);
    println!();

    // Example 6: Metadata Operations
    println!("6. METADATA OPERATIONS");
    indexer.db().set_metadata(
        "last_sync_timestamp",
        &chrono::Utc::now().timestamp().to_string(),
    )?;
    if let Some(timestamp) = indexer.db().get_metadata("last_sync_timestamp")? {
        println!("   Last sync timestamp: {}", timestamp);
    }
    println!();

    // Example 7: Database Statistics
    println!("7. DATABASE STATISTICS");
    let block_count = indexer.db().get_block_count()?;
    let tx_count = indexer.db().get_transaction_count()?;
    let event_count = indexer.db().get_event_count()?;

    println!("   Blocks: {}", block_count);
    println!("   Transactions: {}", tx_count);
    println!("   Events: {}", event_count);
    println!();

    println!("✓ All query examples completed!");
    println!("\nNote: These examples show the query API.");
    println!("In production, you would first index blocks using:");
    println!("  indexer.index_block(&block)");
    println!("  indexer.sync_to_latest(fetch_fn)");

    Ok(())
}
