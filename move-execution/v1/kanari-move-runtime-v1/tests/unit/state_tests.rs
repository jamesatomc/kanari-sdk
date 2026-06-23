use super::*;
#[path = "test_support.rs"]
mod test_support;

use test_support::{dao_address, set_native_supply_for_test, test_addr};

#[test]
fn in_memory_state_initializes_smt_backend() {
    let state = StateManager::new_in_memory_with_smt();
    assert!(state.smt.is_some());
}

#[test]
fn treasury_update_syncs_native_total_supply() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let owner = test_addr("0x1")?;

    let mut cs = ChangeSet::new();
    cs.add_treasury(owner, KANARI_TOKEN_TYPE.to_string(), 777);
    state.apply_changeset(&cs)?;

    assert_eq!(state.total_supply, 777);
    Ok(())
}

#[test]
fn object_writeback_does_not_restore_native_gas_debit() -> Result<()> {
    let sender = test_addr("0x1111")?;
    let dao = dao_address()?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;
    let dao_balance_before = state
        .get_account(&dao)
        .map(|account| account.native_balance())
        .unwrap_or(0);

    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;
    let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());

    let mut create = ChangeSet::new();
    create.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: sender,
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
            data: coin_data.clone(),
            version: 1,
        },
    ));
    state.apply_changeset(&create)?;
    assert_eq!(state.get_account(&sender).unwrap().native_balance(), 1_000);

    let mut charge_gas_with_writeback = ChangeSet::new();
    charge_gas_with_writeback.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: sender,
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
            data: coin_data,
            version: 2,
        },
    ));
    charge_gas_with_writeback
        .get_or_create_change(sender)
        .debit(100);
    charge_gas_with_writeback.collect_gas(dao, 100);

    state.apply_changeset(&charge_gas_with_writeback)?;

    assert_eq!(state.get_account(&sender).unwrap().native_balance(), 900);
    assert_eq!(
        state.get_account(&dao).unwrap().native_balance(),
        dao_balance_before + 100
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
fn zero_effect_sequence_batch_rejects_overflow() -> Result<()> {
    let owner = test_addr("0x1111")?;
    let mut state = StateManager::new_in_memory();

    let mut account = Account::new(owner);
    account.sequence_number = u64::MAX;
    state.save_account(&account)?;

    let error = state
        .apply_zero_effect_sequence_batch([(owner, 1)])
        .expect_err("overflow must fail");
    assert!(error.to_string().contains("Sequence number overflow"));

    Ok(())
}

#[test]
fn token_balance_hint_is_ignored_when_owner_is_recomputed_from_objects() -> Result<()> {
    let sender = test_addr("0x1111")?;
    let dao = dao_address()?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;
    let dao_balance_before = state
        .get_account(&dao)
        .map(|account| account.native_balance())
        .unwrap_or(0);

    set_native_supply_for_test(&mut state, base.total_supply + 210_000)?;

    let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    coin_data[UID_SIZE..].copy_from_slice(&210_000u64.to_le_bytes());

    let mut init = ChangeSet::new();
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: sender,
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
            data: coin_data.clone(),
            version: 1,
        },
    ));
    state.apply_changeset(&init)?;

    let mut cs = ChangeSet::new();
    cs.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: sender,
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
            data: coin_data.clone(),
            version: 2,
        },
    ));
    cs.add_token_balance_set(sender, KANARI_TOKEN_TYPE.to_string(), 210_000);
    cs.get_or_create_change(sender).debit(100);
    cs.collect_gas(dao, 100);

    state.apply_changeset(&cs)?;

    assert_eq!(
        state.get_account(&sender).unwrap().native_balance(),
        209_900
    );
    assert_eq!(
        state.get_account(&dao).unwrap().native_balance(),
        dao_balance_before + 100
    );
    assert_eq!(
        state
            .token_supply_summary(KANARI_TOKEN_TYPE)?
            .wallet_visible_supply,
        base.wallet_visible_supply + 210_000
    );
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn validate_supply_invariants_detects_native_supply_overcount() -> Result<()> {
    let alice = test_addr("0x1111")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    let account = Account::with_native_balance(alice, 500);
    state.save_account(&account)?;
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
    let alice = test_addr("0x1111")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(KANARI_TOKEN_TYPE)?;

    let account = Account::with_native_balance(alice, 500);
    state.save_account(&account)?;
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
    let owner = test_addr("0x1111")?;
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
    let owner = test_addr("0x1111")?;
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
fn compute_state_root_reflects_overlay_before_commit() -> Result<()> {
    let publisher = test_addr("0x1111")?;
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
    let publisher = test_addr("0x1111")?;
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
    let owner = test_addr("0x1111")?;
    let mut state = StateManager::new_in_memory();

    let account = Account::with_native_balance(owner, 100);
    state.save_account(&account)?;
    state.commit()?;
    let root_before = state.compute_state_root();

    state
        .store
        .save(b"tx_receipt/local", &"node-local-receipt")?;
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
        "node-local metadata and orphan object-storage keys must not affect canonical state root"
    );

    Ok(())
}

#[test]
fn compute_state_root_tracks_indexed_canonical_objects() -> Result<()> {
    let owner = test_addr("0x1111")?;
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
    let publisher = test_addr("0x1111")?;
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
    let publisher = test_addr("0x1111")?;
    let owner = test_addr("0x2222")?;

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
