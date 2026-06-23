#![allow(dead_code)]

use kanari_core::{BlockchainEngine, Checkpoint, CheckpointSyncData, DagVertex};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::p2p::PeerInfoMsg;
use crate::sync::SyncManager;

pub fn new_test_engine() -> Arc<BlockchainEngine> {
    Arc::new(BlockchainEngine::new_in_memory().unwrap())
}

pub fn new_sync_manager() -> SyncManager {
    let engine = new_test_engine();
    let (network_tx, _network_rx) = mpsc::unbounded_channel();
    SyncManager::new(engine, network_tx, "local-peer".to_string(), None)
}

pub fn peer_info(height: u64, checkpoint: &str, state_root: &str) -> PeerInfoMsg {
    PeerInfoMsg {
        height,
        peer_id: "peer-1".to_string(),
        timestamp: 1,
        latest_checkpoint_hash: checkpoint.to_string(),
        latest_state_root: state_root.to_string(),
        total_transactions: 0,
    }
}

pub fn test_dag_vertex(round: u64, author: &str) -> DagVertex {
    DagVertex::new(
        round,
        author.to_string(),
        "kanari-test".to_string(),
        vec![],
        vec![],
        vec![0u8; 32],
        round,
    )
}

pub fn apply_empty_checkpoint(engine: &BlockchainEngine, sequence: u64) {
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

pub fn checkpoint_sync(engine: &BlockchainEngine, sequence: u64) -> CheckpointSyncData {
    engine.get_checkpoint_sync(sequence).unwrap()
}
