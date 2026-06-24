// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_types::transaction::SignedTransaction;
use std::collections::HashMap;

/// Transaction scheduler for deterministic conflict-safe execution waves.
pub struct TransactionScheduler;

impl TransactionScheduler {
    fn target_wave_idx(keys: &[String], key_last_wave: &HashMap<String, usize>) -> usize {
        keys.iter()
            .filter_map(|key| key_last_wave.get(key).copied())
            .map(|last_wave| last_wave + 1)
            .max()
            .unwrap_or(0)
    }

    /// Schedule transactions into deterministic execution waves.
    ///
    /// A transaction is parallelized only when its complete mutable access set
    /// is known before execution. An arbitrary Move call or module publish is a
    /// serial barrier: it executes after every preceding wave and every later
    /// transaction executes after it. This fail-closed rule prevents hidden
    /// global/dynamic-field conflicts from producing divergent state roots.
    pub fn schedule(transactions: Vec<SignedTransaction>) -> Vec<Vec<SignedTransaction>> {
        let mut waves: Vec<Vec<SignedTransaction>> = Vec::new();
        let mut key_last_wave: HashMap<String, usize> =
            HashMap::with_capacity(transactions.len().saturating_mul(2));
        let mut serial_barrier_wave: Option<usize> = None;

        for tx in transactions {
            let complete_access_set = tx.transaction.has_complete_conflict_set();
            let keys = tx.transaction.get_conflict_keys();

            let target_wave_idx = if complete_access_set {
                let conflict_wave = Self::target_wave_idx(&keys, &key_last_wave);
                serial_barrier_wave
                    .map(|barrier| conflict_wave.max(barrier.saturating_add(1)))
                    .unwrap_or(conflict_wave)
            } else {
                // A fresh wave after all preceding work isolates unknown access.
                waves.len()
            };

            while waves.len() <= target_wave_idx {
                waves.push(Vec::new());
            }
            waves[target_wave_idx].push(tx);

            for key in keys {
                key_last_wave.insert(key, target_wave_idx);
            }
            if !complete_access_set {
                serial_barrier_wave = Some(target_wave_idx);
            }
        }

        waves
    }
}

#[cfg(test)]
#[path = "../tests/unit/scheduler_tests.rs"]
mod tests;
