// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Fuzz Testing for kanari-move-runtime
//! 
//! This test suite uses random/fuzzed inputs to find edge cases and potential crashes.

#[cfg(test)]
mod fuzz_tests {
    use kanari_move_runtime::changeset::{ChangeSet, AccountChange, CreatedObject};
    use kanari_move_runtime::state::{Account, StateManager};
    use kanari_move_runtime::scheduler::TransactionScheduler;
    use kanari_move_runtime::storage::persistent_store::PersistentStore;
    use kanari_move_runtime::kanari_gas_meter::KanariGasMeter;
    use move_core_types::account_address::AccountAddress;
    use kanari_types::balance::BalanceRecord;
    use rand::Rng;
    use std::collections::BTreeMap;

    // ==================== Helper Functions ====================

    fn random_account_address(rng: &mut impl Rng) -> AccountAddress {
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes);
        AccountAddress::new(bytes)
    }

    fn random_string(rng: &mut impl Rng, max_len: usize) -> String {
        let len = rng.gen_range(1..=max_len);
        (0..len)
            .map(|_| rng.gen_range(b'a'..=b'z') as char)
            .collect()
    }

    fn random_bytes(rng: &mut impl Rng, max_len: usize) -> Vec<u8> {
        let len = rng.gen_range(0..=max_len);
        (0..len).map(|_| rng.gen()).collect()
    }

    // ==================== ChangeSet Fuzz Tests ====================

    #[test]
    fn fuzz_changeset_with_random_data() {
        let mut rng = rand::thread_rng();
        
        for _ in 0..100 {
            let mut cs = ChangeSet::new();
            
            // Add random token balances
            let addr = random_account_address(&mut rng);
            let token_type = format!("0x{}::{}::{}", 
                hex::encode(&rng.gen::<[u8; 16]>()),
                random_string(&mut rng, 10),
                random_string(&mut rng, 10)
            );
            let balance = BalanceRecord::new(rng.gen());
            
            cs.add_token_balance_set(addr, token_type, balance);
            
            // Should not panic
            assert!(cs.token_balance_sets.len() >= 1);
        }
    }

    #[test]
    fn fuzz_account_change_with_extreme_values() {
        let mut rng = rand::thread_rng();
        
        for _ in 0..50 {
            let addr = random_account_address(&mut rng);
            let mut ac = AccountChange::new(addr);
            
            // Test with extreme debit/credit values
            let extreme_value = rng.gen::<u64>();
            ac.debit(extreme_value);
            ac.credit(extreme_value / 2);
            
            // Should not panic or overflow unexpectedly
            let _ = ac.balance_delta;
        }
    }

    #[test]
    fn fuzz_changeset_many_operations() {
        let mut rng = rand::thread_rng();
        
        for iteration in 0..50 {
            let mut cs = ChangeSet::new();
            let num_ops = rng.gen_range(1..=100);
            
            for _ in 0..num_ops {
                let addr = random_account_address(&mut rng);
                let token_type = random_string(&mut rng, 50);
                let balance = BalanceRecord::new(rng.gen());
                
                cs.add_token_balance_set(addr, token_type, balance);
            }
            
            // Verify no panic occurred
            assert_eq!(cs.token_balance_sets.len(), num_ops);
        }
    }

    // ==================== Account Fuzz Tests ====================

    #[test]
    fn fuzz_account_with_random_balances() {
        let mut rng = rand::thread_rng();
        
        for _ in 0..100 {
            let addr = random_account_address(&mut rng);
            let initial_balance = rng.gen::<u64>();
            let mut account = Account::new(addr, initial_balance);
            
            // Add random token balances
            let num_tokens = rng.gen_range(1..=20);
            for i in 0..num_tokens {
                let token_type = format!("token_{}", i);
                let balance = rng.gen::<u64>();
                account.set_token_balance(token_type, BalanceRecord::new(balance));
            }
            
            // Verify all balances are retrievable
            for i in 0..num_tokens {
                let token_type = format!("token_{}", i);
                let _ = account.get_token_balance(&token_type);
            }
        }
    }

    #[test]
    fn fuzz_account_module_additions() {
        let mut rng = rand::thread_rng();
        
        for _ in 0..50 {
            let addr = random_account_address(&mut rng);
            let mut account = Account::new(addr, 1000);
            
            let num_modules = rng.gen_range(1..=100);
            for _ in 0..num_modules {
                let module_name = random_string(&mut rng, 100);
                account.add_module(module_name);
            }
            
            // Should not panic
            assert!(account.modules.len() <= num_modules);
        }
    }

    // ==================== Scheduler Fuzz Tests ====================

    #[test]
    fn fuzz_scheduler_with_random_transactions() {
        use kanari_types::transaction::{Transaction, SignedTransaction};
        let mut rng = rand::thread_rng();
        
        for _ in 0..20 {
            let num_txs = rng.gen_range(1..=50);
            let mut txs = Vec::new();
            
            for _ in 0..num_txs {
                let sender = format!("{:#x}", random_account_address(&mut rng));
                let module = random_string(&mut rng, 20);
                let has_object = rng.gen_bool(0.5);
                
                let mut args = Vec::new();
                if has_object {
                    let mut obj_id = vec![0u8; 32];
                    rng.fill(&mut obj_id[..]);
                    args.push(obj_id);
                }
                
                let tx = Transaction::ExecuteFunction {
                    sender,
                    module,
                    function: "test".to_string(),
                    type_args: vec![],
                    args,
                    gas_limit: rng.gen_range(100..=10000),
                    gas_price: rng.gen_range(1..=100),
                    sequence_number: rng.gen(),
                };
                txs.push(SignedTransaction::new(tx));
            }
            
            // Schedule should not panic
            let waves = TransactionScheduler::schedule(txs);
            
            // Verify all transactions are scheduled
            let total_scheduled: usize = waves.iter().map(|w| w.len()).sum();
            assert_eq!(total_scheduled, num_txs);
        }
    }

    #[test]
    fn fuzz_scheduler_same_account_sequential() {
        use kanari_types::transaction::{Transaction, SignedTransaction};
        let mut rng = rand::thread_rng();
        
        // All transactions from same account must be sequential
        let num_txs = rng.gen_range(10..=100);
        let mut txs = Vec::new();
        let sender = format!("{:#x}", random_account_address(&mut rng));
        
        for i in 0..num_txs {
            let tx = Transaction::ExecuteFunction {
                sender: sender.clone(),
                module: format!("module_{}", i),
                function: "test".to_string(),
                type_args: vec![],
                args: vec![],
                gas_limit: 1000,
                gas_price: 1,
                sequence_number: i as u64,
            };
            txs.push(SignedTransaction::new(tx));
        }
        
        let waves = TransactionScheduler::schedule(txs);
        
        // Each wave should have exactly 1 transaction
        for wave in &waves {
            assert_eq!(wave.len(), 1, "Same-account transactions must be sequential");
        }
        assert_eq!(waves.len(), num_txs);
    }

    // ==================== PersistentStore Fuzz Tests ====================

    #[test]
    fn fuzz_persistent_store_random_keys_values() {
        let mut rng = rand::thread_rng();
        let store = PersistentStore::open_in_memory().unwrap();
        
        let mut stored_keys = Vec::new();
        
        // Write random data
        for _ in 0..100 {
            let key = random_bytes(&mut rng, 100);
            let value = random_bytes(&mut rng, 1000);
            
            store.save_raw(&key, &value).unwrap();
            stored_keys.push(key);
        }
        
        // Read back and verify
        for key in &stored_keys {
            let retrieved = store.load::<Vec<u8>>(key).unwrap();
            assert!(retrieved.is_some());
        }
    }

    #[test]
    fn fuzz_persistent_store_delete_random() {
        let mut rng = rand::thread_rng();
        let store = PersistentStore::open_in_memory().unwrap();
        
        // Insert many keys
        let mut keys = Vec::new();
        for _ in 0..50 {
            let key = random_bytes(&mut rng, 50);
            let value = random_bytes(&mut rng, 100);
            store.save_raw(&key, &value).unwrap();
            keys.push(key);
        }
        
        // Delete random subset
        let num_to_delete = rng.gen_range(10..=40);
        for key in keys.iter().take(num_to_delete) {
            store.delete(key).unwrap();
        }
        
        // Verify deleted keys are gone
        for key in keys.iter().take(num_to_delete) {
            assert!(store.load::<Vec<u8>>(key).unwrap().is_none());
        }
        
        // Verify remaining keys still exist
        for key in keys.iter().skip(num_to_delete) {
            assert!(store.load::<Vec<u8>>(key).unwrap().is_some());
        }
    }

    #[test]
    fn fuzz_persistent_store_large_values() {
        let store = PersistentStore::open_in_memory().unwrap();
        let mut rng = rand::thread_rng();
        
        // Store large values
        for i in 0..10 {
            let key = format!("large_key_{}", i);
            let size = rng.gen_range(10000..=100000);
            let value = random_bytes(&mut rng, size);
            
            store.save_raw(key.as_bytes(), &value).unwrap();
        }
        
        // Verify all can be retrieved
        for i in 0..10 {
            let key = format!("large_key_{}", i);
            let retrieved = store.load::<Vec<u8>>(key.as_bytes()).unwrap();
            assert!(retrieved.is_some());
        }
    }

    // ==================== Gas Meter Fuzz Tests ====================

    #[test]
    fn fuzz_gas_meter_random_charges() {
        let mut rng = rand::thread_rng();
        
        for _ in 0..100 {
            let limit = rng.gen_range(1000..=100000);
            let mut meter = KanariGasMeter::new(limit);
            
            let mut total_charged = 0u64;
            
            loop {
                let charge = rng.gen_range(1..=1000);
                match meter.charge_step(charge) {
                    Ok(_) => {
                        total_charged = total_charged.saturating_add(charge);
                    }
                    Err(_) => {
                        // Out of gas - expected behavior
                        break;
                    }
                }
                
                // Safety break to prevent infinite loops
                if total_charged > limit * 2 {
                    break;
                }
            }
        }
    }

    #[test]
    fn fuzz_gas_meter_edge_cases() {
        let mut rng = rand::thread_rng();
        
        // Test with various limits
        for limit in [0, 1, 10, 100, 1000, u64::MAX] {
            let mut meter = KanariGasMeter::new(limit);
            
            // Try charging various amounts
            for _ in 0..10 {
                let charge = rng.gen_range(0..=limit.max(1000));
                let _ = meter.charge_step(charge);
            }
        }
    }

    // ==================== StateManager Fuzz Tests ====================

    #[test]
    fn fuzz_statemanager_apply_changesets() {
        let mut rng = rand::thread_rng();
        let mut state = StateManager::new_in_memory();
        
        for _ in 0..50 {
            let mut cs = ChangeSet::new();
            let num_accounts = rng.gen_range(1..=10);
            
            for _ in 0..num_accounts {
                let addr = random_account_address(&mut rng);
                let token_type = random_string(&mut rng, 30);
                let balance = rng.gen::<u64>();
                
                cs.add_token_balance_set(addr, token_type, BalanceRecord::new(balance));
            }
            
            // Apply should not panic
            let result = state.apply_changeset(&cs);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn fuzz_statemanager_concurrent_reads() {
        let mut rng = rand::thread_rng();
        let mut state = StateManager::new_in_memory();
        
        // Setup: Create some accounts
        let mut addresses = Vec::new();
        for _ in 0..20 {
            let addr = random_account_address(&mut rng);
            addresses.push(addr);
            
            let mut cs = ChangeSet::new();
            cs.add_token_balance_set(addr, "token".to_string(), BalanceRecord::new(1000));
            state.apply_changeset(&cs).unwrap();
        }
        
        // Random reads should not panic
        for _ in 0..100 {
            let idx = rng.gen_range(0..addresses.len());
            let addr = addresses[idx];
            
            // These operations should not panic
            let _ = state.get_account(&addr);
            let _ = state.get_owned_objects(&addr);
        }
    }

    // ==================== CreatedObject Fuzz Tests ====================

    #[test]
    fn fuzz_created_object_random_data() {
        let mut rng = rand::thread_rng();
        
        for _ in 0..100 {
            let owner = random_account_address(&mut rng);
            let type_str = random_string(&mut rng, 100);
            let data = random_bytes(&mut rng, 1000);
            let version = rng.gen();
            
            let obj = CreatedObject {
                owner,
                uid: None,
                type_: type_str,
                data,
                version,
            };
            
            // Basic validation - should not panic
            assert_eq!(obj.owner, owner);
            assert_eq!(obj.version, version);
        }
    }

    #[test]
    fn fuzz_changeset_serialization_roundtrip() {
        let mut rng = rand::thread_rng();
        
        for _ in 0..50 {
            let mut cs = ChangeSet::new();
            let num_entries = rng.gen_range(1..=20);
            
            for _ in 0..num_entries {
                let addr = random_account_address(&mut rng);
                let token_type = random_string(&mut rng, 50);
                let balance = rng.gen::<u64>();
                cs.add_token_balance_set(addr, token_type, BalanceRecord::new(balance));
            }
            
            // Serialize
            let serialized = bcs::to_bytes(&cs).expect("Serialization should succeed");
            
            // Deserialize
            let deserialized: ChangeSet = bcs::from_bytes(&serialized)
                .expect("Deserialization should succeed");
            
            // Verify roundtrip
            assert_eq!(cs.token_balance_sets.len(), deserialized.token_balance_sets.len());
        }
    }
}
