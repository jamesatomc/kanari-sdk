use super::*;
use kanari_types::error::KanariUnwrapExt;

fn set_native_supply_for_test(state: &mut StateManager, total_supply: u64) -> Result<()> {
    state.total_supply = total_supply;
    state.store.save(b"total_supply", &total_supply)?;
    state.store.save(
        &StateManager::supply_key(KANARI_TOKEN_TYPE),
        &TreasuryCap { total_supply },
    )?;
    Ok(())
}

#[test]
fn treasury_update_syncs_native_total_supply() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let owner = AccountAddress::from_hex_literal("0x1")?;
    let updated_supply = state.total_supply + 777;

    let mut cs = ChangeSet::new();
    cs.add_treasury(owner, KANARI_TOKEN_TYPE.to_string(), updated_supply);
    state.apply_changeset(&cs)?;

    assert_eq!(state.total_supply, updated_supply);
    Ok(())
}

#[test]
fn validate_supply_invariants_detects_native_supply_overcount() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    state.save_account(&Account::with_native_balance(alice, 500))?;
    set_native_supply_for_test(&mut state, base.total_supply + 400)?;
    state.global_token_supplies.insert(
        KANARI_TOKEN_TYPE.to_string(),
        base.wallet_visible_supply + 500,
    );

    let err = state
        .validate_supply_invariants()
        .expect_err("validation should detect overcount");
    assert!(err.to_string().contains(&format!(
        "wallet_visible_supply={}",
        base.wallet_visible_supply + 500
    )));

    Ok(())
}

#[test]
fn validate_supply_invariants_allows_native_supply_locked_in_objects() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    state.save_account(&Account::with_native_balance(alice, 500))?;
    set_native_supply_for_test(&mut state, base.total_supply + 600)?;
    state.global_token_supplies.insert(
        KANARI_TOKEN_TYPE.to_string(),
        base.wallet_visible_supply + 500,
    );

    let summary = state.token_supply_summary(KANARI_TOKEN_TYPE)?;
    assert_eq!(summary.total_supply, base.total_supply + 600);
    assert_eq!(
        summary.wallet_visible_supply,
        base.wallet_visible_supply + 500
    );
    assert_eq!(summary.object_locked_supply, 100);

    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn token_supply_summary_uses_treasury_supply_for_custom_tokens() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::test::TEST";
    let mut state = StateManager::new_in_memory();

    let mut cs = ChangeSet::new();
    cs.add_treasury(owner, token_type.to_string(), 1_000);
    cs.add_token_balance_set(owner, token_type.to_string(), 250);
    state.apply_changeset(&cs)?;

    let summary = state.token_supply_summary(token_type)?;
    assert_eq!(summary.total_supply, 1_000);
    assert_eq!(summary.wallet_visible_supply, 250);
    assert_eq!(summary.object_locked_supply, 750);

    Ok(())
}

#[test]
fn object_locked_coin_ledger_tracks_defi_lock_and_release() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::test::TEST";
    let coin_type = format!("0x2::coin::Coin<{}>", token_type);
    let deal_type = format!("0x2::escrow::EscrowDeal<{}>", token_type);
    let mut state = StateManager::new_in_memory();

    let mut full_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    full_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut init = ChangeSet::new();
    init.add_treasury(owner, token_type.to_string(), 1_000);
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: full_coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&init)?;

    let mut remaining_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    remaining_coin_data[UID_SIZE..].copy_from_slice(&900u64.to_le_bytes());
    let mut lock = ChangeSet::new();
    lock.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: remaining_coin_data,
            version: 2,
        },
    ));
    lock.created_objects.push((
        "0xbbbb".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: deal_type.clone(),
            data: vec![1, 2, 3],
            version: 1,
        },
    ));
    state.apply_changeset(&lock)?;

    let summary = state.token_supply_summary(token_type)?;
    assert_eq!(summary.total_supply, 1_000);
    assert_eq!(summary.wallet_visible_supply, 900);
    assert_eq!(summary.object_locked_supply, 100);
    let locked_records = state.load_object_locked_coin_records()?;
    assert_eq!(locked_records.len(), 1);
    assert_eq!(locked_records[0].holder_object_id, "0xbbbb");
    assert_eq!(locked_records[0].amount, 100);

    let mut released_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    released_coin_data[UID_SIZE..].copy_from_slice(&100u64.to_le_bytes());
    let mut release = ChangeSet::new();
    release.created_objects.push((
        "0xbbbb".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: deal_type,
            data: vec![4, 5, 6],
            version: 2,
        },
    ));
    release.created_objects.push((
        "0xcccc".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: coin_type,
            data: released_coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&release)?;

    let summary = state.token_supply_summary(token_type)?;
    assert_eq!(summary.wallet_visible_supply, 1_000);
    assert_eq!(summary.object_locked_supply, 0);
    assert!(state.load_object_locked_coin_records()?.is_empty());

    Ok(())
}

#[test]
fn owned_object_index_canonicalizes_object_ids_across_alias_updates() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let padded_id = format!("0x{:0>64}", "abcd");
    let canonical_id = AccountAddress::from_hex_literal(&padded_id)?.to_hex_literal();
    assert_ne!(padded_id, canonical_id);

    let mut state = StateManager::new_in_memory();
    let mut init = ChangeSet::new();
    init.created_objects.push((
        padded_id.clone(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: "0x2::test::Object".to_string(),
            data: vec![1, 2, 3],
            version: 1,
        },
    ));
    state.apply_changeset(&init)?;

    assert_eq!(state.get_owned_objects(&owner)?, vec![canonical_id.clone()]);

    let mut update = ChangeSet::new();
    update.created_objects.push((
        canonical_id.clone(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: "0x2::test::Object".to_string(),
            data: vec![4, 5, 6],
            version: 2,
        },
    ));
    state.apply_changeset(&update)?;

    assert_eq!(state.get_owned_objects(&owner)?, vec![canonical_id]);

    Ok(())
}

#[test]
fn compute_state_root_reflects_overlay_before_commit() -> Result<()> {
    let publisher = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();
    let root_before = state.compute_state_root();

    let mut cs = ChangeSet::new();
    cs.publish_module(publisher, "example".to_string());
    state.apply_changeset(&cs)?;

    let root_after = state.compute_state_root();
    assert_ne!(
        root_before, root_after,
        "pending overlay writes should affect speculative state roots"
    );

    Ok(())
}

#[test]
fn compute_state_root_is_stable_across_in_memory_commit() -> Result<()> {
    let publisher = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();

    let mut cs = ChangeSet::new();
    cs.publish_module(publisher, "example".to_string());
    state.apply_changeset(&cs)?;

    let pending_root = state.compute_state_root();
    state.commit()?;
    let committed_root = state.compute_state_root();

    assert_eq!(
        pending_root, committed_root,
        "logical state root must not change when overlay is flushed"
    );
    assert!(
        state
            .get_account(&publisher)
            .map(|account| account.modules.contains("example"))
            .unwrap_or(false)
    );

    Ok(())
}

#[test]
fn compute_state_root_ignores_runtime_local_store_keys() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();

    let account = Account::with_native_balance(owner, 100);
    state.save_account(&account)?;
    state.commit()?;
    let root_before = state.compute_state_root();

    state
        .store
        .save(b"module_index", &vec!["local".to_string()])?;
    state.store.save(b"module:0x1:Local", &vec![1u8, 2, 3])?;
    state
        .store
        .save(b"framework_hash:stdlib", &"node-local-hash")?;
    state
        .store
        .save(b"framework_manifest:stdlib", &vec!["Local"])?;
    state
        .store
        .save(b"object_index", &vec!["0xdead".to_string()])?;
    state
        .store
        .save(b"owner_index:\x00", &vec!["0xdead".to_string()])?;
    state
        .store
        .save(b"object:0xdead", &"orphan-runtime-object")?;
    state.store.save(b"df_0xdead_local", &vec![9u8])?;

    assert_eq!(
        root_before,
        state.compute_state_root(),
        "runtime metadata and orphan object-storage keys must not affect canonical state root"
    );

    Ok(())
}

#[test]
fn compute_state_root_tracks_indexed_canonical_objects() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();

    let mut create = ChangeSet::new();
    create.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
            data: vec![1, 2, 3],
            version: 1,
        },
    ));
    state.apply_changeset(&create)?;
    let first_root = state.compute_state_root();

    let mut update = ChangeSet::new();
    update.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
            data: vec![4, 5, 6],
            version: 2,
        },
    ));
    state.apply_changeset(&update)?;

    assert_ne!(
        first_root,
        state.compute_state_root(),
        "indexed canonical object changes must remain part of state root"
    );

    Ok(())
}

#[test]
fn compute_state_root_is_stable_across_rocksdb_commit() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let Ok(store) = PersistentStore::open_with_path(Some(temp_dir.path().join("state"))) else {
        return Ok(());
    };
    let store = Arc::new(store);
    let publisher = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new(store);

    let mut cs = ChangeSet::new();
    cs.publish_module(publisher, "example".to_string());
    state.apply_changeset(&cs)?;

    let pending_root = state.compute_state_root();
    state.commit()?;
    let committed_root = state.compute_state_root();

    assert_eq!(
        pending_root, committed_root,
        "logical state root must not change when RocksDB overlay is flushed"
    );

    Ok(())
}

fn materialized_sparse_root_for_test(state: &StateManager) -> Result<Vec<u8>> {
    let mut entries: BTreeMap<Vec<u8>, Vec<u8>> =
        state.store.logical_entries()?.into_iter().collect();
    for (key, value_opt) in &state.overlay {
        if let Some(value) = value_opt {
            entries.insert(key.clone(), value.clone());
        } else {
            entries.remove(key);
        }
    }
    StateManager::retain_canonical_state_root_entries(&mut entries);
    Ok(smt::compute_sparse_root(&entries.into_iter().collect::<Vec<_>>()).to_vec())
}

#[test]
fn compute_state_root_matches_materialized_sparse_root_for_rocksdb() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let Ok(store) = PersistentStore::open_with_path(Some(temp_dir.path().join("state"))) else {
        return Ok(());
    };
    let publisher = AccountAddress::from_hex_literal("0x1111")?;
    let owner = AccountAddress::from_hex_literal("0x2222")?;

    let mut state = StateManager::new(Arc::new(store));

    let mut cs = ChangeSet::new();
    cs.publish_module(publisher, "example".to_string());
    cs.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
            data: vec![1, 2, 3],
            version: 1,
        },
    ));

    state.apply_changeset(&cs)?;

    assert_eq!(
        state.compute_state_root(),
        materialized_sparse_root_for_test(&state)?,
        "incremental SMT root must match a fully materialized sparse root before commit"
    );

    state.commit()?;

    assert_eq!(
        state.compute_state_root(),
        materialized_sparse_root_for_test(&state)?,
        "committed SMT root must match a fully materialized sparse root"
    );
    Ok(())
}

#[test]
fn apply_changeset_rejects_insufficient_debit_without_partial_writes() -> Result<()> {
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let recipient = AccountAddress::from_hex_literal("0x2222")?;
    let mut state = StateManager::new_in_memory();
    state.save_account(&Account::with_native_balance(sender, 5))?;
    let root_before = state.compute_state_root();

    let mut changeset = ChangeSet::new();
    changeset.transfer(sender, recipient, 10);

    let error = state.apply_changeset(&changeset).unwrap_err();
    assert!(error.to_string().contains("Insufficient native balance"));
    assert_eq!(state.compute_state_root(), root_before);
    assert!(state.get_account(&recipient).is_none());
    assert_eq!(
        state
            .get_account(&sender)
            .invariant("sender account should exist")
            .native_balance(),
        5
    );

    Ok(())
}

#[test]
fn apply_changeset_rejects_supply_invariant_violation_without_mutating_live_state() -> Result<()> {
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let recipient = AccountAddress::from_hex_literal("0x2222")?;
    let mut state = StateManager::new_in_memory();

    state.save_account(&Account::with_native_balance(sender, 500))?;
    set_native_supply_for_test(&mut state, 400)?;
    state
        .global_token_supplies
        .insert(KANARI_TOKEN_TYPE.to_string(), 500);

    let root_before = state.compute_state_root();
    let sender_balance_before = state
        .get_account(&sender)
        .invariant("sender account should exist")
        .native_balance();

    let mut changeset = ChangeSet::new();
    changeset.transfer(sender, recipient, 10);

    let error = state.apply_changeset(&changeset).unwrap_err();
    assert!(error.to_string().contains("native supply overcount"));
    assert_eq!(state.compute_state_root(), root_before);
    assert!(state.get_account(&recipient).is_none());
    assert_eq!(
        state
            .get_account(&sender)
            .invariant("sender account should exist")
            .native_balance(),
        sender_balance_before
    );

    Ok(())
}

#[test]
fn unrelated_object_creation_preserves_native_balance_cache() -> Result<()> {
    let owner = kanari_types::address::Address::dev_account_address();
    let mut state = StateManager::new_in_memory();
    let before_balance = state
        .get_account(&owner)
        .invariant("owner account should exist")
        .native_balance();
    let before_visible = state.indexed_wallet_supply(KANARI_TOKEN_TYPE)?;

    let mut changeset = ChangeSet::new();
    changeset.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: "0x2::coin::CoinMetadata<0x2::test::TEST>".to_string(),
            data: vec![1, 2, 3],
            version: 1,
        },
    ));
    state.apply_changeset(&changeset)?;

    assert_eq!(
        state
            .get_account(&owner)
            .invariant("owner account should exist")
            .native_balance(),
        before_balance
    );
    assert_eq!(
        state.indexed_wallet_supply(KANARI_TOKEN_TYPE)?,
        before_visible
    );

    Ok(())
}

#[test]
fn recompute_owner_balances_preserves_native_gas_adjustments() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let dao = AccountAddress::from_hex_literal("0x2222")?;
    let token_type = "0x2::test::TEST";
    let coin_type = format!("0x2::coin::Coin<{}>", token_type);
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    state.save_account(&Account::with_native_balance(owner, 500))?;
    set_native_supply_for_test(&mut state, base.total_supply + 500)?;
    state.global_token_supplies.insert(
        KANARI_TOKEN_TYPE.to_string(),
        base.wallet_visible_supply + 500,
    );

    let before_balance = state
        .get_account(&owner)
        .invariant("owner account should exist")
        .native_balance();

    let mut gas_only = ChangeSet::new();
    gas_only.get_or_create_change(owner).debit(210);
    gas_only.collect_gas(dao, 210);
    state.apply_changeset(&gas_only)?;
    let after_gas_balance = state
        .get_account(&owner)
        .invariant("owner account should exist")
        .native_balance();
    assert_eq!(after_gas_balance, before_balance - 210);

    let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut mint = ChangeSet::new();
    mint.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: coin_type,
            data: coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&mint)?;

    assert_eq!(
        state
            .get_account(&owner)
            .invariant("owner account should exist")
            .native_balance(),
        after_gas_balance
    );

    Ok(())
}

#[test]
fn native_coin_object_transfer_applies_gas_delta_without_supply_overcount() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;
    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;

    let coin_type = format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE);
    let mut alice_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    alice_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut init = ChangeSet::new();
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: alice_coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&init)?;

    let mut alice_remaining_data = vec![0u8; UID_SIZE + U64_SIZE];
    alice_remaining_data[UID_SIZE..].copy_from_slice(&900u64.to_le_bytes());
    let mut bob_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    bob_coin_data[0] = 0xbb;
    bob_coin_data[UID_SIZE..].copy_from_slice(&100u64.to_le_bytes());

    let mut transfer = ChangeSet::new();
    transfer.get_or_create_change(alice).debit(10);
    transfer.collect_gas(gas_collector, 10);
    transfer.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: alice_remaining_data,
            version: 2,
        },
    ));
    transfer.created_objects.push((
        "0xbbbb".to_string(),
        CreatedObject {
            owner: bob,
            uid: None,
            id: None,
            type_: coin_type,
            data: bob_coin_data,
            version: 1,
        },
    ));

    state.apply_changeset(&transfer)?;

    assert_eq!(
        state
            .get_account(&alice)
            .invariant("alice account should exist")
            .native_balance(),
        890
    );
    assert_eq!(
        state
            .get_account(&bob)
            .invariant("bob account should exist")
            .native_balance(),
        100
    );
    assert_eq!(
        state
            .get_account(&gas_collector)
            .invariant("gas collector account should exist")
            .native_balance(),
        10
    );
    assert_eq!(
        state
            .token_supply_summary(KANARI_TOKEN_TYPE)?
            .wallet_visible_supply,
        base.wallet_visible_supply + 1_000
    );
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn native_token_balance_hints_do_not_double_count_transfers() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    state.save_account(&Account::with_native_balance(alice, 1_000))?;
    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;
    state.global_token_supplies.insert(
        KANARI_TOKEN_TYPE.to_string(),
        base.wallet_visible_supply + 1_000,
    );
    let before_summary = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    let mut transfer = ChangeSet::new();
    transfer.transfer(alice, bob, 100);
    transfer.add_token_balance_set(bob, KANARI_TOKEN_TYPE.to_string(), 100);

    state.apply_changeset(&transfer)?;

    assert_eq!(
        state
            .get_account(&alice)
            .invariant("alice account should exist")
            .native_balance(),
        900
    );
    assert_eq!(
        state
            .get_account(&bob)
            .invariant("bob account should exist")
            .native_balance(),
        100
    );
    assert_eq!(
        state
            .token_supply_summary(KANARI_TOKEN_TYPE)?
            .wallet_visible_supply,
        before_summary.wallet_visible_supply
    );
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn get_account_returns_none_for_missing_account() -> Result<()> {
    let state = StateManager::new_in_memory();
    let missing = AccountAddress::from_hex_literal("0x4242")?;

    assert!(state.get_account(&missing).is_none());
    assert!(state.get_account_by_hex("0x4242").is_none());

    Ok(())
}

#[test]
fn native_coin_object_full_transfer_subtracts_gas_from_moved_coin() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;
    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;

    let coin_type = format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE);
    let mut alice_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    alice_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut init = ChangeSet::new();
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: alice_coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&init)?;

    let mut moved_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    moved_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());

    let mut transfer = ChangeSet::new();
    transfer.get_or_create_change(alice).debit(10);
    transfer.collect_gas(gas_collector, 10);
    transfer.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: bob,
            uid: None,
            id: None,
            type_: coin_type,
            data: moved_coin_data,
            version: 2,
        },
    ));

    state.apply_changeset(&transfer)?;

    assert_eq!(
        state
            .get_account(&alice)
            .map(|account| account.native_balance())
            .unwrap_or(0),
        0
    );
    assert_eq!(
        state
            .get_account(&bob)
            .invariant("bob account should exist")
            .native_balance(),
        990
    );
    assert_eq!(
        state
            .get_account(&gas_collector)
            .invariant("gas collector account should exist")
            .native_balance(),
        10
    );
    assert_eq!(
        state
            .token_supply_summary(KANARI_TOKEN_TYPE)?
            .wallet_visible_supply,
        base.wallet_visible_supply + 1_000
    );
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn custom_token_mint_repairs_stale_native_visible_supply_cache() -> Result<()> {
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let custom_token = "0x2::usdc::USDC";
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    state.save_account(&Account::with_native_balance(sender, 10_000))?;
    set_native_supply_for_test(&mut state, base.total_supply + 10_000)?;
    state.global_token_supplies.insert(
        KANARI_TOKEN_TYPE.to_string(),
        base.wallet_visible_supply + 10_000 + 6_614,
    );

    let mut mint = ChangeSet::new();
    mint.get_or_create_change(sender).debit(210);
    mint.collect_gas(gas_collector, 210);
    mint.add_treasury(sender, custom_token.to_string(), 1_000_000);
    mint.add_token_balance_set(sender, custom_token.to_string(), 1_000);

    state.apply_changeset(&mint)?;

    assert_eq!(
        state
            .token_supply_summary(KANARI_TOKEN_TYPE)?
            .wallet_visible_supply,
        base.wallet_visible_supply + 10_000
    );
    assert_eq!(
        state
            .get_account(&sender)
            .invariant("sender account should exist")
            .native_balance(),
        9_790
    );
    assert_eq!(
        state
            .get_account(&gas_collector)
            .invariant("gas collector account should exist")
            .native_balance(),
        210
    );
    assert_eq!(
        state
            .token_supply_summary(custom_token)?
            .wallet_visible_supply,
        1_000
    );
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn custom_token_mint_updates_supply_from_treasury_cap_object() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::usdc::USDC";
    let cap_type = format!("0x2::coin::TreasuryCap<{}>", token_type);
    let mut state = StateManager::new_in_memory();

    let mut setup_cap_data = vec![0u8; UID_SIZE + U64_SIZE];
    setup_cap_data[UID_SIZE..].copy_from_slice(&0u64.to_le_bytes());
    let mut setup = ChangeSet::new();
    setup.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: cap_type.clone(),
            data: setup_cap_data,
            version: 1,
        },
    ));
    state.apply_changeset(&setup)?;
    assert_eq!(state.token_supply_summary(token_type)?.total_supply, 0);

    let mut mint_cap_data = vec![0u8; UID_SIZE + U64_SIZE];
    mint_cap_data[UID_SIZE..].copy_from_slice(&1_000_000u64.to_le_bytes());
    let mut mint = ChangeSet::new();
    mint.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            uid: None,
            id: None,
            type_: cap_type,
            data: mint_cap_data,
            version: 2,
        },
    ));
    mint.add_token_balance_set(owner, token_type.to_string(), 1_000_000);

    state.apply_changeset(&mint)?;

    let summary = state.token_supply_summary(token_type)?;
    assert_eq!(summary.total_supply, 1_000_000);
    assert_eq!(summary.wallet_visible_supply, 1_000_000);
    assert_eq!(summary.object_locked_supply, 0);
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn apply_changeset_repairs_existing_native_wallet_overcount_before_custom_token_mint() -> Result<()>
{
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let stale_account = AccountAddress::from_hex_literal("0xffff")?;
    let custom_token = "0x2::usdc::USDC";
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    state.save_account(&Account::with_native_balance(sender, 10_000))?;
    state.save_account(&Account::with_native_balance(stale_account, 6_614))?;
    set_native_supply_for_test(&mut state, base.total_supply + 10_000)?;

    let mut mint = ChangeSet::new();
    mint.get_or_create_change(sender).debit(210);
    mint.collect_gas(gas_collector, 210);
    mint.add_treasury(sender, custom_token.to_string(), 1_000_000);
    mint.add_token_balance_set(sender, custom_token.to_string(), 1_000);

    state.apply_changeset(&mint)?;

    assert_eq!(
        state
            .get_account(&stale_account)
            .invariant("stale account should still exist")
            .native_balance(),
        0
    );
    assert_eq!(
        state
            .get_account(&sender)
            .invariant("sender account should exist")
            .native_balance(),
        9_790
    );
    assert_eq!(
        state
            .get_account(&gas_collector)
            .invariant("gas collector account should exist")
            .native_balance(),
        210
    );
    assert_eq!(
        state
            .token_supply_summary(KANARI_TOKEN_TYPE)?
            .wallet_visible_supply,
        base.wallet_visible_supply + 10_000
    );
    assert_eq!(
        state.token_supply_summary(custom_token)?.total_supply,
        1_000_000
    );
    assert_eq!(
        state
            .token_supply_summary(custom_token)?
            .wallet_visible_supply,
        1_000
    );
    state.validate_supply_invariants()?;

    Ok(())
}
