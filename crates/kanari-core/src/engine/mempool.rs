// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use ahash::AHashSet;

type VerifiedMempoolTx = (
    SignedTransaction,
    Vec<u8>,
    String,
    u64,
    String,
    String,
    Vec<String>,
    usize,
);
type MempoolTxMetadata = (Vec<u8>, String, u64, String, String, Vec<String>);

/// Trait for normalizing address strings used across the engine.
pub(crate) trait NormalizeAddr {
    fn normalize_addr(addr: &str) -> String;
}

impl NormalizeAddr for BlockchainEngine {
    fn normalize_addr(addr: &str) -> String {
        use std::str::FromStr;
        KanariAddress::from_str(addr)
            .map(|a| a.to_hex())
            .unwrap_or_else(|_| addr.trim_start_matches("0x").to_lowercase())
    }
}

impl BlockchainEngine {
    pub(crate) fn decrement_pending_count(counts: &mut ahash::AHashMap<String, u64>, key: &str) {
        let should_remove = if let Some(count) = counts.get_mut(key) {
            *count = count.saturating_sub(1);
            *count == 0
        } else {
            false
        };
        if should_remove {
            counts.remove(key);
        }
    }

    pub(crate) fn pending_record_size(
        signed_tx: &SignedTransaction,
        metadata: &PendingTransactionMetadata,
    ) -> Result<usize> {
        let transaction_bytes = bcs::serialized_size(signed_tx)?;
        let effects_bytes = metadata
            .preview_effects
            .as_ref()
            .map(bcs::serialized_size)
            .transpose()?
            .unwrap_or(0);
        transaction_bytes
            .checked_add(effects_bytes)
            .and_then(|size| size.checked_add(128))
            .context("Pending transaction memory accounting overflow")
    }

    pub fn submit_transactions_batch_with_metadata(
        &self,
        signed_txs: Vec<SignedTransaction>,
        metadata: PendingTransactionMetadata,
    ) -> Result<Vec<Vec<u8>>> {
        self.submit_transactions_batch_internal(signed_txs, metadata)
    }

    pub fn submit_transactions_batch(
        &self,
        signed_txs: Vec<SignedTransaction>,
    ) -> Result<Vec<Vec<u8>>> {
        self.submit_transactions_batch_internal(signed_txs, PendingTransactionMetadata::default())
    }

    fn submit_transactions_batch_internal(
        &self,
        signed_txs: Vec<SignedTransaction>,
        metadata: PendingTransactionMetadata,
    ) -> Result<Vec<Vec<u8>>> {
        if signed_txs.is_empty() {
            return Ok(Vec::new());
        }

        // Early size check to avoid unnecessary work
        let batch_size = signed_txs.len();
        let pending_len = self.mempool_read().pending_txs.len();

        if pending_len.saturating_add(batch_size) > MAX_MEMPOOL_SIZE {
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
        let mut verified_txs: Vec<VerifiedMempoolTx> = signed_txs
            .into_par_iter()
            .map(|signed_tx| -> Result<VerifiedMempoolTx> {
                let verified = signed_tx.into_verified()?;
                let tx_hash = verified.hash().to_vec();
                let tx = verified.transaction();
                Self::validate_transaction_admission_shape(tx)?;
                let sender = tx.sender_address();
                let normalized_sender = sender_cache
                    .get(sender)
                    .cloned()
                    .unwrap_or_else(|| {
                        log::warn!(
                            "[MEMPOOL] Sender normalization cache miss for verified transaction; normalizing inline"
                        );
                        Self::normalize_addr(sender)
                    });
                let nonce = tx.nonce();
                let primary_access_key = tx.primary_access_key();
                let congestion_access_key = Self::congestion_access_key(tx);
                let access_keys = tx.object_access_keys();
                let signed_tx = verified.into_signed_transaction();
                let size_bytes = Self::pending_record_size(&signed_tx, &metadata)?;
                Ok((
                    signed_tx,
                    tx_hash,
                    normalized_sender,
                    nonce,
                    primary_access_key,
                    congestion_access_key,
                    access_keys,
                    size_bytes,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let batch_bytes = verified_txs.iter().try_fold(0usize, |total, item| {
            total
                .checked_add(item.7)
                .context("Mempool batch byte size overflow")
        })?;

        verified_txs.sort_by(|a, b| {
            a.4.cmp(&b.4)
                .then_with(|| a.2.cmp(&b.2))
                .then_with(|| a.3.cmp(&b.3))
                .then_with(|| a.1.cmp(&b.1))
        });

        let batch_metadata: Vec<MempoolTxMetadata> = verified_txs
            .iter()
            .map(
                |(_, hash, sender, sequence, primary_access, congestion_access, access_keys, _)| {
                    (
                        hash.clone(),
                        sender.clone(),
                        *sequence,
                        primary_access.clone(),
                        congestion_access.clone(),
                        access_keys.clone(),
                    )
                },
            )
            .collect();

        // Check executed transactions in parallel. Persistent index corruption
        // must fail admission closed instead of making an old transaction look
        // uncommitted.
        let executed_hashes = {
            use rayon::prelude::*;
            batch_metadata
                .par_iter()
                .map(|(tx_hash, _, _, _, _, _)| -> Result<Option<Vec<u8>>> {
                    Ok(self
                        .try_is_transaction_committed(tx_hash)?
                        .then(|| tx_hash.clone()))
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<AHashSet<_>>()
        };

        // Account sequence is intentionally not a consensus admission rule. Object refs,
        // gas refs, and duplicate transaction hashes are the canonical replay/double-spend guards.
        let mut mempool = self.mempool_write();
        if mempool.pending_txs.len().saturating_add(batch_size) > MAX_MEMPOOL_SIZE {
            anyhow::bail!("Mempool is currently full. Please try again later.");
        }
        if mempool.pending_bytes.saturating_add(batch_bytes) > MAX_MEMPOOL_BYTES {
            anyhow::bail!("Mempool byte budget is exhausted. Please try again later.");
        }
        let mut batch_hashes = AHashSet::with_capacity(batch_size);
        let mut accepted_hashes = Vec::with_capacity(batch_size);
        let mut accepted_counts_by_sender = ahash::AHashMap::new();
        let mut accepted_counts_by_access = ahash::AHashMap::new();
        let mut accepted_counts_by_primary_access = ahash::AHashMap::new();
        let mut accepted_counts_by_congestion_access = ahash::AHashMap::new();
        for (tx_hash, sender, _, primary_access, congestion_access, access_keys) in &batch_metadata
        {
            if mempool.pending_tx_hashes.contains(tx_hash) || !batch_hashes.insert(tx_hash.clone())
            {
                let tx_hash_hex = hex::encode(tx_hash);
                anyhow::bail!("Transaction {} already in pending pool", tx_hash_hex);
            }
            if executed_hashes.contains(tx_hash) {
                let tx_hash_hex = hex::encode(tx_hash);
                anyhow::bail!("Transaction {} already executed", tx_hash_hex);
            }

            let current_lane_depth = mempool
                .pending_primary_access_counts
                .get(primary_access)
                .copied()
                .unwrap_or(0)
                .saturating_add(
                    accepted_counts_by_primary_access
                        .get(primary_access)
                        .copied()
                        .unwrap_or(0),
                );
            if current_lane_depth >= MAX_PENDING_PER_PRIMARY_ACCESS_LANE {
                anyhow::bail!(
                    "Transaction lane {} is saturated: {} pending transaction(s) already target this primary access key, max {}",
                    primary_access,
                    current_lane_depth,
                    MAX_PENDING_PER_PRIMARY_ACCESS_LANE
                );
            }
            let current_congestion_depth = mempool
                .pending_congestion_access_counts
                .get(congestion_access)
                .copied()
                .unwrap_or(0)
                .saturating_add(
                    accepted_counts_by_congestion_access
                        .get(congestion_access)
                        .copied()
                        .unwrap_or(0),
                );
            if current_congestion_depth >= MAX_PENDING_PER_PRIMARY_ACCESS_LANE {
                anyhow::bail!(
                    "Transaction congestion lane {} is saturated: {} pending transaction(s) already touch this canonical access key, max {}",
                    congestion_access,
                    current_congestion_depth,
                    MAX_PENDING_PER_PRIMARY_ACCESS_LANE
                );
            }

            accepted_hashes.push(tx_hash.clone());
            *accepted_counts_by_sender.entry(sender.clone()).or_insert(0) += 1;
            *accepted_counts_by_primary_access
                .entry(primary_access.clone())
                .or_insert(0) += 1;
            *accepted_counts_by_congestion_access
                .entry(congestion_access.clone())
                .or_insert(0) += 1;
            *accepted_counts_by_access
                .entry(primary_access.clone())
                .or_insert(0) += 1;
            for access_key in access_keys {
                *accepted_counts_by_access
                    .entry(access_key.clone())
                    .or_insert(0) += 1;
            }
        }

        // Commit admission and all derived indexes under the same lock used for checks.
        mempool.pending_txs.extend(verified_txs.into_iter().map(
            |(
                signed_tx,
                tx_hash,
                normalized_sender,
                _,
                primary_access_key,
                congestion_access_key,
                access_keys,
                size_bytes,
            )| {
                PendingTransactionRecord {
                    signed_tx,
                    metadata: metadata.clone(),
                    tx_hash,
                    normalized_sender,
                    primary_access_key,
                    congestion_access_key,
                    access_keys,
                    size_bytes,
                }
            },
        ));
        mempool
            .pending_tx_hashes
            .extend(accepted_hashes.iter().cloned());
        mempool.pending_bytes = mempool.pending_bytes.saturating_add(batch_bytes);
        for (sender, count) in &accepted_counts_by_sender {
            *mempool
                .pending_sender_counts
                .entry(sender.clone())
                .or_insert(0) += *count;
        }
        for (primary_access, count) in &accepted_counts_by_primary_access {
            *mempool
                .pending_primary_access_counts
                .entry(primary_access.clone())
                .or_insert(0) += *count;
        }
        for (congestion_access, count) in &accepted_counts_by_congestion_access {
            *mempool
                .pending_congestion_access_counts
                .entry(congestion_access.clone())
                .or_insert(0) += *count;
        }
        for (access_key, count) in &accepted_counts_by_access {
            *mempool
                .pending_access_counts
                .entry(access_key.clone())
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
        Self::validate_transaction_admission_shape(&tx)?;

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

    #[cfg(test)]
    pub(crate) fn pending_tx_count_for_primary_access(&self, key: &str) -> u64 {
        self.mempool_read()
            .pending_primary_access_counts
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn pending_tx_count_for_congestion_access(&self, key: &str) -> u64 {
        self.mempool_read()
            .pending_congestion_access_counts
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    pub fn pending_access_keys_snapshot(&self) -> std::collections::HashSet<String> {
        self.mempool_read()
            .pending_access_counts
            .keys()
            .cloned()
            .collect()
    }

    pub(crate) fn remove_pending_sender_counts(
        counts: &mut ahash::AHashMap<String, u64>,
        transactions: &[PendingTransactionRecord],
    ) {
        if transactions.is_empty() {
            return;
        }

        for tx in transactions {
            Self::decrement_pending_count(counts, &tx.normalized_sender);
        }
    }

    pub(crate) fn remove_pending_access_counts(
        counts: &mut ahash::AHashMap<String, u64>,
        transactions: &[PendingTransactionRecord],
    ) {
        if transactions.is_empty() {
            return;
        }

        for tx in transactions {
            let mut keys = tx.access_keys.clone();
            keys.push(tx.primary_access_key.clone());
            keys.sort();
            keys.dedup();
            for key in keys {
                Self::decrement_pending_count(counts, &key);
            }
        }
    }

    pub(crate) fn remove_pending_primary_access_counts(
        counts: &mut ahash::AHashMap<String, u64>,
        transactions: &[PendingTransactionRecord],
    ) {
        if transactions.is_empty() {
            return;
        }

        for tx in transactions {
            Self::decrement_pending_count(counts, &tx.primary_access_key);
        }
    }

    pub(crate) fn remove_pending_congestion_access_counts(
        counts: &mut ahash::AHashMap<String, u64>,
        transactions: &[PendingTransactionRecord],
    ) {
        if transactions.is_empty() {
            return;
        }

        for tx in transactions {
            Self::decrement_pending_count(counts, &tx.congestion_access_key);
        }
    }

    pub fn remove_pending_transactions_by_hashes(
        &self,
        hashes: &[Vec<u8>],
    ) -> Vec<PendingTransactionRecord> {
        if hashes.is_empty() {
            return Vec::new();
        }

        let target_hashes = hashes.iter().cloned().collect::<AHashSet<_>>();
        let mut mempool = self.mempool_write();
        let removed_transactions = mempool
            .pending_txs
            .iter()
            .filter(|tx| target_hashes.contains(&tx.tx_hash))
            .cloned()
            .collect::<Vec<_>>();

        if removed_transactions.is_empty() {
            return Vec::new();
        }

        mempool
            .pending_txs
            .retain(|tx| !target_hashes.contains(&tx.tx_hash));
        let removed_bytes = removed_transactions.iter().fold(0usize, |total, record| {
            total.saturating_add(record.size_bytes)
        });
        mempool.pending_bytes = mempool.pending_bytes.saturating_sub(removed_bytes);
        mempool
            .pending_tx_hashes
            .retain(|hash| !target_hashes.contains(hash));
        Self::remove_pending_sender_counts(
            &mut mempool.pending_sender_counts,
            &removed_transactions,
        );
        Self::remove_pending_primary_access_counts(
            &mut mempool.pending_primary_access_counts,
            &removed_transactions,
        );
        Self::remove_pending_congestion_access_counts(
            &mut mempool.pending_congestion_access_counts,
            &removed_transactions,
        );
        Self::remove_pending_access_counts(
            &mut mempool.pending_access_counts,
            &removed_transactions,
        );
        self.record_invalid_pending_drop(removed_transactions.len());
        removed_transactions
    }
}
