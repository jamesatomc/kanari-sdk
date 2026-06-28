
use super::BlockchainEngine;
use crate::blockchain::Blockchain;
use crate::consensus::{Checkpoint, PersistentDagState};
use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
use kanari_move_runtime_v1::state::Account;
use kanari_types::balance::BalanceRecord;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use kanari_types::transaction::{SignedTransaction, Transaction};
use move_core_types::account_address::AccountAddress;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn signed_transfer_from(
    sender: &kanari_crypto::keys::KeyPair,
    sequence_number: u64,
) -> SignedTransaction {
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = Transaction::new_transfer(
        sender.tagged_address(),
        recipient.address,
        1,
        sequence_number,
    );
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    signed_tx
}

fn fund_sender(engine: &BlockchainEngine, address: &str, balance: u64) {
    let addr = AccountAddress::from_hex_literal(address).unwrap();
    let mut account = Account::with_native_balance(addr, balance);
    account.set_token_balance(KANARI_TOKEN_TYPE.to_string(), BalanceRecord::new(balance));
    engine
        .state
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .save_account(&account)
        .unwrap();
}

fn secure_consensus_keys(
    authorities: &[String],
    local_authority: &str,
) -> (ed25519_dalek::SigningKey, BTreeMap<String, Vec<u8>>) {
    let mut public_keys = BTreeMap::new();
    let mut local_signing_key = None;

    for (index, authority) in authorities.iter().enumerate() {
        let seed = [index as u8 + 11; 32];
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
        if authority == local_authority {
            local_signing_key = Some(signing_key.clone());
        }
        public_keys.insert(
            authority.clone(),
            signing_key.verifying_key().to_bytes().to_vec(),
        );
    }

    (
        local_signing_key.expect("local authority must be in authority set"),
        public_keys,
    )
}

#[test]
fn mainnet_defaults_enable_strict_runtime_guards() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    unsafe {
        std::env::set_var("KANARI_NETWORK", "mainnet");
        std::env::remove_var("KANARI_REQUIRE_PERSISTENT_STORAGE");
        std::env::remove_var("KANARI_STRICT_CHECKPOINT_ROOTS");
    }

    assert!(BlockchainEngine::strict_persistence_required());
    assert!(BlockchainEngine::strict_checkpoint_roots_required());

    unsafe {
        std::env::remove_var("KANARI_NETWORK");
    }
}

#[test]
fn explicit_env_overrides_strict_runtime_guards() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    unsafe {
        std::env::set_var("KANARI_NETWORK", "mainnet");
        std::env::set_var("KANARI_REQUIRE_PERSISTENT_STORAGE", "false");
        std::env::set_var("KANARI_STRICT_CHECKPOINT_ROOTS", "0");
    }

    assert!(!BlockchainEngine::strict_persistence_required());
    assert!(!BlockchainEngine::strict_checkpoint_roots_required());

    unsafe {
        std::env::remove_var("KANARI_NETWORK");
        std::env::remove_var("KANARI_REQUIRE_PERSISTENT_STORAGE");
        std::env::remove_var("KANARI_STRICT_CHECKPOINT_ROOTS");
    }
}

#[test]
fn dag_engine_requires_explicit_consensus_signing_key() {
    let engine = BlockchainEngine::new_in_memory().unwrap();

    let err = engine.produce_checkpoint().unwrap_err();

    assert!(err.to_string().contains("requires an explicit signing key"));
}

#[test]
fn configured_dag_engine_rejects_empty_checkpoint() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    engine.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    engine
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();

    let err = engine.produce_checkpoint().unwrap_err();

    assert!(err.to_string().contains("No new transactions"));
    assert_eq!(engine.get_stats().height, 0);
}

#[test]
fn restarted_engine_does_not_create_empty_dag_progress() {
    let temp_dir = tempfile::tempdir().unwrap();
    let data_dir = temp_dir.path().to_str().unwrap();
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];

    {
        let mut engine = BlockchainEngine::new_dir(data_dir).unwrap();
        if engine.persistent_store.is_none() {
            return;
        }
        engine.set_authorities("0x1".to_string(), authorities.clone());
        let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
        engine
            .set_consensus_signing_key(local_key, public_keys)
            .unwrap();

        let err = engine.produce_checkpoint().unwrap_err();
        assert!(err.to_string().contains("No new transactions"));
        assert_eq!(engine.get_stats().height, 0);
    }

    let mut restarted = BlockchainEngine::new_dir(data_dir).unwrap();
    if restarted.persistent_store.is_none() {
        return;
    }
    restarted.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    restarted
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();

    assert_eq!(restarted.get_stats().pending_transactions, 0);
    assert_eq!(restarted.get_stats().height, 0);
    let err = restarted.produce_checkpoint().unwrap_err();
    assert!(err.to_string().contains("No new transactions"));
}

#[test]
fn restart_repairs_missing_transaction_history_from_dag_state() {
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 0);

    let genesis = Checkpoint::genesis();
    let prev_hash = genesis.hash().unwrap();
    let vertex_id = [9u8; 32];
    let state_root = vec![7u8; 32];
    let timestamp = 42;

    let broken_checkpoint = Checkpoint::new(
        1,
        vec![vertex_id],
        Vec::new(),
        state_root.clone(),
        timestamp,
        prev_hash.clone(),
    );
    let good_checkpoint = Checkpoint::new(
        1,
        vec![vertex_id],
        vec![tx],
        state_root,
        timestamp,
        prev_hash,
    );

    let mut broken_chain = Blockchain::new();
    broken_chain
        .add_checkpoint_with_validation(broken_checkpoint, false)
        .unwrap();
    let mut chain = Arc::new(RwLock::new(broken_chain));

    let repaired = BlockchainEngine::repair_blockchain_from_dag_state(
        &mut chain,
        Some(&PersistentDagState {
            vertices: Vec::new(),
            checkpoints: vec![genesis, good_checkpoint],
            current_round: 1,
            last_checkpoint_round: 1,
        }),
    )
    .unwrap();

    assert!(repaired);
    let repaired_chain = chain.read().unwrap_or_else(|e| e.into_inner());
    assert_eq!(repaired_chain.height(), 1);
    assert_eq!(repaired_chain.get_transaction_count(), 1);
    assert_eq!(repaired_chain.latest_checkpoint().transactions.len(), 1);
}

#[test]
fn committed_transaction_history_survives_metadata_stripping() {
    let temp_dir = tempfile::tempdir().unwrap();
    let data_dir = temp_dir.path().to_str().unwrap();
    let engine = BlockchainEngine::new_dir(data_dir).unwrap();
    if engine.persistent_store.is_none() {
        return;
    }

    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 0);
    let tx_hash = tx.transaction_hash().to_vec();
    let genesis_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };
    let checkpoint = Checkpoint::new(
        1,
        vec![[7u8; 32]],
        vec![tx],
        vec![9u8; 32],
        42,
        genesis_hash,
    );

    {
        let mut chain = engine.blockchain.write().unwrap_or_else(|e| e.into_inner());
        chain
            .add_checkpoint_with_validation(checkpoint, false)
            .unwrap();
        engine.persist_blockchain_snapshot(&chain).unwrap();
    }

    let latest = engine.list_committed_transactions_from_history(10, |_| true);
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].1, 1);
    assert_eq!(latest[0].0.transaction_hash(), tx_hash.as_slice());

    let found = engine
        .get_committed_transaction_from_history(&tx_hash)
        .expect("transaction must be found in persistent history");
    assert_eq!(found.1, 1);
    assert_eq!(found.0.transaction_hash(), tx_hash.as_slice());
}

#[test]
fn batch_submit_accepts_contiguous_sequences_for_same_sender() {
    let engine = BlockchainEngine::new().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx0 = signed_transfer_from(&sender, 0);
    let tx1 = signed_transfer_from(&sender, 1);

    let hashes = engine.submit_transactions_batch(vec![tx0, tx1]).unwrap();

    assert_eq!(hashes.len(), 2);
    assert_eq!(engine.pending_transaction_len(), 2);
}

#[test]
fn batch_submit_accepts_shuffled_contiguous_sequences_for_same_sender() {
    let engine = BlockchainEngine::new().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx0 = signed_transfer_from(&sender, 0);
    let tx1 = signed_transfer_from(&sender, 1);
    let tx2 = signed_transfer_from(&sender, 2);

    let hashes = engine
        .submit_transactions_batch(vec![tx2.clone(), tx0.clone(), tx1.clone()])
        .unwrap();

    assert_eq!(hashes.len(), 3);
    let pending = engine.pending_transactions_snapshot();
    let pending_sequences = pending
        .iter()
        .map(|tx| tx.transaction.sequence_number())
        .collect::<Vec<_>>();
    assert_eq!(pending_sequences, vec![0, 1, 2]);
}

#[test]
fn gas_application_does_not_increment_sequence_twice() {
    let sender = AccountAddress::random();
    let mut changeset = ChangeSet::new();
    changeset.get_or_create_change(sender).increment_sequence();

    BlockchainEngine::apply_gas_and_sequence(&mut changeset, sender, 10, 10).unwrap();

    let sender_change = changeset.account_changes.get(&sender).unwrap();
    assert_eq!(sender_change.sequence_increment, 1);
    assert_eq!(sender_change.balance_delta, -10);
}

#[test]
fn failed_transaction_cannot_mint_unpaid_gas_to_dao() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let signed_tx = signed_transfer_from(&sender, 0);
    let dao =
        AccountAddress::from_hex_literal(kanari_types::address::Address::DAO_ADDRESS).unwrap();
    let (supply_before, dao_balance_before) = {
        let state = engine.state_read();
        (
            state.total_supply,
            state
                .get_account(&dao)
                .map(|account| account.native_balance())
                .unwrap_or(0),
        )
    };

    let (_, changeset) = engine.execute_transaction_immediate(signed_tx).unwrap();
    assert!(!changeset.success);
    {
        let mut state = engine.state_write();
        state.apply_changeset(&changeset).unwrap();
        assert_eq!(state.total_supply, supply_before);
        assert_eq!(
            state
                .get_account(&dao)
                .map(|account| account.native_balance())
                .unwrap_or(0),
            dao_balance_before
        );
        let sender_address = AccountAddress::from_hex_literal(&sender.address).unwrap();
        let sender_account = state.get_account(&sender_address).unwrap();
        assert_eq!(sender_account.native_balance(), 0);
        assert_eq!(sender_account.sequence_number, 1);
    }
}

#[test]
fn batch_submit_rejects_duplicate_transactions() {
    let engine = BlockchainEngine::new().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 0);

    let err = engine
        .submit_transactions_batch(vec![tx.clone(), tx])
        .unwrap_err();

    assert!(err.to_string().contains("already in pending pool"));
}

#[test]
fn batch_submit_rejects_transaction_already_indexed_in_pending_pool() {
    let engine = BlockchainEngine::new().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 0);

    engine.submit_transactions_batch(vec![tx.clone()]).unwrap();
    let err = engine.submit_transactions_batch(vec![tx]).unwrap_err();

    assert!(err.to_string().contains("already in pending pool"));
}

#[test]
fn batch_submit_rejects_sequence_gaps() {
    let engine = BlockchainEngine::new().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 1);

    let err = engine.submit_transactions_batch(vec![tx]).unwrap_err();

    assert!(err.to_string().contains("Sequence number too high"));
}

#[test]
fn deterministic_parallel_execution_matches_strict_serial_root() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let mut txs = Vec::new();

    for _ in 0..16 {
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        fund_sender(&engine, &sender.address, 1_000_000);

        let tx =
            Transaction::new_transfer(sender.tagged_address(), recipient.address.clone(), 1, 0);
        let mut signed_tx = SignedTransaction::new(tx);
        signed_tx
            .sign(&sender.private_key, sender.curve_type)
            .unwrap();
        txs.push(signed_tx);
    }

    let base_state = engine
        .state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let strict_state = Arc::new(RwLock::new(base_state.clone()));
    let parallel_state = Arc::new(RwLock::new(base_state));

    let strict_counts = engine
        .execute_tx_waves_parallel(txs.clone(), &strict_state, Some(123), false, true)
        .unwrap();
    let parallel_counts = engine
        .execute_tx_waves_deterministic_parallel(txs, &parallel_state, Some(123), false)
        .unwrap();

    let strict_root = strict_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();
    let parallel_root = parallel_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();

    assert_eq!(strict_counts, parallel_counts);
    assert_eq!(strict_root, parallel_root);
}

#[test]
fn account_info_prefers_native_state_balance_over_stale_coin_object() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let owner = AccountAddress::from_hex_literal("0x1111").unwrap();

    let mut coin_data = vec![0u8; 32];
    coin_data.extend_from_slice(&1_000u64.to_le_bytes());
    let mut cs = ChangeSet::new();
    cs.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
            data: coin_data,
            version: 1,
        },
    ));
    engine
        .state_write()
        .apply_changeset_without_supply_validation(&cs)
        .unwrap();

    let mut account = Account::with_native_balance(owner, 790);
    account.set_token_balance(KANARI_TOKEN_TYPE.to_string(), BalanceRecord::new(790));
    engine.state_write().save_account(&account).unwrap();

    let info = engine
        .get_account_info("0x1111")
        .expect("account info should be available");

    assert_eq!(
        info.token_balances.get(KANARI_TOKEN_TYPE).copied(),
        Some(790)
    );
}
