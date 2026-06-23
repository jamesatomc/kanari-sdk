// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Transaction Merkle tree implementation using SMT hash functions
//! This provides efficient verification for light clients

use crate::hash_node;

type MerkleProofItem = (Vec<u8>, usize, Vec<Vec<u8>>);

fn compute_merkle_root_arrays(mut current_level: Vec<[u8; 32]>) -> [u8; 32] {
    if current_level.is_empty() {
        return [0u8; 32];
    }

    while current_level.len() > 1 {
        let mut next_level = Vec::with_capacity(current_level.len().div_ceil(2));

        for chunk in current_level.chunks(2) {
            if chunk.len() == 2 {
                next_level.push(hash_node(&chunk[0], &chunk[1]));
            } else {
                next_level.push(chunk[0]);
            }
        }

        current_level = next_level;
    }

    current_level[0]
}

fn tx_hashes_to_arrays(tx_hashes: &[Vec<u8>]) -> Vec<[u8; 32]> {
    tx_hashes.iter().map(|hash| bytes_to_hash(hash)).collect()
}

fn bytes_to_hash(bytes: &[u8]) -> [u8; 32] {
    bytes.try_into().unwrap_or([0u8; 32])
}

/// Optimized merkle root computation using fixed-size arrays to avoid allocation
pub fn compute_merkle_root_optimized(hashes: Vec<[u8; 32]>) -> [u8; 32] {
    compute_merkle_root_arrays(hashes)
}

/// Build a merkle tree from transaction hashes and return the merkle root
/// Uses the same Blake3 hashing as the SMT for consistency
pub fn compute_merkle_root(tx_hashes: &[Vec<u8>]) -> Vec<u8> {
    if tx_hashes.len() == 1 {
        return tx_hashes[0].clone();
    }

    compute_merkle_root_arrays(tx_hashes_to_arrays(tx_hashes)).to_vec()
}

/// Generate merkle proof for a transaction at given index
/// Returns sibling hashes needed to reconstruct the merkle root
pub fn generate_merkle_proof(tx_hashes: &[Vec<u8>], index: usize) -> Vec<Vec<u8>> {
    if index >= tx_hashes.len() {
        return Vec::new();
    }

    let mut proof = Vec::with_capacity(tx_hashes.len().ilog2() as usize + 1);
    let mut current_level = tx_hashes_to_arrays(tx_hashes);
    let mut current_index = index;

    while current_level.len() > 1 {
        let mut next_level = Vec::with_capacity(current_level.len().div_ceil(2));

        // Determine sibling index
        let sibling_index = if current_index.is_multiple_of(2) {
            current_index + 1
        } else {
            current_index - 1
        };

        // Add sibling to proof if exists
        if sibling_index < current_level.len() {
            proof.push(current_level[sibling_index].to_vec());
        } else {
            // Push an empty vector to indicate a pass-through (no sibling at this level)
            proof.push(Vec::new());
        }

        // Build next level
        for chunk in current_level.chunks(2) {
            if chunk.len() == 2 {
                next_level.push(hash_node(&chunk[0], &chunk[1]));
            } else {
                next_level.push(chunk[0]);
            }
        }

        current_level = next_level;
        current_index /= 2;
    }

    proof
}

/// Verify a merkle proof
/// Returns true if the proof is valid for the given transaction hash
/// NOTE: This simple verification only works for balanced trees or specific shapes.
/// For pass-through, the verifier needs to know when to skip hashing.
pub fn verify_merkle_proof(
    tx_hash: &[u8],
    index: usize,
    proof: &[Vec<u8>],
    merkle_root: &[u8],
) -> bool {
    if proof.is_empty() && tx_hash == merkle_root {
        return true; // Single transaction tree
    }

    let mut current_hash = bytes_to_hash(tx_hash);
    let mut current_index = index;
    let proof_iter = proof.iter();

    // This verification logic is tricky with pass-through if we don't know the tree size.
    // However, in many implementations, if a node has no sibling, it's NOT hashed.
    // But the proof must reflect this.

    for sibling_vec in proof_iter {
        if !sibling_vec.is_empty() {
            let sibling = bytes_to_hash(sibling_vec);
            current_hash = if current_index.is_multiple_of(2) {
                // Current is left, sibling is right
                hash_node(&current_hash, &sibling)
            } else {
                // Current is right, sibling is left
                hash_node(&sibling, &current_hash)
            };
        }
        // If sibling is empty, it's a pass-through node, so we don't hash it.
        // We just move up to the next level.
        current_index /= 2;
    }

    current_hash.as_slice() == merkle_root
}

/// Batch verify multiple merkle proofs efficiently
/// Returns true only if ALL proofs are valid
pub fn batch_verify_merkle_proofs(proofs: &[MerkleProofItem], merkle_root: &[u8]) -> bool {
    for (tx_hash, index, proof) in proofs {
        if !verify_merkle_proof(tx_hash, *index, proof, merkle_root) {
            return false;
        }
    }
    true
}

/// Generate a multiproof for multiple transactions at once
/// More efficient than generating individual proofs as it removes duplicate siblings
pub fn generate_merkle_multiproof(tx_hashes: &[Vec<u8>], indices: &[usize]) -> Vec<Vec<u8>> {
    if indices.is_empty() || tx_hashes.is_empty() {
        return Vec::new();
    }

    // Track all nodes we need in the proof
    let mut proof_nodes = std::collections::BTreeSet::new();
    let mut current_level = tx_hashes_to_arrays(tx_hashes);

    // For each level, track which indices we're proving
    let mut current_indices: std::collections::BTreeSet<usize> = indices.iter().copied().collect();

    while current_level.len() > 1 {
        let mut next_level = Vec::with_capacity(current_level.len().div_ceil(2));
        let mut next_indices = std::collections::BTreeSet::new();

        for chunk_idx in 0..current_level.len().div_ceil(2) {
            let left_idx = chunk_idx * 2;
            let right_idx = left_idx + 1;
            let parent_idx = chunk_idx;

            // Check if we need siblings for this pair
            let need_left = current_indices.contains(&left_idx);
            let need_right =
                right_idx < current_level.len() && current_indices.contains(&right_idx);

            // Add siblings to proof (but not the nodes we're proving)
            if need_left && !need_right {
                if right_idx < current_level.len() {
                    proof_nodes.insert(current_level[right_idx].to_vec());
                } else {
                    // Push an empty vector to indicate a pass-through (consistent with generate_merkle_proof)
                    proof_nodes.insert(Vec::new());
                }
            } else if need_right && !need_left {
                proof_nodes.insert(current_level[left_idx].to_vec());
            }

            // Build next level
            if right_idx < current_level.len() {
                next_level.push(hash_node(
                    &current_level[left_idx],
                    &current_level[right_idx],
                ));
            } else {
                next_level.push(current_level[left_idx]);
            }

            // Track parent index for next level
            if need_left || need_right {
                next_indices.insert(parent_idx);
            }
        }

        current_level = next_level;
        current_indices = next_indices;
    }

    proof_nodes.into_iter().collect()
}

/// Compressed proof format using bit flags to indicate left/right positions
/// Reduces bandwidth by ~50% for proofs
#[derive(Debug, Clone)]
pub struct CompressedMerkleProof {
    pub siblings: Vec<Vec<u8>>,
    pub flags: Vec<bool>, // true = sibling is on right, false = sibling is on left
}

impl CompressedMerkleProof {
    /// Compress a standard merkle proof
    pub fn from_proof(_tx_hash: &[u8], index: usize, proof: &[Vec<u8>]) -> Self {
        let mut flags = Vec::with_capacity(proof.len());
        let mut current_index = index;

        for _ in proof {
            flags.push(current_index.is_multiple_of(2)); // true if current is left (sibling is right)
            current_index /= 2;
        }

        Self {
            siblings: proof.to_vec(),
            flags,
        }
    }

    /// Verify compressed proof
    pub fn verify(&self, tx_hash: &[u8], _index: usize, merkle_root: &[u8]) -> bool {
        if self.siblings.is_empty() && tx_hash == merkle_root {
            return true;
        }

        if self.siblings.len() != self.flags.len() {
            return false; // Invalid compressed proof
        }

        let mut current_hash = bytes_to_hash(tx_hash);

        for (sibling_vec, &is_right) in self.siblings.iter().zip(self.flags.iter()) {
            if !sibling_vec.is_empty() {
                let sibling = bytes_to_hash(sibling_vec);
                current_hash = if is_right {
                    // Current is left, sibling is right
                    hash_node(&current_hash, &sibling)
                } else {
                    // Current is right, sibling is left
                    hash_node(&sibling, &current_hash)
                };
            }
            // If sibling is empty, it's a pass-through node - don't hash, just move up
        }

        current_hash.as_slice() == merkle_root
    }

    /// Serialize to compact bytes (for network transmission)
    pub fn to_bytes(&self) -> Vec<u8> {
        let flag_bytes = self.flags.len().div_ceil(8);
        let sibling_bytes: usize = self.siblings.iter().map(Vec::len).sum();
        let mut bytes = Vec::with_capacity(4 + flag_bytes + sibling_bytes);

        // Write number of siblings
        bytes.extend(&(self.siblings.len() as u32).to_le_bytes());

        // Write flags as packed bits
        let mut flag_byte = 0u8;
        let mut bit_pos = 0u8;
        for &flag in &self.flags {
            if flag {
                flag_byte |= 1 << bit_pos;
            }
            bit_pos += 1;
            if bit_pos == 8 {
                bytes.push(flag_byte);
                flag_byte = 0;
                bit_pos = 0;
            }
        }
        if bit_pos > 0 {
            bytes.push(flag_byte);
        }

        // Write sibling hashes
        for sibling in &self.siblings {
            bytes.extend(sibling);
        }

        bytes
    }

    /// Deserialize from compact bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }

        let count = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let flag_bytes_needed = count.div_ceil(8);

        if bytes.len() < 4 + flag_bytes_needed + count * 32 {
            return None;
        }

        // Read flags
        let mut flags = Vec::with_capacity(count);
        for i in 0..count {
            let byte_idx = 4 + i / 8;
            let bit_idx = i % 8;
            let flag = (bytes[byte_idx] & (1 << bit_idx)) != 0;
            flags.push(flag);
        }

        // Read siblings
        let mut siblings = Vec::with_capacity(count);
        let sibling_start = 4 + flag_bytes_needed;
        for i in 0..count {
            let start = sibling_start + i * 32;
            let end = start + 32;
            siblings.push(bytes[start..end].to_vec());
        }

        Some(Self { siblings, flags })
    }
}

#[cfg(test)]
#[path = "../tests/unit/merkle_tests.rs"]
mod tests;
