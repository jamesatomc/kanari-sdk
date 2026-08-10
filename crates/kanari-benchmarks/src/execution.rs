// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use kanari_core::{BlockchainEngine, CheckpointProductionInfo};
use kanari_types::transaction::SignedTransaction;
use std::time::Instant;

const DEFAULT_PRODUCTION_CHUNK_SIZE: usize = 50_000;

pub fn execute_admission_path(
    engine: &BlockchainEngine,
    signed_txs: Vec<SignedTransaction>,
) -> Result<(CheckpointProductionInfo, f64)> {
    let requested = signed_txs.len();
    let submit_start = Instant::now();
    let accepted = engine.submit_transactions_batch(signed_txs)?;
    let submit_secs = submit_start.elapsed().as_secs_f64();

    Ok((
        CheckpointProductionInfo {
            vertex_id: "admission-mode".to_string(),
            round: 0,
            tx_count: accepted.len(),
            executed: 0,
            failed: requested.saturating_sub(accepted.len()),
            events: vec![],
            checkpoint: None,
            vertex: None,
        },
        submit_secs,
    ))
}

pub fn execute_production_path(
    engine: &BlockchainEngine,
    signed_txs: Vec<SignedTransaction>,
) -> Result<(CheckpointProductionInfo, f64, f64)> {
    let chunk_size = production_chunk_size(signed_txs.len());
    let mut chunks = signed_txs.chunks(chunk_size);
    let first_chunk = chunks
        .next()
        .context("No transactions supplied to production benchmark")?;
    let mut submit_secs = submit_chunk(engine, first_chunk)?;
    let mut produce_secs = 0.0;
    let mut aggregate: Option<CheckpointProductionInfo> = None;

    loop {
        while engine.pending_transaction_len() > 0 {
            let produce_start = Instant::now();
            let block_info = engine.produce_checkpoint()?;
            produce_secs += produce_start.elapsed().as_secs_f64();
            merge_checkpoint_info(&mut aggregate, block_info);
        }

        let Some(next_chunk) = chunks.next() else {
            break;
        };
        let chunk_submit_secs = submit_chunk(engine, next_chunk)?;
        submit_secs += chunk_submit_secs;
        if chunk_profile_enabled() {
            eprintln!(
                "benchmark chunk submitted: txs={} submit={:.6}s",
                next_chunk.len(),
                chunk_submit_secs
            );
        }
    }
    let block_info = aggregate.context("No checkpoint was produced for submitted transactions")?;

    Ok((block_info, submit_secs, produce_secs))
}

pub fn execute_owned_fast_path(
    engine: &BlockchainEngine,
    signed_txs: Vec<SignedTransaction>,
) -> Result<(CheckpointProductionInfo, f64, f64)> {
    let chunk_size = production_chunk_size(signed_txs.len());
    let mut chunks = signed_txs.chunks(chunk_size);
    let first_chunk = chunks
        .next()
        .context("No transactions supplied to owned-fastpath benchmark")?;
    let mut submit_secs = submit_chunk(engine, first_chunk)?;
    let mut produce_secs = 0.0;
    let mut aggregate: Option<CheckpointProductionInfo> = None;

    loop {
        while engine.pending_transaction_len() > 0 {
            let produce_start = Instant::now();
            let block_info = engine.produce_owned_fast_checkpoint()?;
            produce_secs += produce_start.elapsed().as_secs_f64();
            if block_info.tx_count == 0 {
                anyhow::bail!(
                    "owned-fastpath made no progress with {} pending transactions",
                    engine.pending_transaction_len()
                );
            }
            merge_checkpoint_info(&mut aggregate, block_info);
        }

        let Some(next_chunk) = chunks.next() else {
            break;
        };
        submit_secs += submit_chunk(engine, next_chunk)?;
    }
    let block_info = aggregate
        .context("No owned-fastpath checkpoint was produced for submitted transactions")?;

    Ok((block_info, submit_secs, produce_secs))
}

pub fn execute_immediate(
    engine: &BlockchainEngine,
    signed_txs: Vec<SignedTransaction>,
) -> Result<CheckpointProductionInfo> {
    let mut executed = 0usize;
    let mut failed = 0usize;
    let mut failure_samples = Vec::new();

    for signed_tx in signed_txs {
        match engine.execute_transaction_immediate(signed_tx) {
            Ok((_tx_hash, changeset)) => {
                if changeset.success {
                    let mut state = engine.state_write();
                    state.apply_changeset(&changeset)?;
                    state.commit()?;
                    executed += 1;
                } else {
                    if failure_samples.len() < 3 {
                        failure_samples.push(Some("changeset marked failed".to_string()));
                    }
                    failed += 1;
                }
            }
            Err(err) => {
                if failure_samples.len() < 3 {
                    failure_samples.push(Some(err.to_string()));
                }
                failed += 1;
            }
        }
    }

    if !failure_samples.is_empty() {
        eprintln!("Immediate-mode failure samples:");
        for sample in failure_samples.into_iter().flatten() {
            eprintln!("  - {sample}");
        }
    }

    Ok(mode_info("immediate-mode", executed, failed))
}

fn mode_info(vertex_id: &str, executed: usize, failed: usize) -> CheckpointProductionInfo {
    CheckpointProductionInfo {
        vertex_id: vertex_id.to_string(),
        round: 0,
        tx_count: executed + failed,
        executed,
        failed,
        events: vec![],
        checkpoint: None,
        vertex: None,
    }
}

fn production_chunk_size(tx_count: usize) -> usize {
    std::env::var("KANARI_BENCH_CHUNK_SIZE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PRODUCTION_CHUNK_SIZE)
        .min(tx_count.max(1))
}

fn chunk_profile_enabled() -> bool {
    std::env::var("KANARI_BENCH_CHUNK_PROFILE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn submit_chunk(engine: &BlockchainEngine, chunk: &[SignedTransaction]) -> Result<f64> {
    let submit_start = Instant::now();
    engine.submit_transactions_batch(chunk.to_vec())?;
    Ok(submit_start.elapsed().as_secs_f64())
}

fn merge_checkpoint_info(
    aggregate: &mut Option<CheckpointProductionInfo>,
    block_info: CheckpointProductionInfo,
) {
    if let Some(total) = aggregate {
        total.tx_count += block_info.tx_count;
        total.executed += block_info.executed;
        total.failed += block_info.failed;
        total.vertex_id = block_info.vertex_id;
        total.round = block_info.round;
        total.checkpoint = block_info.checkpoint;
        total.vertex = block_info.vertex;
    } else {
        *aggregate = Some(block_info);
    }
}
