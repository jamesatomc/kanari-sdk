// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::{
    BlockchainEngine, MAX_PERSISTED_RECENT_TX_HASHES, PersistedTransactionLocation,
    TransactionExecutionReceipt,
};
use crate::blockchain::Blockchain;
use crate::consensus::Checkpoint;
use anyhow::{Context, Result, bail};
use kanari_move_runtime_v1::state::StateManager;
use kanari_types::transaction::SignedTransaction;
use log::info;
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

type PreparedCheckpointState = (
    Vec<u8>,
    StateManager,
    Vec<TransactionExecutionReceipt>,
);

impl BlockchainEngine {
    fn apply_system_prologue_to_state(
        &self,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp_ms: u64,
    ) -> Result<()> {
        let runtime = &self.runtime_pool[0];
        let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
        let clock_id = runtime.ensure_system_clock(&mut state_write)?;
        let changeset = runtime.execute_clock_consensus_commit_prologue(clock_id, timestamp_ms)?;
        state_write.apply_changeset(&changeset)?;
        Ok(())
    }

    fn ensure_checkpoint_root_matches(
        &self,
        checkpoint: &Checkpoint,
        computed_root: &[u8],
    ) -> Result<()> {
        if self.checkpoint_root_matches(
            checkpoint.sequence,
            computed_root,
            &checkpoint.state_root,
        )? {
            return Ok(());
        }

        bail!(
            "[ENGINE] State root mismatch for checkpoint {}. expected={}, computed={}",
            checkpoint.sequence,
            hex::encode(&checkpoint.state_root),
            hex::encode(computed_root)
        );
    }

    pub(crate) fn prepare_checkpoint_state(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<PreparedCheckpointState> {
        let state_snapshot = self.state_read().clone();
        let state_arc = Arc::new(RwLock::new(state_snapshot));
        let to_execute: Vec<SignedTransaction> = {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            checkpoint
                .transactions
                .iter()
                .filter(|signed_tx| {
                    !chain.is_transaction_hash_executed(signed_tx.transaction_hash())
                })
                .cloned()
                .collect()
        };

        if !checkpoint.transactions.is_empty() {
            self.apply_system_prologue_to_state(&state_arc, checkpoint.timestamp)?;
        }

        let execution = self.execute_tx_waves_strict_serial_with_receipts(
            to_execute,
            &state_arc,
            Some(checkpoint.timestamp),
            false,
        )?;

        let verified_state = state_arc.read().unwrap_or_else(|e| e.into_inner()).clone();
        let computed_root = verified_state.compute_state_root();
        Ok((computed_root, verified_state, execution.receipts))
    }

    fn candidate_chain_with_checkpoint(&self, checkpoint: &Checkpoint) -> Result<Blockchain> {
        let mut candidate = self
            .blockchain
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        candidate.add_checkpoint_with_validation(checkpoint.clone(), true)?;
        Ok(candidate)
    }

    fn stage_checkpoint_commit(
        &self,
        state: &mut StateManager,
        checkpoint: &Checkpoint,
        receipts: &[TransactionExecutionReceipt],
        candidate_chain: &Blockchain,
    ) -> Result<()> {
        state.stage_value(
            &Self::checkpoint_metadata_key(checkpoint.sequence),
            &Self::checkpoint_without_transactions(checkpoint),
        )?;

        let mut recent_hashes = state
            .store
            .load::<Vec<Vec<u8>>>(Self::recent_transaction_hashes_key())?
            .unwrap_or_default();
        let mut recent_set: HashSet<Vec<u8>> = recent_hashes.iter().cloned().collect();

        for tx in checkpoint.transactions.iter() {
            let tx_hash = tx.transaction_hash().to_vec();
            state.stage_value(&Self::transaction_payload_key(&tx_hash), tx)?;
            state.stage_value(
                &Self::transaction_index_key(&tx_hash),
                &PersistedTransactionLocation {
                    checkpoint_sequence: checkpoint.sequence,
                    state_root: checkpoint.state_root.clone(),
                },
            )?;
            if recent_set.insert(tx_hash.clone()) {
                recent_hashes.push(tx_hash);
            }
        }

        if recent_hashes.len() > MAX_PERSISTED_RECENT_TX_HASHES {
            let trim = recent_hashes.len() - MAX_PERSISTED_RECENT_TX_HASHES;
            recent_hashes.drain(0..trim);
        }
        state.stage_value(Self::recent_transaction_hashes_key(), &recent_hashes)?;
        state.stage_value(
            &Self::checkpoint_transactions_key(checkpoint.sequence),
            &checkpoint.transactions,
        )?;

        for receipt in receipts {
            state.stage_value(
                &Self::transaction_receipt_key(&receipt.transaction_hash),
                receipt,
            )?;
        }

        let mut slim_chain = candidate_chain.clone();
        for stored_checkpoint in &mut slim_chain.dag_checkpoints {
            if !stored_checkpoint.transactions.is_empty() {
                *stored_checkpoint = Self::checkpoint_without_transactions(stored_checkpoint);
            }
        }
        state.stage_value(b"blockchain", &slim_chain)?;
        Ok(())
    }

    fn remove_committed_from_mempool(&self, checkpoint: &Checkpoint) {
        let mut mempool = self.mempool_write();
        let committed_hashes: HashSet<Vec<u8>> = checkpoint
            .transactions
            .iter()
            .map(|tx| tx.transaction_hash().to_vec())
            .collect();
        let removed_transactions = mempool
            .pending_txs
            .iter()
            .filter(|tx| committed_hashes.contains(tx.transaction_hash()))
            .cloned()
            .collect::<Vec<_>>();
        mempool
            .pending_txs
            .retain(|tx| !committed_hashes.contains(tx.transaction_hash()));
        mempool
            .pending_tx_hashes
            .retain(|hash| !committed_hashes.contains(hash));
        Self::remove_pending_sender_counts(
            &mut mempool.pending_sender_counts,
            &removed_transactions,
        );
    }

    fn finalize_checkpoint(
        &self,
        checkpoint: Checkpoint,
        mut new_state: StateManager,
        receipts: Vec<TransactionExecutionReceipt>,
        validate_supply: bool,
    ) -> Result<()> {
        if validate_supply {
            new_state
                .validate_supply_invariants()
                .context("Supply invariants failed before checkpoint commit")?;
        }

        let candidate_chain = self.candidate_chain_with_checkpoint(&checkpoint)?;
        self.stage_checkpoint_commit(
            &mut new_state,
            &checkpoint,
            &receipts,
            &candidate_chain,
        )?;
        new_state
            .commit()
            .context("Failed to atomically commit checkpoint state and metadata")?;

        *self.state_write() = new_state;
        *self.blockchain.write().unwrap_or_else(|e| e.into_inner()) = candidate_chain;
        self.remove_committed_from_mempool(&checkpoint);

        for runtime in &self.runtime_pool {
            runtime.clear_object_cache()?;
            runtime.reload_vm_cache()?;
        }
        Ok(())
    }

    pub(crate) fn apply_prepared_checkpoint(
        &self,
        checkpoint: Checkpoint,
        verified_state: StateManager,
        receipts: Vec<TransactionExecutionReceipt>,
        validate_supply: bool,
    ) -> Result<()> {
        self.finalize_checkpoint(checkpoint, verified_state, receipts, validate_supply)
    }

    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {
        info!(
            "[ENGINE] Applying checkpoint {} with {} txs",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        let (computed_root, verified_state, receipts) =
            self.prepare_checkpoint_state(&checkpoint)?;
        self.ensure_checkpoint_root_matches(&checkpoint, &computed_root)?;
        self.apply_prepared_checkpoint(checkpoint, verified_state, receipts, true)
    }
}
