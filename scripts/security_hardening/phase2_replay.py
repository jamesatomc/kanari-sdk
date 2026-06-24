from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "crates/kanari-core/src/engine/mempool.rs"
    text = read(path)
    text = text.replace(
        "type VerifiedMempoolTransaction = (SignedTransaction, Vec<u8>, String, u64);",
        '''type VerifiedMempoolTransaction = (SignedTransaction, Vec<u8>, String, u64);

const MAX_TRANSACTION_BYTES: usize = 256 * 1024;
const MAX_TRANSACTION_ARGS: usize = 128;
const MAX_TRANSACTION_ARG_BYTES: usize = 64 * 1024;
const MAX_TRANSACTION_TYPE_ARGS: usize = 32;''',
        1,
    )
    helper = '''
    fn persisted_transaction_exists(&self, tx_hash: &[u8]) -> bool {
        let Some(store) = &self.persistent_store else {
            return false;
        };
        let mut key = b"tx_index/".to_vec();
        key.extend_from_slice(hex::encode(tx_hash).as_bytes());
        store
            .logical_entries()
            .ok()
            .is_some_and(|entries| entries.iter().any(|(entry_key, _)| entry_key == &key))
    }

    fn validate_transaction_shape(tx: &Transaction) -> Result<()> {
        let encoded = bcs::to_bytes(tx)?;
        anyhow::ensure!(encoded.len() <= MAX_TRANSACTION_BYTES, "transaction exceeds byte limit");
        if let Transaction::ExecuteFunction { type_args, args, .. } = tx {
            anyhow::ensure!(type_args.len() <= MAX_TRANSACTION_TYPE_ARGS, "too many type arguments");
            anyhow::ensure!(args.len() <= MAX_TRANSACTION_ARGS, "too many transaction arguments");
            anyhow::ensure!(
                args.iter().all(|arg| arg.len() <= MAX_TRANSACTION_ARG_BYTES),
                "transaction argument exceeds byte limit"
            );
        }
        anyhow::ensure!(
            !tx.is_legacy_native_balance_call(),
            "legacy account-ledger KANARI transfer/burn is disabled; use a Coin<KANARI> object call"
        );
        Ok(())
    }
'''
    text = text.replace("impl BlockchainEngine {", "impl BlockchainEngine {" + helper, 1)
    text = text.replace(
        '''                Self::validate_transaction_gas(verified.transaction())?;
                let tx_hash = verified.hash().to_vec();''',
        '''                Self::validate_transaction_gas(verified.transaction())?;
                Self::validate_transaction_shape(verified.transaction())?;
                let tx_hash = verified.hash().to_vec();''',
        1,
    )
    text = text.replace(
        '''            if chain.is_transaction_hash_executed(tx_hash) {
                anyhow::bail!("Transaction {} already executed", hex::encode(tx_hash));
            }''',
        '''            if chain.is_transaction_hash_executed(tx_hash) || self.persisted_transaction_exists(tx_hash) {
                anyhow::bail!("Transaction {} already executed", hex::encode(tx_hash));
            }''',
        1,
    )
    write(path, text)

    path = "crates/kanari-core/src/engine/apply_checkpoint.rs"
    text = read(path)
    text = text.replace(
        "use std::sync::{Arc, RwLock};",
        "use std::collections::{BTreeMap, HashSet};\nuse std::sync::{Arc, RwLock};",
        1,
    )
    helper = '''
    fn checkpoint_persisted_transaction_exists(&self, tx_hash: &[u8]) -> bool {
        let Some(store) = &self.persistent_store else {
            return false;
        };
        let mut key = b"tx_index/".to_vec();
        key.extend_from_slice(hex::encode(tx_hash).as_bytes());
        store
            .logical_entries()
            .ok()
            .is_some_and(|entries| entries.iter().any(|(entry_key, _)| entry_key == &key))
    }

    fn validate_checkpoint_transactions(
        &self,
        checkpoint: &Checkpoint,
        state: &StateManager,
    ) -> Result<()> {
        const MAX_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
        let config = kanari_types::GasConfig::default();
        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        let mut total_declared_gas = 0u64;
        let mut total_bytes = 0usize;
        let mut seen = HashSet::new();
        let mut expected_sequences: BTreeMap<move_core_types::account_address::AccountAddress, u64> = BTreeMap::new();

        for signed_tx in checkpoint.transactions.iter() {
            let tx_hash = signed_tx.verified_transaction_hash()?;
            anyhow::ensure!(seen.insert(tx_hash.clone()), "duplicate transaction in checkpoint");
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

            let sender = kanari_types::address::Address::parse_to_account_address(tx.sender_address())?;
            let expected = expected_sequences.entry(sender).or_insert_with(|| {
                state.get_account(&sender).map(|account| account.sequence_number).unwrap_or(0)
            });
            anyhow::ensure!(
                tx.sequence_number() == *expected,
                "invalid checkpoint sequence for {}: expected {}, got {}",
                sender,
                *expected,
                tx.sequence_number()
            );
            *expected = expected.checked_add(1).ok_or_else(|| anyhow::anyhow!("sequence overflow"))?;
        }
        anyhow::ensure!(total_declared_gas <= config.max_gas_per_block, "checkpoint exceeds block gas limit");
        anyhow::ensure!(total_bytes <= MAX_CHECKPOINT_BYTES, "checkpoint exceeds byte limit");
        Ok(())
    }
'''
    text = text.replace("impl BlockchainEngine {", "impl BlockchainEngine {" + helper, 1)
    old = '''        let state_snapshot = self.state_read().clone();
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
        };'''
    new = '''        let state_snapshot = self.state_read().clone();
        self.validate_checkpoint_transactions(checkpoint, &state_snapshot)?;
        let state_arc = Arc::new(RwLock::new(state_snapshot));
        let to_execute: Vec<SignedTransaction> = checkpoint.transactions.iter().cloned().collect();'''
    if old not in text:
        raise RuntimeError("prepare checkpoint transaction block not found")
    text = text.replace(old, new, 1)

    text = text.replace(
        '''        if validate_supply {
            new_state
                .validate_supply_invariants()
                .context("Supply invariants failed before checkpoint commit")?;
        }

        {''',
        '''        if validate_supply {
            new_state
                .validate_supply_invariants()
                .context("Supply invariants failed before checkpoint commit")?;
        }

        let commit_store = new_state.store.clone();
        commit_store
            .save(b"pending_checkpoint_commit", &checkpoint)
            .context("Failed to persist checkpoint commit journal")?;

        {''',
        1,
    )
    text = text.replace(
        '''        self.finalize_checkpoint_metadata(checkpoint)
    }''',
        '''        self.finalize_checkpoint_metadata(checkpoint)?;
        commit_store
            .delete(b"pending_checkpoint_commit")
            .context("Failed to clear checkpoint commit journal")?;
        Ok(())
    }''',
        1,
    )
    write(path, text)

    path = "crates/kanari-core/src/blockchain.rs"
    text = read(path)
    text = text.replace(
        '''        if self.dag_checkpoints.len() > MAX_RETAINED_BLOCKS
            && let Some(evicted) = self.dag_checkpoints.pop_front()
        {
            for tx in evicted.transactions.iter() {
                let hash = tx.transaction_hash().to_vec();
                self.executed_tx_hashes.remove(&hash);
                self.tx_location_index.remove(&hash);
            }
        }''',
        '''        if self.dag_checkpoints.len() > MAX_RETAINED_BLOCKS {
            self.dag_checkpoints.pop_front();
            // Block bodies may be pruned, but replay markers are never removed here.
            // Durable tx_index entries remain authoritative across restarts.
        }''',
        1,
    )
    write(path, text)
