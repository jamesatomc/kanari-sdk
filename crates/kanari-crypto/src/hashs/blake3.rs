// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Cryptographic hash using Blake3 (faster alternative).
#[must_use]
pub fn hash_data_blake3(data: &[u8]) -> Vec<u8> {
    hash_data_blake3_array(data).to_vec()
}

/// Cryptographic hash using Blake3 and returning a fixed 32-byte array.
#[must_use]
pub fn hash_data_blake3_array(data: &[u8]) -> [u8; 32] {
    hash_data_blake3_chunks_array(&[data])
}

/// Cryptographic hash using Blake3 over multiple input chunks.
///
/// This keeps callers from materializing temporary concatenation buffers when
/// hashing domain-separated structured data such as SMT leaves and nodes.
#[must_use]
pub fn hash_data_blake3_chunks(chunks: &[&[u8]]) -> Vec<u8> {
    hash_data_blake3_chunks_array(chunks).to_vec()
}

/// Cryptographic hash using Blake3 over multiple input chunks, returning a fixed 32-byte array.
#[must_use]
pub fn hash_data_blake3_chunks_array(chunks: &[&[u8]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    *hasher.finalize().as_bytes()
}
