// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_types::transaction::SignedTransaction;
use std::collections::{HashMap, HashSet};

/// Transaction Scheduler for parallel execution
/// Organizes transactions into "waves" where transactions in the same wave can be executed in parallel.
pub struct TransactionScheduler;

impl TransactionScheduler {
    /// Schedule transactions into parallel execution waves based on object conflicts.
    /// Uses a "Earliest Wave" algorithm to maximize parallelism.
    ///
    /// Algorithm:
    /// 1. Track the last wave index assigned to each conflict key (Object ID/Address).
    /// 2. For each transaction, determine the earliest possible wave index:
    ///    wave_idx = max(last_wave_index[key] for key in tx_keys) + 1
    /// 3. Assign the transaction to that wave.
    /// 4. Update last_wave_index for all keys involved in the transaction.
    ///
    /// This ensures that:
    /// - Transactions with conflicts are ordered sequentially (preserving causal order).
    /// - Transactions without conflicts are placed in the earliest possible wave (maximizing parallelism).
    pub fn schedule(transactions: Vec<SignedTransaction>) -> Vec<Vec<SignedTransaction>> {
        if transactions.is_empty() {
            return vec![];
        }

        // 🚀 FAST PATH (อัปเกรดเพื่อ 100K TPS): 
        // เช็คก่อนเลยว่าถ้าธุรกรรมทั้งหมดไม่มีการแก้ไข Object ที่ซ้ำกันเลย (Fully Parallelizable)
        // ให้จับทั้งหมดมัดรวมเป็น Wave เดียว แล้วส่งไปให้ CPU ทุก Core รันพร้อมกันทันที
        // (ข้ามขั้นตอนการจัดคิวที่กิน CPU ด้านล่างไปเลย)
        if Self::is_fully_parallelizable(&transactions) {
            return vec![transactions];
        }

        let mut waves: Vec<Vec<SignedTransaction>> = Vec::new();
        // เก็บว่า Object Key นี้ ถูกใช้งานล่าสุดที่ Wave ไหน
        let mut last_used_in_wave: HashMap<String, usize> = HashMap::new();

        for tx in transactions {
            let keys = tx.transaction.get_conflict_keys();
            let mut target_wave = 0;

            // หา Wave ที่ต่ำที่สุดที่สามารถเอาธุรกรรมนี้ไปแทรกได้โดยไม่ชนใคร
            for key in &keys {
                if let Some(&wave_idx) = last_used_in_wave.get(key)
                    && wave_idx >= target_wave
                {
                    target_wave = wave_idx + 1; // ต้องขยับไป Wave ถัดไป
                }
            }

            // ถ้าต้องเปิด Wave ใหม่
            while waves.len() <= target_wave {
                waves.push(Vec::new());
            }

            waves[target_wave].push(tx);

            // อัปเดตสถานะการจอง Lock ของ Keys
            for key in keys {
                last_used_in_wave.insert(key, target_wave);
            }
        }

        waves
    }

    /// (Optional) สำหรับรันแบบ Block-STM เต็มรูปแบบในอนาคตที่ใช้ Optimistic Execution
    /// ปัจจุบันเราใช้ Explicit Wave ที่ปลอดภัย 100% ควบคู่ไปกับ Rayon Par_Iter
    pub fn is_fully_parallelizable(transactions: &[SignedTransaction]) -> bool {
        let mut seen_keys = HashSet::new();
        for tx in transactions {
            let keys = tx.transaction.get_conflict_keys();
            for k in keys {
                if !seen_keys.insert(k) {
                    return false; // มีการชนกันเกิดขึ้น
                }
            }
        }
        true
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
