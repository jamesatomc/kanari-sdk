// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::BlockchainEngine;
use anyhow::{Context, Result};
use centauri::consensus::Checkpoint;
use kanari_move_runtime::state::StateManager;
use kanari_types::transaction::SignedTransaction;
use log::{error, info, warn};
use std::sync::{Arc, RwLock};

impl BlockchainEngine {
    /// Helper: Common steps for finalizing Checkpoint to database
    fn finalize_checkpoint(&self, checkpoint: Checkpoint, new_state: StateManager) -> Result<()> {
        // 1. Update canonical state
        {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            *state = new_state;
            state
                .commit()
                .context("Failed to commit state to RocksDB")?;
        }

        // 2. Update blockchain
        {
            let mut chain = self.blockchain.write().unwrap_or_else(|e| e.into_inner());
            chain.add_checkpoint_with_validation(checkpoint.clone(), false)?;
        }

        // 3. Remove committed transactions from pending pool
        {
            let mut pending = self.pending_txs.write().unwrap_or_else(|e| e.into_inner());
            let committed_hashes: std::collections::HashSet<_> =
                checkpoint.transactions.iter().map(|tx| tx.hash()).collect();
            pending.retain(|tx| !committed_hashes.contains(&tx.hash()));
        }

        // 4. Persist blockchain state
        if let Some(store) = &self.persistent_store {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = store.save(b"blockchain", &*chain) {
                error!("Failed to persist blockchain: {}", e);
            }
        }
        Ok(())
    }

    pub fn apply_checkpoint_optimized(
        &self,
        checkpoint: Checkpoint,
        precomputed_state: Arc<RwLock<StateManager>>,
    ) -> Result<()> {
        info!(
            "[ENGINE] Applying checkpoint {} (OPTIMIZED)",
            checkpoint.sequence
        );

        let computed_root = precomputed_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .compute_state_root();
        if computed_root != checkpoint.state_root {
            warn!("[ENGINE] State root mismatch! Fallback to standard application.");
            return self.apply_checkpoint(checkpoint);
        }

        // Call Helper
        self.finalize_checkpoint(
            checkpoint,
            precomputed_state
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        )
    }

    pub fn apply_checkpoint(&self, mut checkpoint: Checkpoint) -> Result<()> {
        info!(
            "[ENGINE] Applying checkpoint {} with {} txs",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        let state_snapshot = self.state.read().unwrap_or_else(|e| e.into_inner()).clone();
        let state_arc = Arc::new(RwLock::new(state_snapshot));
        let mut to_execute: Vec<SignedTransaction> = Vec::new();

        {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            for signed_tx in &checkpoint.transactions {
                if !chain.is_transaction_executed(&hex::encode(signed_tx.hash())) {
                    to_execute.push(signed_tx.clone());
                }
            }
        }

        // 🚨 Call Helper from engine.rs instead of writing new Par_iter loop
        let (_executed_count, _) = self.execute_tx_waves_parallel(
            to_execute,
            &state_arc,
            Some(checkpoint.timestamp),
            true, // persist_objects = true
            true, // strict_mode = true (throw immediately on failure)
        )?;

        let verified_state = {
            let state_read = state_arc.read().unwrap_or_else(|e| e.into_inner());
            let computed_root = state_read.compute_state_root();
            if computed_root != checkpoint.state_root {
                warn!("[ENGINE] State root mismatch! Updating to computed root.");
                checkpoint.state_root = computed_root;
            }
            state_read.clone()
        };

        // Call Helper
        self.finalize_checkpoint(checkpoint, verified_state)
    }
}
