// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Optimized Sparse Merkle Tree (SMT) and binary Merkle tree implementation.
//! This crate provides the cryptographic foundations for Kanari state and block verification.
//!
//! FEATURES:
//! - Full Sparse Merkle Tree (SMT) with 256-bit keyspace and default (zero) leaves.
//! - Efficient membership and non-membership proofs.
//! - Binary Merkle tree for transaction inclusion proofs in block headers.
//! - High-performance hashing using BLAKE3.
//!
//! IMPLEMENTATION DETAILS:
//! - SMT uses domain-separated hashing: H(0x00 || ...) for leaves, H(0x01 || ...) for nodes.
//! - Persistence is backed by RocksDB for high-throughput batched writes.
//! - Transaction tree supports standard inclusion proofs for light clients.

pub use kanari_db_common::open_or_get_db;

mod hash;
mod sparse_merkle;
pub use sparse_merkle::SmtNodeOverlay;
pub use sparse_merkle::SparseMerkleTree;
pub use sparse_merkle::compute_sparse_root;
pub use sparse_merkle::default_hashes;
pub use sparse_merkle::verify_proof;

pub use hash::*;
mod merkle;
pub use merkle::{
    CompressedMerkleProof, batch_verify_merkle_proofs, compute_merkle_root,
    compute_merkle_root_optimized, generate_merkle_multiproof, generate_merkle_proof,
    verify_merkle_proof,
};
