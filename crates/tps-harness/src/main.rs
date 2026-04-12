// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use kanari_core::BlockchainEngine;
use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_types::transaction::{SignedTransaction, Transaction};
use move_core_types::account_address::AccountAddress;
use std::time::Instant;

#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() -> Result<()> {
    // Simple harness: create engine, pre-fund accounts, submit N transfers into pending_txs,
    // call produce_block() once and measure duration.
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100_000);

    eprintln!("TPS harness: creating engine and preparing {} txs", n);
    let temp_dir = tempfile::Builder::new().prefix("kanari_tps").tempdir()?;
    let engine = BlockchainEngine::new_dir(temp_dir.path().to_str().unwrap())?;

    // Warmup the engine and DAG
    eprintln!("Warming up engine...");
    engine.produce_block()?;

    // Generate keypairs for senders and recipients in parallel
    eprintln!("Generating keypairs...");
    let senders: Vec<_> = (0..n)
        .map(|_| generate_keypair(CurveType::Ed25519).unwrap())
        .collect();
    let recipients: Vec<_> = (0..n)
        .map(|_| generate_keypair(CurveType::Ed25519).unwrap())
        .collect();

    // Pre-fund sender accounts
    eprintln!("Funding accounts...");
    {
        let mut state = engine.state.write().unwrap();
        for kp in &senders {
            if let Ok(addr) = AccountAddress::from_hex_literal(kp.address.as_str()) {
                let mut acc = state.get_account(&addr).unwrap_or_else(|| {
                    kanari_core::kanari_move_runtime::state::Account::new(addr, 0)
                });
                acc.balance = 1_000_000_000_000; // large balance
                state.save_account(&acc).expect("Failed to save account");
            }
        }
    }

    // Prepare transactions and push signed transactions to pending_txs
    eprintln!("Signing transactions...");
    let signed_txs: Vec<_> = senders
        .iter()
        .zip(recipients.iter())
        .map(|(sender, recipient)| {
            let from = sender.address.clone();
            let to = recipient.address.clone();
            let _sender_addr = AccountAddress::from_hex_literal(&from).unwrap();

            // Note: In a real scenario, we'd query sequence number from state.
            // Here we know it's 0 (or 1 if warmup used it, but we used empty warmup)
            // But wait, if warmup used 0 txs, seq is 0.
            // If we reuse senders, seq might be > 0. But we generated new senders.
            let sequence_number = 0;

            let tx = Transaction::new_transfer(from, to, 1, sequence_number);
            let mut signed_tx = SignedTransaction::new(tx);
            signed_tx
                .sign(&sender.private_key, sender.curve_type)
                .unwrap();
            signed_tx
        })
        .collect();

    {
        let mut pending = engine.pending_txs.write().unwrap();
        pending.extend(signed_txs);
    }

    eprintln!("Starting produce_block() for {} txs...", n);
    let start = Instant::now();
    let info = engine.produce_block()?;
    let dur = start.elapsed();
    let tps = (info.tx_count as f64) / dur.as_secs_f64();

    eprintln!(
        "Done: executed={} failed={} tx_count={}",
        info.executed, info.failed, info.tx_count
    );
    eprintln!("Duration: {:.3}s — TPS: {:.2}", dur.as_secs_f64(), tps);

    Ok(())
}
