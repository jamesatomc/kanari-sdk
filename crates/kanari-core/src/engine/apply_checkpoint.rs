// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::BlockchainEngine;
use crate::consensus::Checkpoint;
use anyhow::{Context, Result, bail};
use kanari_move_runtime_v1::state::StateManager;
use kanari_types::transaction::SignedTransaction;
use log::info;
use std::sync::{Arc, RwLock};

pub(crate) struct PreparedCheckpointState {
    pub state_root: Vec<u8>,
    pub state: StateManager,
    pub transactions: Vec<SignedTransaction>,
    pub effects: Vec<kanari_types::transaction::TransactionEffects>,
}

impl BlockchainEngine {
    fn requires_runtime_side_effect_persistence(transactions: &[SignedTransaction]) -> bool {
        transactions.iter().any(|signed_tx| {
            if signed_tx.transaction.is_native_balance_call() {
                return false;
            }

            matches!(
                signed_tx.transaction,
                kanari_types::transaction::Transaction::PublishModule { .. }
                    | kanari_types::transaction::Transaction::PublishPackage { .. }
                    | kanari_types::transaction::Transaction::UpgradeModule { .. }
                    | kanari_types::transaction::Transaction::UpgradePackage { .. }
                    | kanari_types::transaction::Transaction::ExecuteFunction { .. }
            )
        })
    }

    pub(crate) fn apply_system_prologue_to_state(
        &self,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp_ms: u64,
        persist_objects: bool,
    ) -> Result<()> {
        let runtime = &self.runtime_pool[0];
        let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
        let clock_id = runtime.ensure_system_clock(&mut state_write)?;
        let changeset = runtime.execute_clock_consensus_commit_prologue(clock_id, timestamp_ms)?;
        // Apply without supply validation first, then repair legacy overcount
        // that may be exposed by the clock prologue changeset.
        state_write
            .apply_changeset_without_supply_validation(&changeset)
            .context("Failed to apply clock prologue changeset")?;

        if persist_objects {
            runtime.persist_created_objects(&changeset)?;
            runtime.persist_deleted_objects(&changeset)?;
        }

        state_write
            .repair_legacy_native_wallet_overcount()
            .context("Failed to repair native wallet overcount after clock prologue")?;
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
        let mut state_snapshot = self.state_read().clone();
        state_snapshot
            .repair_legacy_native_wallet_overcount()
            .context(
                "Failed to repair legacy native wallet overcount before checkpoint state execution",
            )?;
        let state_arc = Arc::new(RwLock::new(state_snapshot));
        let to_execute: Vec<SignedTransaction> = checkpoint
            .transactions
            .iter()
            .filter(|signed_tx| !self.is_transaction_committed(signed_tx.transaction_hash()))
            .cloned()
            .collect();

        if !checkpoint.transactions.is_empty() {
            self.apply_system_prologue_to_state(&state_arc, checkpoint.timestamp, false)?;
        }

        let (_, _, transaction_effects) = self
            .execute_tx_waves_deterministic_parallel_with_effects(
                to_execute.clone(),
                &state_arc,
                Some(checkpoint.timestamp),
                false, // persist_objects = false
            )?;

        {
            let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
            state_write
                .repair_legacy_native_wallet_overcount()
                .context("Failed to repair legacy native wallet overcount after checkpoint state execution")?;
        }

        let verified_state = state_arc.read().unwrap_or_else(|e| e.into_inner()).clone();
        let computed_root = verified_state.try_compute_state_root()?;
        Ok(PreparedCheckpointState {
            state_root: computed_root,
            state: verified_state,
            transactions: to_execute,
            effects: transaction_effects,
        })
    }

    /// Helper: Common steps for finalizing Checkpoint to database
    fn finalize_checkpoint(
        &self,
        checkpoint: Checkpoint,
        new_state: StateManager,
        validate_supply: bool,
    ) -> Result<()> {
        let has_module_publish = checkpoint.transactions.iter().any(|signed_tx| {
            matches!(
                signed_tx.transaction,
                kanari_types::transaction::Transaction::PublishModule { .. }
                    | kanari_types::transaction::Transaction::PublishPackage { .. }
                    | kanari_types::transaction::Transaction::UpgradeModule { .. }
                    | kanari_types::transaction::Transaction::UpgradePackage { .. }
            )
        });
        if validate_supply {
            new_state
                .validate_supply_invariants()
                .context("Supply invariants failed before checkpoint commit")?;
        }

        {
            // Keep the previous live state installed until the staged state and
            // durable recovery marker have committed successfully. A RocksDB
            // failure must not leave RPC/execution observing a new in-memory
            // state paired with old checkpoint metadata.
            let mut live_state = self.state_write();
            let mut committed_state = new_state;
            if self.persistent_store.is_some() {
                let marker = bcs::to_bytes(&checkpoint)
                    .context("Failed to serialize durable checkpoint commit marker")?;
                committed_state
                    .commit_with_raw_update(Self::pending_checkpoint_commit_key().to_vec(), marker)
                    .context("Failed to commit state and checkpoint marker to RocksDB")?;
            } else {
                committed_state
                    .commit()
                    .context("Failed to commit state to RocksDB")?;
            }
            *live_state = committed_state;
        }

        for runtime in &self.runtime_pool {
            runtime.clear_object_cache()?;
        }

        self.finalize_checkpoint_metadata(checkpoint)?;

        if has_module_publish {
            for runtime in &self.runtime_pool {
                runtime
                    .refresh_committed_modules()
                    .context("Failed to refresh Move VM modules after checkpoint commit")?;
            }
        }

        if let Some(store) = &self.persistent_store {
            store
                .delete(Self::pending_checkpoint_commit_key())
                .context("Failed to clear durable checkpoint commit marker")?;
        }

        Ok(())
    }

    fn finalize_checkpoint_metadata(&self, checkpoint: Checkpoint) -> Result<()> {
        // 1. Update blockchain metadata in-memory.
        {
            let mut chain = self.blockchain.write().unwrap_or_else(|e| e.into_inner());
            chain.add_checkpoint_with_validation(checkpoint.clone(), true)?;
        }

        // 2. Persist blockchain state before draining the live mempool view.
        if self.persistent_store.is_some() {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = self.persist_blockchain_snapshot(&chain) {
                drop(chain);
                anyhow::bail!(
                    "Failed to persist blockchain metadata for checkpoint {}: {}. The committed state is protected by a durable recovery marker; restart the node to finish metadata recovery.",
                    checkpoint.sequence,
                    e
                );
            }
        }

        // 3. Remove committed transactions from pending pool.
        {
            let mut mempool = self.mempool_write();
            let committed_hashes: std::collections::HashSet<_> = checkpoint
                .transactions
                .iter()
                .map(|tx| tx.transaction_hash().to_vec())
                .collect();
            let removed_transactions = mempool
                .pending_txs
                .iter()
                .filter(|tx| committed_hashes.contains(tx.signed_tx.transaction_hash()))
                .cloned()
                .collect::<Vec<_>>();
            mempool
                .pending_txs
                .retain(|tx| !committed_hashes.contains(tx.signed_tx.transaction_hash()));
            let removed_bytes = removed_transactions.iter().fold(0usize, |total, record| {
                total.saturating_add(
                    Self::pending_record_size(&record.signed_tx, &record.metadata)
                        .unwrap_or(crate::engine::MAX_TRANSACTION_BYTES),
                )
            });
            mempool.pending_bytes = mempool.pending_bytes.saturating_sub(removed_bytes);
            mempool
                .pending_tx_hashes
                .retain(|hash| !committed_hashes.contains(hash));
            Self::remove_pending_sender_counts(
                &mut mempool.pending_sender_counts,
                &removed_transactions,
            );
            Self::remove_pending_access_counts(
                &mut mempool.pending_access_counts,
                &removed_transactions,
            );
            Self::remove_pending_primary_access_counts(
                &mut mempool.pending_primary_access_counts,
                &removed_transactions,
            );
        }

        Ok(())
    }
    pub(crate) fn apply_prepared_checkpoint(
        &self,
        checkpoint: Checkpoint,
        verified_state: StateManager,
        to_execute: Vec<SignedTransaction>,
        validate_supply: bool,
    ) -> Result<()> {
        if !to_execute.is_empty() && Self::requires_runtime_side_effect_persistence(&to_execute) {
            tracing::debug!(
                checkpoint = checkpoint.sequence,
                "Skipping checkpoint side-effect replay: verified_state commit is the canonical source of truth, and replaying persistence against the same backing store can skew object versions during restart/recovery paths."
            );
        }

        self.finalize_checkpoint(checkpoint, verified_state, validate_supply)
    }

    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {
        info!(
            "[ENGINE] Applying checkpoint {} with {} txs",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        let prepared = self.prepare_checkpoint_state(&checkpoint)?;
        self.ensure_checkpoint_root_matches(&checkpoint, &prepared.state_root)?;

        self.apply_prepared_checkpoint(checkpoint, prepared.state, prepared.transactions, true)
    }
}
