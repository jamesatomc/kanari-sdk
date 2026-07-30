use super::*;
use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
use kanari_move_runtime_v1::state::OwnerState;
use kanari_types::balance::BalanceRecord;
use kanari_types::coin::{CoinModule, TreasuryCap};
use kanari_types::gas_coin::GAS_COIN;
use kanari_types::transaction::{ObjectRef, SignedTransaction, Transaction};
use move_core_types::account_address::AccountAddress;
use proptest::prelude::*;

#[test]
fn dag_repair_requests_quorum_without_a_local_vertex_after_restart() {
    let policy = DagProductionPolicy {
        current_round: 42,
        target_round: 43,
        parent_round: 42,
        parent_author_count: 1,
        quorum_size: 3,
        parent_ids: vec![],
        parent_authors: vec!["authority-a".to_string()],
        missing_parent_authors: vec!["authority-b".to_string(), "authority-c".to_string()],
    };

    assert!(policy.should_wait_for_current_round_quorum());
}

fn authority_key(seed: u8) -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
}

fn signed_transfer(nonce: u64) -> SignedTransaction {
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = Transaction::new_transfer_with_object_ref(
        sender.tagged_address(),
        ObjectRef::new("0xaaaa", Some(1), Some("0xtestdigest".to_string())),
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
    address: &str,
    coin_object_id: &str,
    balance: u64,
) {
    let owner = AccountAddress::from_hex_literal(address).unwrap();
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
            type_: CoinModule::coin_type(GAS_COIN),
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
    let next_balance = owner_state
        .token_balances
        .get(GAS_COIN)
        .map(|record| record.value.saturating_add(balance))
        .unwrap_or(balance);
    owner_state.set_token_balance(GAS_COIN.to_string(), BalanceRecord::new(next_balance));
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

fn signed_transfer_with_refs(
    sender: &kanari_crypto::keys::KeyPair,
    recipient: &str,
    coin_object_id: &str,
    coin_balance: u64,
    gas_object_id: &str,
    gas_balance: u64,
    nonce: u64,
) -> SignedTransaction {
    let mut coin_data = vec![0u8; 40];
    coin_data[32..40].copy_from_slice(&coin_balance.to_le_bytes());
    let mut gas_data = vec![0u8; 40];
    gas_data[32..40].copy_from_slice(&gas_balance.to_le_bytes());

    let mut tx = Transaction::new_transfer_with_object_ref_and_gas(
        sender.tagged_address(),
        ObjectRef::new(
            coin_object_id,
            Some(1),
            Some(format!(
                "0x{}",
                hex::encode(kanari_crypto::hash_data_blake3(&coin_data))
            )),
        ),
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
        gas_payment.payment_objects = vec![ObjectRef::new(
            gas_object_id,
            Some(1),
            Some(format!(
                "0x{}",
                hex::encode(kanari_crypto::hash_data_blake3(&gas_data))
            )),
        )];
    }
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    signed_tx
}

fn signed_network_vertex(
    author: &str,
    signing_key: &ed25519_dalek::SigningKey,
    round: u64,
    parents: Vec<(String, u64, [u8; 32])>,
) -> DagVertex {
    let tx = signed_transfer(0);
    let author_index = author.trim_start_matches("auth").parse::<u64>().unwrap() - 1;
    let includes = if round == 1 {
        vec![
            *Block::genesis(MysticetiAuthority::new(0)).reference(),
            *Block::genesis(MysticetiAuthority::new(1)).reference(),
        ]
    } else {
        parents
            .iter()
            .map(|(parent_author, parent_round, _)| {
                let index = parent_author
                    .trim_start_matches("auth")
                    .parse::<u64>()
                    .unwrap()
                    - 1;
                MysticetiBlockReference {
                    authority: MysticetiAuthority::new(index),
                    round: *parent_round,
                    digest: mysticeti_dag::crypto::BlockDigest::synthetic(
                        *parent_round,
                        MysticetiAuthority::new(index),
                    ),
                }
            })
            .collect()
    };
    let native_tx = signed_tx_to_mysticeti_transaction(&tx).unwrap();
    let crypto =
        MysticetiCryptoEngine::enabled(MysticetiSigner::from_bytes(signing_key.to_bytes()));
    let block = MysticetiBlockData::new(Block::new(
        MysticetiAuthority::new(author_index),
        round,
        includes,
        vec![native_tx],
        123_000_000,
        &crypto,
    ));
    let canonical_parents = block
        .includes()
        .iter()
        .map(|reference| {
            (
                format!("auth{}", reference.authority.index() + 1),
                reference.round,
                mysticeti_reference_to_vertex_id(reference),
            )
        })
        .collect();
    let mut vertex = DagVertex::new(
        round,
        author.to_string(),
        "kanari-v2-mysticeti".to_string(),
        canonical_parents,
        vec![tx],
        123,
    );
    vertex.id = mysticeti_reference_to_vertex_id(block.reference());
    vertex.signature = block.signature().as_ref().to_vec();
    vertex.native_block = block.serialized_bytes().to_vec();
    vertex
}

fn owned_objects_key(owner: &AccountAddress) -> Vec<u8> {
    let mut key = b"owned_objects:".to_vec();
    key.extend_from_slice(owner.as_ref());
    key
}

fn poison_noncanonical_indexes(engine: &BlockchainEngine, owner: &str) {
    let owner_address = AccountAddress::from_hex_literal(owner).unwrap();
    let state = engine.state.write().unwrap_or_else(|e| e.into_inner());
    state
        .store
        .save(b"owner_index", &vec!["0xdead".to_string()])
        .unwrap();
    state
        .store
        .save(b"object_index", &vec!["0xdead".to_string()])
        .unwrap();
    state
        .store
        .save(
            &owned_objects_key(&owner_address),
            &vec!["0xdead".to_string()],
        )
        .unwrap();
    state
        .store
        .save(b"global_token_supplies", &BTreeMap::<String, u64>::new())
        .unwrap();
}

fn first_canonical_snapshot_divergence(
    left: &BlockchainEngine,
    right: &BlockchainEngine,
) -> Option<String> {
    let left_snapshot = left
        .state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .canonical_state_snapshot();
    let right_snapshot = right
        .state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .canonical_state_snapshot();

    for key in left_snapshot.keys().chain(right_snapshot.keys()) {
        match (left_snapshot.get(key), right_snapshot.get(key)) {
            (Some(left_value), Some(right_value)) if left_value == right_value => {}
            (Some(left_value), Some(right_value)) => {
                return Some(format!(
                    "key={} left={} right={}",
                    String::from_utf8_lossy(key),
                    hex::encode(left_value),
                    hex::encode(right_value)
                ));
            }
            (Some(left_value), None) => {
                return Some(format!(
                    "key={} missing_on_right left={}",
                    String::from_utf8_lossy(key),
                    hex::encode(left_value)
                ));
            }
            (None, Some(right_value)) => {
                return Some(format!(
                    "key={} missing_on_left right={}",
                    String::from_utf8_lossy(key),
                    hex::encode(right_value)
                ));
            }
            (None, None) => {}
        }
    }

    None
}

#[test]
fn test_dag_engine_defaults_to_mysticeti_protocol() {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let signing_key = authority_key(11);
    let mut public_keys = BTreeMap::new();
    public_keys.insert(
        "auth1".to_string(),
        signing_key.verifying_key().to_bytes().to_vec(),
    );
    for (authority, seed) in [("auth2", 12), ("auth3", 13), ("auth4", 14)] {
        public_keys.insert(
            authority.to_string(),
            authority_key(seed).verifying_key().to_bytes().to_vec(),
        );
    }
    let dag_engine = DagEngine::new_secure(
        engine,
        "auth1".to_string(),
        vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ],
        signing_key,
        public_keys,
    )
    .unwrap();

    let state = lock_read(&dag_engine.state);
    assert_eq!(state.mysticeti.protocol.wave_length, 3);
    assert_eq!(state.mysticeti.protocol.direct_commit_quorum, 3);
    assert!(state.mysticeti.protocol.pipeline);
    assert!(state.mysticeti.protocol.leader_wait);
}

#[test]
fn test_dag_engine_secure_constructor_rejects_mismatched_local_key() {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let expected = authority_key(11);
    let wrong = authority_key(33);
    let mut public_keys = BTreeMap::new();
    public_keys.insert(
        "auth1".to_string(),
        expected.verifying_key().to_bytes().to_vec(),
    );

    let result = DagEngine::new_secure(
        engine,
        "auth1".to_string(),
        vec!["auth1".to_string()],
        wrong,
        public_keys,
    );

    assert!(result.is_err());
}

#[test]
fn test_distinct_authorities_exchange_native_blocks_and_commit_same_dag() {
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    let engines = authorities
        .iter()
        .map(|local| {
            let mut engine = BlockchainEngine::new_in_memory().unwrap();
            engine.set_authorities(local.clone(), authorities.clone());
            let (key, public_keys) = secure_consensus_keys(&authorities, local);
            engine.set_consensus_signing_key(key, public_keys).unwrap();
            Arc::new(engine)
        })
        .collect::<Vec<_>>();

    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    for engine in &engines {
        fund_sender_with_coin(engine, &sender.address, "0xaaaa", 2_000_000);
        fund_sender_with_coin(engine, &sender.address, "0x1001", 1_000_000);
    }

    // Empty Mysticeti waves must keep consensus live without inflating the
    // externally visible blockchain checkpoint height.
    for _ in 0..8 {
        let vertices = engines
            .iter()
            .filter_map(|engine| engine.produce_checkpoint().ok()?.vertex)
            .collect::<Vec<_>>();
        for vertex in vertices {
            for engine in &engines {
                if engine.authority_id() != vertex.author {
                    engine.add_network_dag_vertex(vertex.clone()).unwrap();
                }
            }
        }
    }
    assert!(engines.iter().all(|engine| engine.get_stats().height == 0));

    let transaction = signed_transfer_with_refs(
        &sender,
        &recipient.address,
        "0xaaaa",
        2_000_000,
        "0x1001",
        1_000_000,
        1,
    );
    let transaction_hash = transaction.transaction_hash().to_vec();
    engines[0]
        .submit_transactions_batch(vec![transaction])
        .unwrap();

    for _ in 0..12 {
        let vertices = engines
            .iter()
            .filter_map(|engine| engine.produce_checkpoint().ok()?.vertex)
            .collect::<Vec<_>>();
        for vertex in vertices {
            for engine in &engines {
                if engine.authority_id() != vertex.author {
                    engine.add_network_dag_vertex(vertex.clone()).unwrap();
                }
            }
        }
    }

    let checkpoints = engines
        .iter()
        .map(|engine| {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            assert!(
                chain.height() > 0,
                "Mysticeti should commit after multiple waves"
            );
            assert!(chain.is_transaction_hash_executed(&transaction_hash));
            (chain.height(), chain.latest_checkpoint().hash().unwrap())
        })
        .collect::<Vec<_>>();
    assert!(checkpoints.windows(2).all(|pair| pair[0] == pair[1]));

    // A bounded live broadcast must not fail merely because older parent
    // ancestry exceeds its transport budget. Local Mysticeti verification still
    // requires omitted parents before any received checkpoint can commit.
    for engine in &engines {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        let roots = chain.latest_checkpoint().vertices.clone();
        drop(chain);
        let evidence = engine.dag_vertices_for_checkpoint_sync(&roots, 1).unwrap();
        assert!(evidence.len() <= 1);
    }
}

fn build_test_dag_engine(
    authorities: Vec<String>,
    local_authority: &str,
) -> (Arc<BlockchainEngine>, DagEngine, ed25519_dalek::SigningKey) {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let local_key = authority_key(11);
    let remote_key = authority_key(22);
    let mut public_keys = BTreeMap::new();
    for auth in &authorities {
        let key = if auth == local_authority {
            &local_key
        } else {
            &remote_key
        };
        public_keys.insert(auth.clone(), key.verifying_key().to_bytes().to_vec());
    }
    let dag_engine = DagEngine::new_secure(
        engine.clone(),
        local_authority.to_string(),
        authorities,
        local_key,
        public_keys,
    )
    .unwrap();
    (engine, dag_engine, remote_key)
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
fn test_add_network_vertex_accepts_valid_remote_vertex() {
    let (_engine, dag_engine, remote_key) =
        build_test_dag_engine(vec!["auth1".to_string(), "auth2".to_string()], "auth1");

    let vertex = signed_network_vertex("auth2", &remote_key, 1, vec![]);
    dag_engine.add_network_vertex(vertex).unwrap();

    let vertices = dag_engine.vertices_for_sync(usize::MAX).unwrap();
    assert_eq!(vertices.len(), 1);
    assert_eq!(vertices[0].author, "auth2");
}

#[test]
fn test_add_network_vertex_rejects_invalid_signature() {
    let (engine, dag_engine, _remote_key) =
        build_test_dag_engine(vec!["auth1".to_string(), "auth2".to_string()], "auth1");
    let wrong_key = authority_key(33);

    let vertex = signed_network_vertex("auth2", &wrong_key, 1, vec![]);
    let error = dag_engine.add_network_vertex(vertex).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Invalid canonical Mysticeti block")
    );
    assert_eq!(engine.get_stats().height, 0);
}

#[test]
fn test_add_network_vertex_rejects_invalid_parent_clock() {
    let (engine, dag_engine, remote_key) =
        build_test_dag_engine(vec!["auth1".to_string(), "auth2".to_string()], "auth1");

    let mut vertex = signed_network_vertex("auth2", &remote_key, 2, vec![]);
    vertex.parents = vec![
        ("auth1".to_string(), 1u64, [1u8; 32]),
        ("auth2".to_string(), 1u64, [2u8; 32]),
    ];
    let error = dag_engine.add_network_vertex(vertex).unwrap_err();
    let error_chain = format!("{error:#}");
    assert!(
        error_chain.contains("Missing parent")
            || error_chain.contains("missing parents")
            || error_chain.contains("Threshold clock is not valid"),
        "unexpected error: {error_chain}"
    );
    assert_eq!(engine.get_stats().height, 0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn arbitrary_byzantine_native_block_never_advances_checkpoint(
        payload in prop::collection::vec(any::<u8>(), 0..2048),
    ) {
        let (engine, dag_engine, remote_key) = build_test_dag_engine(
            vec!["auth1".to_string(), "auth2".to_string()],
            "auth1",
        );
        let mut vertex = signed_network_vertex("auth2", &remote_key, 1, vec![]);
        vertex.native_block = payload;
        vertex.signature = vec![0; 64];

        let _ = dag_engine.add_network_vertex(vertex);
        prop_assert_eq!(engine.get_stats().height, 0);
    }
}

/// Opt-in high-volume Byzantine corpus. Every malformed native block must be
/// rejected without allowing a checkpoint to advance.
#[test]
#[ignore = "long-running Byzantine Mysticeti soak test"]
fn long_run_byzantine_native_blocks_cannot_advance_checkpoint() {
    use proptest::test_runner::{Config, TestRunner};

    let mut runner = TestRunner::new(Config {
        cases: 64,
        max_shrink_iters: 0,
        ..Config::default()
    });
    let strategy = prop::collection::vec(any::<u8>(), 0..2_048);

    runner
        .run(&strategy, |payload| {
            let (engine, dag_engine, remote_key) =
                build_test_dag_engine(vec!["auth1".to_string(), "auth2".to_string()], "auth1");
            let mut vertex = signed_network_vertex("auth2", &remote_key, 1, vec![]);
            vertex.native_block = payload;
            vertex.signature = vec![0; 64];

            let _ = dag_engine.add_network_vertex(vertex);
            prop_assert_eq!(engine.get_stats().height, 0);
            Ok(())
        })
        .expect("Byzantine native data must never advance a checkpoint");
}

#[test]
fn test_produce_vertex_only_includes_conflict_free_subset() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    engine.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    engine
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();

    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_c = generate_keypair(CurveType::Ed25519).unwrap();

    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 2_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0x1001", 1_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0x1002", 1_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0xbbbb", 1_000_000);
    fund_sender_with_coin(&engine, &sender.address, "0x2001", 1_000_000);

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
        &sender,
        &recipient_c.address,
        "0xbbbb",
        1_000_000,
        "0x2001",
        1_000_000,
        3,
    );

    engine
        .submit_transactions_batch(vec![tx_c, tx_b, tx_a])
        .unwrap();

    let info = engine.produce_checkpoint().unwrap();
    let committed_hashes = info
        .vertex
        .expect("vertex should be present")
        .transactions
        .iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();

    assert_eq!(committed_hashes.len(), 2);
    assert_eq!(engine.pending_transaction_len(), 3);
}

#[test]
fn test_produce_vertex_drains_conflicting_transactions_across_rounds() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    engine.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    engine
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();

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

    let first = engine.produce_checkpoint().unwrap();
    assert_eq!(first.tx_count, 1);
    assert_eq!(engine.pending_transaction_len(), 2);

    let second = engine.produce_checkpoint().unwrap_err();
    assert!(second.to_string().contains("DAG_WAITING"));
}

#[test]
fn test_produce_vertex_committed_set_is_stable_across_submit_order() {
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_c = generate_keypair(CurveType::Ed25519).unwrap();

    let build_engine_with_same_state = || {
        let mut engine = BlockchainEngine::new_in_memory().unwrap();
        engine.set_authorities("0x1".to_string(), authorities.clone());
        let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
        engine
            .set_consensus_signing_key(local_key, public_keys)
            .unwrap();
        fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 2_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x1001", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x1002", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0xbbbb", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x2001", 1_000_000);
        engine
    };

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
        &sender,
        &recipient_c.address,
        "0xbbbb",
        1_000_000,
        "0x2001",
        1_000_000,
        3,
    );

    let engine_a = build_engine_with_same_state();
    engine_a
        .submit_transactions_batch(vec![tx_c.clone(), tx_b.clone(), tx_a.clone()])
        .unwrap();
    let committed_a = engine_a
        .produce_checkpoint()
        .unwrap()
        .vertex
        .expect("vertex should be present")
        .transactions
        .iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();

    let engine_b = build_engine_with_same_state();
    engine_b
        .submit_transactions_batch(vec![tx_b, tx_a, tx_c])
        .unwrap();
    let committed_b = engine_b
        .produce_checkpoint()
        .unwrap()
        .vertex
        .expect("vertex should be present")
        .transactions
        .iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();

    assert_eq!(committed_a, committed_b);
}

#[test]
fn test_produce_vertex_allows_empty_mempool_for_consensus_liveness() {
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
}

#[test]
fn test_idle_producer_runs_periodic_mysticeti_cleanup() {
    let (_engine, dag_engine, _) = build_test_dag_engine(vec!["auth1".to_string()], "auth1");
    {
        let mut state = lock_write(&dag_engine.state);
        state.mysticeti.last_cleanup = Instant::now() - Duration::from_secs(11);
    }

    dag_engine.produce_vertex().unwrap();

    let state = lock_read(&dag_engine.state);
    assert!(state.mysticeti.last_cleanup.elapsed() < Duration::from_secs(1));
}

#[test]
fn test_latest_own_vertices_returns_tail_not_genesis_rounds() {
    let (_engine, dag_engine, _) = build_test_dag_engine(vec!["auth1".to_string()], "auth1");

    for _ in 0..20 {
        dag_engine.produce_vertex().unwrap();
    }

    let latest = dag_engine.latest_own_vertices(3).unwrap();
    assert_eq!(latest.len(), 3);
    assert!(latest[0].round > 1, "must not return the oldest blocks");
    assert!(latest.windows(2).all(|pair| pair[0].round < pair[1].round));

    let highest_round = dag_engine
        .vertices_for_sync(usize::MAX)
        .unwrap()
        .iter()
        .map(|vertex| vertex.round)
        .max()
        .unwrap();
    assert_eq!(latest.last().unwrap().round, highest_round);
}

#[test]
fn test_vertices_for_sync_scans_retained_store_not_fixed_round_window() {
    let (_engine, dag_engine, _) = build_test_dag_engine(vec!["auth1".to_string()], "auth1");

    let old_vertex = dag_engine
        .produce_vertex()
        .unwrap()
        .vertex
        .expect("single-authority test node should produce a vertex");

    for _ in 0..320 {
        dag_engine.produce_vertex().unwrap();
    }

    let vertices = dag_engine.vertices_for_sync(usize::MAX).unwrap();
    assert!(
        vertices.iter().any(|vertex| vertex.id == old_vertex.id),
        "retained vertices must not disappear behind a fixed round window"
    );

    let none = dag_engine.vertices_for_sync(0).unwrap();
    assert!(none.is_empty());
}

#[test]
fn test_checkpoint_sync_finds_roots_across_retained_mysticeti_store() {
    let (_engine, dag_engine, _) = build_test_dag_engine(vec!["auth1".to_string()], "auth1");

    let old_root = dag_engine
        .produce_vertex()
        .unwrap()
        .vertex
        .expect("single-authority test node should produce a vertex")
        .id;

    for _ in 0..320 {
        dag_engine.produce_vertex().unwrap();
    }

    let evidence = dag_engine
        .checkpoint_vertices_for_sync(&[old_root], 1)
        .expect("checkpoint root still retained in Mysticeti should be servable");

    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].id, old_root);
}

#[test]
fn test_checkpoint_sync_reports_only_missing_parent_rounds() {
    let (_engine, dag_engine, _) = build_test_dag_engine(vec!["auth1".to_string()], "auth1");
    let existing = dag_engine
        .produce_vertex()
        .unwrap()
        .vertex
        .expect("single-authority node should produce a vertex");
    let mut buffered = existing.clone();
    buffered.parents = vec![
        ("auth1".to_string(), existing.round, existing.id),
        ("auth1".to_string(), 77, [0x77; 32]),
        ("auth1".to_string(), 78, [0x78; 32]),
    ];

    assert_eq!(
        dag_engine.missing_parent_rounds_for_sync(&[buffered]),
        vec![77, 78]
    );
}

#[test]
fn test_produce_vertex_burst_drains_conflicts_while_preserving_independent_throughput() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    engine.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    engine
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();

    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let conflict_recipients = (0..4)
        .map(|_| generate_keypair(CurveType::Ed25519).unwrap())
        .collect::<Vec<_>>();
    let independent_recipients = (0..3)
        .map(|_| generate_keypair(CurveType::Ed25519).unwrap())
        .collect::<Vec<_>>();

    fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 4_000_000);
    for gas_idx in 0..4 {
        fund_sender_with_coin(
            &engine,
            &sender.address,
            &format!("0x10{:02}", gas_idx),
            1_000_000,
        );
    }

    for independent_idx in 0..3 {
        fund_sender_with_coin(
            &engine,
            &sender.address,
            &format!("0xbb{:02}", independent_idx),
            1_000_000,
        );
        fund_sender_with_coin(
            &engine,
            &sender.address,
            &format!("0x20{:02}", independent_idx),
            1_000_000,
        );
    }

    let mut submitted = Vec::new();
    for (idx, recipient) in conflict_recipients.iter().enumerate() {
        submitted.push(signed_transfer_with_refs(
            &sender,
            &recipient.address,
            "0xaaaa",
            4_000_000,
            &format!("0x10{:02}", idx),
            1_000_000,
            (idx + 1) as u64,
        ));
    }
    for (idx, recipient) in independent_recipients.iter().enumerate() {
        submitted.push(signed_transfer_with_refs(
            &sender,
            &recipient.address,
            &format!("0xbb{:02}", idx),
            1_000_000,
            &format!("0x20{:02}", idx),
            1_000_000,
            (idx + 100) as u64,
        ));
    }

    engine.submit_transactions_batch(submitted).unwrap();

    let first = engine.produce_checkpoint().unwrap();
    assert_eq!(
        first.tx_count, 4,
        "1 conflicting + 3 independent should fit in round 1"
    );
    assert_eq!(engine.pending_transaction_len(), 7);
    assert!(
        engine
            .produce_checkpoint()
            .unwrap_err()
            .to_string()
            .contains("DAG_WAITING")
    );
}

#[test]
#[ignore = "superseded by distinct-authority native-block consensus integration test"]
fn test_multi_node_same_root_and_committed_set_across_conflict_burst() {
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let conflict_recipients = (0..4)
        .map(|_| generate_keypair(CurveType::Ed25519).unwrap())
        .collect::<Vec<_>>();
    let independent_recipients = (0..3)
        .map(|_| generate_keypair(CurveType::Ed25519).unwrap())
        .collect::<Vec<_>>();

    let build_engine = || {
        let mut engine = BlockchainEngine::new_in_memory().unwrap();
        engine.set_authorities("0x1".to_string(), authorities.clone());
        let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
        engine
            .set_consensus_signing_key(local_key, public_keys)
            .unwrap();

        fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 4_000_000);
        for gas_idx in 0..4 {
            fund_sender_with_coin(
                &engine,
                &sender.address,
                &format!("0x10{:02}", gas_idx),
                1_000_000,
            );
        }
        for independent_idx in 0..3 {
            fund_sender_with_coin(
                &engine,
                &sender.address,
                &format!("0xbb{:02}", independent_idx),
                1_000_000,
            );
            fund_sender_with_coin(
                &engine,
                &sender.address,
                &format!("0x20{:02}", independent_idx),
                1_000_000,
            );
        }

        engine
    };

    let mut submitted = Vec::new();
    for (idx, recipient) in conflict_recipients.iter().enumerate() {
        submitted.push(signed_transfer_with_refs(
            &sender,
            &recipient.address,
            "0xaaaa",
            4_000_000,
            &format!("0x10{:02}", idx),
            1_000_000,
            (idx + 1) as u64,
        ));
    }
    for (idx, recipient) in independent_recipients.iter().enumerate() {
        submitted.push(signed_transfer_with_refs(
            &sender,
            &recipient.address,
            &format!("0xbb{:02}", idx),
            1_000_000,
            &format!("0x20{:02}", idx),
            1_000_000,
            (idx + 100) as u64,
        ));
    }

    let engine_a = build_engine();
    engine_a
        .submit_transactions_batch(submitted.clone())
        .expect("node A should accept shared burst");

    let engine_b = build_engine();
    engine_b
        .submit_transactions_batch(submitted)
        .expect("node B should accept shared burst");

    let mut round = 0usize;
    while engine_a.pending_transaction_len() > 0 || engine_b.pending_transaction_len() > 0 {
        round += 1;
        let info_a = engine_a
            .produce_checkpoint()
            .unwrap_or_else(|e| panic!("node A failed on round {round}: {e}"));
        let info_b = engine_b
            .produce_checkpoint()
            .unwrap_or_else(|e| panic!("node B failed on round {round}: {e}"));

        let hashes_a = info_a
            .vertex
            .as_ref()
            .expect("node A vertex should be present")
            .transactions
            .iter()
            .map(|tx| tx.transaction_hash().to_vec())
            .collect::<Vec<_>>();
        let hashes_b = info_b
            .vertex
            .as_ref()
            .expect("node B vertex should be present")
            .transactions
            .iter()
            .map(|tx| tx.transaction_hash().to_vec())
            .collect::<Vec<_>>();
        assert_eq!(
            hashes_a, hashes_b,
            "committed tx set diverged at round {round}"
        );

        let root_a = {
            let chain = engine_a
                .blockchain
                .read()
                .unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().state_root.clone()
        };
        let root_b = {
            let chain = engine_b
                .blockchain
                .read()
                .unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().state_root.clone()
        };
        assert_eq!(root_a, root_b, "state root diverged at round {round}");
    }

    assert_eq!(engine_a.pending_transaction_len(), 0);
    assert_eq!(engine_b.pending_transaction_len(), 0);
}

#[test]
fn test_multi_node_ignores_noncanonical_index_and_cache_drift() {
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();

    let build_engine = || {
        let mut engine = BlockchainEngine::new_in_memory().unwrap();
        engine.set_authorities("0x1".to_string(), authorities.clone());
        let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
        engine
            .set_consensus_signing_key(local_key, public_keys)
            .unwrap();
        fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 2_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x1001", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0xbbbb", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x2001", 1_000_000);
        engine
    };

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
        "0xbbbb",
        1_000_000,
        "0x2001",
        1_000_000,
        2,
    );

    let engine_a = build_engine();
    let engine_b = build_engine();
    poison_noncanonical_indexes(&engine_b, &sender.address);

    engine_a
        .submit_transactions_batch(vec![tx_a.clone(), tx_b.clone()])
        .unwrap();
    engine_b
        .submit_transactions_batch(vec![tx_a, tx_b])
        .unwrap();

    let info_a = engine_a.produce_checkpoint().unwrap();
    let info_b = engine_b.produce_checkpoint().unwrap();

    let hashes_a = info_a
        .vertex
        .as_ref()
        .expect("node A vertex should be present")
        .transactions
        .iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();
    let hashes_b = info_b
        .vertex
        .as_ref()
        .expect("node B vertex should be present")
        .transactions
        .iter()
        .map(|tx| tx.transaction_hash().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(hashes_a, hashes_b);

    let root_a = {
        let chain = engine_a
            .blockchain
            .read()
            .unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().state_root.clone()
    };
    let root_b = {
        let chain = engine_b
            .blockchain
            .read()
            .unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().state_root.clone()
    };
    assert_eq!(root_a, root_b);
    assert!(
        first_canonical_snapshot_divergence(&engine_a, &engine_b).is_none(),
        "canonical snapshot should remain identical when only non-canonical indexes drift"
    );
}

#[test]
fn test_multi_node_same_committed_set_implies_same_canonical_snapshot() {
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_a = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_b = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient_c = generate_keypair(CurveType::Ed25519).unwrap();

    let build_engine = || {
        let mut engine = BlockchainEngine::new_in_memory().unwrap();
        engine.set_authorities("0x1".to_string(), authorities.clone());
        let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
        engine
            .set_consensus_signing_key(local_key, public_keys)
            .unwrap();
        fund_sender_with_coin(&engine, &sender.address, "0xaaaa", 3_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x1001", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0xbbbb", 2_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x2001", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0xcccc", 1_000_000);
        fund_sender_with_coin(&engine, &sender.address, "0x3001", 1_000_000);
        engine
    };

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
        "0xbbbb",
        2_000_000,
        "0x2001",
        1_000_000,
        2,
    );
    let tx_c = signed_transfer_with_refs(
        &sender,
        &recipient_c.address,
        "0xcccc",
        1_000_000,
        "0x3001",
        1_000_000,
        3,
    );

    let engine_a = build_engine();
    let engine_b = build_engine();

    engine_a
        .submit_transactions_batch(vec![tx_a.clone(), tx_b.clone(), tx_c.clone()])
        .unwrap();
    engine_b
        .submit_transactions_batch(vec![tx_c, tx_a, tx_b])
        .unwrap();

    let info_a = engine_a.produce_checkpoint().unwrap();
    let info_b = engine_b.produce_checkpoint().unwrap();

    let hashes_a = info_a
        .vertex
        .as_ref()
        .expect("node A vertex should be present")
        .transactions
        .iter()
        .map(|tx| hex::encode(tx.transaction_hash()))
        .collect::<Vec<_>>();
    let hashes_b = info_b
        .vertex
        .as_ref()
        .expect("node B vertex should be present")
        .transactions
        .iter()
        .map(|tx| hex::encode(tx.transaction_hash()))
        .collect::<Vec<_>>();
    assert_eq!(
        hashes_a, hashes_b,
        "committed set should match before state comparison"
    );

    let divergence = first_canonical_snapshot_divergence(&engine_a, &engine_b);
    assert!(
        divergence.is_none(),
        "canonical snapshot diverged despite identical committed set: {}",
        divergence.unwrap_or_default()
    );
}
