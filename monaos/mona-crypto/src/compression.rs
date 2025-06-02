// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

//! Data compression functionality
//!
//! This module provides compression and decompression functionality
//! to reduce the size of data before encryption, resulting in smaller ciphertexts.

use std::io;
use zstd::bulk::{compress, decompress};

/// Compress data using zstd with high compression level
pub fn compress_data(data: &[u8]) -> Result<Vec<u8>, io::Error> {
    // Use compression level 19 for very high compression ratio
    // (default is 3, max is 22 but very slow)
    compress(data, 19)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Compression error: {}", e)))
}

/// Decompress data that was compressed with zstd
pub fn decompress_data(data: &[u8]) -> Result<Vec<u8>, io::Error> {
    // 10MB maximum size limit to prevent decompression bombs
    decompress(data, 10_485_760)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Decompression error: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_roundtrip() {
        let original = b"This is some test data that should compress well due to repetition. \
                         This is some test data that should compress well due to repetition.";

        let compressed = compress_data(original).unwrap();
        let decompressed = decompress_data(&compressed).unwrap();

        assert_eq!(decompressed, original);
        // Verify compression actually reduces size
        assert!(compressed.len() < original.len());
    }
}
