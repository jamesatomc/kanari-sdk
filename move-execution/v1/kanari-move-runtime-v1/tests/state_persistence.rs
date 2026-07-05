use anyhow::Result;
use kanari_move_runtime_v1::storage::persistent_store::PersistentStore;
use kanari_move_runtime_v1::{
    ChangeSet,
    state::{Account, StateManager},
};
use kanari_types::error::KanariUnwrapExt;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use move_core_types::account_address::AccountAddress;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;

const GENESIS_SUPPLY: u64 = 11_000_000_000_000_000;

fn open_state(db_path: &Path) -> Result<StateManager> {
    let store = Arc::new(PersistentStore::open_with_path(Some(
        db_path.to_path_buf(),
    ))?);
    Ok(StateManager::new(store))
}

fn seed_legacy_supply(db_path: &Path, total_supply: u64) -> Result<()> {
    let store = PersistentStore::open_with_path(Some(db_path.to_path_buf()))?;
    let mut global_supplies = BTreeMap::new();
    global_supplies.insert(KANARI_TOKEN_TYPE.to_string(), total_supply);

    let mut supply_key = b"supply:".to_vec();
    supply_key.extend_from_slice(KANARI_TOKEN_TYPE.as_bytes());

    // Simulate older databases that persisted the native supply but not `total_supply`.
    store.save(&supply_key, &total_supply)?;
    store.save(b"global_token_supplies", &global_supplies)?;
    Ok(())
}

#[test]
fn restart_recovery_preserves_native_supply_invariants() -> Result<()> {
    let temp_dir = tempdir()?;
    let db_path: PathBuf = temp_dir.path().join("state-db");
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let transfer_amount = 1_250_000_000u64;

    seed_legacy_supply(&db_path, GENESIS_SUPPLY)?;

    let mut initial_state = open_state(&db_path)?;
    assert_eq!(initial_state.total_supply, GENESIS_SUPPLY);
    assert_eq!(
        initial_state
            .store
            .load::<u64>(b"total_supply")?
            .invariant("backfilled total_supply must be persisted"),
        GENESIS_SUPPLY
    );
    initial_state.validate_supply_invariants()?;

    initial_state.save_account(&Account::with_native_balance(alice, GENESIS_SUPPLY))?;
    initial_state.commit()?;
    initial_state.validate_supply_invariants()?;
    drop(initial_state);

    let mut reopened_state = open_state(&db_path)?;
    assert_eq!(reopened_state.total_supply, GENESIS_SUPPLY);
    assert_eq!(reopened_state.account_count(), 1);
    assert_eq!(
        reopened_state
            .load_account(&alice)?
            .invariant("alice account must persist")
            .native_balance(),
        GENESIS_SUPPLY
    );
    reopened_state.validate_supply_invariants()?;

    let mut transfer = ChangeSet::new();
    transfer.transfer(alice, bob, transfer_amount);
    reopened_state.apply_changeset(&transfer)?;
    reopened_state.commit()?;
    reopened_state.validate_supply_invariants()?;
    assert_eq!(reopened_state.total_supply, GENESIS_SUPPLY);
    assert_eq!(
        reopened_state
            .load_account(&alice)?
            .invariant("alice account must remain present")
            .native_balance(),
        GENESIS_SUPPLY - transfer_amount
    );
    assert_eq!(
        reopened_state
            .load_account(&bob)?
            .invariant("bob account must be created by transfer")
            .native_balance(),
        transfer_amount
    );
    drop(reopened_state);

    let final_state = open_state(&db_path)?;
    assert_eq!(final_state.total_supply, GENESIS_SUPPLY);
    assert_eq!(final_state.account_count(), 2);
    assert_eq!(
        final_state
            .load_account(&alice)?
            .invariant("alice account must survive restart")
            .native_balance()
            + final_state
                .load_account(&bob)?
                .invariant("bob account must survive restart")
                .native_balance(),
        GENESIS_SUPPLY
    );
    final_state.validate_supply_invariants()?;

    Ok(())
}
