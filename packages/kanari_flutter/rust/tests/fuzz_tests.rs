//! Property-based fuzzing tests for kanari-crypto using proptest
//! These tests generate random inputs to find edge cases and bugs

use proptest::prelude::*;
use rust::{
    CurveType, decrypt_data, encrypt_data, generate_keypair, hash_data, is_password_strong,
    signatures::{sign_message, verify_signature},
};

const DEFAULT_CRYPTO_FUZZ_CASES: u32 = 256;

fn crypto_fuzz_config() -> proptest::test_runner::Config {
    proptest::test_runner::Config {
        cases: std::env::var("KANARI_CRYPTO_PROPTEST_CASES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_CRYPTO_FUZZ_CASES),
        ..proptest::test_runner::Config::default()
    }
}

/// Fuzz test signature verification with random data
#[test]
fn prop_fuzz_signature_verification() {
    proptest!(crypto_fuzz_config(), |(curve_byte in 0u8..3u8, message: Vec<u8>)| {
        // Limit message size to avoid timeout
        prop_assume!(message.len() <= 1024);

        let curve_type = match curve_byte {
            0 => CurveType::K256,
            1 => CurveType::P256,
            2 => CurveType::Ed25519,
            _ => return Ok(()),
        };

        // Generate keypair - should not panic
        let Ok(keypair) = generate_keypair(curve_type) else {
            return Ok(());
        };

        // Sign message - should not panic
        let Ok(signature) = sign_message(&keypair.private_key, &message, curve_type) else {
            return Ok(());
        };

        // Verify with tagged address - should succeed
        let tagged_addr = keypair.tagged_address();
        let verify_result = verify_signature(&tagged_addr, &message, &signature);

        prop_assert!(verify_result.is_ok(), "Verification should not error");
        prop_assert!(verify_result.unwrap(), "Valid signature should verify successfully");

        // Test corrupted signature - should fail or return false
        if !signature.is_empty() {
            let mut corrupted_sig = signature.clone();
            corrupted_sig[0] = corrupted_sig[0].wrapping_add(1);

            if let Ok(result) = verify_signature(&tagged_addr, &message, &corrupted_sig) {
                prop_assert!(!result, "Corrupted signature should fail");
            }
        }

        // Test wrong message - should fail or return false
        if !message.is_empty() {
            let mut wrong_msg = message.clone();
            wrong_msg[0] = wrong_msg[0].wrapping_add(1);

            if let Ok(result) = verify_signature(&tagged_addr, &wrong_msg, &signature) {
                prop_assert!(!result, "Signature should not verify for different message");
            }
        }
    });
}

/// Fuzz test encryption/decryption roundtrip
#[test]
fn prop_fuzz_encryption_roundtrip() {
    proptest!(crypto_fuzz_config(), |(password_bytes: Vec<u8>, plaintext: Vec<u8>)| {
        // Limit sizes
        prop_assume!(password_bytes.len() >= 8 && password_bytes.len() <= 64);
        prop_assume!(plaintext.len() <= 1024);

        // Convert password to string (skip invalid UTF-8)
        let Ok(password) = std::str::from_utf8(&password_bytes) else {
            return Ok(());
        };

        // Skip empty passwords
        prop_assume!(!password.is_empty());

        // Encrypt - should not panic
        let Ok(encrypted) = encrypt_data(&plaintext, password) else {
            return Ok(());
        };

        // Decrypt with correct password - should succeed
        let decrypted = decrypt_data(&encrypted, password);
        prop_assert!(decrypted.is_ok(), "Decryption with correct password should succeed");
        prop_assert_eq!(decrypted.unwrap(), plaintext, "Decrypted data should match original");

        // Decrypt with wrong password - should fail
        let wrong_password = format!("{}_wrong", password);
        let wrong_result = decrypt_data(&encrypted, &wrong_password);
        prop_assert!(wrong_result.is_err(), "Wrong password should fail decryption");
    });
}

/// Fuzz test hashing functions
#[test]
fn prop_fuzz_hash_functions() {
    proptest!(crypto_fuzz_config(), |(data: Vec<u8>)| {
        // Limit data size
        prop_assume!(data.len() <= 1024);

        // Hash multiple times - should be deterministic
        let hash1 = hash_data(&data);
        let hash2 = hash_data(&data);
        prop_assert_eq!(hash1.clone(), hash2, "Hashing same data should produce same result");

        // Hash length should be consistent
        prop_assert_eq!(hash1.len(), 32, "SHA3-256 should produce 32-byte hash");

        // Empty data should still hash
        let empty_hash = hash_data(&[]);
        prop_assert_eq!(empty_hash.len(), 32, "Empty data hash should be 32 bytes");

        // Different data should produce different hashes (most of the time)
        if !data.is_empty() {
            let mut modified = data.clone();
            modified[0] = modified[0].wrapping_add(1);
            let modified_hash = hash_data(&modified);

            // Very small chance of collision, but extremely unlikely
            prop_assert_ne!(hash1, modified_hash, "Different data should produce different hashes");
        }
    });
}

/// Fuzz test password validation
#[test]
fn prop_fuzz_password_validation() {
    proptest!(crypto_fuzz_config(), |(password_bytes: Vec<u8>)| {
        // Limit size
        prop_assume!(password_bytes.len() <= 128);

        // Convert to string
        let Ok(password) = std::str::from_utf8(&password_bytes) else {
            return Ok(());
        };

        let is_strong = is_password_strong(password);

        // Check consistency rules
        if password.len() < 16 {
            prop_assert!(!is_strong, "Passwords shorter than 16 chars should be weak");
        }

        if password.chars().any(|c| c.is_control()) {
            prop_assert!(!is_strong, "Passwords with control characters should be weak");
        }

        // If marked as strong, verify it meets complexity requirements
        if is_strong {
            prop_assert!(password.len() >= 16, "Strong password must be at least 16 chars");

            let has_upper = password.chars().any(|c| c.is_uppercase());
            let has_lower = password.chars().any(|c| c.is_lowercase());
            let has_digit = password.chars().any(|c| c.is_numeric());

            const SPECIAL_CHARS: &str = "!@#$%^&*()_+-=[]{}|;:',.<>?/~`\"";
            let has_special = password.chars().any(|c| SPECIAL_CHARS.contains(c));

            prop_assert!(has_upper, "Strong password should have uppercase");
            prop_assert!(has_lower, "Strong password should have lowercase");
            prop_assert!(has_digit || has_special, "Strong password should have digit or special char");
        }
    });
}

/// Fuzz test key generation across all curves
#[test]
fn prop_fuzz_key_generation() {
    proptest!(crypto_fuzz_config(), |(curve_selector: u8)| {
        // Map to different curve types
        let curve_type = match curve_selector % 9 {
            0 => CurveType::K256,
            1 => CurveType::P256,
            2 => CurveType::Ed25519,
            3 => CurveType::Dilithium2,
            4 => CurveType::Dilithium3,
            5 => CurveType::Dilithium5,
            6 => CurveType::SphincsPlusSha256Robust,
            7 => CurveType::Ed25519Dilithium3,
            8 => CurveType::K256Dilithium3,
            _ => return Ok(()),
        };

        // Generate keypair - should not panic
        let Ok(keypair) = generate_keypair(curve_type) else {
            return Ok(());
        };

        // Verify basic properties
        prop_assert!(keypair.address.starts_with("0x"), "Address should start with 0x");
        prop_assert!(!keypair.public_key.is_empty(), "Public key should not be empty");

        // Verify tagged address format
        let tagged = keypair.tagged_address();
        prop_assert!(tagged.contains(':'), "Tagged address should contain ':' separator");

        // Parse tagged address back
        if let Some((parsed_curve, parsed_addr)) = rust::keys::KeyPair::parse_tagged_address(&tagged) {
            prop_assert_eq!(parsed_curve, curve_type, "Parsed curve should match");
            prop_assert!(!parsed_addr.is_empty(), "Parsed address should not be empty");
        } else {
            panic!("Tagged address should always be parseable");
        }
    });
}
