use super::*;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::transaction::{ObjectOwnerKind, ObjectRef};
use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};

fn address_owner(owner: AccountAddress) -> ObjectOwnerKind {
    ObjectOwnerKind::AddressOwner(owner.to_hex_literal())
}

fn set_native_supply_for_test(state: &mut StateManager, total_supply: u64) -> Result<()> {
    state.total_supply = total_supply;
    state.store.save(b"total_supply", &total_supply)?;
    state.store.save(
        &StateManager::supply_key(GAS_COIN),
        &TreasuryCap { total_supply },
    )?;
    Ok(())
}

#[test]
fn new_state_persists_runtime_and_wallet_index_versions() -> Result<()> {
    let state = StateManager::new_in_memory();
    assert_eq!(
        state.load_internal::<u32>(RUNTIME_STATE_SCHEMA_KEY)?,
        Some(RUNTIME_STATE_SCHEMA_VERSION)
    );
    assert_eq!(
        state.load_internal::<u32>(WALLET_SUPPLY_INDEX_VERSION_KEY)?,
        Some(WALLET_SUPPLY_INDEX_VERSION)
    );
    Ok(())
}

#[test]
fn concurrent_rocksdb_state_initialization_runs_genesis_once() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("state");
    let Ok(first_store) = PersistentStore::open_with_path(Some(db_path.clone())) else {
        return Ok(());
    };
    let second_store = PersistentStore::open_with_path(Some(db_path))?;
    let first_store = Arc::new(first_store);
    let second_store = Arc::new(second_store);
    assert!(
        Arc::ptr_eq(
            &first_store.get_db().expect("RocksDB store must expose DB"),
            &second_store.get_db().expect("RocksDB store must expose DB"),
        ),
        "test requires two PersistentStore wrappers for the same RocksDB handle"
    );
    let barrier = Arc::new(Barrier::new(2));
    let mut initializers = Vec::new();

    for store in [first_store.clone(), second_store] {
        let barrier = barrier.clone();
        initializers.push(std::thread::spawn(move || -> Result<u64> {
            barrier.wait();
            Ok(StateManager::try_new(store)?.total_supply)
        }));
    }

    let supplies = initializers
        .into_iter()
        .map(|initializer| initializer.join().expect("initializer thread panicked"))
        .collect::<Result<Vec<_>>>()?;
    assert!(supplies.iter().all(|supply| *supply > 0));
    assert_eq!(supplies[0], supplies[1]);
    assert_eq!(
        first_store.load::<bool>(GENESIS_INITIALIZED_KEY)?,
        Some(true),
        "concurrent initialization must leave one durable genesis marker"
    );

    let reopened = StateManager::try_new(first_store)?;
    assert_eq!(reopened.total_supply, supplies[0]);
    Ok(())
}

#[test]
fn access_versions_reject_stale_snapshot_and_survive_commit() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let mut changeset = ChangeSet::new();
    let key = b"resource:0x1::test::R".to_vec();
    changeset.record_move_write(key.clone(), Some(vec![1, 2, 3]));
    let access = changeset.deterministic_access_set();
    let before = state.access_version_snapshot();

    state.apply_changeset_without_supply_validation(&changeset)?;
    assert!(!state.validate_access_snapshot(&before, &access));
    let after = state.capture_access_versions(&access);
    assert_eq!(after.get(&key), Some(&1));

    let store = state.store();
    state.commit()?;
    let reopened = StateManager::try_new(store)?;
    assert_eq!(
        reopened.capture_access_versions(&access).get(&key),
        Some(&1)
    );
    Ok(())
}

#[test]
fn object_access_versions_survive_commit_and_reopen() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let key = b"object:0xabc".to_vec();
    let mut access = StateAccessSet::default();
    access.writes.insert(key.clone());

    state.advance_access_versions(&access)?;
    assert_eq!(state.capture_access_versions(&access).get(&key), Some(&1));

    let store = state.store();
    state.commit()?;
    let reopened = StateManager::try_new(store)?;
    assert_eq!(
        reopened.capture_access_versions(&access).get(&key),
        Some(&1)
    );
    Ok(())
}

#[test]
fn access_versions_only_invalidate_keys_that_changed() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let snapshot = state.access_version_snapshot();
    let mut write_a = ChangeSet::new();
    write_a.record_move_write(b"resource:a".to_vec(), Some(vec![1]));
    let mut read_a = ChangeSet::new();
    read_a.record_resolver_reads([b"resource:a".to_vec()]);
    let mut read_b = ChangeSet::new();
    read_b.record_resolver_reads([b"resource:b".to_vec()]);

    state.apply_changeset_without_supply_validation(&write_a)?;

    assert!(!state.validate_access_snapshot(&snapshot, &read_a.deterministic_access_set()));
    assert!(state.validate_access_snapshot(&snapshot, &read_b.deterministic_access_set()));
    Ok(())
}

#[test]
fn malformed_persisted_access_version_fails_startup_closed() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    state.commit()?;
    let store = state.store();
    let mut key = ACCESS_VERSION_PREFIX.to_vec();
    key.extend_from_slice(b"resource:corrupt");
    store.apply_raw_changes(&[(key, vec![0x80])], &[])?;

    let error = StateManager::try_new(store).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Malformed per-key state access version")
    );
    Ok(())
}

#[test]
fn owner_addresses_rejects_malformed_owner_index_entries() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    state.save_internal(OWNER_INDEX_KEY, &vec!["not-an-address".to_string()])?;

    let error = state.owner_addresses().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Owner index contains invalid address"),
        "{error:#}"
    );
    Ok(())
}

#[test]
fn query_objects_rejects_missing_object_index_entries() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    state.save_internal(b"object_index", &vec!["0xdead".to_string()])?;

    let error = state
        .query_objects(None, None, None, None, None)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Object index references missing object"),
        "{error:#}"
    );
    Ok(())
}

#[test]
fn access_version_overflow_rejects_changeset_without_partial_state() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let key = b"resource:overflow".to_vec();
    state.access_versions.insert(key.clone(), u64::MAX);
    let root_before = state.try_compute_state_root()?;
    let overlay_before = state.overlay.clone();
    let mut changeset = ChangeSet::new();
    changeset.record_move_write(key.clone(), Some(vec![7, 8, 9]));

    let error = state.apply_changeset(&changeset).unwrap_err();

    assert!(error.to_string().contains("State access version overflow"));
    assert_eq!(state.try_compute_state_root()?, root_before);
    assert_eq!(state.overlay, overlay_before);
    assert_eq!(state.access_versions.get(&key), Some(&u64::MAX));
    Ok(())
}

#[test]
fn state_root_fails_closed_when_canonical_index_is_corrupt() -> Result<()> {
    let state = StateManager::new_in_memory();
    state
        .store
        .apply_raw_changes(&[(b"module_index".to_vec(), vec![0x80])], &[])?;

    assert!(state.try_compute_state_root().is_err());
    assert!(state.try_canonical_state_snapshot().is_err());
    Ok(())
}

#[test]
fn supply_validation_fails_closed_when_native_treasury_is_corrupt() -> Result<()> {
    let state = StateManager::new_in_memory();
    let supply_key = StateManager::supply_key(GAS_COIN);
    state
        .store
        .apply_raw_changes(&[(supply_key, vec![0x80])], &[])?;

    assert!(state.validate_supply_invariants().is_err());
    Ok(())
}

#[test]
fn commit_with_raw_update_persists_checkpoint_marker_with_state_overlay() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    state.save_internal(b"test:state", &7u64)?;
    state.commit_with_raw_update(b"test:marker".to_vec(), bcs::to_bytes(&vec![9u8, 9])?)?;

    assert_eq!(state.store.load::<u64>(b"test:state")?, Some(7));
    assert_eq!(
        state.store.load::<Vec<u8>>(b"test:marker")?,
        Some(vec![9, 9])
    );
    Ok(())
}

#[test]
fn move_writes_commit_with_canonical_state_and_update_module_index() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let module_key = b"module:0x42:test".to_vec();
    let module_bytes = vec![1, 2, 3, 4];
    let mut changeset = ChangeSet::new();
    changeset.record_move_write(module_key.clone(), Some(module_bytes.clone()));

    state.apply_changeset(&changeset)?;
    assert_eq!(
        state.load_internal::<Vec<u8>>(&module_key)?,
        Some(module_bytes)
    );
    assert!(
        state
            .load_internal::<Vec<String>>(b"module_index")?
            .unwrap_or_default()
            .contains(&"module:0x42:test".to_string())
    );

    changeset = ChangeSet::new();
    changeset.record_move_write(module_key.clone(), None);
    state.apply_changeset(&changeset)?;
    assert_eq!(state.load_internal::<Vec<u8>>(&module_key)?, None);
    assert!(
        !state
            .load_internal::<Vec<String>>(b"module_index")?
            .unwrap_or_default()
            .contains(&"module:0x42:test".to_string())
    );
    Ok(())
}

#[test]
fn smt_diagnostics_are_read_only_and_full_audit_is_explicit() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let store = Arc::new(PersistentStore::open_with_path(Some(
        temp_dir.path().join("state"),
    ))?);
    let state = StateManager::try_new(store)?;

    let status = state.smt_diagnostics(false)?;
    assert!(status.enabled);
    assert!(!status.audit_requested);
    assert!(!status.audit_performed);
    assert!(status.persisted_root.is_some());
    assert!(status.persisted_leaf_count.is_none());
    assert!(status.consistent.is_none());
    assert_eq!(status.overlay_entries, 0);

    let audited = state.smt_diagnostics(true)?;
    assert!(audited.audit_requested);
    assert!(audited.audit_performed);
    assert_eq!(audited.consistent, Some(true));
    assert!(audited.consistency_error.is_none());
    assert!(audited.persisted_leaf_count.is_some_and(|count| count > 0));
    assert_eq!(audited.persisted_root, Some(audited.effective_root));

    Ok(())
}

#[test]
fn native_owner_overflow_is_rejected_without_mutating_state() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0xdead")?;
    let mut state = StateManager::new_in_memory();
    state.save_owner_state(&OwnerState::with_native_balance(owner, u64::MAX))?;
    let root_before = state.compute_state_root();

    let mut changeset = ChangeSet::new();
    changeset.mint(owner, 1);
    let error = state.apply_changeset(&changeset).unwrap_err();

    assert!(error.to_string().contains("Native owner balance overflow"));
    assert_eq!(state.compute_state_root(), root_before);
    assert_eq!(
        state
            .get_owner_state(&owner)
            .invariant("overflow test owner should exist")
            .native_balance(),
        u64::MAX
    );
    Ok(())
}

#[test]
fn identical_system_clock_replay_does_not_increment_object_version() -> Result<()> {
    let clock_id = "0xaade8aa25002489bbcfca67637daf4dac78f4c88606e0dfd5724f323cbda6b5d";
    let mut clock_data = vec![0u8; UID_SIZE + U64_SIZE];
    clock_data[..UID_SIZE].copy_from_slice(&[0xAA; UID_SIZE]);
    clock_data[UID_SIZE..].copy_from_slice(&7u64.to_le_bytes());

    let mut prologue = ChangeSet::new();
    prologue.created_objects.push((
        clock_id.to_string(),
        CreatedObject {
            owner: AccountAddress::ZERO,
            owner_kind: ObjectOwnerKind::Shared,
            uid: None,
            id: None,
            type_: "0x3::clock::Clock".to_string(),
            data: clock_data,
            version: 1,
        },
    ));

    let mut state = StateManager::new_in_memory();
    state.set_system_clock_object_id(AccountAddress::from_hex_literal(clock_id)?)?;
    state.apply_changeset(&prologue)?;
    let root_after_first_apply = state.compute_state_root();
    let version_after_first_apply = state
        .get_object(clock_id)?
        .invariant("clock must exist after first prologue")
        .version;

    state.apply_changeset(&prologue)?;

    assert_eq!(state.compute_state_root(), root_after_first_apply);
    assert_eq!(
        state
            .get_object(clock_id)?
            .invariant("clock must exist after replay")
            .version,
        version_after_first_apply,
        "replaying the same clock timestamp must not change its version"
    );

    let mut next_prologue = prologue.clone();
    next_prologue.created_objects[0].1.data[UID_SIZE..].copy_from_slice(&8u64.to_le_bytes());
    state.apply_changeset(&next_prologue)?;
    assert_eq!(
        state
            .get_object(clock_id)?
            .invariant("clock must exist after the next timestamp")
            .version,
        version_after_first_apply + 1,
        "a new clock timestamp must advance the object version exactly once"
    );
    Ok(())
}

#[test]
fn genesis_seeds_dev_wallet_with_separate_native_gas_coin() -> Result<()> {
    let state = StateManager::new_in_memory();
    let dev = AccountAddress::from_hex_literal(kanari_types::address::Address::DEV_ADDRESS)?;
    let native_coin_type = kanari_types::coin::CoinModule::coin_type(GAS_COIN);

    let native_coin_ids: Vec<_> = state
        .get_owned_objects(&dev)?
        .into_iter()
        .filter_map(|object_id| {
            let object = state.get_object(&object_id).ok().flatten()?;
            (object.type_ == native_coin_type).then_some(object_id)
        })
        .collect();

    assert!(
        native_coin_ids.len() >= 2,
        "genesis dev wallet must have separate native transfer and gas coin objects, found {:?}",
        native_coin_ids
    );
    Ok(())
}

#[test]
fn treasury_update_syncs_native_total_supply() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let owner = AccountAddress::from_hex_literal("0x1")?;
    let updated_supply = state.total_supply + 777;

    let mut cs = ChangeSet::new();
    cs.add_treasury(owner, GAS_COIN.to_string(), updated_supply);
    state.apply_changeset(&cs)?;

    assert_eq!(state.total_supply, updated_supply);
    Ok(())
}

#[test]
fn validate_supply_invariants_detects_native_supply_overcount() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(alice, 500))?;
    set_native_supply_for_test(&mut state, base.total_supply + 400)?;
    state
        .global_token_supplies
        .insert(GAS_COIN.to_string(), base.wallet_visible_supply + 500);

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
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(alice, 500))?;
    set_native_supply_for_test(&mut state, base.total_supply + 600)?;
    state
        .global_token_supplies
        .insert(GAS_COIN.to_string(), base.wallet_visible_supply + 500);

    let summary = state.token_supply_summary(GAS_COIN)?;
    assert_eq!(summary.total_supply, base.total_supply + 600);
    assert_eq!(
        summary.wallet_visible_supply,
        base.wallet_visible_supply + 500
    );
    assert_eq!(summary.object_locked_supply, 0);
    assert_eq!(summary.untracked_supply, 100);

    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn native_supply_summary_prefers_cached_visible_supply_when_owner_index_is_stale() -> Result<()> {
    let dao = AccountAddress::from_hex_literal("0x2222")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(dao, 210))?;
    set_native_supply_for_test(&mut state, base.total_supply + 210)?;
    state
        .global_token_supplies
        .insert(GAS_COIN.to_string(), base.wallet_visible_supply + 210);

    state.store.save(b"owner_index", &Vec::<String>::new())?;

    let summary = state.token_supply_summary(GAS_COIN)?;
    assert_eq!(
        summary.wallet_visible_supply,
        base.wallet_visible_supply + 210
    );
    assert_eq!(summary.object_locked_supply, 0);
    assert_eq!(summary.accounted_supply, summary.total_supply);
    assert_eq!(summary.untracked_supply, 0);

    Ok(())
}

#[test]
fn native_supply_summary_counts_object_fanout_when_owner_ledger_is_stale_low() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;
    set_native_supply_for_test(&mut state, base.total_supply + 51)?;

    let coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);
    for index in 0u8..51 {
        let mut changeset = ChangeSet::new();
        let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
        coin_data[..UID_SIZE].copy_from_slice(&[index; UID_SIZE]);
        coin_data[UID_SIZE..].copy_from_slice(&1u64.to_le_bytes());
        changeset.created_objects.push((
            format!("0x{}", hex::encode([index; UID_SIZE])),
            CreatedObject {
                owner,
                owner_kind: address_owner(owner),
                uid: None,
                id: None,
                type_: coin_type.clone(),
                data: coin_data,
                version: 1,
            },
        ));
        state.apply_changeset(&changeset)?;
    }

    assert_eq!(state.resolve_owner_native_balance(owner)?, 51);
    assert_eq!(
        state.indexed_wallet_supply(GAS_COIN)?,
        base.wallet_visible_supply + 51
    );
    let summary = state.token_supply_summary(GAS_COIN)?;
    assert_eq!(
        summary.wallet_visible_supply,
        base.wallet_visible_supply + 51
    );
    assert_eq!(summary.accounted_supply, summary.total_supply);
    assert_eq!(summary.untracked_supply, 0);

    Ok(())
}

#[test]
fn state_root_ignores_owner_indexes_and_supply_caches() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();

    state.save_owner_state(&OwnerState::with_native_balance(owner, 1_000))?;
    let canonical_root = state.compute_state_root();

    state.save_internal(b"owner_index", &vec![owner.to_hex_literal()])?;
    state.save_internal(
        b"global_token_supplies",
        &BTreeMap::from([(GAS_COIN.to_string(), 1_000u64)]),
    )?;
    state.save_internal(
        b"metadata_symbol:0x2::kanari::KANARI",
        &"KANARI".to_string(),
    )?;
    state.save_internal(
        b"object_locked_coin_records",
        &vec![serde_json::json!({"not":"canonical"})],
    )?;

    let indexed_root = state.compute_state_root();
    assert_eq!(canonical_root, indexed_root);

    Ok(())
}

#[test]
fn token_supply_summary_uses_treasury_supply_for_custom_tokens() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::test::TEST";
    let coin_type = format!("0x2::coin::Coin<{}>", token_type);
    let mut state = StateManager::new_in_memory();

    let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    coin_data[UID_SIZE..].copy_from_slice(&250u64.to_le_bytes());

    let mut cs = ChangeSet::new();
    cs.add_treasury(owner, token_type.to_string(), 1_000);
    cs.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner,
            owner_kind: address_owner(owner),
            uid: None,
            id: None,
            type_: coin_type,
            data: coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&cs)?;

    let summary = state.token_supply_summary(token_type)?;
    assert_eq!(summary.total_supply, 1_000);
    assert_eq!(summary.wallet_visible_supply, 250);
    assert_eq!(summary.object_locked_supply, 0);
    assert_eq!(summary.untracked_supply, 750);

    Ok(())
}

#[test]
fn resolve_owner_token_balances_requires_object_backed_non_native_assets() -> Result<()> {
    let object_owner = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::test::TEST";
    let coin_type = format!("0x2::coin::Coin<{}>", token_type);
    let mut state = StateManager::new_in_memory();

    let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    coin_data[UID_SIZE..].copy_from_slice(&250u64.to_le_bytes());

    let mut changeset = ChangeSet::new();
    changeset.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: object_owner,
            owner_kind: address_owner(object_owner),
            uid: None,
            id: None,
            type_: coin_type,
            data: coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&changeset)?;

    let object_balances = state.resolve_owner_token_balances(object_owner)?;
    assert_eq!(object_balances.get(token_type).copied(), Some(250));

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
            owner_kind: address_owner(owner),
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
            owner_kind: address_owner(owner),
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
            owner_kind: address_owner(owner),
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
            owner_kind: address_owner(owner),
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
            owner_kind: address_owner(owner),
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
            owner_kind: address_owner(owner),
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
            owner_kind: address_owner(owner),
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
            .get_owner_state(&publisher)
            .map(|account| account.modules.contains("example"))
            .unwrap_or(false)
    );

    Ok(())
}

#[test]
fn compute_state_root_ignores_runtime_local_store_keys() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();

    let owner_state = OwnerState::with_native_balance(owner, 100);
    state.save_owner_state(&owner_state)?;
    state.commit()?;
    let root_before = state.compute_state_root();

    // An orphan module that is absent from the canonical module index is runtime-local.
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
            owner_kind: address_owner(owner),
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
            owner_kind: address_owner(owner),
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
    StateManager::retain_canonical_state_root_entries(&mut entries)?;
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
            owner_kind: address_owner(owner),
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
    state.save_owner_state(&OwnerState::with_native_balance(sender, 5))?;
    let root_before = state.compute_state_root();

    let mut changeset = ChangeSet::new();
    changeset.transfer(sender, recipient, 10);

    let error = state.apply_changeset(&changeset).unwrap_err();
    assert!(error.to_string().contains("Insufficient native balance"));
    assert_eq!(state.compute_state_root(), root_before);
    assert!(state.get_owner_state(&recipient).is_none());
    assert_eq!(
        state
            .get_owner_state(&sender)
            .invariant("sender owner state should exist")
            .native_balance(),
        5
    );

    Ok(())
}

#[test]
fn apply_changeset_debits_object_backed_native_balance_without_panicking() -> Result<()> {
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let recipient = AccountAddress::from_hex_literal("0x2222")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;
    set_native_supply_for_test(&mut state, base.total_supply + 10)?;

    let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    coin_data[..UID_SIZE].copy_from_slice(&[0xA5; UID_SIZE]);
    coin_data[UID_SIZE..].copy_from_slice(&10u64.to_le_bytes());
    let mut seed = ChangeSet::new();
    seed.created_objects.push((
        "0xa5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5".to_string(),
        CreatedObject {
            owner: sender,
            owner_kind: address_owner(sender),
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", GAS_COIN),
            data: coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&seed)?;

    state.save_owner_state(&OwnerState::with_native_balance(sender, 0))?;
    assert_eq!(state.resolve_owner_native_balance(sender)?, 10);
    let root_before = state.compute_state_root();

    let mut transfer = ChangeSet::new();
    transfer.transfer(sender, recipient, 8);
    let mut remaining_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    remaining_coin_data[..UID_SIZE].copy_from_slice(&[0xA5; UID_SIZE]);
    remaining_coin_data[UID_SIZE..].copy_from_slice(&2u64.to_le_bytes());
    transfer.created_objects.push((
        "0xa5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5".to_string(),
        CreatedObject {
            owner: sender,
            owner_kind: address_owner(sender),
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", GAS_COIN),
            data: remaining_coin_data,
            version: 2,
        },
    ));

    state.apply_changeset(&transfer)?;
    assert_ne!(state.compute_state_root(), root_before);
    assert_eq!(state.resolve_owner_native_balance(sender)?, 2);
    assert_eq!(state.resolve_owner_native_balance(recipient)?, 8);

    Ok(())
}

#[test]
fn apply_changeset_rejects_supply_invariant_violation_without_mutating_live_state() -> Result<()> {
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let recipient = AccountAddress::from_hex_literal("0x2222")?;
    let mut state = StateManager::new_in_memory();

    state.save_owner_state(&OwnerState::with_native_balance(sender, 500))?;
    set_native_supply_for_test(&mut state, 400)?;
    state
        .global_token_supplies
        .insert(GAS_COIN.to_string(), 500);

    let root_before = state.compute_state_root();
    let sender_balance_before = state
        .get_owner_state(&sender)
        .invariant("sender owner state should exist")
        .native_balance();

    let mut changeset = ChangeSet::new();
    changeset.transfer(sender, recipient, 10);

    let error = state.apply_changeset(&changeset).unwrap_err();
    assert!(error.to_string().contains("native supply overcount"));
    assert_eq!(state.compute_state_root(), root_before);
    assert!(state.get_owner_state(&recipient).is_none());
    assert_eq!(
        state
            .get_owner_state(&sender)
            .invariant("sender owner state should exist")
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
        .get_owner_state(&owner)
        .invariant("owner state should exist")
        .native_balance();
    let before_visible = state.indexed_wallet_supply(GAS_COIN)?;

    let mut changeset = ChangeSet::new();
    changeset.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            owner_kind: address_owner(owner),
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
            .get_owner_state(&owner)
            .invariant("owner state should exist")
            .native_balance(),
        before_balance
    );
    assert_eq!(state.indexed_wallet_supply(GAS_COIN)?, before_visible);

    Ok(())
}

#[test]
fn recompute_owner_balances_preserves_native_gas_adjustments() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let dao = AccountAddress::from_hex_literal("0x2222")?;
    let token_type = "0x2::test::TEST";
    let coin_type = format!("0x2::coin::Coin<{}>", token_type);
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(owner, 500))?;
    set_native_supply_for_test(&mut state, base.total_supply + 500)?;
    state
        .global_token_supplies
        .insert(GAS_COIN.to_string(), base.wallet_visible_supply + 500);

    let before_balance = state
        .get_owner_state(&owner)
        .invariant("owner state should exist")
        .native_balance();

    let mut gas_only = ChangeSet::new();
    gas_only.get_or_create_owner_delta(owner).debit(210);
    gas_only.collect_gas(dao, 210);
    state.apply_changeset(&gas_only)?;
    let after_gas_balance = state
        .get_owner_state(&owner)
        .invariant("owner state should exist")
        .native_balance();
    assert_eq!(after_gas_balance, before_balance - 210);

    let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut mint = ChangeSet::new();
    mint.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            owner_kind: address_owner(owner),
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
            .get_owner_state(&owner)
            .invariant("owner state should exist")
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
    let base = state.token_supply_summary(GAS_COIN)?;
    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;

    let coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);
    let mut alice_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    alice_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut init = ChangeSet::new();
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
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
    transfer.get_or_create_owner_delta(alice).debit(10);
    transfer.collect_gas(gas_collector, 10);
    transfer.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
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
            owner_kind: address_owner(bob),
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
            .get_owner_state(&alice)
            .invariant("alice owner state should exist")
            .native_balance(),
        890
    );
    assert_eq!(
        state
            .get_owner_state(&bob)
            .invariant("bob owner state should exist")
            .native_balance(),
        100
    );
    assert_eq!(
        state
            .get_owner_state(&gas_collector)
            .invariant("gas collector owner state should exist")
            .native_balance(),
        10
    );
    assert_eq!(
        state.token_supply_summary(GAS_COIN)?.wallet_visible_supply,
        base.wallet_visible_supply + 1_000
    );
    let summary = state.token_supply_summary(GAS_COIN)?;
    assert_eq!(summary.accounted_supply, summary.total_supply);
    assert_eq!(summary.untracked_supply, 0);
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn native_token_balance_hints_do_not_double_count_transfers() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(alice, 1_000))?;
    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;
    state
        .global_token_supplies
        .insert(GAS_COIN.to_string(), base.wallet_visible_supply + 1_000);
    let before_summary = state.token_supply_summary(GAS_COIN)?;

    let mut transfer = ChangeSet::new();
    transfer.transfer(alice, bob, 100);
    transfer.add_token_balance_set(bob, GAS_COIN.to_string(), 100);

    state.apply_changeset(&transfer)?;

    assert_eq!(
        state
            .get_owner_state(&alice)
            .invariant("alice owner state should exist")
            .native_balance(),
        900
    );
    assert_eq!(
        state
            .get_owner_state(&bob)
            .invariant("bob owner state should exist")
            .native_balance(),
        100
    );
    assert_eq!(
        state.token_supply_summary(GAS_COIN)?.wallet_visible_supply,
        before_summary.wallet_visible_supply
    );
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn native_self_transfer_preserves_balance_except_explicit_gas() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(alice, 1_000))?;
    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;
    state
        .global_token_supplies
        .insert(GAS_COIN.to_string(), base.wallet_visible_supply + 1_000);

    let mut transfer = ChangeSet::new();
    transfer.transfer(alice, alice, 100);
    state.apply_changeset(&transfer)?;

    assert_eq!(
        state.resolve_owner_native_balance(alice)?,
        1_000,
        "self-transfer must not mint or burn native balance"
    );
    assert_eq!(
        state.token_supply_summary(GAS_COIN)?.wallet_visible_supply,
        base.wallet_visible_supply + 1_000
    );
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn get_owner_state_returns_none_for_missing_owner() -> Result<()> {
    let state = StateManager::new_in_memory();
    let missing = AccountAddress::from_hex_literal("0x4242")?;

    assert!(state.get_owner_state(&missing).is_none());
    assert!(state.get_owner_state_by_hex("0x4242").is_none());

    Ok(())
}

#[test]
fn native_coin_object_full_transfer_subtracts_gas_from_moved_coin() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;
    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;

    let coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);
    let mut alice_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    alice_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut init = ChangeSet::new();
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
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
    transfer.get_or_create_owner_delta(alice).debit(10);
    transfer.collect_gas(gas_collector, 10);
    transfer.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: bob,
            owner_kind: address_owner(bob),
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
            .get_owner_state(&alice)
            .map(|account| account.native_balance())
            .unwrap_or(0),
        0
    );
    assert_eq!(
        state
            .get_owner_state(&bob)
            .invariant("bob owner state should exist")
            .native_balance(),
        990
    );
    assert_eq!(
        state
            .get_owner_state(&gas_collector)
            .invariant("gas collector owner state should exist")
            .native_balance(),
        10
    );
    assert_eq!(
        state.token_supply_summary(GAS_COIN)?.wallet_visible_supply,
        base.wallet_visible_supply + 1_000
    );
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn object_backed_gas_recompute_preserves_prior_owner_only_native_debits() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;
    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;

    let coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);
    let mut coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());

    let mut init = ChangeSet::new();
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&init)?;
    assert_eq!(
        state
            .get_owner_state(&alice)
            .invariant("alice owner state should exist")
            .native_balance(),
        1_000
    );

    // Simulate a legacy owner-only gas debit path like module publish.
    let mut publish_like = ChangeSet::new();
    publish_like.get_or_create_owner_delta(alice).debit(7);
    publish_like.collect_gas(gas_collector, 7);
    state.apply_changeset(&publish_like)?;
    assert_eq!(
        state
            .get_owner_state(&alice)
            .invariant("alice owner state should exist")
            .native_balance(),
        993
    );

    // Then simulate an object-backed gas path touching the same coin object.
    let mut touched_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    touched_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut object_backed_call = ChangeSet::new();
    object_backed_call.get_or_create_owner_delta(alice).debit(3);
    object_backed_call.collect_gas(gas_collector, 3);
    object_backed_call.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
            uid: None,
            id: None,
            type_: coin_type,
            data: touched_coin_data,
            version: 2,
        },
    ));
    state.apply_changeset(&object_backed_call)?;

    assert_eq!(
        state.resolve_owner_native_balance(alice)?,
        990,
        "later object-backed gas must not erase earlier owner-only gas debits"
    );
    assert_eq!(
        state
            .get_owner_state(&alice)
            .invariant("alice owner state should exist")
            .native_balance(),
        990
    );

    Ok(())
}

#[test]
fn implicit_gas_object_adjustment_does_not_apply_total_gas_to_multiple_debit_owners() -> Result<()>
{
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;
    set_native_supply_for_test(&mut state, base.total_supply + 2_000)?;

    let coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);
    let mut alice_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    alice_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut bob_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    bob_coin_data[0] = 0xbb;
    bob_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut init = ChangeSet::new();
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: alice_coin_data,
            version: 1,
        },
    ));
    init.created_objects.push((
        "0xbbbb".to_string(),
        CreatedObject {
            owner: bob,
            owner_kind: address_owner(bob),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: bob_coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&init)?;

    let mut alice_touched = vec![0u8; UID_SIZE + U64_SIZE];
    alice_touched[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut bob_touched = vec![0u8; UID_SIZE + U64_SIZE];
    bob_touched[0] = 0xbb;
    bob_touched[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut changeset = ChangeSet::new();
    changeset.get_or_create_owner_delta(alice).debit(3);
    changeset.get_or_create_owner_delta(bob).debit(4);
    changeset.collect_gas(gas_collector, 7);
    changeset.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: alice_touched,
            version: 2,
        },
    ));
    changeset.created_objects.push((
        "0xbbbb".to_string(),
        CreatedObject {
            owner: bob,
            owner_kind: address_owner(bob),
            uid: None,
            id: None,
            type_: coin_type,
            data: bob_touched,
            version: 2,
        },
    ));

    state.apply_changeset(&changeset)?;

    assert_eq!(state.resolve_owner_native_balance(alice)?, 997);
    assert_eq!(state.resolve_owner_native_balance(bob)?, 996);
    assert_eq!(state.resolve_owner_native_balance(gas_collector)?, 7);

    Ok(())
}

#[test]
fn explicit_gas_object_adjusts_its_owner_even_with_multiple_debit_owners() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;
    set_native_supply_for_test(&mut state, base.total_supply + 2_000)?;

    let coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);
    let mut alice_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    alice_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut bob_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    bob_coin_data[0] = 0xbb;
    bob_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut init = ChangeSet::new();
    init.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: alice_coin_data,
            version: 1,
        },
    ));
    init.created_objects.push((
        "0xbbbb".to_string(),
        CreatedObject {
            owner: bob,
            owner_kind: address_owner(bob),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: bob_coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&init)?;

    let mut bob_touched = vec![0u8; UID_SIZE + U64_SIZE];
    bob_touched[0] = 0xbb;
    bob_touched[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut changeset = ChangeSet::new();
    changeset.get_or_create_owner_delta(alice).debit(7);
    changeset.get_or_create_owner_delta(bob).debit(4);
    changeset.collect_gas(gas_collector, 7);
    changeset
        .gas_object_refs
        .push(ObjectRef::new("0xaaaa".to_string(), Some(1), None));
    changeset.created_objects.push((
        "0xbbbb".to_string(),
        CreatedObject {
            owner: bob,
            owner_kind: address_owner(bob),
            uid: None,
            id: None,
            type_: coin_type,
            data: bob_touched,
            version: 2,
        },
    ));

    state.apply_changeset(&changeset)?;

    assert_eq!(state.resolve_owner_native_balance(alice)?, 993);
    assert_eq!(state.resolve_owner_native_balance(bob)?, 996);
    assert_eq!(state.resolve_owner_native_balance(gas_collector)?, 7);

    Ok(())
}

#[test]
fn custom_token_mint_repairs_stale_native_visible_supply_cache() -> Result<()> {
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let gas_collector = AccountAddress::from_hex_literal("0x3333")?;
    let custom_token = "0x2::usdc::USDC";
    let custom_coin_type = format!("0x2::coin::Coin<{}>", custom_token);
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(sender, 10_000))?;
    set_native_supply_for_test(&mut state, base.total_supply + 10_000)?;
    state.global_token_supplies.insert(
        GAS_COIN.to_string(),
        base.wallet_visible_supply + 10_000 + 6_614,
    );

    let mut mint = ChangeSet::new();
    mint.get_or_create_owner_delta(sender).debit(210);
    mint.collect_gas(gas_collector, 210);
    mint.add_treasury(sender, custom_token.to_string(), 1_000_000);
    let mut custom_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    custom_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    mint.created_objects.push((
        "0xc001".to_string(),
        CreatedObject {
            owner: sender,
            owner_kind: address_owner(sender),
            uid: None,
            id: None,
            type_: custom_coin_type,
            data: custom_coin_data,
            version: 1,
        },
    ));

    state.apply_changeset(&mint)?;

    assert_eq!(
        state.token_supply_summary(GAS_COIN)?.wallet_visible_supply,
        base.wallet_visible_supply + 10_000
    );
    assert_eq!(
        state
            .get_owner_state(&sender)
            .invariant("sender owner state should exist")
            .native_balance(),
        9_790
    );
    assert_eq!(
        state
            .get_owner_state(&gas_collector)
            .invariant("gas collector owner state should exist")
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
fn native_transfer_to_dao_accounts_object_balance_and_gas_credit() -> Result<()> {
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal(kanari_types::address::Address::DAO_ADDRESS)?;
    let gas_collector = bob;
    let coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    set_native_supply_for_test(&mut state, base.total_supply + 1_000)?;

    let mut sender_coin_before = vec![0u8; UID_SIZE + U64_SIZE];
    sender_coin_before[..UID_SIZE].copy_from_slice(&[0xAA; UID_SIZE]);
    sender_coin_before[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    let mut initial = ChangeSet::new();
    initial.created_objects.push((
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: sender_coin_before,
            version: 1,
        },
    ));
    state.apply_changeset(&initial)?;

    let mut sender_coin_after = vec![0u8; UID_SIZE + U64_SIZE];
    sender_coin_after[..UID_SIZE].copy_from_slice(&[0xAA; UID_SIZE]);
    sender_coin_after[UID_SIZE..].copy_from_slice(&800u64.to_le_bytes());

    let mut recipient_coin = vec![0u8; UID_SIZE + U64_SIZE];
    recipient_coin[..UID_SIZE].copy_from_slice(&[0xBB; UID_SIZE]);
    recipient_coin[UID_SIZE..].copy_from_slice(&200u64.to_le_bytes());

    let mut transfer = ChangeSet::new();
    transfer.get_or_create_owner_delta(alice).debit(10);
    transfer.collect_gas(gas_collector, 10);
    transfer.created_objects.push((
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: sender_coin_after,
            version: 2,
        },
    ));
    transfer.created_objects.push((
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        CreatedObject {
            owner: bob,
            owner_kind: address_owner(bob),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: recipient_coin,
            version: 1,
        },
    ));
    state.apply_changeset(&transfer)?;

    assert_eq!(
        state.resolve_owner_native_balance(alice)?,
        790,
        "sender must lose transfer amount plus gas based on canonical coin objects"
    );
    assert_eq!(
        state.resolve_owner_native_balance(bob)?,
        210,
        "recipient must receive the transferred native coin amount plus gas credit"
    );
    let summary = state.token_supply_summary(GAS_COIN)?;
    assert_eq!(summary.untracked_supply, 0);
    assert_eq!(summary.accounted_supply, summary.total_supply);

    let mut sender_coin_second = vec![0u8; UID_SIZE + U64_SIZE];
    sender_coin_second[..UID_SIZE].copy_from_slice(&[0xAA; UID_SIZE]);
    sender_coin_second[UID_SIZE..].copy_from_slice(&690u64.to_le_bytes());
    let mut recipient_coin_second = vec![0u8; UID_SIZE + U64_SIZE];
    recipient_coin_second[..UID_SIZE].copy_from_slice(&[0xCC; UID_SIZE]);
    recipient_coin_second[UID_SIZE..].copy_from_slice(&100u64.to_le_bytes());

    let mut second_transfer = ChangeSet::new();
    second_transfer.get_or_create_owner_delta(alice).debit(10);
    second_transfer.collect_gas(bob, 10);
    second_transfer.created_objects.push((
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        CreatedObject {
            owner: alice,
            owner_kind: address_owner(alice),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: sender_coin_second,
            version: 3,
        },
    ));
    second_transfer.created_objects.push((
        "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string(),
        CreatedObject {
            owner: bob,
            owner_kind: address_owner(bob),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: recipient_coin_second,
            version: 1,
        },
    ));
    state.apply_changeset(&second_transfer)?;

    assert_eq!(state.resolve_owner_native_balance(alice)?, 680);
    assert_eq!(
        state.resolve_owner_native_balance(bob)?,
        320,
        "DAO must preserve prior gas credits while receiving another object and gas credit"
    );
    let summary = state.token_supply_summary(GAS_COIN)?;
    assert_eq!(summary.untracked_supply, 0);
    assert_eq!(summary.accounted_supply, summary.total_supply);

    Ok(())
}

#[test]
fn custom_token_mint_updates_supply_from_treasury_cap_object() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::usdc::USDC";
    let cap_type = format!("0x2::coin::TreasuryCap<{}>", token_type);
    let coin_type = format!("0x2::coin::Coin<{}>", token_type);
    let mut state = StateManager::new_in_memory();

    let mut setup_cap_data = vec![0u8; UID_SIZE + U64_SIZE];
    setup_cap_data[UID_SIZE..].copy_from_slice(&0u64.to_le_bytes());
    let mut setup = ChangeSet::new();
    setup.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            owner_kind: address_owner(owner),
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
            owner_kind: address_owner(owner),
            uid: None,
            id: None,
            type_: cap_type,
            data: mint_cap_data,
            version: 2,
        },
    ));
    let mut minted_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    minted_coin_data[UID_SIZE..].copy_from_slice(&1_000_000u64.to_le_bytes());
    mint.created_objects.push((
        "0xbeef".to_string(),
        CreatedObject {
            owner,
            owner_kind: address_owner(owner),
            uid: None,
            id: None,
            type_: coin_type,
            data: minted_coin_data,
            version: 1,
        },
    ));

    state.apply_changeset(&mint)?;

    let summary = state.token_supply_summary(token_type)?;
    assert_eq!(summary.total_supply, 1_000_000);
    assert_eq!(summary.wallet_visible_supply, 1_000_000);
    assert_eq!(summary.object_locked_supply, 0);
    state.validate_supply_invariants()?;

    Ok(())
}

#[test]
fn custom_token_incoming_coin_adds_to_existing_wallet_balance() -> Result<()> {
    let owner = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::usdc::USDC";
    let cap_type = format!("0x2::coin::TreasuryCap<{}>", token_type);
    let coin_type = format!("0x2::coin::Coin<{}>", token_type);
    let mut state = StateManager::new_in_memory();

    let mut cap_data = vec![0u8; UID_SIZE + U64_SIZE];
    cap_data[UID_SIZE..].copy_from_slice(&200u64.to_le_bytes());
    let mut first_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    first_coin_data[UID_SIZE..].copy_from_slice(&100u64.to_le_bytes());

    let mut setup = ChangeSet::new();
    setup.created_objects.push((
        "0xcafe".to_string(),
        CreatedObject {
            owner,
            owner_kind: address_owner(owner),
            uid: None,
            id: None,
            type_: cap_type,
            data: cap_data,
            version: 1,
        },
    ));
    setup.created_objects.push((
        "0xaaaa".to_string(),
        CreatedObject {
            owner,
            owner_kind: address_owner(owner),
            uid: None,
            id: None,
            type_: coin_type.clone(),
            data: first_coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&setup)?;
    assert_eq!(state.resolve_owner_token_balance(owner, token_type)?, 100);

    let mut incoming_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    incoming_coin_data[UID_SIZE..].copy_from_slice(&50u64.to_le_bytes());
    let mut incoming = ChangeSet::new();
    incoming.created_objects.push((
        "0xbbbb".to_string(),
        CreatedObject {
            owner,
            owner_kind: address_owner(owner),
            uid: None,
            id: None,
            type_: coin_type,
            data: incoming_coin_data,
            version: 1,
        },
    ));
    state.apply_changeset(&incoming)?;

    assert_eq!(
        state.resolve_owner_token_balance(owner, token_type)?,
        150,
        "incoming token coin must add to existing wallet balance"
    );
    let summary = state.token_supply_summary(token_type)?;
    assert_eq!(summary.total_supply, 200);
    assert_eq!(summary.wallet_visible_supply, 150);
    assert_eq!(summary.object_locked_supply, 0);
    assert_eq!(summary.untracked_supply, 50);
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
    let custom_coin_type = format!("0x2::coin::Coin<{}>", custom_token);
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(sender, 10_000))?;
    state.save_owner_state(&OwnerState::with_native_balance(stale_account, 6_614))?;
    set_native_supply_for_test(&mut state, base.total_supply + 10_000)?;

    let mut mint = ChangeSet::new();
    mint.get_or_create_owner_delta(sender).debit(210);
    mint.collect_gas(gas_collector, 210);
    mint.add_treasury(sender, custom_token.to_string(), 1_000_000);
    let mut custom_coin_data = vec![0u8; UID_SIZE + U64_SIZE];
    custom_coin_data[UID_SIZE..].copy_from_slice(&1_000u64.to_le_bytes());
    mint.created_objects.push((
        "0xc002".to_string(),
        CreatedObject {
            owner: sender,
            owner_kind: address_owner(sender),
            uid: None,
            id: None,
            type_: custom_coin_type,
            data: custom_coin_data,
            version: 1,
        },
    ));

    state.apply_changeset(&mint)?;

    assert_eq!(
        state
            .get_owner_state(&stale_account)
            .invariant("stale account should still exist")
            .native_balance(),
        0
    );
    assert_eq!(
        state
            .get_owner_state(&sender)
            .invariant("sender owner state should exist")
            .native_balance(),
        9_790
    );
    assert_eq!(
        state
            .get_owner_state(&gas_collector)
            .invariant("gas collector owner state should exist")
            .native_balance(),
        210
    );
    assert_eq!(
        state.token_supply_summary(GAS_COIN)?.wallet_visible_supply,
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

#[test]
fn repair_legacy_native_wallet_overcount_reserves_locked_native_supply() -> Result<()> {
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let stale_account = AccountAddress::from_hex_literal(
        "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    )?;
    let holder_owner = AccountAddress::from_hex_literal("0x2222")?;
    let mut state = StateManager::new_in_memory();
    let base = state.token_supply_summary(GAS_COIN)?;

    state.save_owner_state(&OwnerState::with_native_balance(sender, 10_000))?;
    state.save_owner_state(&OwnerState::with_native_balance(stale_account, 6_614))?;
    state.save_object_locked_coin_records(&[ObjectLockedCoinRecord {
        holder_object_id: "0xlock".to_string(),
        holder_type: "0x2::escrow::Vault".to_string(),
        owner: holder_owner,
        token_type: GAS_COIN.to_string(),
        amount: 1_000,
    }])?;
    set_native_supply_for_test(&mut state, base.total_supply + 10_000)?;

    state.repair_legacy_native_wallet_overcount()?;

    assert_eq!(
        state
            .get_owner_state(&stale_account)
            .invariant("stale account should still exist")
            .native_balance(),
        0
    );
    assert_eq!(
        state
            .get_owner_state(&sender)
            .invariant("sender owner state should exist")
            .native_balance(),
        10_000
    );

    let summary = state.token_supply_summary(GAS_COIN)?;
    assert_eq!(summary.total_supply, base.total_supply + 10_000);
    assert_eq!(
        summary.wallet_visible_supply,
        base.wallet_visible_supply + 9_000
    );
    assert_eq!(summary.object_locked_supply, 1_000);
    assert_eq!(summary.accounted_supply, summary.total_supply);
    state.validate_supply_invariants()?;

    Ok(())
}
