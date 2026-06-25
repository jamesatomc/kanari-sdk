// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::{
    BlockchainEngine, MAX_PERSISTED_RECENT_TX_HASHES, PersistedTransactionLocation,
    TransactionExecutionReceipt,
};
use crate::{blockchain::Blockchain, consensus::Checkpoint};
use anyhow::{Context, Result, bail};
use kanari_move_runtime_v1::state::StateManager;
use kanari_types::transaction::SignedTransaction;
use log::info;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, RwLock};

type PreparedCheckpointState = (
    Vec<u8>,
    StateManager,
    Vec<SignedTransaction>,
    Vec<TransactionExecutionReceipt>,
);

impl BlockchainEngine {
    fn checkpoint_persisted_transaction_exists(&self, tx_hash: &[u8]) -> bool {
        let Some(store) = &self.persistent_store else {
            return false;
        };
        let mut key = b"tx_index/".to_vec();
        key.extend_from_slice(hex::encode(tx_hash).as_bytes());
        store.contains_key(&key).unwrap_or(false)
    }

    fn validate_checkpoint_transactions(
        &self,
        checkpoint: &Checkpoint,
        state: &StateManager,
    ) -> Result<()> {
        const MAX_CHECKPOINT_BYTES: usize = 128 * 1024;
        let config = kanari_types::GasConfig::default();
        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        let mut total_declared_gas = 0u64;
        let mut total_bytes = 0usize;
        let mut seen = HashSet::new();
        let mut expected_sequences: BTreeMap<
            move_core_types::account_address::AccountAddress,
            u64,
        > = BTreeMap::new();

        for signed_tx in checkpoint.transactions.iter() {
            let tx_hash = signed_tx.verified_transaction_hash()?;
            anyhow::ensure!(
                seen.insert(tx_hash.clone()),
                "duplicate transaction in checkpoint"
            );
            anyhow::ensure!(
                !chain.is_transaction_hash_executed(&tx_hash)
                    && !self.checkpoint_persisted_transaction_exists(&tx_hash),
                "checkpoint contains an already committed transaction"
            );
            let tx = &signed_tx.transaction;
            anyhow::ensure!(
                !tx.is_legacy_native_balance_call(),
                "checkpoint contains a disabled legacy native-balance call"
            );
            Self::validate_transaction_gas(tx)?;
            total_declared_gas = total_declared_gas
                .checked_add(tx.gas_limit())
                .ok_or_else(|| anyhow::anyhow!("checkpoint gas overflow"))?;
            total_bytes = total_bytes
                .checked_add(bcs::to_bytes(signed_tx)?.len())
                .ok_or_else(|| anyhow::anyhow!("checkpoint byte count overflow"))?;

            let sender =
                kanari_types::address::Address::parse_to_account_address(tx.sender_address())?;
            let expected = expected_sequences.entry(sender).or_insert_with(|| {
                state
                    .get_account(&sender)
                    .map(|account| account.sequence_number)
                    .unwrap_or(0)
            });
            anyhow::ensure!(
                tx.sequence_number() == *expected,
                "invalid checkpoint sequence for {}: expected {}, got {}",
                sender,
                *expected,
                tx.sequence_number()
            );
            *expected = expected
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("sequence overflow"))?;
        }
        anyhow::ensure!(
            total_declared_gas <= config.max_gas_per_block,
            "checkpoint exceeds block gas limit"
        );
        anyhow::ensure!(
            total_bytes <= MAX_CHECKPOINT_BYTES,
            "checkpoint exceeds byte limit"
        );
        Ok(())
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
    ) -> Result<PreparedCheckpointState> {
        let state_snapshot = self.state_read().clone();
        self.validate_checkpoint_transactions(checkpoint, &state_snapshot)?;
        let state_arc = Arc::new(RwLock::new(state_snapshot));
        let to_execute: Vec<SignedTransaction> = checkpoint.transactions.iter().cloned().collect();

        if !checkpoint.transactions.is_empty() {
            self.apply_system_prologue_to_state(&state_arc, checkpoint.timestamp, false)?;
        }

        let execution = self.execute_tx_waves_strict_serial_with_receipts(
            to_execute.clone(),
            &state_arc,
            Some(checkpoint.timestamp),
            false, // persist_objects = false
        )?;

        let verified_state = state_arc.read().unwrap_or_else(|e| e.into_inner()).clone();
        let computed_root = verified_state.compute_state_root();
        Ok((
            computed_root,
            verified_state,
            to_execute,
            execution.receipts,
        ))
    }

    fn atomic_checkpoint_updates(
        &self,
        checkpoint: &Checkpoint,
        receipts: &[TransactionExecutionReceipt],
        next_chain: &Blockchain,
        store: &kanari_move_runtime_v1::storage::persistent_store::PersistentStore,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut updates = Vec::new();
        updates.push((
            Self::checkpoint_metadata_key(checkpoint.sequence),
            bcs::to_bytes(&Self::checkpoint_without_transactions(checkpoint))?,
        ));
        if checkpoint.sequence > 0 && !checkpoint.transactions.is_empty() {
            updates.push((
                Self::checkpoint_transactions_key(checkpoint.sequence),
                bcs::to_bytes(&checkpoint.transactions)?,
            ));
        }

        let mut recent_hashes = store
            .load::<Vec<Vec<u8>>>(Self::recent_transaction_hashes_key())?
            .unwrap_or_default();
        let mut recent_set: HashSet<Vec<u8>> = recent_hashes.iter().cloned().collect();
        for transaction in checkpoint.transactions.iter() {
            let transaction_hash = transaction.transaction_hash().to_vec();
            updates.push((
                Self::transaction_payload_key(&transaction_hash),
                bcs::to_bytes(transaction)?,
            ));
            updates.push((
                Self::transaction_index_key(&transaction_hash),
                bcs::to_bytes(&PersistedTransactionLocation {
                    checkpoint_sequence: checkpoint.sequence,
                    state_root: checkpoint.state_root.clone(),
                })?,
            ));
            if recent_set.insert(transaction_hash.clone()) {
                recent_hashes.push(transaction_hash);
            }
        }
        if recent_hashes.len() > MAX_PERSISTED_RECENT_TX_HASHES {
            let remove = recent_hashes.len() - MAX_PERSISTED_RECENT_TX_HASHES;
            recent_hashes.drain(0..remove);
        }
        updates.push((
            Self::recent_transaction_hashes_key().to_vec(),
            bcs::to_bytes(&recent_hashes)?,
        ));
        for receipt in receipts {
            updates.push((
                Self::transaction_receipt_key(&receipt.transaction_hash),
                bcs::to_bytes(receipt)?,
            ));
        }

        let mut slim_chain = next_chain.clone();
        for persisted in &mut slim_chain.dag_checkpoints {
            *persisted = Self::checkpoint_without_transactions(persisted);
        }
        updates.push((b"blockchain".to_vec(), bcs::to_bytes(&slim_chain)?));
        Ok(updates)
    }

    /// Atomically commit state, transaction indexes, receipts and checkpoint metadata.
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

        let mut next_chain = self
            .blockchain
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        next_chain.add_checkpoint_with_validation(checkpoint.clone(), true)?;
        let updates = self.atomic_checkpoint_updates(
            &checkpoint,
            &receipts,
            &next_chain,
            new_state.store.as_ref(),
        )?;
        new_state
            .commit_with_extra_raw_changes(&updates, &[b"pending_checkpoint_commit".to_vec()])
            .context("Failed to atomically commit checkpoint state and metadata")?;

        {
            let mut state = self.state_write();
            *state = new_state;
        }
        {
            let mut chain = self
                .blockchain
                .write()
                .unwrap_or_else(|error| error.into_inner());
            *chain = next_chain;
        }
        for runtime in &self.runtime_pool {
            if let Err(error) = runtime.clear_object_cache() {
                log::error!(
                    "Failed to clear runtime object cache after committed checkpoint: {}",
                    error
                );
            }
            if let Err(error) = runtime.reload_vm_cache() {
                log::error!(
                    "Failed to reload runtime cache after committed checkpoint: {}",
                    error
                );
            }
        }

        let mut mempool = self.mempool_write();
        let committed_hashes: HashSet<_> = checkpoint
            .transactions
            .iter()
            .map(|transaction| transaction.transaction_hash().to_vec())
            .collect();
        mempool
            .pending_txs
            .retain(|transaction| !committed_hashes.contains(transaction.transaction_hash()));
        mempool
            .pending_tx_hashes
            .retain(|hash| !committed_hashes.contains(hash));
        Self::remove_pending_sender_counts(
            &mut mempool.pending_sender_counts,
            checkpoint.transactions.as_ref(),
        );
        Ok(())
    }

    pub(crate) fn apply_prepared_checkpoint(
        &self,
        checkpoint: Checkpoint,
        verified_state: StateManager,
        _to_execute: Vec<SignedTransaction>,
        receipts: Vec<TransactionExecutionReceipt>,
        validate_supply: bool,
    ) -> Result<()> {
        self.finalize_checkpoint(checkpoint, verified_state, receipts, validate_supply)
    }

    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {
        checkpoint.verify_certificate(&self.consensus_public_keys, self.authorities.len())?;
        info!(
            "[ENGINE] Applying checkpoint {} with {} txs",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        let (computed_root, verified_state, to_execute, receipts) =
            self.prepare_checkpoint_state(&checkpoint)?;
        self.ensure_checkpoint_root_matches(&checkpoint, &computed_root)?;

        self.apply_prepared_checkpoint(checkpoint, verified_state, to_execute, receipts, true)
    }
}
