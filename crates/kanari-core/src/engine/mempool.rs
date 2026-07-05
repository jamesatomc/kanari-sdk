// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use ahash::AHashSet;
use kanari_types::error::KanariUnwrapExt;

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
            anyhow::bail!("Transaction batch exceeds the maximum mempool capacity");
        }

        let mut sender_cache = ahash::AHashMap::with_capacity(batch_size);
        for signed_tx in &signed_txs {
            let sender = signed_tx.transaction.sender_address();
            sender_cache
                .entry(sender.to_string())
                .or_insert_with(|| Self::normalize_addr(sender));
        }

        // Signature verification and hashing are intentionally performed outside the
        // mempool write lock. Admission is revalidated and reserved atomically below.
        let mut verified_txs = signed_txs
            .into_par_iter()
            .map(|signed_tx| -> Result<(SignedTransaction, Vec<u8>, String)> {
                let verified = signed_tx.into_verified()?;
                let tx_hash = verified.hash().to_vec();
                let sender = verified.transaction().sender_address();
                let normalized_sender = sender_cache
                    .get(sender)
                    .invariant("sender cache must contain every batch sender")
                    .clone();
                Ok((verified.into_signed_transaction(), tx_hash, normalized_sender))
            })
            .collect::<Result<Vec<_>>>()?;

        verified_txs.sort_by(|a, b| {
            a.2.cmp(&b.2)
                .then_with(|| a.1.cmp(&b.1))
        });

        let batch_metadata: Vec<(Vec<u8>, String)> = verified_txs
            .iter()
            .map(|(_, hash, sender)| (hash.clone(), sender.clone()))
            .collect();

        // Executed transaction lookup is also expensive and does not mutate admission
        // state, so it remains outside the mempool lock.
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
                use rayon::prelude::*;
                batch_metadata
                    .par_iter()
                    .filter_map(|(tx_hash, _)| {
                        if chain.is_transaction_hash_executed(tx_hash) {
                            Some(tx_hash.clone())
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .collect::<AHashSet<_>>()
            }
        };

        // Validate invariants that are independent of the current mempool state.
        let mut batch_hashes = AHashSet::with_capacity(batch_size);
        let mut accepted_hashes = Vec::with_capacity(batch_size);
        let mut accepted_counts_by_sender = ahash::AHashMap::new();

        for (tx_hash, sender) in &batch_metadata {
            if !batch_hashes.insert(tx_hash.clone()) {
                anyhow::bail!(
                    "Transaction {} appears more than once in the submitted batch",
                    hex::encode(tx_hash)
                );
            }
            if executed_hashes.contains(tx_hash) {
                anyhow::bail!("Transaction {} already executed", hex::encode(tx_hash));
            }

            accepted_hashes.push(tx_hash.clone());
            *accepted_counts_by_sender.entry(sender.clone()).or_insert(0u64) += 1;
        }

        // H-05 remediation: validate all pending-dependent conditions and reserve the
        // hashes/sequences under one write lock. No concurrent admission can pass a
        // stale snapshot between validation and insertion.
        {
            let mut mempool = self.mempool_write();

            if mempool.pending_txs.len().saturating_add(batch_size) > MAX_MEMPOOL_SIZE {
                log::warn!("[MEMPOOL] Rejecting batch: queue would exceed max size");
                anyhow::bail!("Mempool is currently full. Please try again later.");
            }

            for (tx_hash, _) in &batch_metadata {
                if mempool.pending_tx_hashes.contains(tx_hash) {
                    anyhow::bail!(
                        "Transaction {} already in pending pool",
                        hex::encode(tx_hash)
                    );
                }
            }

            mempool.pending_txs.extend(
                verified_txs
                    .into_iter()
                    .map(|(signed_tx, _, _)| signed_tx),
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
            let state_snapshot = self.state_read().clone();
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
