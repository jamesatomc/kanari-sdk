// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_core::blockchain::{Block, Blockchain};

#[test]
fn test_fork_detection_and_resolution() {
    let mut blockchain = Blockchain::new();

    // Build main chain: Genesis -> Block 1 -> Block 2
    let block1 = Block::new(
        1,
        blockchain.latest_block().hash(),
        vec![0u8; 32],
        vec![],
        vec![],
    );
    blockchain.add_block(block1.clone()).unwrap();

    let block2 = Block::new(
        2,
        blockchain.latest_block().hash(),
        vec![0u8; 32],
        vec![],
        vec![],
    );
    blockchain.add_block(block2.clone()).unwrap();

    assert_eq!(blockchain.height(), 2);

    // Create fork: Genesis -> Block 1 -> Block 2' -> Block 3'
    let block2_fork = Block::new(
        2,
        block1.hash(),
        vec![1u8; 32], // Different state root
        vec![],
        vec![],
    );

    let block3_fork = Block::new(
        3,
        block2_fork.hash(),
        vec![2u8; 32],
        vec![],
        vec![],
    );

    // Fork includes common ancestor (block 1) and divergent blocks
    let fork_blocks = vec![
        blockchain.get_block(0).unwrap().clone(), // Genesis
        block1.clone(),                           // Common ancestor
        block2_fork.clone(),                      // Divergent
        block3_fork.clone(),                      // Divergent
    ];
    let reorganized = blockchain.handle_fork(fork_blocks).unwrap();

    assert!(reorganized, "Should have reorganized to longer fork");
    assert_eq!(blockchain.height(), 3, "Height should be 3 after fork");
    assert_eq!(
        blockchain.latest_block().hash(),
        block3_fork.hash(),
        "Latest block should be from fork"
    );

    // Old chain should be stored in forks
    assert_eq!(blockchain.get_forks().len(), 1, "Should have 1 fork stored");
}

#[test]
fn test_fork_rejection_when_shorter() {
    let mut blockchain = Blockchain::new();

    // Build main chain: Genesis -> Block 1 -> Block 2 -> Block 3
    let block1 = Block::new(
        1,
        blockchain.latest_block().hash(),
        vec![0u8; 32],
        vec![],
        vec![],
    );
    blockchain.add_block(block1.clone()).unwrap();

    let block2 = Block::new(
        2,
        blockchain.latest_block().hash(),
        vec![0u8; 32],
        vec![],
        vec![],
    );
    blockchain.add_block(block2.clone()).unwrap();

    let block3 = Block::new(
        3,
        blockchain.latest_block().hash(),
        vec![0u8; 32],
        vec![],
        vec![],
    );
    blockchain.add_block(block3).unwrap();

    assert_eq!(blockchain.height(), 3);

    // Create shorter fork: Genesis -> Block 1 -> Block 2'
    let block2_fork = Block::new(
        2,
        block1.hash(),
        vec![1u8; 32], // Different state root
        vec![],
        vec![],
    );

    // Fork includes common blocks
    let fork_blocks = vec![
        blockchain.get_block(0).unwrap().clone(), // Genesis
        block1.clone(),                           // Common
        block2_fork,                              // Divergent
    ];
    let reorganized = blockchain.handle_fork(fork_blocks).unwrap();

    assert!(
        !reorganized,
        "Should NOT reorganize to shorter fork"
    );
    assert_eq!(blockchain.height(), 3, "Height should still be 3");

    // Shorter fork should be stored
    assert_eq!(blockchain.get_forks().len(), 1, "Should have 1 fork stored");
}

#[test]
fn test_prune_old_forks() {
    let mut blockchain = Blockchain::new();

    // Build main chain to height 10
    for i in 1..=10 {
        let block = Block::new(
            i,
            blockchain.latest_block().hash(),
            vec![0u8; 32],
            vec![],
            vec![],
        );
        blockchain.add_block(block).unwrap();
    }

    // Create old fork at height 2
    let block1 = blockchain.get_block(1).unwrap();
    let old_fork = vec![Block::new(
        2,
        block1.hash(),
        vec![1u8; 32],
        vec![],
        vec![],
    )];
    blockchain.handle_fork(old_fork).unwrap();

    // Create recent fork at height 9
    let block8 = blockchain.get_block(8).unwrap();
    let recent_fork = vec![Block::new(
        9,
        block8.hash(),
        vec![2u8; 32],
        vec![],
        vec![],
    )];
    blockchain.handle_fork(recent_fork).unwrap();

    assert_eq!(blockchain.get_forks().len(), 2, "Should have 2 forks");

    // Prune forks older than 5 blocks
    blockchain.prune_forks(5);

    // Only recent fork should remain
    assert_eq!(
        blockchain.get_forks().len(),
        1,
        "Should have 1 fork after pruning"
    );
}

#[test]
fn test_canonical_chain() {
    let mut blockchain = Blockchain::new();

    // Build chain
    for i in 1..=5 {
        let block = Block::new(
            i,
            blockchain.latest_block().hash(),
            vec![0u8; 32],
            vec![],
            vec![],
        );
        blockchain.add_block(block).unwrap();
    }

    let canonical = blockchain.get_canonical_chain();

    assert_eq!(canonical.len(), 6, "Should have 6 blocks (including genesis)");
    assert_eq!(canonical[0].header.height, 0, "First block should be genesis");
    assert_eq!(canonical[5].header.height, 5, "Last block should be height 5");
}
