// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::transaction::{ObjectRef, Transaction};

fn test_tx(nonce: u64) -> SignedTransaction {
    SignedTransaction::new(Transaction::new_transfer_with_object_ref(
        "0x1".to_string(),
        ObjectRef::new("0xaaaa", Some(1), Some("0xtestdigest".to_string())),
        "0x2".to_string(),
        1,
        nonce,
    ))
}

#[test]
fn transaction_count_uses_monotonic_lifetime_counter() {
    let mut chain = Blockchain::new();
    let checkpoint = Checkpoint::new(
        1,
        vec![[1u8; 32]],
        vec![test_tx(0), test_tx(1)],
        vec![2u8; 32],
        1,
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash"),
    );
    chain
        .add_checkpoint_with_validation(checkpoint, false)
        .invariant("test operation");
    chain.total_transaction_count = 99;

    assert_eq!(chain.get_transaction_count(), 99);
}

#[test]
fn transaction_count_survives_checkpoint_retention_and_index_rebuild() {
    let mut chain = Blockchain::new();
    for sequence in 1..=1_005 {
        let transactions = match sequence {
            1 => vec![test_tx(1)],
            2 => vec![test_tx(2)],
            _ => Vec::new(),
        };
        let checkpoint = Checkpoint::new(
            sequence,
            vec![[sequence as u8; 32]],
            transactions,
            vec![sequence as u8; 32],
            sequence,
            chain
                .latest_checkpoint()
                .hash()
                .invariant("checkpoint hash"),
        );
        chain
            .add_checkpoint_with_validation(checkpoint, false)
            .invariant("test operation");
    }

    assert_eq!(chain.dag_checkpoints.len(), 1_000);
    assert_eq!(chain.dag_checkpoints.front().unwrap().sequence, 0);
    assert_eq!(chain.retained_transaction_count(), 0);
    assert_eq!(chain.get_transaction_count(), 2);

    let encoded = serde_json::to_vec(&chain).invariant("serialize blockchain");
    let mut restarted: Blockchain =
        serde_json::from_slice(&encoded).invariant("deserialize blockchain");
    restarted.rebuild_tx_hash_index();

    assert_eq!(restarted.get_transaction_count(), 2);
}

#[test]
fn legacy_snapshot_missing_genesis_is_repaired_without_growing_retention_window() {
    let mut chain = Blockchain::new();
    for sequence in 1..=1_000 {
        let checkpoint = Checkpoint::new(
            sequence,
            vec![[sequence as u8; 32]],
            Vec::new(),
            vec![sequence as u8; 32],
            sequence,
            chain
                .latest_checkpoint()
                .hash()
                .invariant("checkpoint hash"),
        );
        chain
            .add_checkpoint_with_validation(checkpoint, false)
            .invariant("test operation");
    }

    chain.dag_checkpoints.pop_front();
    let checkpoint = Checkpoint::new(
        1_001,
        vec![[233u8; 32]],
        Vec::new(),
        vec![233u8; 32],
        1_001,
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash"),
    );
    chain
        .add_checkpoint_with_validation(checkpoint, false)
        .invariant("test operation");
    assert_eq!(chain.dag_checkpoints.len(), 1_000);
    assert_ne!(chain.dag_checkpoints.front().unwrap().sequence, 0);

    assert!(chain.ensure_genesis_retained());
    assert_eq!(chain.dag_checkpoints.len(), 1_000);
    assert_eq!(chain.dag_checkpoints.front().unwrap().sequence, 0);
    assert!(!chain.ensure_genesis_retained());
}

#[test]
fn checkpoint_validation_rejects_non_consecutive_sequence() {
    let mut chain = Blockchain::new();
    let checkpoint = Checkpoint::new(
        2,
        vec![[1u8; 32]],
        vec![test_tx(0)],
        vec![2u8; 32],
        1,
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash"),
    );

    let err = chain
        .add_checkpoint_with_validation(checkpoint, true)
        .unwrap_err();

    assert!(
        err.to_string().contains("Invalid checkpoint sequence"),
        "unexpected error: {err}"
    );
}

#[test]
fn checkpoint_validation_rejects_wrong_previous_hash() {
    let mut chain = Blockchain::new();
    let checkpoint = Checkpoint::new(
        1,
        vec![[1u8; 32]],
        vec![test_tx(0)],
        vec![2u8; 32],
        1,
        vec![9u8; 32],
    );

    let err = chain
        .add_checkpoint_with_validation(checkpoint, true)
        .unwrap_err();

    assert!(
        err.to_string().contains("Invalid previous checkpoint hash"),
        "unexpected error: {err}"
    );
}

#[test]
fn checkpoint_validation_rejects_duplicate_transactions_within_checkpoint() {
    let mut chain = Blockchain::new();
    let tx = test_tx(0);
    let checkpoint = Checkpoint::new(
        1,
        vec![[1u8; 32]],
        vec![tx.clone(), tx],
        vec![2u8; 32],
        1,
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash"),
    );

    let err = chain
        .add_checkpoint_with_validation(checkpoint, true)
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("Duplicate transaction found within checkpoint"),
        "unexpected error: {err}"
    );
}

#[test]
fn checkpoint_validation_rejects_replayed_transaction_from_prior_checkpoint() {
    let mut chain = Blockchain::new();
    let tx = test_tx(0);
    let checkpoint = Checkpoint::new(
        1,
        vec![[1u8; 32]],
        vec![tx.clone()],
        vec![2u8; 32],
        1,
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash"),
    );
    chain
        .add_checkpoint_with_validation(checkpoint, true)
        .invariant("test checkpoint must commit");

    let replay_checkpoint = Checkpoint::new(
        2,
        vec![[3u8; 32]],
        vec![tx],
        vec![4u8; 32],
        2,
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash"),
    );

    let err = chain
        .add_checkpoint_with_validation(replay_checkpoint, true)
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("Replay attack detected: Transaction already executed"),
        "unexpected error: {err}"
    );
}
