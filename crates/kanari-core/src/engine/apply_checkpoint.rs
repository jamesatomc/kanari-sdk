// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::BlockchainEngine;
use crate::consensus::Checkpoint;
use anyhow::{Context, Result, bail};
use kanari_move_runtime_v1::state::{PrecomputedSmtChanges, StateManager};
use kanari_types::transaction::SignedTransaction;
use log::info;
use std::sync::{Arc, RwLock};

pub(crate) struct PreparedCheckpointState {
    pub state_root: Vec<u8>,
    pub smt_changes: Option<PrecomputedSmtChanges>,
    pub state: StateManager,
    pub transactions: Vec<SignedTransaction>,
    pub effects: Vec<kanari_types::transaction::TransactionEffects>,
}

impl BlockchainEngine {
    fn changeset_needs_native_wallet_repair(
        changeset: &kanari_move_runtime_v1::changeset::ChangeSet,
    ) -> bool {
        !changeset.owner_deltas.is_empty()
            || !changeset.native_gas_credits.is_empty()
            || !changeset.treasuries.is_empty()
            || !changeset.token_balance_sets.is_empty()
            || !changeset.gas_object_refs.is_empty()
    }

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
        let use_vm_prologue = std::env::var("KANARI_CLOCK_PROLOGUE_VM")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);
        let changeset = if use_vm_prologue {
            runtime.execute_clock_consensus_commit_prologue(clock_id, timestamp_ms)?
        } else {
            runtime.build_native_clock_consensus_commit_prologue(
                &state_write,
                clock_id,
                timestamp_ms,
            )?
        };
        // Apply without supply validation first, then repair legacy overcount
        // that may be exposed by the clock prologue changeset.
        state_write
            .apply_changeset_without_supply_validation(&changeset)
            .context("Failed to apply clock prologue changeset")?;

        if persist_objects {
            runtime.persist_created_objects(&changeset)?;
            runtime.persist_deleted_objects(&changeset)?;
        }

        if Self::changeset_needs_native_wallet_repair(&changeset) {
            state_write
                .repair_legacy_native_wallet_overcount()
                .context("Failed to repair native wallet overcount after clock prologue")?;
        }
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
        self.prepare_checkpoint_state_inner(checkpoint, false)
    }

    pub(crate) fn prepare_conflict_free_checkpoint_state(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<PreparedCheckpointState> {
        self.prepare_checkpoint_state_inner(checkpoint, true)
    }

    fn prepare_checkpoint_state_inner(
        &self,
        checkpoint: &Checkpoint,
        assume_conflict_free: bool,
    ) -> Result<PreparedCheckpointState> {
        let profile = matches!(
            std::env::var("KANARI_CHECKPOINT_PREPARE_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let started_at = std::time::Instant::now();
        let mut state_snapshot = self.state_read().clone();
        if assume_conflict_free {
            let validate_owned_fastpath = std::env::var("KANARI_VALIDATE_OWNED_FASTPATH_SUPPLY")
                .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
                .unwrap_or(false);
            if validate_owned_fastpath {
                state_snapshot
                    .repair_cached_native_wallet_overcount()
                    .context(
                        "Failed to repair cached native wallet overcount before checkpoint state execution",
                    )?;
            }
        } else {
            state_snapshot
                .repair_legacy_native_wallet_overcount()
                .context(
                    "Failed to repair legacy native wallet overcount before checkpoint state execution",
                )?;
        }
        let state_arc = Arc::new(RwLock::new(state_snapshot));
        let cloned_at = std::time::Instant::now();
        let mut to_execute = Vec::new();
        for signed_tx in checkpoint.transactions.iter() {
            if !self.try_is_transaction_committed(signed_tx.transaction_hash())? {
                to_execute.push(signed_tx.clone());
            }
        }

        if !checkpoint.transactions.is_empty() {
            self.apply_system_prologue_to_state(&state_arc, checkpoint.timestamp, false)?;
        }
        let prologue_at = std::time::Instant::now();

        let (_, _, transaction_effects) = if assume_conflict_free {
            self.execute_conflict_free_transactions_parallel_with_effects(
                to_execute.clone(),
                &state_arc,
                Some(checkpoint.timestamp),
                false,
            )?
        } else {
            self.execute_tx_waves_deterministic_parallel_with_effects(
                to_execute.clone(),
                &state_arc,
                Some(checkpoint.timestamp),
                false, // persist_objects = false
            )?
        };
        let executed_at = std::time::Instant::now();

        {
            let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
            if assume_conflict_free {
                let validate_owned_fastpath =
                    std::env::var("KANARI_VALIDATE_OWNED_FASTPATH_SUPPLY")
                        .map(|value| {
                            matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES")
                        })
                        .unwrap_or(false);
                if validate_owned_fastpath {
                    state_write
                        .repair_cached_native_wallet_overcount()
                        .context("Failed to repair cached native wallet overcount after checkpoint state execution")?;
                }
            } else {
                state_write
                    .repair_legacy_native_wallet_overcount()
                    .context("Failed to repair legacy native wallet overcount after checkpoint state execution")?;
            }
        }
        let repaired_at = std::time::Instant::now();

        let verified_state = state_arc.read().unwrap_or_else(|e| e.into_inner()).clone();
        let (computed_root, smt_changes) =
            verified_state.try_compute_state_root_with_smt_changes()?;
        let rooted_at = std::time::Instant::now();
        if profile {
            eprintln!(
                "checkpoint prepare profile: txs={} clone_repair={:.6}s prologue={:.6}s execute={:.6}s repair={:.6}s root={:.6}s total={:.6}s",
                checkpoint.transactions.len(),
                cloned_at.duration_since(started_at).as_secs_f64(),
                prologue_at.duration_since(cloned_at).as_secs_f64(),
                executed_at.duration_since(prologue_at).as_secs_f64(),
                repaired_at.duration_since(executed_at).as_secs_f64(),
                rooted_at.duration_since(repaired_at).as_secs_f64(),
                rooted_at.duration_since(started_at).as_secs_f64(),
            );
        }
        Ok(PreparedCheckpointState {
            state_root: computed_root,
            smt_changes,
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
        precomputed_smt_changes: Option<PrecomputedSmtChanges>,
        validate_supply: bool,
    ) -> Result<()> {
        let profile = matches!(
            std::env::var("KANARI_CHECKPOINT_FINALIZE_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let started_at = std::time::Instant::now();
        let has_module_publish = checkpoint.transactions.iter().any(|signed_tx| {
            matches!(
                signed_tx.transaction,
                kanari_types::transaction::Transaction::PublishModule { .. }
                    | kanari_types::transaction::Transaction::PublishPackage { .. }
                    | kanari_types::transaction::Transaction::UpgradeModule { .. }
                    | kanari_types::transaction::Transaction::UpgradePackage { .. }
            )
        });
        let inspected_at = std::time::Instant::now();
        if validate_supply {
            new_state
                .validate_supply_invariants()
                .context("Supply invariants failed before checkpoint commit")?;
        }
        let validated_at = std::time::Instant::now();
        let inline_checkpoint_metadata = self.persistent_store.as_ref().is_some_and(|store| {
            !Self::persist_blockchain_json_snapshot_enabled(store)
                && Self::history_retention_checkpoints().is_none()
        });

        {
            // Keep the previous live state installed until the staged state and
            // durable recovery marker have committed successfully. A RocksDB
            // failure must not leave RPC/execution observing a new in-memory
            // state paired with old checkpoint metadata.
            let mut live_state = self.state_write();
            let mut committed_state = new_state;
            if let Some(store) = &self.persistent_store {
                let mut raw_deletes = Vec::new();
                let mut raw_updates = if inline_checkpoint_metadata {
                    let (updates, deletes) =
                        Self::checkpoint_persistence_raw_changes(store, &checkpoint, true)
                            .context("Failed to encode checkpoint metadata for state commit")?;
                    raw_deletes = deletes;
                    updates
                } else {
                    Vec::new()
                };
                let marker = bcs::to_bytes(&checkpoint)
                    .context("Failed to serialize durable checkpoint commit marker")?;
                raw_updates.push((Self::pending_checkpoint_commit_key().to_vec(), marker));
                committed_state
                    .commit_with_raw_changes_verified_root_and_smt_changes(
                        raw_updates,
                        raw_deletes,
                        &checkpoint.state_root,
                        precomputed_smt_changes,
                    )
                    .context("Failed to commit state and checkpoint marker to RocksDB")?;
            } else {
                committed_state
                    .commit()
                    .context("Failed to commit state to RocksDB")?;
            }
            *live_state = committed_state;
        }
        let committed_at = std::time::Instant::now();

        for runtime in &self.runtime_pool {
            runtime.clear_object_cache()?;
        }
        let cache_cleared_at = std::time::Instant::now();

        self.finalize_checkpoint_metadata(checkpoint, inline_checkpoint_metadata)?;
        let metadata_at = std::time::Instant::now();

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
        let durable_marker_at = std::time::Instant::now();

        if profile {
            eprintln!(
                "checkpoint finalize profile: inspect={:.6}s validate={:.6}s commit={:.6}s clear_cache={:.6}s metadata_mempool={:.6}s durable_marker={:.6}s total={:.6}s",
                inspected_at.duration_since(started_at).as_secs_f64(),
                validated_at.duration_since(inspected_at).as_secs_f64(),
                committed_at.duration_since(validated_at).as_secs_f64(),
                cache_cleared_at.duration_since(committed_at).as_secs_f64(),
                metadata_at.duration_since(cache_cleared_at).as_secs_f64(),
                durable_marker_at.duration_since(metadata_at).as_secs_f64(),
                durable_marker_at.duration_since(started_at).as_secs_f64(),
            );
        }

        Ok(())
    }

    fn finalize_checkpoint_metadata(
        &self,
        checkpoint: Checkpoint,
        checkpoint_metadata_already_persisted: bool,
    ) -> Result<()> {
        let profile = matches!(
            std::env::var("KANARI_CHECKPOINT_METADATA_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let started_at = std::time::Instant::now();
        let tx_count = checkpoint.transactions.len();
        // 1. Update blockchain metadata in-memory.
        {
            let mut chain = self.blockchain.write().unwrap_or_else(|e| e.into_inner());
            chain.add_checkpoint_with_validation(checkpoint.clone(), true)?;
        }
        let chain_added_at = std::time::Instant::now();

        // 2. Persist blockchain state before draining the live mempool view.
        if self.persistent_store.is_some() && !checkpoint_metadata_already_persisted {
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
        let persisted_at = std::time::Instant::now();

        // 3. Remove committed transactions from pending pool.
        {
            let mut mempool = self.mempool_write();
            let committed_hashes_vec = checkpoint
                .transactions
                .iter()
                .map(|tx| tx.transaction_hash().to_vec())
                .collect::<Vec<_>>();
            let can_clear_all_pending = mempool.pending_txs.len() == committed_hashes_vec.len()
                && mempool
                    .pending_txs
                    .iter()
                    .zip(committed_hashes_vec.iter())
                    .all(|(pending, committed_hash)| pending.tx_hash == *committed_hash);
            if can_clear_all_pending {
                mempool.pending_txs.clear();
                mempool.pending_tx_hashes.clear();
                mempool.pending_sender_counts.clear();
                mempool.pending_access_counts.clear();
                mempool.pending_primary_access_counts.clear();
                mempool.pending_congestion_access_counts.clear();
                mempool.pending_bytes = 0;
            } else {
                let committed_hashes: ahash::AHashSet<_> =
                    committed_hashes_vec.into_iter().collect();
                let removed_metadata = mempool
                    .pending_txs
                    .iter()
                    .filter(|tx| committed_hashes.contains(&tx.tx_hash))
                    .map(|tx| {
                        (
                            tx.normalized_sender.clone(),
                            tx.primary_access_key.clone(),
                            tx.congestion_access_key.clone(),
                            tx.access_keys.clone(),
                            tx.size_bytes,
                        )
                    })
                    .collect::<Vec<_>>();
                let removed_bytes = removed_metadata
                    .iter()
                    .fold(0usize, |total, (_, _, _, _, size_bytes)| {
                        total.saturating_add(*size_bytes)
                    });
                mempool
                    .pending_txs
                    .retain(|tx| !committed_hashes.contains(&tx.tx_hash));
                mempool.pending_bytes = mempool.pending_bytes.saturating_sub(removed_bytes);
                mempool
                    .pending_tx_hashes
                    .retain(|hash| !committed_hashes.contains(hash));
                for (sender, primary_access_key, congestion_access_key, access_keys, _) in
                    removed_metadata
                {
                    Self::decrement_pending_count(&mut mempool.pending_sender_counts, &sender);
                    Self::decrement_pending_count(
                        &mut mempool.pending_primary_access_counts,
                        &primary_access_key,
                    );
                    Self::decrement_pending_count(
                        &mut mempool.pending_congestion_access_counts,
                        &congestion_access_key,
                    );
                    Self::decrement_pending_count(
                        &mut mempool.pending_access_counts,
                        &primary_access_key,
                    );
                    for access_key in access_keys {
                        Self::decrement_pending_count(
                            &mut mempool.pending_access_counts,
                            &access_key,
                        );
                    }
                }
            }
        }
        let mempool_drained_at = std::time::Instant::now();

        if profile {
            eprintln!(
                "checkpoint metadata profile: txs={} chain_add={:.6}s persist_snapshot={:.6}s mempool_drain={:.6}s total={:.6}s",
                tx_count,
                chain_added_at.duration_since(started_at).as_secs_f64(),
                persisted_at.duration_since(chain_added_at).as_secs_f64(),
                mempool_drained_at
                    .duration_since(persisted_at)
                    .as_secs_f64(),
                mempool_drained_at.duration_since(started_at).as_secs_f64(),
            );
        }

        Ok(())
    }
    pub(crate) fn apply_prepared_checkpoint(
        &self,
        checkpoint: Checkpoint,
        verified_state: StateManager,
        smt_changes: Option<PrecomputedSmtChanges>,
        to_execute: Vec<SignedTransaction>,
        validate_supply: bool,
    ) -> Result<()> {
        if !to_execute.is_empty() && Self::requires_runtime_side_effect_persistence(&to_execute) {
            tracing::debug!(
                checkpoint = checkpoint.sequence,
                "Skipping checkpoint side-effect replay: verified_state commit is the canonical source of truth, and replaying persistence against the same backing store can skew object versions during restart/recovery paths."
            );
        }

        self.finalize_checkpoint(checkpoint, verified_state, smt_changes, validate_supply)
    }

    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {
        info!(
            "[ENGINE] Applying checkpoint {} with {} txs",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        let prepared = self.prepare_checkpoint_state(&checkpoint)?;
        self.ensure_checkpoint_root_matches(&checkpoint, &prepared.state_root)?;

        self.apply_prepared_checkpoint(
            checkpoint,
            prepared.state,
            prepared.smt_changes,
            prepared.transactions,
            true,
        )
    }
}
