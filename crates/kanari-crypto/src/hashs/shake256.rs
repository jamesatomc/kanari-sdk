// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Cryptographic hash using SHAKE256 with 256-bit output (quantum-resistant).
#[must_use]
pub fn hash_data_shake256(data: &[u8]) -> Vec<u8> {
    hash_data_shake256_chunks(&[data])
}

/// Cryptographic hash using SHAKE256 over multiple input chunks with 256-bit output.
#[must_use]
pub fn hash_data_shake256_chunks(chunks: &[&[u8]]) -> Vec<u8> {
    hash_data_shake256_custom_chunks(chunks, 32)
}

/// Cryptographic hash using SHAKE256 with custom output length.
#[must_use]
pub fn hash_data_shake256_custom(data: &[u8], output_len: usize) -> Vec<u8> {
    hash_data_shake256_custom_chunks(&[data], output_len)
}

/// Cryptographic hash using SHAKE256 over multiple input chunks with custom output length.
#[must_use]
pub fn hash_data_shake256_custom_chunks(chunks: &[&[u8]], output_len: usize) -> Vec<u8> {
    use shake::{
        Shake256,
        digest::{ExtendableOutput, Update, XofReader},
    };
    let mut hasher = Shake256::default();
    for chunk in chunks {
        hasher.update(chunk);
    }
    let mut reader = hasher.finalize_xof();
    let mut output = vec![0u8; output_len];
    reader.read(&mut output);
    output
}
