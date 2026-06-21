// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_types::transaction::{SignedTransaction, Transaction};
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

    fn arg_declares_conflict_key(arg: &[u8]) -> bool {
        if arg.len() == 32 {
            return true;
        }

        let parsed_string = bcs::from_bytes::<String>(arg)
            .or_else(|_| std::str::from_utf8(arg).map(|s| s.to_string()));
        let Ok(parsed_string) = parsed_string else {
            return false;
        };
        let trimmed = parsed_string.trim();
        let hex_str = if trimmed.starts_with("0x") {
            trimmed.to_string()
        } else {
            format!("0x{}", trimmed)
        };
        move_core_types::account_address::AccountAddress::from_hex_literal(&hex_str).is_ok()
    }

    fn conflict_keys_are_complete(tx: &SignedTransaction) -> bool {
        match &tx.transaction {
            Transaction::PublishModule { .. } => false,
            Transaction::ExecuteFunction { args, .. } => {
                if tx.transaction.native_call().is_some() {
                    return true;
                }
                args.iter().any(|arg| Self::arg_declares_conflict_key(arg))
            }
        }
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
        if transactions
            .iter()
            .any(|tx| !Self::conflict_keys_are_complete(tx))
        {
            return transactions.into_iter().map(|tx| vec![tx]).collect();
        }

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
mod tests {
    use super::*;
    use kanari_types::transaction::Transaction;

    fn create_dummy_tx(sender: &str, module: &str, object: Option<&str>) -> SignedTransaction {
        let mut args = Vec::new();
        if let Some(obj) = object {
            // Mock object ID as 32 bytes
            let mut id = vec![0u8; 32];
            // Fill with object string bytes for uniqueness (simplified)
            let bytes = obj.as_bytes();
            for (i, b) in bytes.iter().enumerate().take(32) {
                id[i] = *b;
            }
            args.push(id);
        }

        let tx = Transaction::ExecuteFunction {
            sender: sender.to_string(),
            module: module.to_string(),
            function: "test".to_string(),
            type_args: vec![],
            args,
            gas_limit: 1000,
            gas_price: 1,
            sequence_number: 0,
        };
        SignedTransaction::new(tx)
    }

    #[test]
    fn test_schedule_parallel() {
        // Tx1: A -> uses Obj1
        // Tx2: B -> uses Obj2
        // Tx3: C -> uses Obj1
        // Tx4: D -> uses Obj2

        // Expected:
        // Wave 0: Tx1, Tx2 (independent)
        // Wave 1: Tx3 (conflicts with Tx1), Tx4 (conflicts with Tx2)

        // We use different modules to avoid module-level conflicts
        let tx1 = create_dummy_tx("A", "M1", Some("Obj1"));
        let tx2 = create_dummy_tx("B", "M2", Some("Obj2"));
        let tx3 = create_dummy_tx("C", "M3", Some("Obj1"));
        let tx4 = create_dummy_tx("D", "M4", Some("Obj2"));

        let txs = vec![tx1, tx2, tx3, tx4];
        let waves = TransactionScheduler::schedule(txs);

        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].len(), 2); // Tx1, Tx2
        assert_eq!(waves[1].len(), 2); // Tx3, Tx4
    }

    #[test]
    fn test_schedule_chain() {
        // Tx1: A
        // Tx2: A (depends on Tx1)
        // Tx3: A (depends on Tx2)

        let tx1 = create_dummy_tx("A", "M1", None);
        let tx2 = create_dummy_tx("A", "M2", None);
        let tx3 = create_dummy_tx("A", "M3", None);

        let txs = vec![tx1, tx2, tx3];
        let waves = TransactionScheduler::schedule(txs);

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].len(), 1);
        assert_eq!(waves[1].len(), 1);
        assert_eq!(waves[2].len(), 1);
    }

    #[test]
    fn test_schedule_unknown_conflicts_fall_back_to_serial_waves() {
        let tx1 = create_dummy_tx("A", "M1", None);
        let tx2 = create_dummy_tx("B", "M2", None);

        let waves = TransactionScheduler::schedule(vec![tx1, tx2]);

        assert_eq!(waves.len(), 2);
        assert!(waves.iter().all(|wave| wave.len() == 1));
    }

    #[test]
    fn test_schedule_complex() {
        // Tx1: A (Obj1)
        // Tx2: B (Obj1) -> Conflicts with Tx1
        // Tx3: C (Obj2) -> Independent
        // Tx4: D (Obj1) -> Conflicts with Tx2
        // Tx5: E (Obj2) -> Conflicts with Tx3

        // Expected:
        // Wave 0: Tx1, Tx3
        // Wave 1: Tx2, Tx5
        // Wave 2: Tx4

        let tx1 = create_dummy_tx("A", "M1", Some("Obj1"));
        let tx2 = create_dummy_tx("B", "M2", Some("Obj1"));
        let tx3 = create_dummy_tx("C", "M3", Some("Obj2"));
        let tx4 = create_dummy_tx("D", "M4", Some("Obj1"));
        let tx5 = create_dummy_tx("E", "M5", Some("Obj2"));

        let txs = vec![tx1, tx2, tx3, tx4, tx5];
        let waves = TransactionScheduler::schedule(txs);

        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].len(), 2); // Tx1, Tx3
        assert_eq!(waves[1].len(), 2); // Tx2, Tx5
        assert_eq!(waves[2].len(), 1); // Tx4
    }
}
