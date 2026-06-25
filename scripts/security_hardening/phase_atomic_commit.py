from __future__ import annotations

import re
from .common import read, write


def apply() -> None:
    path = "move-execution/v1/kanari-move-runtime-v1/src/state.rs"
    text = read(path)
    text = text.replace(
        "let use_incremental_smt = if self.store.get_db().is_some() {\n                true",
        "let use_incremental_smt = if self.store.get_db().is_some() {\n                !self.smt_dirty",
        1,
    )

    pattern = re.compile(
        r"    /// Commit pending overlay changes to the persistent store and update SMT\n"
        r"    pub fn commit\(&mut self\) -> Result<\(\)> \{.*?\n"
        r"    \}\n\n"
        r"    // Helper to write to overlay",
        re.S,
    )
    replacement = '''    /// Commit canonical state and checkpoint metadata in one backend batch.
    pub fn commit_with_extra_raw_changes(
        &mut self,
        extra_updates: &[(Vec<u8>, Vec<u8>)],
        extra_deletes: &[Vec<u8>],
    ) -> Result<()> {
        let mut updates = Vec::with_capacity(self.overlay.len() + extra_updates.len());
        let mut deletes = Vec::with_capacity(self.overlay.len() + extra_deletes.len());
        for (key, value) in &self.overlay {
            match value {
                Some(value) => updates.push((key.clone(), value.clone())),
                None => deletes.push(key.clone()),
            }
        }
        updates.extend_from_slice(extra_updates);
        deletes.extend_from_slice(extra_deletes);

        let (smt_updates, smt_deletes) = self.smt_changes_from_pending_delta();
        self.store.apply_raw_changes(&updates, &deletes)?;

        if !self.smt_dirty {
            if let Some(smt) = &self.smt {
                let is_in_memory = self.store.get_db().is_none();
                let change_count = smt_updates.len().saturating_add(smt_deletes.len());
                if is_in_memory && change_count > IN_MEMORY_SMT_INCREMENTAL_THRESHOLD {
                    self.smt_dirty = true;
                } else if let Err(error) = (|| -> Result<()> {
                    if !smt_updates.is_empty() {
                        smt.insert(&smt_updates)?;
                    }
                    if !smt_deletes.is_empty() {
                        smt.delete(&smt_deletes)?;
                    }
                    Ok(())
                })() {
                    log::error!("SMT cache update failed after canonical batch commit: {}", error);
                    self.smt_dirty = true;
                }
            }
        }

        self.persisted_canonical_root_entries = self.canonical_root_entries.clone();
        self.pending_smt_changes.clear();
        self.overlay.clear();
        Ok(())
    }

    pub fn commit(&mut self) -> Result<()> {
        self.commit_with_extra_raw_changes(&[], &[])
    }

    // Helper to write to overlay'''
    text, count = pattern.subn(replacement, text, count=1)
    if count != 1:
        raise RuntimeError("StateManager commit method was not found")

    startup_marker = '''        state
            .ensure_smt_initialized()
            .context("Failed to initialize state SMT")?;
'''
    startup_replacement = startup_marker + '''
        if let Some(tree) = &state.smt {
            let expected = smt::compute_sparse_root(
                &state
                    .canonical_root_entries
                    .clone()
                    .into_iter()
                    .collect::<Vec<_>>(),
            );
            if tree.root_hash().map(|root| root.to_vec()).unwrap_or_default() != expected {
                log::warn!("Persisted SMT cache differs from canonical state; using materialized roots");
                state.smt_dirty = true;
            }
        }
'''
    if startup_marker not in text:
        raise RuntimeError("SMT startup marker was not found")
    text = text.replace(startup_marker, startup_replacement, 1)
    write(path, text)

    path = "crates/kanari-core/src/engine.rs"
    text = read(path)
    old = '''    fn checkpoint_without_transactions(checkpoint: &Checkpoint) -> Checkpoint {
        Checkpoint::new(
            checkpoint.sequence,
            checkpoint.vertices.clone(),
            Vec::new(),
            checkpoint.state_root.clone(),
            checkpoint.timestamp,
            checkpoint.prev_checkpoint_hash.clone(),
        )
    }'''
    new = '''    fn checkpoint_without_transactions(checkpoint: &Checkpoint) -> Checkpoint {
        let mut slim = checkpoint.clone();
        slim.transactions = Vec::new().into();
        slim
    }'''
    if old not in text:
        raise RuntimeError("checkpoint_without_transactions helper was not found")
    write(path, text.replace(old, new, 1))

    path = "crates/kanari-core/src/engine/apply_checkpoint.rs"
    text = read(path)
    text = text.replace(
        "use super::{BlockchainEngine, TransactionExecutionReceipt};",
        "use super::{BlockchainEngine, PersistedTransactionLocation, TransactionExecutionReceipt, MAX_PERSISTED_RECENT_TX_HASHES};",
        1,
    )
    text = text.replace(
        "use crate::consensus::Checkpoint;",
        "use crate::{blockchain::Blockchain, consensus::Checkpoint};",
        1,
    )
    text, removed = re.subn(
        r"\n    fn requires_runtime_side_effect_persistence\(transactions: &\[SignedTransaction\]\) -> bool \{.*?\n    \}\n",
        "\n",
        text,
        count=1,
        flags=re.S,
    )
    if removed != 1:
        raise RuntimeError("runtime side-effect helper was not found")

    marker = "    /// Helper: Common steps for finalizing Checkpoint to database\n"
    helper = '''    fn atomic_checkpoint_updates(
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

'''
    if marker not in text:
        raise RuntimeError("checkpoint finalization marker was not found")
    text = text.replace(marker, helper + marker, 1)

    block_pattern = re.compile(
        r"    /// Helper: Common steps for finalizing Checkpoint to database\n"
        r"    fn finalize_checkpoint\(.*?\n"
        r"    fn finalize_checkpoint_metadata\(.*?\n"
        r"    \}\n"
        r"    pub\(crate\) fn apply_prepared_checkpoint",
        re.S,
    )
    block_replacement = '''    /// Atomically commit state, transaction indexes, receipts and checkpoint metadata.
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
            .commit_with_extra_raw_changes(
                &updates,
                &[b"pending_checkpoint_commit".to_vec()],
            )
            .context("Failed to atomically commit checkpoint state and metadata")?;

        {
            let mut state = self.state_write();
            *state = new_state;
        }
        {
            let mut chain = self.blockchain.write().unwrap_or_else(|error| error.into_inner());
            *chain = next_chain;
        }
        for runtime in &self.runtime_pool {
            if let Err(error) = runtime.clear_object_cache() {
                log::error!("Failed to clear runtime object cache after committed checkpoint: {}", error);
            }
            if let Err(error) = runtime.reload_vm_cache() {
                log::error!("Failed to reload runtime cache after committed checkpoint: {}", error);
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

    pub(crate) fn apply_prepared_checkpoint'''
    text, count = block_pattern.subn(block_replacement, text, count=1)
    if count != 1:
        raise RuntimeError("checkpoint finalization block was not found")

    prepared_pattern = re.compile(
        r"    pub\(crate\) fn apply_prepared_checkpoint\(\n"
        r"        &self,\n"
        r"        checkpoint: Checkpoint,\n"
        r"        verified_state: StateManager,\n"
        r"        to_execute: Vec<SignedTransaction>,\n"
        r"        receipts: Vec<TransactionExecutionReceipt>,\n"
        r"        validate_supply: bool,\n"
        r"    \) -> Result<\(\)> \{.*?\n"
        r"        self\.finalize_checkpoint\(checkpoint, verified_state, receipts, validate_supply\)\n"
        r"    \}",
        re.S,
    )
    prepared_replacement = '''    pub(crate) fn apply_prepared_checkpoint(
        &self,
        checkpoint: Checkpoint,
        verified_state: StateManager,
        _to_execute: Vec<SignedTransaction>,
        receipts: Vec<TransactionExecutionReceipt>,
        validate_supply: bool,
    ) -> Result<()> {
        self.finalize_checkpoint(checkpoint, verified_state, receipts, validate_supply)
    }'''
    text, count = prepared_pattern.subn(prepared_replacement, text, count=1)
    if count != 1:
        raise RuntimeError("prepared checkpoint block was not found")
    write(path, text)
