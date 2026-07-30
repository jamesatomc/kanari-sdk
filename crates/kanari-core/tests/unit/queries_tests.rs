use super::*;
use crate::{CheckpointSyncData, consensus::Checkpoint};
use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
use kanari_move_runtime_v1::state::OwnerState;
use kanari_types::address::Address as KanariAddress;
use kanari_types::balance::BalanceRecord;
use kanari_types::coin::{CoinModule, TreasuryCap};
use kanari_types::gas_coin::GAS_COIN;
use kanari_types::transaction::{
    ObjectChange, ObjectChangeKind, ObjectGraphEdge, ObjectGraphEdgeKind, ObjectRef,
    SignedTransaction, Transaction,
};

fn signed_transfer(nonce: u64) -> SignedTransaction {
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let mut coin_data = vec![0u8; 40];
    coin_data[32..40].copy_from_slice(&1_000_000u64.to_le_bytes());
    let tx = Transaction::new_transfer_with_object_ref(
        sender.tagged_address(),
        ObjectRef::new(
            "0xaaaa",
            Some(1),
            Some(format!(
                "0x{}",
                hex::encode(kanari_crypto::hash_data_blake3(&coin_data))
            )),
        ),
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

fn fund_sender_with_coin(
    engine: &BlockchainEngine,
    owner: move_core_types::account_address::AccountAddress,
    coin_object_id: &str,
    balance: u64,
) {
    let mut coin_data = vec![0u8; 40];
    coin_data[32..40].copy_from_slice(&balance.to_le_bytes());
    let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());
    let previous_total = state.total_supply;
    let previous_visible = state
        .global_token_supplies
        .get(GAS_COIN)
        .copied()
        .unwrap_or(previous_total);

    let mut create_coin = ChangeSet::new();
    create_coin.created_objects.push((
        coin_object_id.to_string(),
        CreatedObject {
            owner,
            owner_kind: kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                owner.to_hex_literal(),
            ),
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", GAS_COIN),
            data: coin_data,
            version: 1,
        },
    ));
    state
        .apply_changeset_without_supply_validation(&create_coin)
        .unwrap();

    let mut owner_state = state
        .get_owner_state(&owner)
        .unwrap_or_else(|| OwnerState::new(owner));
    owner_state.set_token_balance(GAS_COIN.to_string(), BalanceRecord::new(balance));
    state.save_owner_state(&owner_state).unwrap();

    let updated_total = previous_total.saturating_add(balance);
    let updated_visible = previous_visible.saturating_add(balance);
    state.total_supply = updated_total;
    state.store.save(b"total_supply", &updated_total).unwrap();
    state
        .store
        .save(
            format!("supply:{}", GAS_COIN).as_bytes(),
            &TreasuryCap {
                total_supply: updated_total,
            },
        )
        .unwrap();
    state
        .global_token_supplies
        .insert(GAS_COIN.to_string(), updated_visible);
    state
        .store
        .save(b"global_token_supplies", &state.global_token_supplies)
        .unwrap();
}

#[test]
fn fresh_engine_owner_query_exposes_separate_genesis_native_gas_coin() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    assert_fresh_engine_exposes_separate_genesis_native_gas_coin(&engine);
}

#[test]
fn fresh_persistent_engine_owner_query_exposes_separate_genesis_native_gas_coin() {
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = BlockchainEngine::new_dir(temp_dir.path().to_str().unwrap()).unwrap();
    assert_fresh_engine_exposes_separate_genesis_native_gas_coin(&engine);
}

#[test]
fn genesis_manifest_round_trips_and_matches_a_fresh_node() {
    let source = BlockchainEngine::new_in_memory().unwrap();
    let manifest = source.genesis_manifest("devnet").unwrap();
    let target = BlockchainEngine::new_in_memory().unwrap();

    target
        .validate_genesis_manifest(&manifest, "devnet")
        .unwrap();
    assert_eq!(manifest, target.genesis_manifest("devnet").unwrap());
}

#[test]
fn genesis_manifest_rejects_network_or_root_mismatch() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let mut manifest = engine.genesis_manifest("devnet").unwrap();

    let error = engine
        .validate_genesis_manifest(&manifest, "mainnet")
        .unwrap_err();
    assert!(error.to_string().contains("network mismatch"));

    manifest.network = "devnet".to_string();
    manifest.genesis_state_root = "deadbeef".to_string();
    let error = engine
        .validate_genesis_manifest(&manifest, "devnet")
        .unwrap_err();
    assert!(error.to_string().contains("state root mismatch"));
}

#[test]
fn genesis_manifest_file_is_portable_json() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested").join("genesis.json");

    engine.write_genesis_manifest(&path, "devnet").unwrap();
    let loaded = BlockchainEngine::read_genesis_manifest(&path).unwrap();
    engine.validate_genesis_manifest(&loaded, "devnet").unwrap();
}

#[test]
fn state_snapshot_round_trips_into_an_empty_data_dir() {
    let source = BlockchainEngine::new_in_memory().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let snapshot_path = directory.path().join("snapshots").join("devnet.json");
    let target_dir = directory.path().join("node5");

    let exported = source
        .export_state_snapshot(&snapshot_path, "devnet")
        .unwrap();
    let imported = BlockchainEngine::import_state_snapshot(
        &snapshot_path,
        &target_dir,
        "devnet",
        &exported.checkpoint_hash,
    )
    .unwrap();

    assert_eq!(exported.checkpoint_height, imported.checkpoint_height);
    assert_eq!(exported.checkpoint_hash, imported.checkpoint_hash);
    assert_eq!(exported.state_root, imported.state_root);
}

#[test]
fn state_snapshot_rejects_untrusted_checkpoint_hash_before_import() {
    let source = BlockchainEngine::new_in_memory().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let snapshot_path = directory.path().join("snapshot.json");
    let target_dir = directory.path().join("node5");
    source
        .export_state_snapshot(&snapshot_path, "devnet")
        .unwrap();

    let error = BlockchainEngine::import_state_snapshot(
        &snapshot_path,
        &target_dir,
        "devnet",
        "0xdeadbeef",
    )
    .unwrap_err();
    assert!(error.to_string().contains("externally trusted hash"));
    assert!(!target_dir.exists());
}

#[test]
fn state_snapshot_rejects_tampered_entries_before_import() {
    let source = BlockchainEngine::new_in_memory().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let snapshot_path = directory.path().join("snapshot.json");
    let target_dir = directory.path().join("node5");
    source
        .export_state_snapshot(&snapshot_path, "devnet")
        .unwrap();

    let mut snapshot = BlockchainEngine::read_state_snapshot(&snapshot_path).unwrap();
    snapshot.entries[0].value.push('0');
    std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();

    let error =
        BlockchainEngine::import_trusted_state_snapshot(&snapshot_path, &target_dir, "devnet")
            .unwrap_err();
    assert!(error.to_string().contains("entries hash mismatch"));
}

fn assert_fresh_engine_exposes_separate_genesis_native_gas_coin(engine: &BlockchainEngine) {
    let owner = KanariAddress::DEV_ADDRESS;
    let native_coin_type = CoinModule::coin_type(GAS_COIN);

    let owner_info = engine
        .get_owner_info(owner)
        .expect("dev owner should exist after genesis");
    let owned_objects = owner_info
        .owned_objects
        .expect("owner query should include object list");
    let native_coin_ids: Vec<_> = owned_objects
        .into_iter()
        .filter(|object| object.type_ == native_coin_type)
        .map(|object| object.id)
        .collect();

    assert!(
        native_coin_ids.len() >= 2,
        "fresh engine query must expose separate native transfer and gas coin objects, found {:?}",
        native_coin_ids
    );
}

#[test]
fn try_get_owner_info_rejects_malformed_owner_state() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let owner = KanariAddress::DEV_ADDRESS;
    let owner_addr = KanariAddress::parse_to_account_address(owner).unwrap();
    let owner_key = {
        let mut key = b"account:".to_vec();
        key.extend_from_slice(owner_addr.as_ref());
        key
    };
    engine
        .state_read()
        .store
        .apply_raw_changes(&[(owner_key, vec![0x80])], &[])
        .unwrap();

    let error = engine.try_get_owner_info(owner).unwrap_err();
    assert!(
        error.to_string().contains("Failed to load owner state"),
        "{error:#}"
    );
}

#[test]
fn sync_checkpoint_from_data_rejects_uncertified_checkpoint() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };
    let state_root = engine
        .state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();
    let checkpoint = Checkpoint::new(1, vec![], vec![], state_root, 42, prev_hash);
    let sync_data = CheckpointSyncData {
        checkpoint,
        dag_vertices: Vec::new(),
    };

    let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
    assert!(error.to_string().contains("has not been committed"));
    assert_eq!(engine.get_stats().height, 0);
}

#[test]
fn sync_checkpoint_from_data_rejects_root_mismatch() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };
    let signed_tx = signed_transfer(0);
    let sender =
        KanariAddress::parse_to_account_address(signed_tx.transaction.sender_address()).unwrap();
    fund_sender_with_coin(&engine, sender, "0xaaaa", 1_000_000);
    let checkpoint = Checkpoint::new(1, vec![], vec![signed_tx], vec![9u8; 32], 42, prev_hash);
    let sync_data = CheckpointSyncData {
        checkpoint,
        dag_vertices: Vec::new(),
    };

    let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
    assert!(error.to_string().contains("has not been committed"));
    assert_eq!(engine.get_stats().height, 0);
}

#[test]
fn sync_checkpoint_root_mismatch_does_not_mutate_local_state() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let height_before = engine.get_stats().height;
    let root_before = engine.latest_checkpoint_state_root_hex();
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };

    // This models a peer built with a different genesis/state schema. The
    // payload is otherwise structurally valid, but its advertised root is
    // intentionally not the root produced by this node.
    let signed_tx = signed_transfer(0);
    let sender =
        KanariAddress::parse_to_account_address(signed_tx.transaction.sender_address()).unwrap();
    fund_sender_with_coin(&engine, sender, "0xaaaa", 1_000_000);
    let checkpoint = Checkpoint::new(1, vec![], vec![signed_tx], vec![0xabu8; 32], 42, prev_hash);

    let error = engine
        .sync_checkpoint_from_data(&CheckpointSyncData {
            checkpoint,
            dag_vertices: Vec::new(),
        })
        .unwrap_err();

    assert!(error.to_string().contains("has not been committed"));
    assert_eq!(engine.get_stats().height, height_before);
    assert_eq!(engine.latest_checkpoint_state_root_hex(), root_before);
}

#[test]
fn block_queries_include_checkpoint_object_changes() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };
    let checkpoint = Checkpoint::new(1, vec![], vec![], vec![9u8; 32], 42, prev_hash)
        .with_object_changes(vec![ObjectChange {
            change_type: ObjectChangeKind::Created,
            object_ref: ObjectRef::new("0x1", Some(1), Some("0xabc".to_string())),
            previous_object_ref: None,
            type_: Some("0x2::test::Thing".to_string()),
            owner: None,
            previous_owner: None,
            previous_version: None,
        }])
        .with_object_graph_edges(vec![ObjectGraphEdge {
            source_object_ref: ObjectRef::new("0xgas", Some(1), Some("0xdef".to_string())),
            target_object_ref: ObjectRef::new("0x1", Some(1), Some("0xabc".to_string())),
            relation: ObjectGraphEdgeKind::GasCreate,
        }]);

    {
        let mut chain = engine.blockchain.write().unwrap_or_else(|e| e.into_inner());
        chain
            .add_checkpoint_with_validation(checkpoint, false)
            .unwrap();
    }

    let block = engine.get_block(1).expect("block should exist");
    let full_block = engine.get_full_block(1).expect("full block should exist");
    assert_eq!(block.object_changes.len(), 1);
    assert_eq!(full_block.object_changes.len(), 1);
    assert_eq!(block.object_changes[0].object_ref.object_id, "0x1");
    assert_eq!(block.object_graph_edges.len(), 1);
    assert_eq!(full_block.object_graph_edges.len(), 1);
}

#[test]
fn block_from_full_data_rejects_invalid_hash_fields() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let mut full_block = engine
        .get_full_block(0)
        .expect("genesis block should exist");

    full_block.prev_hash = "not-hex".to_string();
    let error = BlockchainEngine::try_block_from_full_data(&full_block).unwrap_err();
    assert!(
        error.to_string().contains("block prev_hash"),
        "unexpected error: {error}"
    );

    let mut full_block = engine
        .get_full_block(0)
        .expect("genesis block should exist");
    full_block.state_root = "0x01".to_string();
    let error = BlockchainEngine::try_block_from_full_data(&full_block).unwrap_err();
    assert!(
        error.to_string().contains("block state_root"),
        "unexpected error: {error}"
    );
}
