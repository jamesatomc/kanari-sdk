use anyhow::Result;
use kanari_core::{BlockchainEngine, CheckpointProductionInfo};
use kanari_types::transaction::SignedTransaction;
use std::time::Instant;

pub fn execute_production_path(
    engine: &BlockchainEngine,
    signed_txs: Vec<SignedTransaction>,
) -> Result<(CheckpointProductionInfo, f64, f64)> {
    let submit_start = Instant::now();
    engine.submit_transactions_batch(signed_txs)?;
    let submit_secs = submit_start.elapsed().as_secs_f64();

    let produce_start = Instant::now();
    let block_info = engine.produce_checkpoint()?;
    let produce_secs = produce_start.elapsed().as_secs_f64();

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
        checkpoint_votes: vec![],
        vertex: None,
    }
}
