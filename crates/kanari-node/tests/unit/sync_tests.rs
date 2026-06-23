use std::collections::BTreeSet;

use super::*;
use kanari_core::Checkpoint;

#[path = "test_support.rs"]
mod test_support;

use test_support::{
    apply_empty_checkpoint, checkpoint_sync, new_sync_manager, new_test_engine, peer_info,
    test_dag_vertex,
};

#[test]
fn missing_parent_errors_are_buffered_for_retry() {
    assert!(SyncManager::should_buffer_dag_vertex_error(
        "Missing parent abc for DAG vertex def"
    ));
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
    let source_engine = new_test_engine();
    apply_empty_checkpoint(source_engine.as_ref(), 1);
    apply_empty_checkpoint(source_engine.as_ref(), 2);
    let checkpoint_one = checkpoint_sync(source_engine.as_ref(), 1);
    let checkpoint_two = checkpoint_sync(source_engine.as_ref(), 2);

    let engine = new_test_engine();
    let (network_tx, _network_rx) = mpsc::unbounded_channel();
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
