// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use ahash::AHashSet;

type VerifiedMempoolTransaction = (SignedTransaction, Vec<u8>, String, u64);

impl BlockchainEngine {
    pub fn submit_transactions_batch(
        &self,
        signed_txs: Vec<SignedTransaction>,
    ) -> Result<Vec<Vec<u8>>> {
        if signed_txs.is_empty() {
            return Ok(Vec::new());
        }

        // This is only an early capacity check. The authoritative check is performed
        // again while holding the blockchain read lock and mempool write lock so a
        // checkpoint cannot commit between validation and insertion.
        let batch_size = signed_txs.len();
        if self
            .mempool_read()
            .pending_txs
            .len()
            .saturating_add(batch_size)
            > MAX_MEMPOOL_SIZE
        {
            log::warn!("[MEMPOOL] Rejecting batch: Queue would exceed max size");
            anyhow::bail!("Mempool is currently full. Please try again later.");
        }

        let mut sender_cache = ahash::AHashMap::with_capacity(batch_size);
        for signed_tx in &signed_txs {
            let sender = signed_tx.transaction.sender_address();
            sender_cache
                .entry(sender.to_string())
                .or_insert_with(|| Self::normalize_addr(sender));
        }

        // Hash, verify, and extract metadata in one parallel pass.
        let mut verified_txs = signed_txs
            .into_par_iter()
            .map(|signed_tx| -> Result<VerifiedMempoolTransaction> {
                let verified = signed_tx.into_verified()?;
                Self::validate_transaction_gas(verified.transaction())?;
                let tx_hash = verified.hash().to_vec();
                let sender = verified.transaction().sender_address();
                let normalized_sender = sender_cache
                    .get(sender)
                    .expect("sender cache must contain every batch sender")
                    .clone();
                let sequence_number = verified.transaction().sequence_number();
                Ok((
                    verified.into_signed_transaction(),
                    tx_hash,
                    normalized_sender,
                    sequence_number,
                ))
            })
            .collect::<Result<Vec<_>>>()?;

        verified_txs.sort_by(|a, b| {
            a.2.cmp(&b.2)
                .then_with(|| a.3.cmp(&b.3))
                .then_with(|| a.1.cmp(&b.1))
        });

        self.admit_verified_transactions(verified_txs)
    }

    /// Atomically revalidate and insert a verified transaction batch.
    ///
    /// The blockchain read guard is intentionally held until after the mempool write.
    /// Checkpoint finalization must acquire the blockchain write lock before draining
    /// committed transactions from the mempool. This lock ordering guarantees one of
    /// two outcomes for a racing transaction/checkpoint:
    ///
    /// 1. the transaction is inserted first and the checkpoint subsequently removes it; or
    /// 2. the checkpoint commits first and this method rejects the already-executed hash.
    ///
    /// Without this final guarded validation, a transaction gossip message can pass an
    /// earlier executed-hash check, get committed concurrently, and then be written back
    /// into the mempool after the checkpoint drain. The stale transaction is later
    /// speculatively executed again and can violate the native supply invariant.
    pub(crate) fn admit_verified_transactions(
        &self,
        verified_txs: Vec<VerifiedMempoolTransaction>,
    ) -> Result<Vec<Vec<u8>>> {
        if verified_txs.is_empty() {
            return Ok(Vec::new());
        }

        let batch_size = verified_txs.len();
        let mut batch_hashes = AHashSet::with_capacity(batch_size);
        let mut accepted_hashes = Vec::with_capacity(batch_size);
        let mut accepted_counts_by_sender = ahash::AHashMap::new();
        let mut sequence_groups = ahash::AHashMap::new();

        for (_, tx_hash, sender, tx_seq) in &verified_txs {
            if !batch_hashes.insert(tx_hash.clone()) {
                anyhow::bail!(
                    "Transaction {} is duplicated in submitted batch",
                    hex::encode(tx_hash)
                );
            }

            accepted_hashes.push(tx_hash.clone());
            *accepted_counts_by_sender.entry(sender.clone()).or_insert(0) += 1;
            sequence_groups
                .entry(sender.clone())
                .or_insert_with(Vec::new)
                .push(*tx_seq);
        }

        for tx_sequences in sequence_groups.values_mut() {
            tx_sequences.sort_unstable();
        }

        // Keep this guard alive through insertion. A checkpoint cannot update the
        // executed transaction index until this read guard is released.
        let chain = match self.blockchain.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Blockchain lock poisoned in mempool admission, recovering...");
                poisoned.into_inner()
            }
        };
        let state = self.state_read();
        let mut mempool = self.mempool_write();

        if mempool.pending_txs.len().saturating_add(batch_size) > MAX_MEMPOOL_SIZE {
            anyhow::bail!("Mempool is currently full. Please try again later.");
        }

        // Recheck against live state while checkpoint finalization is excluded by the
        // blockchain read guard. The earlier signature/gas work is deliberately outside
        // these locks, but all mutable admission conditions are checked here.
        for tx_hash in &accepted_hashes {
            if mempool.pending_tx_hashes.contains(tx_hash) {
                anyhow::bail!(
                    "Transaction {} already in pending pool",
                    hex::encode(tx_hash)
                );
            }
            if chain.is_transaction_hash_executed(tx_hash) {
                anyhow::bail!("Transaction {} already executed", hex::encode(tx_hash));
            }
        }

        for (sender, tx_sequences) in &sequence_groups {
            let base_sequence = KanariAddress::parse_to_account_address(sender)
                .ok()
                .and_then(|sender_addr| state.get_account(&sender_addr))
                .map(|account| account.sequence_number)
                .unwrap_or(0);
            let expected_start = base_sequence
                .checked_add(
                    mempool
                        .pending_sender_counts
                        .get(sender)
                        .copied()
                        .unwrap_or(0),
                )
                .ok_or_else(|| {
                    anyhow::anyhow!("Pending sequence number overflow for {}", sender)
                })?;

            for (expected_seq, tx_seq) in (expected_start..).zip(tx_sequences.iter().copied()) {
                if tx_seq < expected_seq {
                    anyhow::bail!(
                        "Sequence number too low: expected {}, got {}",
                        expected_seq,
                        tx_seq
                    );
                }
                if tx_seq > expected_seq {
                    anyhow::bail!(
                        "Sequence number too high: expected {}, got {}, sender: {}",
                        expected_seq,
                        tx_seq,
                        sender
                    );
                }
            }
        }

        mempool.pending_txs.extend(
            verified_txs
                .into_iter()
                .map(|(signed_tx, _, _, _)| signed_tx),
        );
        mempool
            .pending_tx_hashes
            .extend(accepted_hashes.iter().cloned());
        for (sender, count) in &accepted_counts_by_sender {
            *mempool
                .pending_sender_counts
                .entry(sender.clone())
                .or_insert(0) += *count;
        }

        Ok(accepted_hashes)
    }

    pub fn execute_transaction_immediate(
        &self,
        signed_tx: SignedTransaction,
    ) -> Result<(Vec<u8>, ChangeSet)> {
        let verified = signed_tx.into_verified()?;
        let tx_hash = verified.hash().to_vec();
        let tx = verified.into_signed_transaction().transaction;

        let changeset = {
            let mut state_snapshot = self.state_read().clone();
            let sender_addr = tx.sender_address();
            let addr = KanariAddress::parse_to_account_address(sender_addr)?;

            for _ in 0..self.pending_tx_count_for_sender(sender_addr) {
                if let Some(mut acct) = state_snapshot.get_account(&addr) {
                    acct.increment_sequence();
                    if let Err(e) = state_snapshot.save_account(&acct) {
                        error!("Failed to save account during sequence update: {}", e);
                    }
                }
            }
            let state_arc = Arc::new(RwLock::new(state_snapshot));
            let runtime = self.runtime_pool[0]
                .spawn_isolated_worker()
                .context("Failed to create isolated runtime for immediate execution")?;
            let changeset =
                self.execute_transaction_with_runtime(&tx, &runtime, &state_arc, None)?;
            runtime.clear_object_cache()?;
            changeset
        };

        Ok((tx_hash, changeset))
    }

    pub(crate) fn pending_tx_count_for_sender(&self, sender: &str) -> u64 {
        let normalized_sender = Self::normalize_addr(sender);
        self.mempool_read()
            .pending_sender_counts
            .get(&normalized_sender)
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn remove_pending_sender_counts(
        counts: &mut ahash::AHashMap<String, u64>,
        transactions: &[SignedTransaction],
    ) {
        if transactions.is_empty() {
            return;
        }

        for tx in transactions {
            let sender = Self::normalize_addr(tx.transaction.sender_address());
            let should_remove = if let Some(count) = counts.get_mut(&sender) {
                *count = count.saturating_sub(1);
                *count == 0
            } else {
                false
            };
            if should_remove {
                counts.remove(&sender);
            }
        }
    }

    pub(crate) fn normalize_addr(addr: &str) -> String {
        use std::str::FromStr;
        KanariAddress::from_str(addr)
            .map(|a| a.to_hex())
            .unwrap_or_else(|_| addr.trim_start_matches("0x").to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::Checkpoint;
    use kanari_crypto::keys::{CurveType, generate_keypair};

    fn signed_transfer(sequence_number: u64) -> SignedTransaction {
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        let transaction = Transaction::new_transfer(
            sender.tagged_address(),
            recipient.address,
            1,
            sequence_number,
        );
        let mut signed = SignedTransaction::new(transaction);
        signed.sign(&sender.private_key, sender.curve_type).unwrap();
        signed
    }

    #[test]
    fn final_admission_rejects_transaction_committed_during_validation_window() {
        let engine = BlockchainEngine::new_in_memory().unwrap();
        let signed_tx = signed_transfer(0);
        let verified = signed_tx.clone().into_verified().unwrap();
        let tx_hash = verified.hash().to_vec();
        let sender = BlockchainEngine::normalize_addr(verified.transaction().sender_address());
        let sequence_number = verified.transaction().sequence_number();
        let verified_tx = verified.into_signed_transaction();

        // Model the race: expensive verification completed while the transaction was
        // uncommitted, then a checkpoint committed it before the final mempool write.
        let previous_hash = {
            let chain = engine
                .blockchain
                .read()
                .unwrap_or_else(|error| error.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let checkpoint = Checkpoint::new(
            1,
            vec![[7u8; 32]],
            vec![signed_tx],
            vec![9u8; 32],
            1,
            previous_hash,
        );
        engine
            .blockchain
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .add_checkpoint_with_validation(checkpoint, false)
            .unwrap();

        let error = engine
            .admit_verified_transactions(vec![(verified_tx, tx_hash, sender, sequence_number)])
            .unwrap_err();

        assert!(error.to_string().contains("already executed"));
        assert_eq!(engine.pending_transaction_len(), 0);
    }
}
