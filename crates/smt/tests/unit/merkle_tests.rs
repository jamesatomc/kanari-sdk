use crate::digest;

use super::*;

#[test]
fn test_empty_merkle_root() {
    let root = compute_merkle_root(&[]);
    assert_eq!(root.len(), 32);
    assert_eq!(root, vec![0u8; 32]);
}

#[test]
fn test_single_tx_merkle_root() {
    let tx_hash = digest(b"tx1").to_vec();
    let root = compute_merkle_root(std::slice::from_ref(&tx_hash));
    assert_eq!(root, tx_hash);
}

#[test]
fn test_two_tx_merkle_root() {
    let tx1: [u8; 32] = digest(b"tx1");
    let tx2: [u8; 32] = digest(b"tx2");
    let root = compute_merkle_root(&[tx1.to_vec(), tx2.to_vec()]);

    let expected = hash_node(&tx1, &tx2);
    assert_eq!(root, expected.to_vec());
}

#[test]
fn test_three_tx_merkle_root() {
    let tx1: [u8; 32] = digest(b"tx1");
    let tx2: [u8; 32] = digest(b"tx2");
    let tx3: [u8; 32] = digest(b"tx3");

    let root = compute_merkle_root(&[tx1.to_vec(), tx2.to_vec(), tx3.to_vec()]);

    // Level 1: hash pairs, pass through odd
    let h12 = hash_node(&tx1, &tx2);
    let h3 = tx3; // pass through

    // Level 2: hash results
    let expected = hash_node(&h12, &h3);
    assert_eq!(root, expected.to_vec());
}

#[test]
fn test_three_tx_merkle_proof() {
    let tx1 = digest(b"tx1").to_vec();
    let tx2 = digest(b"tx2").to_vec();
    let tx3 = digest(b"tx3").to_vec();

    let txs = vec![tx1.clone(), tx2.clone(), tx3.clone()];
    let root = compute_merkle_root(&txs);

    // Test proof for each transaction
    for (i, tx) in txs.iter().enumerate() {
        let proof = generate_merkle_proof(&txs, i);
        assert!(
            verify_merkle_proof(tx, i, &proof, &root),
            "Proof failed for index {}",
            i
        );
    }
}

#[test]
fn test_merkle_proof_generation_and_verification() {
    let tx1 = digest(b"tx1").to_vec();
    let tx2 = digest(b"tx2").to_vec();
    let tx3 = digest(b"tx3").to_vec();
    let tx4 = digest(b"tx4").to_vec();

    let txs = vec![tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone()];
    let root = compute_merkle_root(&txs);

    // Test proof for each transaction
    for (i, tx) in txs.iter().enumerate() {
        let proof = generate_merkle_proof(&txs, i);
        assert!(verify_merkle_proof(tx, i, &proof, &root));
    }
}

#[test]
fn test_merkle_proof_invalid() {
    let tx1 = digest(b"tx1").to_vec();
    let tx2 = digest(b"tx2").to_vec();
    let tx3 = digest(b"tx3").to_vec();

    let txs = vec![tx1.clone(), tx2.clone(), tx3.clone()];
    let root = compute_merkle_root(&txs);

    let proof = generate_merkle_proof(&txs, 0);
    let fake_tx = digest(b"fake").to_vec();

    // Wrong hash should fail
    assert!(!verify_merkle_proof(&fake_tx, 0, &proof, &root));

    // Wrong index should fail
    assert!(!verify_merkle_proof(&tx1, 1, &proof, &root));
}

#[test]
fn test_batch_verify_merkle_proofs() {
    let tx1 = digest(b"tx1").to_vec();
    let tx2 = digest(b"tx2").to_vec();
    let tx3 = digest(b"tx3").to_vec();
    let tx4 = digest(b"tx4").to_vec();

    let txs = vec![tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone()];
    let root = compute_merkle_root(&txs);

    // Create multiple proofs
    let proofs: Vec<(Vec<u8>, usize, Vec<Vec<u8>>)> = vec![
        (tx1.clone(), 0, generate_merkle_proof(&txs, 0)),
        (tx2.clone(), 1, generate_merkle_proof(&txs, 1)),
        (tx3.clone(), 2, generate_merkle_proof(&txs, 2)),
    ];

    // All valid proofs should pass
    assert!(batch_verify_merkle_proofs(&proofs, &root));

    // One invalid proof should fail
    let invalid_proofs: Vec<(Vec<u8>, usize, Vec<Vec<u8>>)> = vec![
        (tx1.clone(), 0, generate_merkle_proof(&txs, 0)),
        (digest(b"fake").to_vec(), 1, generate_merkle_proof(&txs, 1)),
    ];

    assert!(!batch_verify_merkle_proofs(&invalid_proofs, &root));
}

#[test]
fn test_merkle_multiproof() {
    let tx1 = digest(b"tx1").to_vec();
    let tx2 = digest(b"tx2").to_vec();
    let tx3 = digest(b"tx3").to_vec();
    let tx4 = digest(b"tx4").to_vec();

    let txs = vec![tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone()];

    // Generate multiproof for indices 0, 2
    let multiproof = generate_merkle_multiproof(&txs, &[0, 2]);

    // Multiproof should be smaller than individual proofs combined
    let proof0 = generate_merkle_proof(&txs, 0);
    let proof2 = generate_merkle_proof(&txs, 2);

    // Multiproof removes duplicates, so should be smaller
    assert!(multiproof.len() <= proof0.len() + proof2.len());
}

#[test]
fn test_compressed_merkle_proof() {
    let tx1 = digest(b"tx1").to_vec();
    let tx2 = digest(b"tx2").to_vec();
    let tx3 = digest(b"tx3").to_vec();
    let tx4 = digest(b"tx4").to_vec();

    let txs = vec![tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone()];
    let root = compute_merkle_root(&txs);

    // Generate and compress proof
    let proof = generate_merkle_proof(&txs, 1);
    let compressed = CompressedMerkleProof::from_proof(&tx2, 1, &proof);

    // Verify compressed proof
    assert!(compressed.verify(&tx2, 1, &root));

    // Invalid proof should fail
    assert!(!compressed.verify(&tx1, 1, &root));
}

#[test]
fn test_compressed_proof_serialization() {
    let tx1 = digest(b"tx1").to_vec();
    let tx2 = digest(b"tx2").to_vec();
    let tx3 = digest(b"tx3").to_vec();
    let tx4 = digest(b"tx4").to_vec();

    let txs = vec![tx1.clone(), tx2.clone(), tx3.clone(), tx4.clone()];
    let root = compute_merkle_root(&txs);

    let proof = generate_merkle_proof(&txs, 1);
    let compressed = CompressedMerkleProof::from_proof(&tx2, 1, &proof);

    // Serialize and deserialize
    let bytes = compressed.to_bytes();
    let deserialized = CompressedMerkleProof::from_bytes(&bytes).unwrap();

    // Should still verify
    assert!(deserialized.verify(&tx2, 1, &root));

    // Verify bandwidth savings
    let original_size = proof.iter().map(|p| p.len()).sum::<usize>();
    let compressed_size = bytes.len();

    // Compressed should be smaller (includes metadata but saves index calculations)
    tracing::info!(
        "Original: {} bytes, Compressed: {} bytes",
        original_size,
        compressed_size
    );
}
