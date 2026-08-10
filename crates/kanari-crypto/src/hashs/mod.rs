// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod blake3;
pub mod sha2;
pub mod sha3;
pub mod shake256;

pub use blake3::{
    hash_data_blake3, hash_data_blake3_array, hash_data_blake3_chunks,
    hash_data_blake3_chunks_array,
};
pub use sha2::{
    hash_data_sha2_256, hash_data_sha2_256_chunks, hash_data_sha2_512, hash_data_sha2_512_chunks,
};
pub use sha3::{hash_data_sha3_256_chunks, hash_data_sha3_512, hash_data_sha3_512_chunks};
pub use shake256::{
    hash_data_shake256, hash_data_shake256_chunks, hash_data_shake256_custom,
    hash_data_shake256_custom_chunks,
};

/// Hash algorithm options (including quantum-resistant)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HashAlgorithm {
    /// SHA2-256 algorithm (widely compatible classical hash)
    Sha2_256,
    /// SHA2-512 algorithm (widely compatible classical hash, 512-bit output)
    Sha2_512,
    /// SHA3-256 algorithm (default, quantum-resistant)
    #[default]
    Sha3_256,
    /// SHA3-512 algorithm (higher security, quantum-resistant)
    Sha3_512,
    /// Blake3 algorithm (faster, equally secure)
    Blake3,
    /// SHAKE256 (extendable output, quantum-resistant)
    Shake256,
}

/// Cryptographic hash using SHA3-256 (default)
#[must_use]
pub fn hash_data(data: &[u8]) -> Vec<u8> {
    hash_data_sha3_256_chunks(&[data])
}

/// Cryptographic hash using the specified algorithm
#[must_use]
pub fn hash_data_with_algorithm(data: &[u8], algorithm: HashAlgorithm) -> Vec<u8> {
    hash_data_with_algorithm_chunks(&[data], algorithm)
}

/// Cryptographic hash over multiple input chunks using the specified algorithm.
#[must_use]
pub fn hash_data_with_algorithm_chunks(chunks: &[&[u8]], algorithm: HashAlgorithm) -> Vec<u8> {
    match algorithm {
        HashAlgorithm::Sha2_256 => hash_data_sha2_256_chunks(chunks),
        HashAlgorithm::Sha2_512 => hash_data_sha2_512_chunks(chunks),
        HashAlgorithm::Sha3_256 => hash_data_sha3_256_chunks(chunks),
        HashAlgorithm::Sha3_512 => hash_data_sha3_512_chunks(chunks),
        HashAlgorithm::Blake3 => hash_data_blake3_chunks(chunks),
        HashAlgorithm::Shake256 => hash_data_shake256_chunks(chunks),
    }
}
