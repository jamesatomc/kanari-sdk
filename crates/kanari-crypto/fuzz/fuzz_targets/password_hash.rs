// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#![no_main]

use kanari_crypto::{
    HashAlgorithm, hash_data, hash_data_with_algorithm, is_password_strong,
    signatures::secure_clear,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Test password validation
    if let Ok(password) = std::str::from_utf8(data) {
        let is_strong = is_password_strong(password);

        // Verify password rules are consistently applied
        if password.len() < 16 {
            assert!(!is_strong, "Passwords shorter than 16 chars should be weak");
        } else if password.chars().any(|c| c.is_control()) {
            assert!(
                !is_strong,
                "Passwords with control characters should be weak"
            );
        } else {
            // For longer passwords, check complexity requirements
            let has_upper = password.chars().any(|c| c.is_uppercase());
            let has_lower = password.chars().any(|c| c.is_lowercase());
            let has_digit = password.chars().any(|c| c.is_numeric());

            const SPECIAL_CHARS: &str = "!@#$%^&*()_+-=[]{}|;:',.<>?/~`\"";
            let has_special = password.chars().any(|c| SPECIAL_CHARS.contains(c));

            if has_upper && has_lower && has_digit && has_special {
                // Should be strong (unless it's in common passwords list)
                // We can't assert here because common passwords are rejected
            }
        }
    }

    // Test hashing functions - should never panic
    let hash1 = hash_data(data);
    assert_eq!(hash1.len(), 32, "SHA3-256 should produce 32-byte hash");

    let hash2 = hash_data_with_algorithm(data, HashAlgorithm::Sha3_256);
    assert_eq!(
        hash1, hash2,
        "hash_data and hash_data_with_algorithm should match"
    );

    let hash3 = hash_data_with_algorithm(data, HashAlgorithm::Sha3_512);
    assert_eq!(hash3.len(), 64, "SHA3-512 should produce 64-byte hash");

    let hash4 = hash_data_with_algorithm(data, HashAlgorithm::Blake3);
    assert_eq!(hash4.len(), 32, "Blake3 should produce 32-byte hash");

    let hash5 = hash_data_with_algorithm(data, HashAlgorithm::Shake256);
    assert_eq!(
        hash5.len(),
        32,
        "SHAKE256 default should produce 32-byte hash"
    );

    // Test deterministic hashing
    let hash_again = hash_data(data);
    assert_eq!(
        hash1, hash_again,
        "Hashing same data should produce same result"
    );

    // Test that different algorithms produce different hashes (for non-empty input)
    if !data.is_empty() {
        assert_ne!(
            hash1, hash3,
            "Different hash algorithms should produce different results"
        );
    }

    // Test secure_clear
    if !data.is_empty() {
        let mut test_vec = data.to_vec();
        let original_len = test_vec.len();

        secure_clear(&mut test_vec);

        assert!(
            test_vec.iter().all(|&b| b == 0),
            "All bytes should be zeroed"
        );
        assert_eq!(
            test_vec.len(),
            original_len,
            "Length should remain unchanged"
        );
    }
});
