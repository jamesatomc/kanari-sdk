use super::*;
use crate::{CheckpointSyncData, consensus::Checkpoint};
use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_types::error::KanariUnwrapExt;
use kanari_types::transaction::{SignedTransaction, Transaction};

fn signed_transfer(_sequence_number: u64) -> SignedTransaction {
    let sender = generate_keypair(CurveType::Ed25519).invariant("ed25519 keypair");
    let recipient = generate_keypair(CurveType::Ed25519).invariant("ed25519 keypair");
    let tx = Transaction::new_transfer(
        sender.tagged_address(),
        recipient.address,
        1,
    );
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .invariant("test operation");
    signed_tx
}

#[test]
fn sync_checkpoint_from_data_rejects_empty_checkpoint() {
    let engine = BlockchainEngine::new_in_memory().invariant("in-memory engine");
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash")
    };
    let state_root = engine
        .state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();
    let checkpoint = Checkpoint::new(1, vec![], vec![], state_root, 42, prev_hash);
    let sync_data = CheckpointSyncData { checkpoint };

    let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Refusing to sync empty checkpoint")
    );
    assert_eq!(engine.get_stats().height, 0);
}

#[test]
fn sync_checkpoint_from_data_rejects_root_mismatch() {
    let engine = BlockchainEngine::new_in_memory().invariant("in-memory engine");
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash")
    };
    let signed_tx = signed_transfer(0);
    let checkpoint = Checkpoint::new(1, vec![], vec![signed_tx], vec![9u8; 32], 42, prev_hash);
    let sync_data = CheckpointSyncData { checkpoint };

    let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
    assert!(error.to_string().contains("state root mismatch"));
    assert_eq!(engine.get_stats().height, 0);
}
