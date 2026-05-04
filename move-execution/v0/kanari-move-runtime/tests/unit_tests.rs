// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Comprehensive Unit Tests for kanari-move-runtime
//! 
//! This test suite covers all core modules with >90% code coverage goal.

#[cfg(test)]
mod unit_tests {
    use kanari_move_runtime::changeset::{ChangeSet, AccountChange, CreatedObject};
    use kanari_move_runtime::state::{Account, StateManager};
    use kanari_move_runtime::scheduler::TransactionScheduler;
    use kanari_move_runtime::storage::persistent_store::PersistentStore;
    use kanari_move_runtime::kanari_gas_meter::KanariGasMeter;
    use move_core_types::account_address::AccountAddress;
    use kanari_types::balance::BalanceRecord;
    use kanari_types::coin::TreasuryCap;

    // ==================== ChangeSet Tests ====================

    #[test]
    fn test_changeset_new() {
        let cs = ChangeSet::new();
        assert!(cs.account_changes.is_empty());
        assert!(cs.events.is_empty());
        assert!(cs.treasuries.is_empty());
        assert!(cs.nft_caps.is_empty());
        assert!(cs.token_balance_sets.is_empty());
        assert!(cs.created_objects.is_empty());
        assert!(cs.deleted_objects.is_empty());
        assert!(cs.transferred_objects.is_empty());
    }

    #[test]
    fn test_account_change_new() {
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let ac = AccountChange::new(addr);
        assert_eq!(ac.address, addr);
        assert_eq!(ac.balance_delta, 0);
        assert_eq!(ac.sequence_increment, 0);
        assert!(ac.modules_added.is_empty());
    }

    #[test]
    fn test_account_change_debit_credit() {
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let mut ac = AccountChange::new(addr);
        
        ac.debit(100);
        assert_eq!(ac.balance_delta, -100);
        
        ac.credit(50);
        assert_eq!(ac.balance_delta, -50);
        
        ac.credit(100);
        assert_eq!(ac.balance_delta, 50);
    }

    #[test]
    fn test_account_change_sequence_increment() {
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let mut ac = AccountChange::new(addr);
        
        ac.increment_sequence();
        assert_eq!(ac.sequence_increment, 1);
        
        ac.increment_sequence();
        assert_eq!(ac.sequence_increment, 2);
    }

    #[test]
    fn test_account_change_add_module() {
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let mut ac = AccountChange::new(addr);
        
        ac.add_module("module1".to_string());
        ac.add_module("module2".to_string());
        ac.add_module("module1".to_string()); // Duplicate should not be added
        
        assert_eq!(ac.modules_added.len(), 2);
        assert!(ac.modules_added.contains("module1"));
        assert!(ac.modules_added.contains("module2"));
    }

    #[test]
    fn test_changeset_add_token_balance_set() {
        let mut cs = ChangeSet::new();
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let balance = BalanceRecord::new(1000);
        
        cs.add_token_balance_set(addr, "0x2::coin::Coin".to_string(), balance);
        
        assert_eq!(cs.token_balance_sets.len(), 1);
        assert_eq!(cs.token_balance_sets[0].0, addr);
        assert_eq!(cs.token_balance_sets[0].2.value(), 1000);
    }

    #[test]
    fn test_changeset_add_treasury() {
        let mut cs = ChangeSet::new();
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let treasury = TreasuryCap::new(1000000);
        
        cs.add_treasury(addr, "0x2::kanari::KANARI".to_string(), treasury.clone());
        
        assert_eq!(cs.treasuries.len(), 1);
        assert_eq!(cs.treasuries[0].0, addr);
        assert_eq!(cs.treasuries[0].2.total_supply(), treasury.total_supply());
    }

    // ==================== Account Tests ====================

    #[test]
    fn test_account_new() {
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let account = Account::new(addr, 1000);
        
        assert_eq!(account.address, addr);
        assert_eq!(account.balance, 1000);
        assert_eq!(account.sequence_number, 0);
        assert!(account.modules.is_empty());
        assert!(account.token_balances.is_empty());
    }

    #[test]
    fn test_account_add_module() {
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let mut account = Account::new(addr, 1000);
        
        account.add_module("module1".to_string());
        account.add_module("module2".to_string());
        account.add_module("module1".to_string()); // Duplicate
        
        assert_eq!(account.modules.len(), 2);
        assert!(account.modules.contains("module1"));
        assert!(account.modules.contains("module2"));
    }

    #[test]
    fn test_account_set_get_token_balance() {
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let mut account = Account::new(addr, 1000);
        
        let token_type = "0x2::coin::Coin<0x2::kanari::KANARI>";
        account.set_token_balance(token_type.to_string(), BalanceRecord::new(500));
        
        assert_eq!(account.get_token_balance(token_type), 500);
        assert_eq!(account.get_token_balance("nonexistent"), 0);
    }

    #[test]
    fn test_account_to_hex_string() {
        let addr = AccountAddress::from_hex_literal("0x1234").unwrap();
        let account = Account::new(addr, 1000);
        
        assert_eq!(account.to_hex_string(), "0x1234");
    }

    #[test]
    fn test_account_increment_sequence() {
        let addr = AccountAddress::from_hex_literal("0x1").unwrap();
        let mut account = Account::new(addr, 1000);
        
        account.increment_sequence();
        assert_eq!(account.sequence_number, 1);
        
        account.increment_sequence();
        assert_eq!(account.sequence_number, 2);
    }

    // ==================== Scheduler Tests ====================

    #[test]
    fn test_scheduler_empty_input() {
        let txs = vec![];
        let waves = TransactionScheduler::schedule(txs);
        assert!(waves.is_empty());
    }

    #[test]
    fn test_scheduler_single_transaction() {
        use kanari_types::transaction::{Transaction, SignedTransaction};
        
        let tx = create_test_tx("0x1", "module1", None);
        let waves = TransactionScheduler::schedule(vec![tx]);
        
        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0].len(), 1);
    }

    #[test]
    fn test_scheduler_no_conflicts() {
        use kanari_types::transaction::{Transaction, SignedTransaction};
        
        // Three transactions with different senders (no conflicts)
        let tx1 = create_test_tx("0x1", "module1", None);
        let tx2 = create_test_tx("0x2", "module2", None);
        let tx3 = create_test_tx("0x3", "module3", None);
        
        let waves = TransactionScheduler::schedule(vec![tx1, tx2, tx3]);
        
        assert_eq!(waves.len(), 1);
        assert_eq!(waves[0].len(), 3);
    }

    #[test]
    fn test_scheduler_with_object_conflicts() {
        use kanari_types::transaction::{Transaction, SignedTransaction};
        
        // Tx1 and Tx3 access same object, Tx2 is independent
        let tx1 = create_test_tx("0x1", "module1", Some("obj1"));
        let tx2 = create_test_tx("0x2", "module2", Some("obj2"));
        let tx3 = create_test_tx("0x3", "module3", Some("obj1"));
        
        let waves = TransactionScheduler::schedule(vec![tx1, tx2, tx3]);
        
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].len(), 2); // tx1, tx2
        assert_eq!(waves[1].len(), 1); // tx3
    }

    #[test]
    fn test_scheduler_sequential_same_account() {
        use kanari_types::transaction::{Transaction, SignedTransaction};
        
        // All transactions from same account must be sequential
        let tx1 = create_test_tx("0x1", "module1", None);
        let tx2 = create_test_tx("0x1", "module2", None);
        let tx3 = create_test_tx("0x1", "module3", None);
        
        let waves = TransactionScheduler::schedule(vec![tx1, tx2, tx3]);
        
        assert_eq!(waves.len(), 3);
        assert_eq!(waves[0].len(), 1);
        assert_eq!(waves[1].len(), 1);
        assert_eq!(waves[2].len(), 1);
    }

    fn create_test_tx(sender: &str, module: &str, object: Option<&str>) -> SignedTransaction {
        use kanari_types::transaction::Transaction;
        
        let mut args = Vec::new();
        if let Some(obj) = object {
            let mut id = vec![0u8; 32];
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

    // ==================== PersistentStore Tests ====================

    #[test]
    fn test_persistent_store_in_memory() {
        let store = PersistentStore::open_in_memory().unwrap();
        
        let key = b"test_key";
        let value = b"test_value";
        
        store.save_raw(key, value).unwrap();
        
        let retrieved = store.load::<Vec<u8>>(key).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), value.to_vec());
    }

    #[test]
    fn test_persistent_store_serialization() {
        let store = PersistentStore::open_in_memory().unwrap();
        
        let key = b"serialized_key";
        let data = vec![1u8, 2, 3, 4, 5];
        
        store.save(key, &data).unwrap();
        
        let retrieved: Vec<u8> = store.load(key).unwrap().unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_persistent_store_delete() {
        let store = PersistentStore::open_in_memory().unwrap();
        
        let key = b"delete_key";
        let value = b"delete_value";
        
        store.save_raw(key, value).unwrap();
        assert!(store.load::<Vec<u8>>(key).unwrap().is_some());
        
        store.delete(key).unwrap();
        assert!(store.load::<Vec<u8>>(key).unwrap().is_none());
    }

    #[test]
    fn test_persistent_store_nonexistent_key() {
        let store = PersistentStore::open_in_memory().unwrap();
        
        let result = store.load::<Vec<u8>>(b"nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_persistent_store_flush() {
        let store = PersistentStore::open_in_memory().unwrap();
        
        let key = b"flush_key";
        store.save_raw(key, b"flush_value").unwrap();
        
        // Flush should not error
        store.flush().unwrap();
        
        // Data should still be there
        assert!(store.load::<Vec<u8>>(key).unwrap().is_some());
    }

    // ==================== Gas Meter Tests ====================

    #[test]
    fn test_gas_meter_creation() {
        let steps_limit = 10000;
        let meter = KanariGasMeter::new(steps_limit);
        
        assert_eq!(meter.steps_used, 0);
        assert_eq!(meter.steps_limit, steps_limit);
    }

    #[test]
    fn test_gas_meter_charge_step() {
        let steps_limit = 10000;
        let mut meter = KanariGasMeter::new(steps_limit);
        
        let result = meter.charge_step(100);
        assert!(result.is_ok());
        assert_eq!(meter.steps_used, 100);
    }

    #[test]
    fn test_gas_meter_charge_multiple_steps() {
        let steps_limit = 10000;
        let mut meter = KanariGasMeter::new(steps_limit);
        
        meter.charge_step(100).unwrap();
        meter.charge_step(200).unwrap();
        meter.charge_step(300).unwrap();
        
        assert_eq!(meter.steps_used, 600);
    }

    #[test]
    fn test_gas_meter_out_of_gas() {
        let steps_limit = 1000;
        let mut meter = KanariGasMeter::new(steps_limit);
        
        meter.charge_step(900).unwrap();
        let result = meter.charge_step(200);
        
        assert!(result.is_err());
        assert_eq!(meter.steps_used, 1100); // saturating_add
    }

    #[test]
    fn test_gas_meter_exact_limit() {
        let steps_limit = 1000;
        let mut meter = KanariGasMeter::new(steps_limit);
        
        meter.charge_step(1000).unwrap();
        assert_eq!(meter.steps_used, 1000);
        
        let result = meter.charge_step(1);
        assert!(result.is_err());
    }

    #[test]
    fn test_gas_meter_remaining_gas() {
        use move_vm_types::gas::GasMeter;
        
        let steps_limit = 10000;
        let mut meter = KanariGasMeter::new(steps_limit);
        
        meter.charge_step(3000).unwrap();
        
        let remaining = meter.remaining_gas();
        assert_eq!(remaining.into_inner(), 7000);
    }

    #[test]
    fn test_gas_meter_saturating_add() {
        let steps_limit = u64::MAX;
        let mut meter = KanariGasMeter::new(steps_limit);
        
        // Should not overflow
        meter.charge_step(u64::MAX).unwrap();
        assert_eq!(meter.steps_used, u64::MAX);
        
        // Additional charge should still work (saturating)
        meter.charge_step(1).unwrap();
        assert_eq!(meter.steps_used, u64::MAX);
    }

    // ==================== CreatedObject Tests ====================

    #[test]
    fn test_created_object_basic() {
        let owner = AccountAddress::from_hex_literal("0x1").unwrap();
        let created_obj = CreatedObject {
            owner,
            uid: None,
            type_: "0x2::coin::Coin".to_string(),
            data: vec![1, 2, 3],
            version: 1,
        };
        
        assert_eq!(created_obj.owner, owner);
        assert_eq!(created_obj.type_, "0x2::coin::Coin");
        assert_eq!(created_obj.data, vec![1, 2, 3]);
        assert_eq!(created_obj.version, 1);
        assert!(created_obj.uid.is_none());
    }

    #[test]
    fn test_created_object_with_uid() {
        use kanari_types::object::UIDRecord;
        
        let owner = AccountAddress::from_hex_literal("0x1").unwrap();
        let uid_record = UIDRecord::new([0u8; 32], 1);
        
        let created_obj = CreatedObject {
            owner,
            uid: Some(uid_record.clone()),
            type_: "0x2::object::Object".to_string(),
            data: vec![4, 5, 6],
            version: 2,
        };
        
        assert!(created_obj.uid.is_some());
        assert_eq!(created_obj.uid.as_ref().unwrap().id(), uid_record.id());
    }
}
