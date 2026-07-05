// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_types::transaction::SignedTransaction;
use std::collections::HashMap;

/// Transaction Scheduler for parallel execution
/// Organizes transactions into "waves" where transactions in the same wave can be executed in parallel.
pub struct TransactionScheduler;

impl TransactionScheduler {
    fn target_wave_idx(keys: &[String], key_last_wave: &HashMap<String, usize>) -> usize {
        keys.iter()
            .filter_map(|key| key_last_wave.get(key).copied())
            .map(|last_wave| last_wave + 1)
            .max()
            .unwrap_or(0)
    }

    /// Schedule transactions into parallel execution waves based on object conflicts.
    /// Uses a "Earliest Wave" algorithm to maximize parallelism.
    ///
    /// Algorithm:
    /// 1. Track the last wave index assigned to each conflict key (Object ID/Address).
    /// 2. For each transaction, determine the earliest possible wave index:
    ///    `wave_idx = max(last_wave_index[key] for key in tx_keys) + 1`
    /// 3. Assign the transaction to that wave.
    /// 4. Update last_wave_index for all keys involved in the transaction.
    ///
    /// This ensures that:
    /// - Transactions with conflicts are ordered sequentially (preserving causal order).
    /// - Transactions without conflicts are placed in the earliest possible wave (maximizing parallelism).
    pub fn schedule(transactions: Vec<SignedTransaction>) -> Vec<Vec<SignedTransaction>> {
        let mut waves: Vec<Vec<SignedTransaction>> = Vec::new();
        // Map: Conflict Key -> Index of the last wave that touched this key
        // We use isize here to represent "no wave yet" as -1, so the first wave is 0.
        // Actually, let's just use usize and 0-based indexing.
        let mut key_last_wave: HashMap<String, usize> =
            HashMap::with_capacity(transactions.len() * 2);

        for tx in transactions {
            let keys = tx.transaction.get_conflict_keys();
            let target_wave_idx = Self::target_wave_idx(&keys, &key_last_wave);

            // Ensure the wave exists
            while waves.len() <= target_wave_idx {
                waves.push(Vec::new());
            }

            // Add transaction to the wave
            waves[target_wave_idx].push(tx);

            // Update the last wave index for all keys
            for key in keys {
                key_last_wave.insert(key, target_wave_idx);
            }
        }

        waves
    }
}

#[cfg(test)]
#[path = "../tests/unit/scheduler_tests.rs"]
mod tests;
