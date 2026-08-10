// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Cryptographic hash using SHA2-256.
#[must_use]
pub fn hash_data_sha2_256(data: &[u8]) -> Vec<u8> {
    hash_data_sha2_256_chunks(&[data])
}

/// Cryptographic hash using SHA2-256 over multiple input chunks.
#[must_use]
pub fn hash_data_sha2_256_chunks(chunks: &[&[u8]]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    hasher.finalize().to_vec()
}

/// Cryptographic hash using SHA2-512.
#[must_use]
pub fn hash_data_sha2_512(data: &[u8]) -> Vec<u8> {
    hash_data_sha2_512_chunks(&[data])
}

/// Cryptographic hash using SHA2-512 over multiple input chunks.
#[must_use]
pub fn hash_data_sha2_512_chunks(chunks: &[&[u8]]) -> Vec<u8> {
    use sha2::{Digest, Sha512};
    let mut hasher = Sha512::new();
    for chunk in chunks {
        hasher.update(chunk);
    }
    hasher.finalize().to_vec()
}
