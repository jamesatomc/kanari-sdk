// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use ahash::AHashSet;

impl BlockchainEngine {
    pub fn submit_transactions_batch(
        &self,
        signed_txs: Vec<SignedTransaction>,
    ) -> Result<Vec<Vec<u8>>> {
        if signed_txs.is_empty() {
            return Ok(Vec::new());
        }

        let batch_size = signed_txs.len();
        if batch_size > MAX_MEMPOOL_SIZE {
            anyhow::bail!("Transaction batch exceeds the maximum mempool size");
        }

        let mut sender_cache = ahash::AHashMap::with_capacity(batch_size);
        for signed_tx in &signed_txs {
            let sender = signed_tx.transaction.sender_address();
            sender_cache
                .entry(sender.to_string())
                .or_insert_with(|| Self::normalize_addr(sender));
        }

        // Signature verification and hashing are expensive, so perform them before
        // entering the serialized admission section. No shared mempool state is read
        // until the write lock is held.
        let mut verified_txs = signed_txs
            .into_par_iter()
            .map(
                |signed_tx| -> Result<(SignedTransaction, Vec<u8>, String, u64)> {
                    let verified = signed_tx.into_verified()?;
                    let tx_hash = verified.hash().to_vec();
                    let sender = verified.transaction().sender_address();
                    let normalized_sender = sender_cache.get(sender).cloned().ok_or_else(|| {
                        anyhow::anyhow!("Sender cache missing verified transaction sender")
                    })?;
                    let sequence_number = verified.transaction().sequence_number();
                    Ok((
                        verified.into_signed_transaction(),
                        tx_hash,
                        normalized_sender,
                        sequence_number,
                    ))
                },
            )
            .collect::<Result<Vec<_>>>()?;

        verified_txs.sort_by(|a, b| {
            a.2.cmp(&b.2)
                .then_with(|| a.3.cmp(&b.3))
                .then_with(|| a.1.cmp(&b.1))
        });

        let batch_metadata: Vec<(Vec<u8>, String, u64)> = verified_txs
            .iter()
            .map(|(_, hash, sender, sequence)| (hash.clone(), sender.clone(), *sequence))
            .collect();

        // Admission is one critical section. Duplicate checks, sequence checks and
        // insertion all observe the same pending-pool snapshot, preventing two
        // concurrent submissions from both passing stale pre-lock validation.
        let mut mempool = self.mempool_write();

        if mempool.pending_txs.len().saturating_add(batch_size) > MAX_MEMPOOL_SIZE {
            log::warn!("[MEMPOOL] Rejecting batch: queue would exceed max size");
            anyhow::bail!("Mempool is currently full. Please try again later.");
        }

        let base_sequences = {
            let state = self.state_read();
            let mut sequences = std::collections::HashMap::with_capacity(batch_metadata.len());
            for (_, sender, _) in &batch_metadata {
                sequences.entry(sender.clone()).or_insert_with(|| {
                    KanariAddress::parse_to_account_address(sender)
                        .ok()
                        .and_then(|sender_addr| state.get_account(&sender_addr))
                        .map(|acc| acc.sequence_number)
                        .unwrap_or(0)
                });
            }
            sequences
        };

        let executed_hashes = {
            let chain = match self.blockchain.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    log::error!(
                        "Blockchain lock poisoned in submit_transactions_batch, recovering..."
                    );
                    poisoned.into_inner()
                }
            };

            if !chain.has_executed_transactions() {
                AHashSet::new()
            } else {
                batch_metadata
                    .iter()
                    .filter_map(|(tx_hash, _, _)| {
                        if chain.is_transaction_hash_executed(tx_hash) {
                            Some(tx_hash.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<AHashSet<_>>()
            }
        };

        let mut batch_hashes = AHashSet::with_capacity(batch_size);
        let mut accepted_hashes = Vec::with_capacity(batch_size);
        let mut accepted_counts_by_sender = ahash::AHashMap::new();
        let mut sequence_groups = ahash::AHashMap::new();

        for (tx_hash, sender, tx_seq) in &batch_metadata {
            if mempool.pending_tx_hashes.contains(tx_hash)
                || !batch_hashes.insert(tx_hash.clone())
            {
                anyhow::bail!(
                    "Transaction {} already in pending pool or batch",
                    hex::encode(tx_hash)
                );
            }
            if executed_hashes.contains(tx_hash) {
                anyhow::bail!("Transaction {} already executed", hex::encode(tx_hash));
            }

            accepted_hashes.push(tx_hash.clone());
            *accepted_counts_by_sender.entry(sender.clone()).or_insert(0u64) += 1;
            sequence_groups
                .entry(sender.clone())
                .or_insert_with(Vec::new)
                .push(*tx_seq);
        }

        for (sender, mut tx_sequences) in sequence_groups {
            tx_sequences.sort_unstable();
            let base_sequence = base_sequences.get(&sender).copied().unwrap_or(0);
            let pending_count = mempool
                .pending_sender_counts
                .get(&sender)
                .copied()
                .unwrap_or(0);
            let expected_start = base_sequence.checked_add(pending_count).ok_or_else(|| {
                anyhow::anyhow!("Sequence number overflow for sender {}", sender)
            })?;

            for (offset, tx_seq) in tx_sequences.into_iter().enumerate() {
                let offset = u64::try_from(offset)
                    .map_err(|_| anyhow::anyhow!("Sequence offset overflow"))?;
                let expected_seq = expected_start.checked_add(offset).ok_or_else(|| {
                    anyhow::anyhow!("Sequence number overflow for sender {}", sender)
                })?;

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
}
