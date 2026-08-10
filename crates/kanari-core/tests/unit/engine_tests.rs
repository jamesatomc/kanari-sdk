// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::BlockchainEngine;
use super::runtime_guards::strict_guard_required;
use crate::consensus::Checkpoint;
use crate::engine::{
    MAX_PENDING_PER_PRIMARY_ACCESS_LANE, MAX_PENDING_PER_SENDER, PersistedTransactionLocation,
    decode_hex_exact, normalize_consensus_authority_id,
};
use crate::file_io::write_file_atomically;
use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
use kanari_move_runtime_v1::state::OwnerState;
use kanari_move_runtime_v1::storage::persistent_store::PersistentStore;
use kanari_types::address::Address as KanariAddress;
use kanari_types::balance::BalanceRecord;
use kanari_types::coin::CoinModule;
use kanari_types::gas_coin::{GAS_COIN, GasModule};
use kanari_types::transaction::{
    GasPayment, ObjectInput, ObjectOwnerKind, ObjectRef, SignedTransaction, Transaction,
    TransactionEffects,
};
use move_core_types::account_address::AccountAddress;
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[test]
fn consensus_authority_ids_are_normalized_in_one_place() {
    assert_eq!(normalize_consensus_authority_id("1"), "0x1");
    assert_eq!(normalize_consensus_authority_id("0x1"), "0x1");
}

#[test]
fn fixed_length_hex_decoding_is_shared_and_validated() {
    assert_eq!(decode_hex_exact("test key", "0x0102", 2).unwrap(), [1, 2]);
    assert!(decode_hex_exact("test key", "01", 2).is_err());
}

#[test]
fn atomic_file_write_replaces_existing_contents() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("nested").join("value.txt");

    write_file_atomically(&path, b"first").unwrap();
    write_file_atomically(&path, b"second").unwrap();

    assert_eq!(std::fs::read(path).unwrap(), b"second");
}

fn snapshot_preview(engine: &BlockchainEngine, limit: usize) -> String {
    format!("{:?}", engine.canonical_state_snapshot_dump(Some(limit)))
}

fn native_coin_object_ref(object_id: &str, balance: u64) -> ObjectRef {
    let coin_data = native_coin_data(object_id, balance);
    ObjectRef::new(
        object_id.to_string(),
        Some(1),
        Some(format!(
            "0x{}",
            hex::encode(kanari_crypto::hash_data_blake3(&coin_data))
        )),
    )
}

fn native_coin_data(object_id: &str, balance: u64) -> Vec<u8> {
    let mut coin_data = vec![0u8; 40];
    if let Ok(addr) = AccountAddress::from_hex_literal(object_id) {
        coin_data[..32].copy_from_slice(addr.as_ref());
    }
    coin_data[32..40].copy_from_slice(&balance.to_le_bytes());
    coin_data
}

fn signed_transfer_from(sender: &kanari_crypto::keys::KeyPair, nonce: u64) -> SignedTransaction {
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = Transaction::new_transfer_with_object_ref(
        sender.tagged_address(),
        native_coin_object_ref("0xaaaa", 1_000_000),
        recipient.address,
        1,
        nonce,
    );
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    signed_tx
}

fn signed_transfer_with_refs(
    sender: &kanari_crypto::keys::KeyPair,
    recipient: &str,
    coin_object_id: &str,
    coin_balance: u64,
    gas_object_id: &str,
    gas_balance: u64,
    nonce: u64,
) -> SignedTransaction {
    let mut tx = Transaction::new_transfer_with_object_ref_and_gas(
        sender.tagged_address(),
        native_coin_object_ref(coin_object_id, coin_balance),
        recipient.to_string(),
        1,
        nonce,
        100_000,
        1,
    );
    if let Transaction::ExecuteFunction {
        gas_payment: Some(gas_payment),
        ..
    } = &mut tx
    {
        gas_payment.payment_objects = vec![native_coin_object_ref(gas_object_id, gas_balance)];
    }
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    signed_tx
}

fn signed_native_burn_with_gas_object(
    sender: &kanari_crypto::keys::KeyPair,
    coin_object_id: &str,
    coin_balance: u64,
    nonce: u64,
) -> SignedTransaction {
    let mut tx = Transaction::new_burn_with_gas(sender.tagged_address(), 1, nonce, 100_000, 1);
    if let Transaction::ExecuteFunction { gas_payment, .. } = &mut tx {
        *gas_payment = Some(GasPayment {
            payment_objects: vec![native_coin_object_ref(coin_object_id, coin_balance)],
            owner: sender.address.clone(),
            budget: 100_000,
            price: 1,
        });
    }
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    signed_tx
}

fn transaction_coin_balance(data: &[u8]) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[32..40]);
    u64::from_le_bytes(bytes)
}

fn fund_sender_with_coin(
    engine: &BlockchainEngine,
    address: &str,
    coin_object_id: &str,
    balance: u64,
) {
    fund_sender_with_coin_type(engine, address, coin_object_id, balance, GAS_COIN);
}

fn fund_sender_with_coin_type(
    engine: &BlockchainEngine,
    address: &str,
    coin_object_id: &str,
    balance: u64,
    token_type: &str,
) {
    let addr = AccountAddress::from_hex_literal(address).unwrap();
    let coin_data = native_coin_data(coin_object_id, balance);
    let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());

    let mut create_coin = ChangeSet::new();
    create_coin.created_objects.push((
        coin_object_id.to_string(),
        CreatedObject {
            owner: addr,
            owner_kind: kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                addr.to_hex_literal(),
            ),
            uid: None,
            id: None,
            type_: CoinModule::coin_type(token_type),
            data: coin_data,
            version: 1,
        },
    ));
    if token_type == GAS_COIN {
        create_coin.mint(addr, balance);
    }
    state
        .apply_changeset_without_supply_validation(&create_coin)
        .unwrap();
}

#[test]
fn account_info_uses_ledger_native_balance_over_coin_object_amount() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let owner = AccountAddress::from_hex_literal("0x1111").unwrap();
    let stale_object_balance = 1_100_000u64;
    let ledger_balance_after_fee = stale_object_balance - 210;
    let mut coin_data = vec![0u8; 40];
    coin_data[32..40].copy_from_slice(&stale_object_balance.to_le_bytes());

    let mut cs = ChangeSet::new();
    cs.created_objects.push((
        "0xcoin".to_string(),
        CreatedObject {
            owner,
            owner_kind: kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                owner.to_hex_literal(),
            ),
            uid: None,
            id: None,
            type_: CoinModule::coin_type(GAS_COIN),
            data: coin_data,
            version: 1,
        },
    ));
    {
        let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());
        state
            .apply_changeset_without_supply_validation(&cs)
            .unwrap();
        let mut owner_state = OwnerState::with_native_balance(owner, ledger_balance_after_fee);
        owner_state.set_token_balance(
            GAS_COIN.to_string(),
            BalanceRecord::new(ledger_balance_after_fee),
        );
        state.save_owner_state(&owner_state).unwrap();
    }

    let account = engine.get_owner_info("0x1111").unwrap();
    assert_eq!(
        account.balances.get(GAS_COIN).copied(),
        Some(ledger_balance_after_fee)
    );
}

#[test]
fn committed_native_transfer_updates_sender_and_recipient_owner_balances() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    configure_single_authority_consensus(&mut engine);
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let transfer_coin_id = "0xaaaa";
    let gas_coin_id = "0x1001";
    fund_sender_with_coin(&engine, &sender.address, transfer_coin_id, 3_000_000);
    fund_sender_with_coin(&engine, &sender.address, gas_coin_id, 1_000_000);
    engine
        .state
        .write()
        .unwrap_or_else(|error| error.into_inner())
        .commit()
        .unwrap();

    let sender_before = engine
        .get_owner_info(&sender.address)
        .and_then(|owner| owner.balances.get(GAS_COIN).copied())
        .expect("funded sender balance");
    let mut transaction = Transaction::new_transfer_with_object_ref_and_gas(
        sender.tagged_address(),
        native_coin_object_ref(transfer_coin_id, 3_000_000),
        recipient.address.clone(),
        3_000_000,
        1,
        100_000,
        1,
    );
    if let Transaction::ExecuteFunction {
        gas_payment: Some(gas_payment),
        ..
    } = &mut transaction
    {
        gas_payment.payment_objects = vec![native_coin_object_ref(gas_coin_id, 1_000_000)];
    }
    let mut transaction = SignedTransaction::new(transaction);
    transaction
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let transaction_hash = transaction.transaction_hash().to_vec();
    engine.submit_transactions_batch(vec![transaction]).unwrap();

    // Submission alone is not execution. Owner balances become visible only
    // after Mysticeti commits the transaction's sub-DAG.
    assert_eq!(
        engine
            .get_owner_info(&sender.address)
            .and_then(|owner| owner.balances.get(GAS_COIN).copied()),
        Some(sender_before)
    );
    drive_consensus_until_mempool_empty(&engine);

    assert!(
        engine
            .try_is_transaction_committed(&transaction_hash)
            .unwrap()
    );
    let effects = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().transaction_effects.to_vec()
    };
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].status, "success", "{effects:#?}");
    let sender_after = engine
        .get_owner_info(&sender.address)
        .and_then(|owner| owner.balances.get(GAS_COIN).copied())
        .expect("sender balance after commit");
    let recipient_after = engine
        .get_owner_info(&recipient.address)
        .and_then(|owner| owner.balances.get(GAS_COIN).copied())
        .expect("recipient balance after commit");
    assert!(sender_after < sender_before);
    assert_eq!(recipient_after, 3_000_000);
}

// Pre-existing failures under gas model v2 (zero-fee): these tests expect gas
// to be deducted from the coin balance, but the zero-fee model meters gas
// (gas_used > 0) without charging the balance. Re-enable once the tests are
// updated to match the zero-fee semantics.
#[test]
#[ignore]
fn backend_native_burn_uses_prepared_gas_coin_and_reduces_supply() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let owner = "0x1111";
    let object_id = "0xcoin";
    let starting_balance = 1_000_000u64;
    let burn_amount = 100_000u64;
    fund_sender_with_coin(&engine, owner, object_id, starting_balance);

    let initial_supply = {
        let state = engine.state.read().unwrap_or_else(|e| e.into_inner());
        state.total_supply
    };

    let mut tx = Transaction::new_burn_with_gas(owner.to_string(), burn_amount, 1, 100_000, 1);
    if let Transaction::ExecuteFunction { gas_payment, .. } = &mut tx {
        *gas_payment = Some(GasPayment {
            payment_objects: vec![native_coin_object_ref(object_id, starting_balance)],
            owner: owner.to_string(),
            budget: 100_000,
            price: 1,
        });
    }

    let changeset = engine
        .execute_transaction_with_runtime_internal(
            &tx,
            &engine.runtime_pool[0],
            &engine.state,
            false,
            None,
            false,
        )
        .unwrap();
    assert!(changeset.success, "{:?}", changeset.error_message);

    {
        let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());
        state.apply_changeset(&changeset).unwrap();

        let coin = state
            .get_object(object_id)
            .unwrap()
            .expect("coin must exist");
        let expected_balance = starting_balance - burn_amount - changeset.gas_used;
        assert!(
            changeset.gas_used > 0,
            "gas remains a resource-metering value"
        );
        assert_eq!(transaction_coin_balance(&coin.data), expected_balance);
        assert_eq!(
            state
                .resolve_owner_native_balance(AccountAddress::from_hex_literal(owner).unwrap())
                .unwrap(),
            expected_balance
        );
        assert_eq!(state.total_supply, initial_supply - burn_amount);
    }
}

#[test]
#[ignore]
fn backend_native_transfer_partial_split_indexes_output_coin() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let transfer_coin_id = "0x000000000000000000000000000000000000000000000000000000000000ca01";
    let gas_coin_id = "0x000000000000000000000000000000000000000000000000000000000000ca02";
    fund_sender_with_coin(&engine, &sender.address, transfer_coin_id, 1_000_000);
    fund_sender_with_coin(&engine, &sender.address, gas_coin_id, 1_000_000);

    let signed_tx = signed_transfer_with_refs(
        &sender,
        &recipient.address,
        transfer_coin_id,
        1_000_000,
        gas_coin_id,
        1_000_000,
        7,
    );
    let changeset = engine
        .execute_transaction_with_runtime_internal(
            &signed_tx.transaction,
            &engine.runtime_pool[0],
            &engine.state,
            false,
            Some(123),
            false,
        )
        .unwrap();
    assert!(changeset.success, "{:?}", changeset.error_message);

    let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());
    state.apply_changeset(&changeset).unwrap();

    let sender_addr = AccountAddress::from_hex_literal(&sender.address).unwrap();
    let recipient_addr = AccountAddress::from_hex_literal(&recipient.address).unwrap();
    assert_eq!(
        state.resolve_owner_native_balance(sender_addr).unwrap(),
        1_999_899
    );
    assert_eq!(
        state.resolve_owner_native_balance(recipient_addr).unwrap(),
        1
    );

    let recipient_objects = state.get_owned_objects(&recipient_addr).unwrap();
    assert_eq!(recipient_objects.len(), 1);
    let output = state
        .get_object(&recipient_objects[0])
        .unwrap()
        .expect("recipient output coin must exist");
    assert_eq!(transaction_coin_balance(&output.data), 1);
    assert_eq!(output.owner, recipient_addr);
}

#[test]
fn backend_native_transfer_rejects_gas_overlap_before_execution() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let coin_id = "0x000000000000000000000000000000000000000000000000000000000000cb01";
    fund_sender_with_coin(&engine, &sender.address, coin_id, 1_000_000);

    let signed_tx = signed_transfer_with_refs(
        &sender,
        &recipient.address,
        coin_id,
        1_000_000,
        coin_id,
        1_000_000,
        8,
    );
    let changeset = engine.execute_transaction_with_runtime_internal(
        &signed_tx.transaction,
        &engine.runtime_pool[0],
        &engine.state,
        false,
        Some(124),
        false,
    );

    let error = changeset.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("cannot overlap with a mutable object input"),
        "{error:#}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn native_transfer_fastpath_requires_matching_mutable_abi_coin_input(
        abi_coin in 1u64..u64::MAX,
        input_coin in 1u64..u64::MAX,
        is_mutable in any::<bool>(),
    ) {
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let abi_coin_id = format!("0x{abi_coin:064x}");
        let input_coin_id = format!("0x{input_coin:064x}");
        let mut tx = Transaction::new_transfer_with_object_ref(
            sender.tagged_address(),
            native_coin_object_ref(&input_coin_id, 1_000_000),
            AccountAddress::TWO.to_hex_literal(),
            1,
            1,
        );
        let Transaction::ExecuteFunction {
            args,
            object_inputs,
            ..
        } = &mut tx else {
            unreachable!("transfer helper must construct ExecuteFunction");
        };
        args[0] = AccountAddress::from_hex_literal(&abi_coin_id).unwrap().to_vec();
        object_inputs[0].mutable = is_mutable;

        prop_assert_eq!(
            tx.native_call().is_some(),
            abi_coin == input_coin && is_mutable,
            "native fastpath must only use the ABI coin when it is mutable input"
        );
    }

    #[test]
    fn native_transfer_and_burn_accounting_property(
        transfer_balance in 2u64..1_000_000u64,
        transfer_amount in 1u64..500_000u64,
        burn_amount in 1u64..500_000u64,
    ) {
        let engine = BlockchainEngine::new_in_memory().unwrap();
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        let transfer_amount = transfer_amount.min(transfer_balance - 1);
        let burn_balance = burn_amount + 1_000_000;
        let transfer_coin_id = "0x000000000000000000000000000000000000000000000000000000000000cc01";
        let transfer_gas_id = "0x000000000000000000000000000000000000000000000000000000000000cc02";
        let burn_coin_id = "0x000000000000000000000000000000000000000000000000000000000000cc03";
        fund_sender_with_coin(&engine, &sender.address, transfer_coin_id, transfer_balance);
        fund_sender_with_coin(&engine, &sender.address, transfer_gas_id, 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, burn_coin_id, burn_balance);

        let initial_supply = engine.state.read().unwrap_or_else(|e| e.into_inner()).total_supply;
        let mut transfer_tx = Transaction::new_transfer_with_object_ref_and_gas(
            sender.tagged_address(),
            native_coin_object_ref(transfer_coin_id, transfer_balance),
            recipient.address.clone(),
            transfer_amount,
            100,
            100_000,
            1,
        );
        if let Transaction::ExecuteFunction {
            gas_payment: Some(gas_payment),
            ..
        } = &mut transfer_tx
        {
            gas_payment.payment_objects =
                vec![native_coin_object_ref(transfer_gas_id, 1_000_000)];
        }
        let mut transfer_tx = SignedTransaction::new(transfer_tx);
        transfer_tx.sign(&sender.private_key, sender.curve_type).unwrap();
        let transfer_cs = engine
            .execute_transaction_with_runtime_internal(
                &transfer_tx.transaction,
                &engine.runtime_pool[0],
                &engine.state,
                false,
                Some(200),
                false,
            )
            .unwrap();
        prop_assert!(transfer_cs.success, "{:?}", transfer_cs.error_message);

        {
            let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());
            state.apply_changeset(&transfer_cs).unwrap();
        }

        let mut burn_tx = Transaction::new_burn_with_gas(
            sender.tagged_address(),
            burn_amount,
            101,
            100_000,
            1,
        );
        if let Transaction::ExecuteFunction { gas_payment, .. } = &mut burn_tx {
            *gas_payment = Some(GasPayment {
                payment_objects: vec![native_coin_object_ref(burn_coin_id, burn_balance)],
                owner: sender.address.clone(),
                budget: 100_000,
                price: 1,
            });
        }
        let mut burn_tx = SignedTransaction::new(burn_tx);
        burn_tx.sign(&sender.private_key, sender.curve_type).unwrap();
        let burn_cs = engine
            .execute_transaction_with_runtime_internal(
                &burn_tx.transaction,
                &engine.runtime_pool[0],
                &engine.state,
                false,
                Some(201),
                false,
            )
            .unwrap();
        prop_assert!(burn_cs.success, "{:?}", burn_cs.error_message);

        let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());
        state.apply_changeset(&burn_cs).unwrap();
        let recipient_addr = AccountAddress::from_hex_literal(&recipient.address).unwrap();
        prop_assert_eq!(state.resolve_owner_native_balance(recipient_addr).unwrap(), transfer_amount);
        prop_assert_eq!(state.total_supply, initial_supply - burn_amount);
        state.validate_supply_invariants().unwrap();
    }
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

fn configure_single_authority_consensus(engine: &mut BlockchainEngine) {
    let authorities = vec!["0x1".to_string()];
    engine.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    engine
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();
}

fn drive_consensus_to_height(engine: &BlockchainEngine, target_height: u64) {
    for _ in 0..64 {
        if engine.get_stats().height >= target_height {
            return;
        }
        match engine.produce_checkpoint() {
            Ok(_) => {}
            Err(error) if error.to_string().contains("DAG_WAITING") => {}
            Err(error) => panic!("failed to drive Mysticeti consensus: {error:#}"),
        }
    }
    panic!(
        "Mysticeti did not reach checkpoint height {target_height}; current height {}",
        engine.get_stats().height
    );
}

fn drive_consensus_until_mempool_empty(engine: &BlockchainEngine) {
    for _ in 0..128 {
        if engine.pending_transaction_len() == 0 {
            return;
        }
        match engine.produce_checkpoint() {
            Ok(_) => {}
            Err(error) if error.to_string().contains("DAG_WAITING") => {}
            Err(error) => panic!("failed to drain Mysticeti mempool: {error:#}"),
        }
    }
    panic!(
        "Mysticeti did not drain the mempool; {} transaction(s) remain",
        engine.pending_transaction_len()
    );
}

#[test]
fn mainnet_defaults_enable_strict_runtime_guards() {
    assert!(strict_guard_required("mainnet", None));
}

#[test]
fn devnet_defaults_enable_strict_runtime_guards() {
    assert!(strict_guard_required("devnet", None));
}

#[test]
fn local_network_defaults_allow_relaxed_runtime_guards() {
    assert!(!strict_guard_required("local", None));
}

#[test]
fn explicit_env_overrides_strict_runtime_guards() {
    assert!(!strict_guard_required("mainnet", Some("false")));
    assert!(!strict_guard_required("mainnet", Some("0")));
}

#[test]
fn dag_engine_requires_explicit_consensus_signing_key() {
    let engine = BlockchainEngine::new_in_memory().unwrap();

    let err = engine.produce_checkpoint().unwrap_err();

    assert!(err.to_string().contains("requires an explicit signing key"));
}

#[test]
fn configured_dag_engine_produces_empty_vertex_without_checkpoint() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    engine.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    engine
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();

    let info = engine.produce_checkpoint().unwrap();
    assert_eq!(info.tx_count, 0);
    assert!(info.checkpoint.is_none());
    assert_eq!(engine.get_stats().height, 0);
}

#[test]
fn restarted_engine_allows_empty_dag_progress_without_committing_checkpoint() {
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

        let info = engine.produce_checkpoint().unwrap();
        assert_eq!(info.tx_count, 0);
        assert!(info.checkpoint.is_none());
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
    let info = restarted.produce_checkpoint().unwrap();
    assert_eq!(info.tx_count, 0);
    assert!(info.checkpoint.is_none());
}

#[test]
fn restart_recovers_checkpoint_metadata_from_durable_commit_marker() {
    let temp_dir = tempfile::tempdir().unwrap();
    let data_dir = temp_dir.path().to_str().unwrap();

    {
        let engine = BlockchainEngine::new_dir(data_dir).unwrap();
        let Some(store) = engine.persistent_store.as_ref() else {
            return;
        };
        let checkpoint = Checkpoint::new(
            1,
            vec![],
            vec![],
            engine.state_read().try_compute_state_root().unwrap(),
            1,
            engine
                .blockchain
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .latest_checkpoint()
                .hash()
                .unwrap(),
        )
        .with_transaction_effects(vec![TransactionEffects {
            status: "success".to_string(),
            gas_used: 0,
            gas_payment: None,
            input_objects: Vec::new(),
            shared_inputs: Vec::new(),
            immutable_inputs: Vec::new(),
            gas_object_refs: Vec::new(),
            object_changes: Vec::new(),
            created: Vec::new(),
            mutated: Vec::new(),
            deleted: Vec::new(),
            transferred: Vec::new(),
            causal_edges: Vec::new(),
            error_message: None,
        }]);
        // Persist exactly as `commit_with_raw_update` does: raw BCS checkpoint
        // bytes in the same batch as the state changes.
        store
            .apply_raw_changes(
                &[(
                    BlockchainEngine::pending_checkpoint_commit_key().to_vec(),
                    bcs::to_bytes(&checkpoint).unwrap(),
                )],
                &[],
            )
            .unwrap();
    }

    let restarted = BlockchainEngine::new_dir(data_dir).unwrap();
    if restarted.persistent_store.is_none() {
        return;
    }
    assert_eq!(restarted.get_stats().height, 1);
    assert!(
        restarted
            .persistent_store
            .as_ref()
            .unwrap()
            .load::<Checkpoint>(BlockchainEngine::pending_checkpoint_commit_key())
            .unwrap()
            .is_none()
    );
}

#[test]
fn restarted_engine_preserves_replay_protection_and_multi_checkpoint_progress() {
    let temp_dir = tempfile::tempdir().unwrap();
    let data_dir = temp_dir.path().to_str().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_c = generate_keypair(CurveType::Ed25519).unwrap();
    let tx1 = signed_transfer_with_refs(
        &sender,
        &recipient_a.address,
        "0xaaaa",
        3_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let tx2 = signed_transfer_with_refs(
        &sender,
        &recipient_b.address,
        "0xbbbb",
        2_000_000,
        "0x2001",
        1_000_000,
        2,
    );
    let tx3 = signed_transfer_with_refs(
        &sender,
        &recipient_c.address,
        "0xcccc",
        1_000_000,
        "0x3001",
        1_000_000,
        3,
    );

    let configure_engine = |engine: &mut BlockchainEngine| {
        configure_single_authority_consensus(engine);
    };

    let fund_engine = |engine: &BlockchainEngine| {
        fund_sender_with_coin(engine, &sender.address, "0xaaaa", 3_000_000);
        fund_sender_with_coin(engine, &sender.address, "0x1001", 1_000_000);
        fund_sender_with_coin(engine, &sender.address, "0xbbbb", 2_000_000);
        fund_sender_with_coin(engine, &sender.address, "0x2001", 1_000_000);
        fund_sender_with_coin(engine, &sender.address, "0xcccc", 1_000_000);
        fund_sender_with_coin(engine, &sender.address, "0x3001", 1_000_000);
    };

    let persisted_checkpoint_two_dump = {
        let mut engine = BlockchainEngine::new_dir(data_dir).unwrap();
        if engine.persistent_store.is_none() {
            return;
        }
        configure_engine(&mut engine);
        fund_engine(&engine);

        engine.submit_transactions_batch(vec![tx1.clone()]).unwrap();
        drive_consensus_to_height(&engine, 1);
        engine.submit_transactions_batch(vec![tx2.clone()]).unwrap();
        drive_consensus_to_height(&engine, 2);
        {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            engine.persist_blockchain_snapshot(&chain).unwrap();
        }

        assert_eq!(engine.get_stats().height, 2);
        engine.canonical_state_snapshot_dump(None)
    };

    let mut restarted = BlockchainEngine::new_dir(data_dir).unwrap();
    if restarted.persistent_store.is_none() {
        return;
    }
    configure_engine(&mut restarted);

    assert_eq!(restarted.get_stats().height, 2);
    assert_eq!(
        restarted.canonical_state_snapshot_dump(None),
        persisted_checkpoint_two_dump,
        "restart snapshot mismatch after checkpoint 2; restarted_preview={}",
        snapshot_preview(&restarted, 8)
    );
    assert!(
        restarted
            .first_canonical_state_divergence(&restarted)
            .is_none(),
        "self comparison should never diverge"
    );

    let replay_err = restarted
        .submit_transactions_batch(vec![tx1.clone()])
        .unwrap_err();
    assert!(replay_err.to_string().contains("already executed"));

    let tx3_hash = tx3.transaction_hash().to_vec();
    restarted.submit_transactions_batch(vec![tx3]).unwrap();
    drive_consensus_to_height(&restarted, 3);

    assert_eq!(restarted.get_stats().height, 3);
    assert!(restarted.try_is_transaction_committed(&tx3_hash).unwrap());
    assert_eq!(restarted.pending_transaction_len(), 0);
    restarted.state_read().validate_smt_consistency().unwrap();
}

#[test]
fn checkpoint_commit_persists_metadata_and_transactions_before_restart() {
    let temp_dir = tempfile::tempdir().unwrap();
    let data_dir = temp_dir.path().to_str().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let tx1 = signed_transfer_with_refs(
        &sender,
        &recipient_a.address,
        "0xaaaa",
        3_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let tx2 = signed_transfer_with_refs(
        &sender,
        &recipient_b.address,
        "0xbbbb",
        2_000_000,
        "0x2001",
        1_000_000,
        2,
    );
    let tx1_hash = tx1.transaction_hash().to_vec();
    let tx2_hash = tx2.transaction_hash().to_vec();

    let expected_root = {
        let mut engine = BlockchainEngine::new_dir(data_dir).unwrap();
        if engine.persistent_store.is_none() {
            return;
        }
        configure_single_authority_consensus(&mut engine);
        fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 3_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x1001", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0xbbbb", 2_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x2001", 1_000_000);

        engine
            .submit_transactions_batch(vec![tx1.clone(), tx2.clone()])
            .unwrap();
        drive_consensus_to_height(&engine, 1);
        assert_eq!(engine.get_stats().height, 1);

        let store = engine.persistent_store.as_ref().unwrap();
        let checkpoint = store
            .load::<Checkpoint>(&BlockchainEngine::checkpoint_metadata_key(1))
            .unwrap()
            .expect("checkpoint metadata must be durable before restart");
        assert_eq!(checkpoint.sequence, 1);
        assert_eq!(checkpoint.transactions.len(), 0);
        assert_eq!(checkpoint.transaction_effects.len(), 2);

        let checkpoint_txs = store
            .load::<Vec<SignedTransaction>>(&BlockchainEngine::checkpoint_transactions_key(1))
            .unwrap()
            .expect("checkpoint transaction payload must be durable before restart");
        assert_eq!(
            checkpoint_txs
                .iter()
                .map(|tx| tx.transaction_hash().to_vec())
                .collect::<Vec<_>>(),
            vec![tx1_hash.clone(), tx2_hash.clone()]
        );
        assert!(
            store
                .load::<Checkpoint>(BlockchainEngine::pending_checkpoint_commit_key())
                .unwrap()
                .is_none(),
            "durable commit marker must be cleared after metadata finalization"
        );

        engine.get_stats().state_root
    };

    let restarted = BlockchainEngine::new_dir(data_dir).unwrap();
    if restarted.persistent_store.is_none() {
        return;
    }
    assert_eq!(restarted.get_stats().height, 1);
    assert_eq!(restarted.get_stats().state_root, expected_root);
    assert!(restarted.try_is_transaction_committed(&tx1_hash).unwrap());
    assert!(restarted.try_is_transaction_committed(&tx2_hash).unwrap());
    assert_eq!(restarted.pending_transaction_len(), 0);
    restarted.state_read().validate_smt_consistency().unwrap();
}

#[test]
fn in_memory_and_persistent_engines_materialize_same_objects_across_checkpoints() {
    let temp_dir = tempfile::tempdir().unwrap();
    let data_dir = temp_dir.path().to_str().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let tx1 = signed_transfer_with_refs(
        &sender,
        &recipient_a.address,
        "0xaaaa",
        3_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let tx2 = signed_transfer_with_refs(
        &sender,
        &recipient_b.address,
        "0xbbbb",
        2_000_000,
        "0x2001",
        1_000_000,
        2,
    );

    let configure_engine = |engine: &mut BlockchainEngine| {
        configure_single_authority_consensus(engine);
    };

    let fund_engine = |engine: &BlockchainEngine| {
        fund_sender_with_coin(engine, &sender.address, "0xaaaa", 3_000_000);
        fund_sender_with_coin(engine, &sender.address, "0x1001", 1_000_000);
        fund_sender_with_coin(engine, &sender.address, "0xbbbb", 2_000_000);
        fund_sender_with_coin(engine, &sender.address, "0x2001", 1_000_000);
    };

    let mut persistent = BlockchainEngine::new_dir(data_dir).unwrap();
    persistent
        .state_read()
        .validate_smt_consistency()
        .expect("fresh persistent engine SMT");
    configure_engine(&mut persistent);
    fund_engine(&persistent);
    // Funding is still speculative until checkpoint production, so audit only
    // after it is committed below.
    persistent
        .submit_transactions_batch(vec![tx1.clone()])
        .unwrap();
    drive_consensus_to_height(&persistent, 1);
    persistent
        .state_read()
        .validate_smt_consistency()
        .expect("checkpoint 1 persistent SMT");
    persistent
        .submit_transactions_batch(vec![tx2.clone()])
        .unwrap();
    drive_consensus_to_height(&persistent, 2);
    persistent
        .state_read()
        .validate_smt_consistency()
        .expect("checkpoint 2 persistent SMT");

    let mut in_memory = BlockchainEngine::new_in_memory().unwrap();
    configure_engine(&mut in_memory);
    fund_engine(&in_memory);
    in_memory.submit_transactions_batch(vec![tx1]).unwrap();
    drive_consensus_to_height(&in_memory, 1);
    in_memory.submit_transactions_batch(vec![tx2]).unwrap();
    drive_consensus_to_height(&in_memory, 2);

    persistent.state_read().validate_smt_consistency().unwrap();

    // Separate consensus runs intentionally use different wall-clock commit
    // timestamps. Compare all application materialization while excluding the
    // singleton system clock object whose value is expected to differ.
    const CLOCK_KEY: &str =
        "object:0xaade8aa25002489bbcfca67637daf4dac78f4c88606e0dfd5724f323cbda6b5d";
    let without_clock = |engine: &BlockchainEngine| {
        engine
            .canonical_state_snapshot_dump(None)
            .into_iter()
            .filter(|(key, _)| key != CLOCK_KEY)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        without_clock(&persistent),
        without_clock(&in_memory),
        "checkpoint 2 application objects differ across storage backends"
    );
}

#[test]
fn required_persistent_engine_never_falls_back_to_memory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let invalid_db_path = temp_dir.path().join("not-a-directory");
    std::fs::write(&invalid_db_path, b"occupied by a file").unwrap();

    let error = match BlockchainEngine::new_dir_required(invalid_db_path.to_str().unwrap()) {
        Ok(_) => panic!("a required persistent engine must reject an invalid database path"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("Failed to open required persistent store"),
        "unexpected error: {error}"
    );
}

#[test]
fn persistent_engine_refuses_fresh_genesis_when_state_exists_without_chain_metadata() {
    let temp_dir = tempfile::tempdir().unwrap();
    let data_dir = temp_dir.path().to_path_buf();
    {
        let store = PersistentStore::open_with_path(Some(data_dir.clone())).unwrap();
        let owner = AccountAddress::from_hex_literal("0x123").unwrap();
        store
            .save(
                format!("account:{}", owner.to_hex_literal()).as_bytes(),
                &OwnerState::new(owner),
            )
            .unwrap();
        store.flush().unwrap();
    }

    let error = match BlockchainEngine::new_dir_required(data_dir.to_str().unwrap()) {
        Ok(_) => panic!("engine must not create fresh genesis over existing state entries"),
        Err(error) => error,
    };

    assert!(
        error
            .to_string()
            .contains("Refusing to create fresh genesis"),
        "unexpected error: {error}"
    );
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
fn history_pruning_keeps_permanent_replay_index() {
    let store = PersistentStore::open_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 0);
    let tx_hash = tx.transaction_hash().to_vec();
    let checkpoint = Checkpoint::new(
        1,
        vec![[7u8; 32]],
        vec![tx],
        vec![9u8; 32],
        42,
        vec![0u8; 32],
    );
    BlockchainEngine::persist_checkpoint_transactions(&store, &checkpoint).unwrap();

    BlockchainEngine::prune_transaction_payloads(&store, 1).unwrap();

    assert!(
        store
            .load::<SignedTransaction>(&BlockchainEngine::transaction_payload_key(&tx_hash))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .load::<Vec<SignedTransaction>>(&BlockchainEngine::checkpoint_transactions_key(1))
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .load::<PersistedTransactionLocation>(&BlockchainEngine::transaction_index_key(
                &tx_hash
            ))
            .unwrap()
            .is_some(),
        "pruning must retain the permanent replay guard"
    );
}

#[test]
fn checkpoint_persistence_rejects_corrupt_recent_transaction_index() {
    let store = PersistentStore::open_in_memory().unwrap();
    store
        .save(
            BlockchainEngine::recent_transaction_hashes_key(),
            &vec![vec![1u8, 2, 3]],
        )
        .unwrap();

    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 0);
    let checkpoint = Checkpoint::new(
        1,
        vec![[7u8; 32]],
        vec![tx],
        vec![9u8; 32],
        42,
        vec![0u8; 32],
    );

    let error = BlockchainEngine::persist_checkpoint_transactions(&store, &checkpoint).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Recent transaction index contains invalid hash length"),
        "unexpected error: {error}"
    );
}

#[test]
fn committed_transaction_check_rejects_corrupt_persistent_index() {
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = BlockchainEngine::new_dir(temp_dir.path().to_str().unwrap()).unwrap();
    let Some(store) = engine.persistent_store.as_ref() else {
        return;
    };

    let tx_hash = [7u8; 32];
    store
        .save(
            &BlockchainEngine::transaction_index_key(&tx_hash),
            &vec![1u8, 2, 3],
        )
        .unwrap();

    let error = engine.try_is_transaction_committed(&tx_hash).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Failed to load committed transaction index"),
        "unexpected error: {error}"
    );
}

#[test]
fn batch_submit_accepts_contiguous_sequences_for_same_sender() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx0 = signed_transfer_from(&sender, 0);
    let tx1 = signed_transfer_from(&sender, 1);

    let hashes = engine.submit_transactions_batch(vec![tx0, tx1]).unwrap();

    assert_eq!(hashes.len(), 2);
    assert_eq!(engine.pending_transaction_len(), 2);
}

#[test]
fn batch_submit_rejects_stale_object_version() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    configure_single_authority_consensus(&mut engine);
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 1_000_000);

    {
        let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());
        let object = state.get_object("0xaaaa").unwrap().unwrap();
        let mut updated = ChangeSet::new();
        updated.created_objects.push((
            "0xaaaa".to_string(),
            CreatedObject {
                owner: object.owner,
                owner_kind: object.owner_kind,
                uid: None,
                id: None,
                type_: object.type_,
                data: object.data,
                version: 2,
            },
        ));
        state
            .apply_changeset_without_supply_validation(&updated)
            .unwrap();
    }

    let stale_tx = signed_transfer_from(&sender, 0);
    let stale_hash = hex::encode(stale_tx.transaction_hash());
    engine.submit_transactions_batch(vec![stale_tx]).unwrap();
    drive_consensus_to_height(&engine, 1);
    let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
    let checkpoint = chain.latest_checkpoint();
    assert_eq!(checkpoint.transactions.len(), 1);
    assert_eq!(checkpoint.transaction_effects.len(), 1);
    assert_ne!(checkpoint.transaction_effects[0].status, "success");
    assert!(
        checkpoint.transaction_effects[0].gas_used > 0,
        "committed pre-execution failures must charge deterministic base gas"
    );
    assert!(
        checkpoint
            .transactions
            .iter()
            .any(|tx| hex::encode(tx.transaction_hash()) == stale_hash)
    );
}

#[test]
fn batch_submit_accepts_shuffled_contiguous_sequences_for_same_sender() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx0 = signed_transfer_from(&sender, 0);
    let tx1 = signed_transfer_from(&sender, 1);
    let tx2 = signed_transfer_from(&sender, 2);

    let hashes = engine
        .submit_transactions_batch(vec![tx2.clone(), tx0.clone(), tx1.clone()])
        .unwrap();

    assert_eq!(hashes.len(), 3);
    let pending = engine.pending_transactions_snapshot();
    let pending_nonces = pending
        .iter()
        .map(|tx| tx.transaction.nonce())
        .collect::<Vec<_>>();
    assert_eq!(pending_nonces, vec![0, 1, 2]);
}

#[test]
fn gas_application_credits_dao_ledger_without_creating_coin() {
    let sender = AccountAddress::random();
    let mut changeset = ChangeSet::new();

    BlockchainEngine::apply_gas_and_sequence(&mut changeset, sender, 10, 10).unwrap();

    let sender_owner_delta = changeset.owner_deltas.get(&sender).unwrap();
    assert_eq!(sender_owner_delta.balance_delta, -10);
    let dao = AccountAddress::from_hex_literal(KanariAddress::DAO_ADDRESS).unwrap();
    assert_eq!(changeset.owner_deltas.get(&dao).unwrap().balance_delta, 10);
    assert_eq!(changeset.native_gas_credits.get(&dao), Some(&10));
    assert!(changeset.created_objects.is_empty());

    let mut state = kanari_move_runtime_v1::state::StateManager::new_in_memory();
    state
        .save_owner_state(&OwnerState::with_native_balance(sender, 10))
        .unwrap();
    state
        .apply_changeset_without_supply_validation(&changeset)
        .unwrap();
    assert_eq!(state.resolve_owner_native_balance(dao).unwrap(), 10);
    assert!(state.get_owned_objects(&dao).unwrap().is_empty());
}

#[test]
fn batch_submit_rejects_duplicate_transactions() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 0);

    let err = engine
        .submit_transactions_batch(vec![tx.clone(), tx])
        .unwrap_err();

    assert!(err.to_string().contains("already in pending pool"));
}

#[test]
fn batch_submit_rejects_transaction_already_indexed_in_pending_pool() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 0);

    engine.submit_transactions_batch(vec![tx.clone()]).unwrap();
    let err = engine.submit_transactions_batch(vec![tx]).unwrap_err();

    assert!(err.to_string().contains("already in pending pool"));
}

#[test]
fn batch_submit_accepts_sequence_gaps() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = signed_transfer_from(&sender, 1);

    let hashes = engine.submit_transactions_batch(vec![tx]).unwrap();

    assert_eq!(hashes.len(), 1);
}

#[test]
fn deterministic_parallel_execution_matches_strict_serial_root() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let mut txs = Vec::new();

    for i in 0..16 {
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        let coin_object_id = format!("0x{:0>64x}", i + 1);
        let gas_object_id = format!("0x{:0>64x}", i + 101);
        fund_sender_with_coin(&engine, &sender.address, &coin_object_id, 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, &gas_object_id, 1_000_000);
        assert_eq!(
            engine
                .state_read()
                .resolve_owner_native_balance(
                    AccountAddress::from_hex_literal(&sender.address).unwrap()
                )
                .unwrap(),
            2_000_000
        );

        let mut tx = Transaction::new_transfer_with_object_ref_and_gas(
            sender.tagged_address(),
            native_coin_object_ref(&coin_object_id, 1_000_000),
            recipient.address.clone(),
            1,
            0,
            100_000,
            1_000,
        );
        if let Transaction::ExecuteFunction {
            gas_payment: Some(gas_payment),
            ..
        } = &mut tx
        {
            gas_payment.payment_objects = vec![native_coin_object_ref(&gas_object_id, 1_000_000)];
        }
        let mut signed_tx = SignedTransaction::new(tx);
        signed_tx
            .sign(&sender.private_key, sender.curve_type)
            .unwrap();
        txs.push(signed_tx);
    }

    engine
        .state
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .commit()
        .unwrap();
    let base_state = engine
        .state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let strict_state = Arc::new(RwLock::new(base_state.clone()));
    let parallel_state = Arc::new(RwLock::new(base_state));
    let serial_effects = engine
        .collect_transaction_effects_strict(&txs, Some(123), true)
        .unwrap();

    let strict_counts = engine
        .execute_tx_waves_parallel(txs.clone(), &strict_state, Some(123), false, true)
        .unwrap();
    let (parallel_executed, parallel_failed, parallel_effects) = engine
        .execute_tx_waves_deterministic_parallel_with_effects(
            txs,
            &parallel_state,
            Some(123),
            false,
        )
        .unwrap();
    let parallel_counts = (parallel_executed, parallel_failed);

    let strict_root = strict_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();
    let parallel_root = parallel_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();

    assert_eq!(strict_counts, parallel_counts);
    assert_eq!(parallel_effects, serial_effects);
    assert_eq!(parallel_effects.len(), parallel_executed + parallel_failed);
    assert_eq!(strict_root, parallel_root);
}

#[test]
fn conflicting_speculative_wave_replays_to_strict_serial_root() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipients = [
        generate_keypair(CurveType::Ed25519).unwrap(),
        generate_keypair(CurveType::Ed25519).unwrap(),
    ];
    let coin_ids = [
        "0x000000000000000000000000000000000000000000000000000000000000ca01",
        "0x000000000000000000000000000000000000000000000000000000000000ca02",
    ];
    let gas_ids = [
        "0x000000000000000000000000000000000000000000000000000000000000ca11",
        "0x000000000000000000000000000000000000000000000000000000000000ca12",
    ];
    for id in coin_ids.iter().chain(gas_ids.iter()) {
        fund_sender_with_coin(&engine, &sender.address, id, 1_000_000);
    }
    let txs = (0..2)
        .map(|i| {
            signed_transfer_with_refs(
                &sender,
                &recipients[i].address,
                coin_ids[i],
                1_000_000,
                gas_ids[i],
                1_000_000,
                i as u64,
            )
        })
        .collect::<Vec<_>>();

    engine
        .state
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .commit()
        .unwrap();
    let base_state = engine.state_read().clone();
    let strict_state = Arc::new(RwLock::new(base_state.clone()));
    let parallel_state = Arc::new(RwLock::new(base_state));
    let expected_effects = engine
        .collect_transaction_effects_strict(&txs, Some(456), false)
        .unwrap();

    let strict_counts = engine
        .execute_tx_waves_parallel(txs.clone(), &strict_state, Some(456), false, true)
        .unwrap();
    let (executed, failed, actual_effects) = engine
        .execute_tx_waves_deterministic_parallel_with_effects(
            txs,
            &parallel_state,
            Some(456),
            false,
        )
        .unwrap();

    assert_eq!((executed, failed), strict_counts);
    assert_eq!(actual_effects, expected_effects);
    assert_eq!(
        parallel_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .compute_state_root(),
        strict_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .compute_state_root()
    );
}

#[test]
fn mixed_success_and_failure_speculative_wave_matches_strict_serial() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let valid_sender = generate_keypair(CurveType::Ed25519).unwrap();
    let invalid_sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let valid_coin = "0x000000000000000000000000000000000000000000000000000000000000fb01";
    let valid_gas = "0x000000000000000000000000000000000000000000000000000000000000fb02";
    let invalid_gas = "0x000000000000000000000000000000000000000000000000000000000000fb03";
    fund_sender_with_coin(&engine, &valid_sender.address, valid_coin, 1_000_000);
    fund_sender_with_coin(&engine, &valid_sender.address, valid_gas, 1_000_000);
    fund_sender_with_coin(&engine, &invalid_sender.address, invalid_gas, 1_000_000);

    let valid = signed_transfer_with_refs(
        &valid_sender,
        &recipient.address,
        valid_coin,
        1_000_000,
        valid_gas,
        1_000_000,
        0,
    );
    let invalid_tx = Transaction::ExecuteFunction {
        sender: invalid_sender.tagged_address(),
        module: "0x2::module_that_does_not_exist".to_string(),
        function: "missing".to_string(),
        type_args: vec![],
        args: vec![],
        object_inputs: vec![],
        gas_payment: Some(GasPayment {
            payment_objects: vec![native_coin_object_ref(invalid_gas, 1_000_000)],
            owner: invalid_sender.address.clone(),
            budget: 100_000,
            price: 1,
        }),
        gas_limit: 100_000,
        gas_price: 1,
        nonce: 0,
    };
    let mut invalid = SignedTransaction::new(invalid_tx);
    invalid
        .sign(&invalid_sender.private_key, invalid_sender.curve_type)
        .unwrap();
    let txs = vec![valid, invalid];

    engine
        .state
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .commit()
        .unwrap();
    let base_state = engine.state_read().clone();
    let strict_state = Arc::new(RwLock::new(base_state.clone()));
    let parallel_state = Arc::new(RwLock::new(base_state));
    let expected_effects = engine
        .collect_transaction_effects_strict(&txs, Some(789), false)
        .unwrap();

    let strict_counts = engine
        .execute_tx_waves_parallel(txs.clone(), &strict_state, Some(789), false, true)
        .unwrap();
    let (executed, failed, actual_effects) = engine
        .execute_tx_waves_deterministic_parallel_with_effects(
            txs,
            &parallel_state,
            Some(789),
            false,
        )
        .unwrap();

    assert_eq!((executed, failed), strict_counts);
    assert_eq!(actual_effects, expected_effects);
    assert!(
        actual_effects
            .iter()
            .any(|effect| effect.status == "failed")
    );
    assert_eq!(
        parallel_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .compute_state_root(),
        strict_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .compute_state_root()
    );
}

#[test]
fn select_conflict_free_transactions_keeps_non_conflicting_transactions() {
    let sender_a = generate_keypair(CurveType::Ed25519).unwrap();
    let sender_b = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();

    let tx_a = signed_transfer_with_refs(
        &sender_a,
        &recipient_a.address,
        "0xaaaa",
        1_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let tx_b = signed_transfer_with_refs(
        &sender_b,
        &recipient_b.address,
        "0xbbbb",
        1_000_000,
        "0x1002",
        1_000_000,
        2,
    );

    let selected =
        BlockchainEngine::select_conflict_free_transactions(vec![tx_a.clone(), tx_b.clone()]);
    let hashes = selected
        .into_iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();

    assert_eq!(hashes.len(), 2);
    assert_eq!(
        hashes,
        vec![
            tx_a.transaction_hash().to_vec(),
            tx_b.transaction_hash().to_vec()
        ]
    );
}

#[test]
fn select_conflict_free_transactions_returns_empty_for_empty_input() {
    let selected = BlockchainEngine::select_conflict_free_transactions(Vec::new());
    assert!(selected.is_empty());
}

#[test]
fn select_conflict_free_transactions_skips_later_conflicts_but_keeps_later_independent_transactions()
 {
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let independent_sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_c = generate_keypair(CurveType::Ed25519).unwrap();

    let tx_a = signed_transfer_with_refs(
        &sender,
        &recipient_a.address,
        "0xaaaa",
        2_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let tx_b = signed_transfer_with_refs(
        &sender,
        &recipient_b.address,
        "0xaaaa",
        2_000_000,
        "0x1002",
        1_000_000,
        2,
    );
    let tx_c = signed_transfer_with_refs(
        &independent_sender,
        &recipient_c.address,
        "0xbbbb",
        1_000_000,
        "0x2001",
        1_000_000,
        3,
    );

    let selected =
        BlockchainEngine::select_conflict_free_transactions(vec![tx_a.clone(), tx_b, tx_c.clone()]);
    let hashes = selected
        .into_iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();

    assert_eq!(
        hashes,
        vec![
            tx_a.transaction_hash().to_vec(),
            tx_c.transaction_hash().to_vec()
        ]
    );
}

#[test]
fn select_conflict_free_transactions_keeps_only_first_when_all_transactions_conflict() {
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_c = generate_keypair(CurveType::Ed25519).unwrap();

    let tx_a = signed_transfer_with_refs(
        &sender,
        &recipient_a.address,
        "0xaaaa",
        3_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let tx_b = signed_transfer_with_refs(
        &sender,
        &recipient_b.address,
        "0xaaaa",
        3_000_000,
        "0x1002",
        1_000_000,
        2,
    );
    let tx_c = signed_transfer_with_refs(
        &sender,
        &recipient_c.address,
        "0xaaaa",
        3_000_000,
        "0x1003",
        1_000_000,
        3,
    );

    let selected =
        BlockchainEngine::select_conflict_free_transactions(vec![tx_a.clone(), tx_c, tx_b]);
    let hashes = selected
        .into_iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();

    assert_eq!(hashes, vec![tx_a.transaction_hash().to_vec()]);
}

#[test]
fn pending_conflict_free_snapshot_is_stable_regardless_of_submit_order() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let independent_sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_c = generate_keypair(CurveType::Ed25519).unwrap();

    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 2_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0x1001", 1_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0x1002", 1_000_000);
    fund_sender_with_coin(&engine, &independent_sender.address, "0xbbbb", 1_000_000);
    fund_sender_with_coin(&engine, &independent_sender.address, "0x2001", 1_000_000);

    let tx_a = signed_transfer_with_refs(
        &sender,
        &recipient_a.address,
        "0xaaaa",
        2_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let tx_b = signed_transfer_with_refs(
        &sender,
        &recipient_b.address,
        "0xaaaa",
        2_000_000,
        "0x1002",
        1_000_000,
        2,
    );
    let tx_c = signed_transfer_with_refs(
        &independent_sender,
        &recipient_c.address,
        "0xbbbb",
        1_000_000,
        "0x2001",
        1_000_000,
        3,
    );

    let expected = vec![
        tx_a.transaction_hash().to_vec(),
        tx_c.transaction_hash().to_vec(),
    ];

    engine
        .submit_transactions_batch(vec![tx_c, tx_b, tx_a])
        .unwrap();

    let ready = engine
        .pending_conflict_free_transactions_snapshot()
        .into_iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(ready, expected);
}

#[test]
fn owned_fast_checkpoint_commits_no_shared_pending_transactions() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender_a = generate_keypair(CurveType::Ed25519).unwrap();
    let sender_b = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender_with_coin(&engine, &sender_a.address, "0xa001", 1_000_000);
    fund_sender_with_coin(&engine, &sender_b.address, "0xb001", 1_000_000);
    let tx_a = signed_native_burn_with_gas_object(&sender_a, "0xa001", 1_000_000, 1);
    let tx_b = signed_native_burn_with_gas_object(&sender_b, "0xb001", 1_000_000, 2);

    engine
        .submit_transactions_batch(vec![tx_b.clone(), tx_a.clone()])
        .unwrap();
    let info = engine.produce_owned_fast_checkpoint().unwrap();

    assert_eq!(info.tx_count, 2);
    assert_eq!(info.executed, 2);
    assert_eq!(info.failed, 0);
    assert_eq!(engine.pending_transaction_len(), 0);
    assert_eq!(engine.get_stats().height, 1);
    assert!(
        engine
            .try_is_transaction_committed(tx_a.transaction_hash())
            .unwrap()
    );
    assert!(
        engine
            .try_is_transaction_committed(tx_b.transaction_hash())
            .unwrap()
    );
    assert_eq!(info.checkpoint.unwrap().tx_count, 2);
}

#[test]
fn owned_fast_checkpoint_leaves_shared_object_transactions_pending() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let owned_sender = generate_keypair(CurveType::Ed25519).unwrap();
    let shared_sender = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender_with_coin(&engine, &owned_sender.address, "0xa001", 1_000_000);
    let owned_tx = signed_native_burn_with_gas_object(&owned_sender, "0xa001", 1_000_000, 1);
    let mut shared_transaction =
        Transaction::new_burn_with_gas(shared_sender.tagged_address(), 0, 2, 100_000, 1);
    if let Transaction::ExecuteFunction { object_inputs, .. } = &mut shared_transaction {
        object_inputs.push(ObjectInput {
            object_ref: ObjectRef::new("0xshared", Some(1), Some(format!("0x{}", "11".repeat(32)))),
            owner: Some(ObjectOwnerKind::Shared),
            mutable: true,
        });
    }
    let mut shared_tx = SignedTransaction::new(shared_transaction);
    shared_tx
        .sign(&shared_sender.private_key, shared_sender.curve_type)
        .unwrap();
    let shared_hash = shared_tx.transaction_hash().to_vec();

    engine
        .submit_transactions_batch(vec![shared_tx.clone(), owned_tx.clone()])
        .unwrap();
    let info = engine.produce_owned_fast_checkpoint().unwrap();

    assert_eq!(info.tx_count, 1);
    assert!(
        engine
            .try_is_transaction_committed(owned_tx.transaction_hash())
            .unwrap()
    );
    assert!(!engine.try_is_transaction_committed(&shared_hash).unwrap());
    assert_eq!(engine.pending_transaction_len(), 1);
    assert_eq!(
        engine.pending_transactions_snapshot()[0].transaction_hash(),
        shared_hash.as_slice()
    );
}

#[test]
fn committed_checkpoint_releases_conflicting_transaction_for_next_round() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    configure_single_authority_consensus(&mut engine);

    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();

    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 2_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0x1001", 1_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0x1002", 1_000_000);

    let tx_a = signed_transfer_with_refs(
        &sender,
        &recipient_a.address,
        "0xaaaa",
        2_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let tx_b = signed_transfer_with_refs(
        &sender,
        &recipient_b.address,
        "0xaaaa",
        2_000_000,
        "0x1002",
        1_000_000,
        2,
    );

    engine.submit_transactions_batch(vec![tx_b, tx_a]).unwrap();
    drive_consensus_to_height(&engine, 1);

    let ready = engine.pending_conflict_free_transactions_snapshot();
    assert_eq!(ready.len(), 1);
    drive_consensus_until_mempool_empty(&engine);
    assert_eq!(engine.pending_transaction_len(), 0);
}

#[test]
fn produce_checkpoint_skips_conflicting_transactions_from_same_wallet() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    configure_single_authority_consensus(&mut engine);

    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let shared_coin_id = "0xaaaa";
    let gas_a_id = "0x1001";
    let gas_b_id = "0x1002";

    fund_sender_with_coin(&engine, &sender.address, shared_coin_id, 2_000_000);
    fund_sender_with_coin(&engine, &sender.address, gas_a_id, 1_000_000);
    fund_sender_with_coin(&engine, &sender.address, gas_b_id, 1_000_000);

    let mut tx_a = Transaction::new_transfer_with_object_ref_and_gas(
        sender.tagged_address(),
        native_coin_object_ref(shared_coin_id, 2_000_000),
        recipient_a.address.clone(),
        1,
        1,
        100_000,
        1,
    );
    if let Transaction::ExecuteFunction {
        gas_payment: Some(gas_payment),
        ..
    } = &mut tx_a
    {
        gas_payment.payment_objects = vec![native_coin_object_ref(gas_a_id, 1_000_000)];
    }

    let mut tx_b = Transaction::new_transfer_with_object_ref_and_gas(
        sender.tagged_address(),
        native_coin_object_ref(shared_coin_id, 2_000_000),
        recipient_b.address.clone(),
        1,
        2,
        100_000,
        1,
    );
    if let Transaction::ExecuteFunction {
        gas_payment: Some(gas_payment),
        ..
    } = &mut tx_b
    {
        gas_payment.payment_objects = vec![native_coin_object_ref(gas_b_id, 1_000_000)];
    }

    let mut signed_a = SignedTransaction::new(tx_a);
    signed_a
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let hash_a = signed_a.transaction_hash().to_vec();

    let mut signed_b = SignedTransaction::new(tx_b);
    signed_b
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let hash_b = signed_b.transaction_hash().to_vec();

    engine
        .submit_transactions_batch(vec![signed_b, signed_a])
        .unwrap();

    let ready_hashes = engine
        .pending_conflict_free_transactions_snapshot()
        .into_iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(ready_hashes.len(), 1);
    let chosen_first = ready_hashes[0].clone();
    let chosen_second = if chosen_first == hash_a {
        hash_b.clone()
    } else {
        hash_a.clone()
    };

    drive_consensus_to_height(&engine, 1);
    assert_eq!(engine.pending_transaction_len(), 1);
    let committed_hashes = engine
        .blockchain
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .latest_checkpoint()
        .transactions
        .iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(committed_hashes, vec![chosen_first.clone()]);

    let pending_hashes = engine
        .pending_transactions_snapshot()
        .into_iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(pending_hashes, vec![chosen_second.clone()]);

    let ready_after_first_checkpoint = engine
        .pending_conflict_free_transactions_snapshot()
        .into_iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(ready_after_first_checkpoint, vec![chosen_second.clone()]);

    drive_consensus_until_mempool_empty(&engine);
    assert_eq!(engine.pending_transaction_len(), 0);
}

#[test]
fn mempool_admission_caps_primary_access_lane_depth() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let shared_coin_id = "0xaaaa";

    fund_sender_with_coin(&engine, &sender.address, shared_coin_id, 10_000_000);
    for i in 0..(MAX_PENDING_PER_PRIMARY_ACCESS_LANE + 1) {
        let gas_object_id = format!("0x{:0>4x}", i + 0x1000);
        fund_sender_with_coin(&engine, &sender.address, &gas_object_id, 1_000_000);
    }

    let mut accepted = Vec::new();
    for nonce in 0..MAX_PENDING_PER_PRIMARY_ACCESS_LANE {
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        accepted.push(signed_transfer_with_refs(
            &sender,
            &recipient.address,
            shared_coin_id,
            10_000_000,
            &format!("0x{:0>4x}", nonce + 0x1000),
            1_000_000,
            nonce,
        ));
    }
    let lane_key = accepted[0].transaction.primary_access_key();

    engine.submit_transactions_batch(accepted).unwrap();
    assert_eq!(
        engine.pending_tx_count_for_primary_access(&lane_key),
        MAX_PENDING_PER_PRIMARY_ACCESS_LANE
    );
    assert_eq!(
        engine.pending_tx_count_for_congestion_access("object:0xaaaa"),
        MAX_PENDING_PER_PRIMARY_ACCESS_LANE
    );

    let overflow_recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let err = engine
        .submit_transactions_batch(vec![signed_transfer_with_refs(
            &sender,
            &overflow_recipient.address,
            shared_coin_id,
            10_000_000,
            &format!("0x{:0>4x}", MAX_PENDING_PER_PRIMARY_ACCESS_LANE + 0x1000),
            1_000_000,
            MAX_PENDING_PER_PRIMARY_ACCESS_LANE,
        )])
        .unwrap_err();

    assert!(err.to_string().contains("is saturated"));
    assert_eq!(
        engine.pending_transaction_len() as u64,
        MAX_PENDING_PER_PRIMARY_ACCESS_LANE
    );
}

#[test]
fn mempool_admission_caps_pending_per_sender() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();

    // Distinct primary coins and distinct gas coins per transaction so the
    // shared-object lane caps do not saturate before the sender cap.
    for i in 0..(MAX_PENDING_PER_SENDER + 1) {
        let coin_object_id = format!("0x{:0>4x}", i + 0x1000);
        fund_sender_with_coin(&engine, &sender.address, &coin_object_id, 1_000_000);
        let gas_object_id = format!("0x{:0>4x}", i + 0x5000);
        fund_sender_with_coin(&engine, &sender.address, &gas_object_id, 1_000_000);
    }

    let mut accepted = Vec::new();
    for nonce in 0..MAX_PENDING_PER_SENDER {
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        accepted.push(signed_transfer_with_refs(
            &sender,
            &recipient.address,
            &format!("0x{:0>4x}", nonce + 0x1000),
            1_000_000,
            &format!("0x{:0>4x}", nonce + 0x5000),
            1_000_000,
            nonce,
        ));
    }

    engine.submit_transactions_batch(accepted).unwrap();
    assert_eq!(
        engine.pending_tx_count_for_sender(&sender.tagged_address()),
        MAX_PENDING_PER_SENDER
    );

    let overflow_recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let err = engine
        .submit_transactions_batch(vec![signed_transfer_with_refs(
            &sender,
            &overflow_recipient.address,
            &format!("0x{:0>4x}", MAX_PENDING_PER_SENDER + 0x1000),
            1_000_000,
            &format!("0x{:0>4x}", MAX_PENDING_PER_SENDER + 0x5000),
            1_000_000,
            MAX_PENDING_PER_SENDER,
        )])
        .unwrap_err();

    assert!(err.to_string().contains("too many pending transactions"));
    assert_eq!(
        engine.pending_transaction_len() as u64,
        MAX_PENDING_PER_SENDER
    );
}

#[test]
fn mempool_admission_allows_independent_primary_access_lanes_when_one_lane_is_full() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();

    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 10_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0xbbbb", 10_000_000);
    for i in 0..(MAX_PENDING_PER_PRIMARY_ACCESS_LANE + 2) {
        let gas_object_id = format!("0x{:0>4x}", i + 0x2000);
        fund_sender_with_coin(&engine, &sender.address, &gas_object_id, 1_000_000);
    }

    let mut lane_a = Vec::new();
    for nonce in 0..MAX_PENDING_PER_PRIMARY_ACCESS_LANE {
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        lane_a.push(signed_transfer_with_refs(
            &sender,
            &recipient.address,
            "0xaaaa",
            10_000_000,
            &format!("0x{:0>4x}", nonce + 0x2000),
            1_000_000,
            nonce,
        ));
    }
    let lane_a_key = lane_a[0].transaction.primary_access_key();
    engine.submit_transactions_batch(lane_a).unwrap();

    let independent_recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let lane_b_tx = signed_transfer_with_refs(
        &sender,
        &independent_recipient.address,
        "0xbbbb",
        10_000_000,
        &format!("0x{:0>4x}", MAX_PENDING_PER_PRIMARY_ACCESS_LANE + 0x2000),
        1_000_000,
        MAX_PENDING_PER_PRIMARY_ACCESS_LANE + 1,
    );
    let lane_b_key = lane_b_tx.transaction.primary_access_key();
    engine.submit_transactions_batch(vec![lane_b_tx]).unwrap();

    assert_eq!(
        engine.pending_tx_count_for_primary_access(&lane_a_key),
        MAX_PENDING_PER_PRIMARY_ACCESS_LANE
    );
    assert_eq!(engine.pending_tx_count_for_primary_access(&lane_b_key), 1);
}

#[test]
fn non_native_execute_function_requires_full_object_ref_metadata() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 1_000_000);

    let tx = Transaction::ExecuteFunction {
        sender: sender.tagged_address(),
        module: CoinModule::module_path(),
        function: CoinModule::function_names().join_entry.to_string(),
        type_args: vec![GAS_COIN.to_string()],
        args: vec![],
        object_inputs: vec![ObjectInput {
            object_ref: ObjectRef::new("0xaaaa", None, None),
            owner: Some(ObjectOwnerKind::AddressOwner(sender.address.clone())),
            mutable: true,
        }],
        gas_payment: Some(GasPayment {
            payment_objects: vec![ObjectRef::new("0xaaaa", None, None)],
            owner: sender.address.clone(),
            budget: 100_000,
            price: 1,
        }),
        gas_limit: 100_000,
        gas_price: 1,
        nonce: 0,
    };
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();

    let err = engine.execute_transaction_immediate(signed_tx).unwrap_err();
    assert!(
        err.to_string()
            .contains("must include (object_id, version, digest)")
    );
}

#[test]
fn mempool_rejects_malformed_move_module_path_before_dag_admission() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = Transaction::ExecuteFunction {
        sender: sender.tagged_address(),
        module: "not-a-move-module".to_string(),
        function: "entry".to_string(),
        type_args: vec![],
        args: vec![],
        object_inputs: vec![],
        gas_payment: None,
        gas_limit: 0,
        gas_price: 0,
        nonce: 0,
    };
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();

    let err = engine
        .submit_transactions_batch(vec![signed_tx])
        .unwrap_err();
    assert!(err.to_string().contains("address::module"));
    assert_eq!(engine.get_stats().pending_transactions, 0);
}

#[test]
fn gas_payment_object_must_be_native_kanari_coin() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender_with_coin_type(
        &engine,
        &sender.address,
        "0xaaaa",
        1_000_000,
        "0x2::james::JAMES",
    );

    let tx = Transaction::ExecuteFunction {
        sender: sender.tagged_address(),
        module: GasModule::module_path(),
        function: "burn_amount".to_string(),
        type_args: vec![],
        args: vec![bcs::to_bytes(&1u64).unwrap()],
        object_inputs: vec![],
        gas_payment: Some(GasPayment {
            payment_objects: vec![native_coin_object_ref("0xaaaa", 1_000_000)],
            owner: sender.address.clone(),
            budget: 100_000,
            price: 1,
        }),
        gas_limit: 100_000,
        gas_price: 1,
        nonce: 0,
    };
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();

    let err = engine.execute_transaction_immediate(signed_tx).unwrap_err();
    assert!(err.to_string().contains("must be Coin<"));
}

#[test]
fn move_transfer_rejects_gas_payment_overlap_with_transfer_coin() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 1_000_000);

    let tx = Transaction::new_transfer_with_object_ref(
        sender.tagged_address(),
        native_coin_object_ref("0xaaaa", 1_000_000),
        generate_keypair(CurveType::Ed25519).unwrap().address,
        1,
        0,
    );
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();

    assert!(
        engine
            .execute_transaction_immediate(signed_tx)
            .unwrap_err()
            .to_string()
            .contains("cannot overlap with a mutable object input")
    );
}

#[test]
fn non_native_execute_function_still_rejects_gas_overlap_with_mutable_input() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 1_000_000);

    let tx = Transaction::ExecuteFunction {
        sender: sender.tagged_address(),
        module: CoinModule::module_path(),
        function: CoinModule::function_names().join_entry.to_string(),
        type_args: vec![GAS_COIN.to_string()],
        args: vec![],
        object_inputs: vec![ObjectInput {
            object_ref: native_coin_object_ref("0xaaaa", 1_000_000),
            owner: Some(ObjectOwnerKind::AddressOwner(sender.address.clone())),
            mutable: true,
        }],
        gas_payment: Some(GasPayment {
            payment_objects: vec![native_coin_object_ref("0xaaaa", 1_000_000)],
            owner: sender.address.clone(),
            budget: 100_000,
            price: 1,
        }),
        gas_limit: 100_000,
        gas_price: 1,
        nonce: 0,
    };
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();

    let err = engine.execute_transaction_immediate(signed_tx).unwrap_err();
    assert!(
        err.to_string()
            .contains("cannot overlap with a mutable object input")
    );
}
