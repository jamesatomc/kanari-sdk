// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Data compression functionality
//!
//! This module provides compression and decompression functionality
//! to reduce the size of data before encryption, resulting in smaller ciphertexts.

use std::io;
use zstd::bulk::{compress, decompress};

/// Compress data using zstd with high compression level
pub fn compress_data(data: &[u8]) -> Result<Vec<u8>, io::Error> {
    // Validate input size to prevent DoS attacks
    const MAX_INPUT_SIZE: usize = 50 * 1024 * 1024; // 50MB
    if data.len() > MAX_INPUT_SIZE {
        return Err(io::Error::other("Input data too large for compression"));
    }

    // Use compression level 10 for good balance of speed/compression
    // (level 19 is too slow and can cause DoS)
    let compressed =
        compress(data, 10).map_err(|e| io::Error::other(format!("Compression error: {}", e)))?;

    // Check compression ratio to detect anomalies
    // Add minimum threshold to avoid division edge cases
    const MIN_COMPRESSED_SIZE: usize = 8; // Minimum 8 bytes
    if compressed.len() >= MIN_COMPRESSED_SIZE && data.len() / compressed.len() > 1000 {
        return Err(io::Error::other("Suspicious compression ratio detected"));
    }

    Ok(compressed)
}

/// Decompress data that was compressed with zstd
pub fn decompress_data(data: &[u8]) -> Result<Vec<u8>, io::Error> {
    // 100MB maximum size limit to prevent decompression bombs
    // This allows for reasonable compression ratios (100:1) for text data
    const MAX_DECOMPRESSED_SIZE: usize = 100 * 1024 * 1024; // 100MB
    const MAX_COMPRESSION_RATIO: usize = 100; // More conservative ratio

    let decompressed = decompress(data, MAX_DECOMPRESSED_SIZE)
        .map_err(|e| io::Error::other(format!("Decompression error: {}", e)))?;

    // Verify decompression ratio is reasonable (max 100:1 instead of 1000:1)
    if !data.is_empty() && decompressed.len() / data.len() > MAX_COMPRESSION_RATIO {
        return Err(io::Error::other(
            "Suspicious decompression ratio detected - possible decompression bomb",
        ));
    }

    Ok(decompressed)
}

#[cfg(test)]
#[path = "../tests/unit/compression_test.rs"]
mod tests;
