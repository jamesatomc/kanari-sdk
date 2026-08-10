// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Small hash abstraction layer using Kanari's canonical crypto crate.

use kanari_crypto::{hash_data_blake3_array, hash_data_blake3_chunks_array};

/// Compute a 32-byte digest of `data` using BLAKE3.
pub fn digest(data: &[u8]) -> [u8; 32] {
    hash_data_blake3_array(data)
}

/// Hash a leaf: H(0x00 || key_hash || value)
pub fn hash_leaf(key_hash: &[u8; 32], value: &[u8]) -> [u8; 32] {
    hash_data_blake3_chunks_array(&[&[0x00], key_hash, value])
}

/// Hash an internal node: H(0x01 || left || right)
pub fn hash_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    hash_data_blake3_chunks_array(&[&[0x01], left, right])
}
