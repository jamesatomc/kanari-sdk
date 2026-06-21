// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::BlockchainEngine;
use crate::consensus::{Checkpoint, CheckpointCertificate};
use anyhow::{Context, Result, bail};
use kanari_move_runtime_v1::state::StateManager;
use kanari_types::transaction::SignedTransaction;
use log::info;
use std::sync::{Arc, RwLock};

impl BlockchainEngine {
    fn requires_runtime_side_effect_persistence(transactions: &[SignedTransaction]) -> bool {
        transactions.iter().any(|signed_tx| {
            if signed_tx.transaction.is_native_balance_call() {
                return false;
            }

            matches!(
                signed_tx.transaction,
                kanari_types::transaction::Transaction::PublishModule { .. }
                    | kanari_types::transaction::Transaction::ExecuteFunction { .. }
            )
        })
    }

    fn apply_system_prologue_to_state(
        &self,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp_ms: u64,
        persist_objects: bool,
    ) -> Result<()> {
        let runtime = &self.runtime_pool[0];
        let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
        let clock_id = runtime.ensure_system_clock(&mut state_write)?;
        let changeset = runtime.execute_clock_consensus_commit_prologue(clock_id, timestamp_ms)?;
        state_write.apply_changeset(&changeset)?;

        if persist_objects {
            runtime.persist_created_objects(&changeset);
            runtime.persist_deleted_objects(&changeset);
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
    ) -> Result<(Vec<u8>, StateManager, Vec<SignedTransaction>)> {
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
            self.apply_system_prologue_to_state(&state_arc, checkpoint.timestamp, false)?;
        }

        self.execute_tx_waves_strict_serial(
            to_execute.clone(),
            &state_arc,
            Some(checkpoint.timestamp),
            false, // persist_objects = false
        )?;

        let verified_state = state_arc.read().unwrap_or_else(|e| e.into_inner()).clone();
        let computed_root = verified_state.compute_state_root();
        Ok((computed_root, verified_state, to_execute))
    }

    /// Helper: Common steps for finalizing Checkpoint to database
    fn finalize_checkpoint(
        &self,
        checkpoint: Checkpoint,
        certificate: Option<CheckpointCertificate>,
        new_state: StateManager,
        validate_supply: bool,
    ) -> Result<()> {
        if validate_supply {
            new_state
                .validate_supply_invariants()
                .context("Supply invariants failed before checkpoint commit")?;
        }

        if let Some(store) = &self.persistent_store {
            Self::persist_pending_checkpoint_journal(store, &checkpoint, certificate.as_ref())?;
        }

        {
            let mut state = self.state_write();
            *state = new_state;
            state
                .commit()
                .context("Failed to commit state to RocksDB")?;
        }

        for runtime in &self.runtime_pool {
            runtime.clear_object_cache()?;
        }

        self.finalize_checkpoint_metadata(checkpoint.clone(), certificate)?;

        if let Some(store) = &self.persistent_store {
            Self::clear_pending_checkpoint_journal(store)?;
        }

        Ok(())
    }

    fn finalize_checkpoint_metadata(
        &self,
        checkpoint: Checkpoint,
        certificate: Option<CheckpointCertificate>,
    ) -> Result<()> {
        // 1. Update blockchain metadata in-memory.
        {
            let mut chain = self.blockchain.write().unwrap_or_else(|e| e.into_inner());
            chain.add_checkpoint_with_validation(checkpoint.clone(), true)?;
        }

        if let (Some(store), Some(certificate)) = (&self.persistent_store, &certificate) {
            Self::persist_checkpoint_certificate(store, certificate)?;
        }

        // 2. Persist blockchain state before draining the live mempool view.
        if self.persistent_store.is_some() {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = self.persist_blockchain_snapshot(&chain) {
                drop(chain);
                let mut rollback_chain = self.blockchain.write().unwrap_or_else(|e| e.into_inner());
                rollback_chain.rollback_latest_checkpoint(checkpoint.sequence);
                anyhow::bail!(
                    "Failed to persist blockchain metadata for checkpoint {}: {}",
                    checkpoint.sequence,
                    e
                );
            }
        }

        // 3. Remove committed transactions from pending pool.
        {
            let mut mempool = self.mempool_write();
            if mempool.pending_txs.len() == checkpoint.transactions.len() {
                mempool.pending_txs.clear();
                mempool.pending_tx_hashes.clear();
                mempool.pending_sender_counts.clear();
            } else {
                let committed_hashes: std::collections::HashSet<_> = checkpoint
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
        }

        Ok(())
    }
    pub(crate) fn apply_prepared_checkpoint(
        &self,
        checkpoint: Checkpoint,
        certificate: Option<CheckpointCertificate>,
        verified_state: StateManager,
        to_execute: Vec<SignedTransaction>,
        validate_supply: bool,
    ) -> Result<()> {
        if !to_execute.is_empty() && Self::requires_runtime_side_effect_persistence(&to_execute) {
            let side_effect_state = Arc::new(RwLock::new(self.state_read().clone()));
            self.apply_system_prologue_to_state(&side_effect_state, checkpoint.timestamp, true)?;
            self.execute_tx_waves_strict_serial(
                to_execute,
                &side_effect_state,
                Some(checkpoint.timestamp),
                true, // persist_objects = true
            )?;
        }

        self.finalize_checkpoint(checkpoint, certificate, verified_state, validate_supply)
    }

    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {
        info!(
            "[ENGINE] Applying checkpoint {} with {} txs",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        let (computed_root, verified_state, to_execute) =
            self.prepare_checkpoint_state(&checkpoint)?;
        self.ensure_checkpoint_root_matches(&checkpoint, &computed_root)?;

        self.apply_prepared_checkpoint(checkpoint, None, verified_state, to_execute, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::Checkpoint;
    use kanari_crypto::keys::{CurveType, generate_keypair};
    use kanari_move_runtime_v1::state::Account;
    use kanari_types::transaction::{SignedTransaction, Transaction};
    use kanari_types::{
        address::Address as KanariAddress, balance::BalanceRecord, kanari::KANARI_TOKEN_TYPE,
    };

    fn signed_transfer(sequence_number: u64) -> SignedTransaction {
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        let tx = Transaction::new_transfer(
            sender.tagged_address(),
            recipient.address,
            1,
            sequence_number,
        );
        let mut signed_tx = SignedTransaction::new(tx);
        signed_tx
            .sign(&sender.private_key, sender.curve_type)
            .unwrap();
        signed_tx
    }

    fn fund_sender(engine: &BlockchainEngine, address: &str, balance: u64) {
        let addr = KanariAddress::parse_to_account_address(address).unwrap();
        let mut account = Account::with_native_balance(addr, balance);
        account.set_token_balance(KANARI_TOKEN_TYPE.to_string(), BalanceRecord::new(balance));
        engine
            .state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .save_account(&account)
            .unwrap();
    }

    #[test]
    fn restart_recovers_checkpoint_metadata_from_pending_journal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let data_dir = temp_dir.path().to_str().unwrap();
        let engine = BlockchainEngine::new_dir(data_dir).unwrap();
        let tx = signed_transfer(0);
        fund_sender(&engine, tx.transaction.sender_address(), 1_000_000);
        let tx_hash = tx.transaction_hash().to_vec();
        let prev_hash = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let draft_checkpoint = Checkpoint::new(
            1,
            vec![[1u8; 32]],
            vec![tx.clone()],
            Vec::new(),
            42,
            prev_hash.clone(),
        );
        let (computed_root, verified_state, _) =
            engine.prepare_checkpoint_state(&draft_checkpoint).unwrap();
        let checkpoint =
            Checkpoint::new(1, vec![[1u8; 32]], vec![tx], computed_root, 42, prev_hash);

        let store = engine.persistent_store.as_ref().unwrap();
        BlockchainEngine::persist_pending_checkpoint_journal(store, &checkpoint, None).unwrap();
        {
            let mut state = engine.state_write();
            *state = verified_state;
            state.commit().unwrap();
        }
        drop(engine);

        let restarted = BlockchainEngine::new_dir(data_dir).unwrap();
        assert_eq!(restarted.get_stats().height, 1);
        let found = restarted.get_committed_transaction_from_history(&tx_hash);
        assert!(found.is_some());
        let store = restarted.persistent_store.as_ref().unwrap();
        assert!(
            BlockchainEngine::load_pending_checkpoint_journal(store)
                .unwrap()
                .is_none()
        );
    }
}
