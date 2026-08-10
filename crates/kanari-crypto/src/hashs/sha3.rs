// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Cryptographic hash using SHA3-256 over multiple input chunks.
#[must_use]
pub fn hash_data_sha3_256_chunks(chunks: &[&[u8]]) -> Vec<u8> {
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    hasher.finalize().to_vec()
}

/// Cryptographic hash using SHA3-512 (quantum-resistant, 512-bit).
#[must_use]
pub fn hash_data_sha3_512(data: &[u8]) -> Vec<u8> {
    hash_data_sha3_512_chunks(&[data])
}

/// Cryptographic hash using SHA3-512 over multiple input chunks.
#[must_use]
pub fn hash_data_sha3_512_chunks(chunks: &[&[u8]]) -> Vec<u8> {
    use sha3::{Digest, Sha3_512};
    let mut hasher = Sha3_512::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    hasher.finalize().to_vec()
}
