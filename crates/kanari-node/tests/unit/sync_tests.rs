use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::*;
use kanari_core::Checkpoint;
use libp2p::identity::Keypair;
use tokio::sync::mpsc;

fn new_sync_manager() -> SyncManager {
    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let (network_tx, _network_rx) = mpsc::channel(128);
    SyncManager::new(engine, network_tx, "local-peer".to_string(), None)
}

fn peer_info(height: u64, checkpoint: &str, state_root: &str) -> PeerInfoMsg {
    PeerInfoMsg {
        height,
        peer_id: "peer-1".to_string(),
        timestamp: 1,
        latest_checkpoint_hash: checkpoint.to_string(),
        latest_state_root: state_root.to_string(),
        total_transactions: 0,
    }
}

#[tokio::test]
async fn authenticated_source_rejects_spoofed_peer_info_identity() {
    let sync = new_sync_manager();
    let source = Keypair::generate_ed25519().public().to_peer_id();
    let spoofed = PeerInfoMsg {
        height: 10_000,
        peer_id: Keypair::generate_ed25519()
            .public()
            .to_peer_id()
            .to_string(),
        timestamp: 1,
        latest_checkpoint_hash: "spoofed".to_string(),
        latest_state_root: "spoofed".to_string(),
        total_transactions: 0,
    };

    sync.handle_message(AuthenticatedP2PMessage {
        source,
        message: P2PMessage::PeerInfo(spoofed),
    })
    .await;

    assert!(sync.peer_heights_guard().is_empty());
    assert!(sync.divergent_peers_guard().is_empty());
    assert!(sync.pending_checkpoint_requests_guard().is_empty());
}

fn test_dag_vertex(round: u64, author: &str) -> DagVertex {
    let mut vertex = DagVertex::new(
        round,
        author.to_string(),
        "kanari-v2-mysticeti".to_string(),
        Vec::<(String, u64, [u8; 32])>::new(),
        vec![],
        round,
    );
    let mut id = [0u8; 32];
    id[..8].copy_from_slice(&round.to_le_bytes());
    vertex.id = id;
    vertex
}

fn apply_empty_checkpoint(engine: &BlockchainEngine, sequence: u64) {
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };
    let state_root = engine
        .state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();
    let checkpoint = Checkpoint::new(sequence, vec![], vec![], state_root, sequence, prev_hash);
    engine.apply_checkpoint(checkpoint).unwrap();
}

#[test]
fn test_retry_cooldown_throttles_rapid_duplicate_checkpoint_request() {
    let sync = new_sync_manager();
    assert!(sync.should_request_checkpoint_sequence(7, 1_000));
    assert!(!sync.should_request_checkpoint_sequence(7, 1_500));
    assert!(sync.should_request_checkpoint_sequence(7, 3_500));
}

#[test]
fn test_dag_vertex_buffer_deduplicates_by_vertex_id() {
    let sync = new_sync_manager();
    let vertex = test_dag_vertex(10, "peer-a");

    sync.buffer_dag_vertex(vertex.clone(), "missing parent");
    sync.buffer_dag_vertex(vertex, "missing parent");

    let buffer = sync.dag_vertex_buffer_guard();
    assert_eq!(buffer.len(), 1);
}

#[test]
fn test_dag_vertex_buffer_evicts_oldest_at_limit() {
    let mut sync = new_sync_manager();
    sync.max_dag_vertex_buffer_size = 2;

    let first = test_dag_vertex(10, "peer-a");
    let second = test_dag_vertex(11, "peer-b");
    let third = test_dag_vertex(12, "peer-c");
    let first_id = first.id;

    sync.buffer_dag_vertex(first, "missing parent");
    sync.buffer_dag_vertex(second, "missing parent");
    sync.buffer_dag_vertex(third, "missing parent");

    let buffer = sync.dag_vertex_buffer_guard();
    assert_eq!(buffer.len(), 2);
    assert!(!buffer.iter().any(|vertex| vertex.id == first_id));
}

#[tokio::test]
async fn test_divergent_peer_is_quarantined_from_sync_targets() {
    let sync = new_sync_manager();
    let stats = sync.engine.get_stats();
    let local_checkpoint_hash = sync.engine.latest_checkpoint_hash_hex();

    sync.handle_peer_info(peer_info(stats.height, &local_checkpoint_hash, "deadbeef"))
        .await;

    assert!(sync.is_peer_divergent("peer-1"));
    assert_eq!(sync.best_peer_for_height(stats.height), None);
    assert_eq!(sync.max_eligible_peer_height(), 0);
}

#[tokio::test]
async fn test_peer_with_same_height_but_different_root_is_never_selected() {
    let sync = new_sync_manager();
    let stats = sync.engine.get_stats();
    let local_checkpoint_hash = sync.engine.latest_checkpoint_hash_hex();
    let local_root = sync.engine.latest_checkpoint_state_root_hex();

    sync.handle_peer_info(peer_info(
        stats.height,
        &local_checkpoint_hash,
        "different-runtime-root",
    ))
    .await;

    assert_ne!(local_root, "different-runtime-root");
    assert!(sync.is_peer_divergent("peer-1"));
    assert_eq!(sync.best_peer_for_height(stats.height + 1), None);
    assert_eq!(sync.max_eligible_peer_height(), 0);
}

#[tokio::test]
async fn test_divergent_peer_does_not_block_a_healthy_sync_source() {
    let sync = new_sync_manager();
    let stats = sync.engine.get_stats();
    let local_checkpoint_hash = sync.engine.latest_checkpoint_hash_hex();

    sync.handle_peer_info(peer_info(
        stats.height,
        &local_checkpoint_hash,
        "incompatible-state-root",
    ))
    .await;

    let mut healthy = peer_info(stats.height + 1, "peer-checkpoint", "peer-root");
    healthy.peer_id = "peer-2".to_string();
    sync.handle_peer_info(healthy).await;

    assert!(sync.is_peer_divergent("peer-1"));
    assert_eq!(
        sync.best_peer_for_height(stats.height + 1),
        Some("peer-2".into())
    );
    assert_eq!(sync.max_eligible_peer_height(), stats.height + 1);
}

#[tokio::test]
async fn test_divergent_peer_is_released_only_after_matching_again() {
    let sync = new_sync_manager();
    let stats = sync.engine.get_stats();
    let local_checkpoint_hash = sync.engine.latest_checkpoint_hash_hex();
    let local_state_root = sync.engine.latest_checkpoint_state_root_hex();

    sync.handle_peer_info(peer_info(
        stats.height,
        &local_checkpoint_hash,
        "incompatible-state-root",
    ))
    .await;
    assert!(sync.is_peer_divergent("peer-1"));

    sync.handle_peer_info(peer_info(
        stats.height,
        &local_checkpoint_hash,
        &local_state_root,
    ))
    .await;

    assert!(!sync.is_peer_divergent("peer-1"));
    assert_eq!(
        sync.best_peer_for_height(stats.height),
        Some("peer-1".into())
    );
}

#[tokio::test]
async fn test_divergent_peer_is_released_when_its_tip_becomes_canonical_history() {
    let sync = new_sync_manager();
    let historical_height = sync.engine.get_stats().height;
    let historical_root = sync.engine.latest_checkpoint_state_root_hex();

    sync.divergent_peers_guard().insert(
        "peer-1".to_string(),
        DivergentPeerInfo {
            height: historical_height,
            latest_checkpoint_hash: "equivalent-dag-checkpoint-hash".to_string(),
            latest_state_root: historical_root,
        },
    );
    apply_empty_checkpoint(&sync.engine, historical_height + 1);

    let mut recovered = peer_info(
        historical_height + 2,
        "future-checkpoint",
        "future-state-root",
    );
    recovered.peer_id = "peer-1".to_string();
    sync.handle_peer_info(recovered).await;

    assert!(!sync.is_peer_divergent("peer-1"));
    assert_eq!(
        sync.best_peer_for_height(historical_height + 2),
        Some("peer-1".to_string())
    );
}

#[tokio::test]
async fn test_checkpoint_hash_mismatch_with_same_state_root_is_eligible() {
    let sync = new_sync_manager();
    let stats = sync.engine.get_stats();
    let local_state_root = sync.engine.latest_checkpoint_state_root_hex();

    sync.handle_peer_info(peer_info(
        stats.height,
        "different-checkpoint",
        &local_state_root,
    ))
    .await;

    assert!(!sync.is_peer_divergent("peer-1"));
    assert_eq!(
        sync.best_peer_for_height(stats.height),
        Some("peer-1".to_string())
    );
    assert_eq!(sync.max_eligible_peer_height(), stats.height);
}

#[test]
fn test_buffered_empty_checkpoint_is_not_applied_when_gap_is_filled() {
    let mut source = BlockchainEngine::new_in_memory().unwrap();
    let authority = "0x1".to_string();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
    source.set_authorities(authority.clone(), vec![authority.clone()]);
    source
        .set_consensus_signing_key(
            signing_key.clone(),
            BTreeMap::from([(authority, signing_key.verifying_key().to_bytes().to_vec())]),
        )
        .unwrap();
    let source_engine = Arc::new(source);
    apply_empty_checkpoint(source_engine.as_ref(), 1);
    apply_empty_checkpoint(source_engine.as_ref(), 2);
    let checkpoint_one = source_engine
        .get_checkpoint_sync(1)
        .unwrap()
        .expect("checkpoint one must exist");
    let checkpoint_two = source_engine
        .get_checkpoint_sync(2)
        .unwrap()
        .expect("checkpoint two must exist");

    let engine = Arc::new(BlockchainEngine::new_in_memory().unwrap());
    let (network_tx, _network_rx) = mpsc::channel(128);
    let sync = SyncManager::new(engine.clone(), network_tx, "local-peer".to_string(), None);

    assert!(
        sync.buffer_checkpoint(checkpoint_two, Some("peer-2"), "test")
            .is_some()
    );
    assert_eq!(engine.get_stats().height, 0);
    assert!(
        sync.buffer_checkpoint(checkpoint_one, Some("peer-2"), "test")
            .is_some()
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(sync.try_apply_buffered_checkpoints());

    assert_eq!(engine.get_stats().height, 0);
    assert_eq!(sync.latest_buffered_sequence(), 2);
}

#[test]
fn test_handle_checkpoint_response_keeps_earlier_pending_requests_until_apply() {
    let sync = new_sync_manager();
    {
        let mut pending = sync.pending_checkpoint_requests_guard();
        pending.insert(1, 10);
        pending.insert(2, 20);
        pending.insert(3, 30);
    }

    let prev_hash = {
        let chain = sync
            .engine
            .blockchain
            .read()
            .unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };
    let bogus_checkpoint = CheckpointSyncData {
        checkpoint: Checkpoint::new(3, vec![], vec![], vec![0u8; 32], 3, prev_hash),
        dag_vertices: Vec::new(),
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(sync.handle_checkpoint_response(
        serde_json::to_string(&bogus_checkpoint).unwrap(),
        Some("peer-2"),
    ));

    let pending_heights: BTreeSet<_> = sync
        .pending_checkpoint_requests_guard()
        .keys()
        .copied()
        .collect();
    assert_eq!(pending_heights, BTreeSet::from([1, 2, 3]));
}
