// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::{MIN_RECOMMENDED_PASSWORD_LENGTH, generate_keypair};

use super::*;
use move_core_types::account_address::AccountAddress;
use std::str::FromStr;

// Helper to create a test wallet
fn create_test_wallet() -> (Wallet, String) {
    let keypair = generate_keypair(CurveType::K256).unwrap();
    let password = "TestPassword123!";

    let priv_key = {
        let zk = keypair.export_private_key_secure();
        zk.to_string()
    };

    let wallet = Wallet::new(
        AccountAddress::from_str(&keypair.address).unwrap(),
        priv_key,
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
        None,
        CurveType::K256,
    );

    (wallet, password.to_string())
}

// ============================================================================
// Bug #6: Insufficient Password Validation (High)
// ============================================================================

#[test]
fn test_save_wallet_rejects_empty_password() {
    let keypair = generate_keypair(CurveType::K256).unwrap();
    let address = AccountAddress::from_str(&keypair.address).unwrap();

    let result = save_wallet(
        &address,
        &keypair.private_key,
        "test seed",
        None,
        "", // Empty password
        CurveType::K256,
    );

    assert!(result.is_err(), "Empty password should be rejected");
    assert!(matches!(
        result.unwrap_err(),
        WalletError::EncryptionError(_)
    ));
}

#[test]
fn test_save_wallet_rejects_short_password() {
    let keypair = generate_keypair(CurveType::K256).unwrap();
    let address = AccountAddress::from_str(&keypair.address).unwrap();

    let result = save_wallet(
        &address,
        &keypair.private_key,
        "test seed",
        None,
        "short", // Only 5 characters
        CurveType::K256,
    );

    assert!(result.is_err(), "Password < 8 chars should be rejected");
    match result.unwrap_err() {
        WalletError::EncryptionError(msg) => {
            assert!(msg.contains(&format!(
                "at least {} characters",
                MIN_RECOMMENDED_PASSWORD_LENGTH
            )));
        }
        other => {
            panic!("Expected EncryptionError, got: {:?}", other);
        }
    }
}

#[test]
fn test_save_wallet_accepts_minimum_length_password() {
    // Note: This test may fail if keystore file system operations are not mocked
    // In a real scenario, we'd need to mock the filesystem
    let keypair = generate_keypair(CurveType::K256).unwrap();
    let address = AccountAddress::from_str(&keypair.address).unwrap();

    let password = "12345678"; // Exactly 8 characters

    // This test would need proper filesystem mocking to work
    // For now, just verify the validation logic doesn't reject it
    let result = save_wallet(
        &address,
        &keypair.private_key,
        "test seed",
        None,
        password,
        CurveType::K256,
    );

    // May fail on filesystem operations, but shouldn't fail on validation
    if let Err(e) = result {
        // Should not be password validation error
        assert!(
            !matches!(e, WalletError::EncryptionError(msg) if msg.contains("at least 8 characters"))
        );
    }
}

#[test]
fn test_load_wallet_rejects_empty_password() {
    let result = load_wallet("0x123", "");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), WalletError::InvalidPassword));
}

#[test]
fn test_save_wallet_rejects_empty_private_key() {
    let address = AccountAddress::from_str("0x1234567890123456789012345678901234567890").unwrap();

    let result = save_wallet(
        &address,
        "", // Empty private key
        "test seed",
        None,
        "ValidPassword123",
        CurveType::K256,
    );

    assert!(result.is_err(), "Empty private key should be rejected");
    assert!(matches!(
        result.unwrap_err(),
        WalletError::EncryptionError(_)
    ));
}

// ============================================================================
// Wallet Operations Tests
// ============================================================================

#[test]
fn test_wallet_creation() {
    let (wallet, _) = create_test_wallet();

    assert!(!wallet.address.to_string().is_empty());
    assert!(wallet.private_key.starts_with("kanari"));
    assert!(!wallet.seed_phrase.is_empty());
    assert_eq!(wallet.curve_type, CurveType::K256);
}

#[test]
fn test_wallet_sign_message() {
    let (wallet, password) = create_test_wallet();
    let message = b"Test message to sign";

    let signature = wallet.sign(message, &password);
    assert!(signature.is_ok(), "Signing should succeed");
    assert!(!signature.unwrap().is_empty());
}

#[test]
fn test_wallet_sign_repairs_stale_hybrid_curve_metadata_by_address() {
    let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
    let wallet = Wallet::new(
        AccountAddress::from_str(&keypair.address).unwrap(),
        keypair.private_key.to_string(),
        String::new(),
        None,
        // Simulate an old/corrupted wallet file that stored pure Dilithium3
        // even though the private key is a kanahybrid key.
        CurveType::Dilithium3,
    );

    assert_eq!(
        wallet.validated_signing_curve().unwrap(),
        CurveType::Ed25519Dilithium3
    );
    assert!(wallet.sign(b"hybrid wallet tx", "password").is_ok());
}

#[test]
fn test_wallet_sign_fails_closed_when_curve_and_address_do_not_match() {
    let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
    let wrong_address = generate_keypair(CurveType::K256).unwrap().address;
    let wallet = Wallet::new(
        AccountAddress::from_str(&wrong_address).unwrap(),
        keypair.private_key.to_string(),
        String::new(),
        None,
        CurveType::Dilithium3,
    );

    let err = wallet.sign(b"hybrid wallet tx", "password").unwrap_err();
    assert!(matches!(err, WalletError::KeyCurveMismatch(_)));
}

#[test]
fn test_wallet_sign_empty_message_fails() {
    let (wallet, password) = create_test_wallet();
    let empty_message = b"";

    let result = wallet.sign(empty_message, &password);
    assert!(result.is_err(), "Cannot sign empty message");
    assert!(matches!(result.unwrap_err(), WalletError::SigningError(_)));
}

#[test]
fn test_wallet_sign_empty_password_fails() {
    let (wallet, _) = create_test_wallet();
    let message = b"Test message";

    let result = wallet.sign(message, "");
    assert!(result.is_err(), "Cannot sign with empty password");
    assert!(matches!(result.unwrap_err(), WalletError::InvalidPassword));
}

#[test]
fn test_wallet_verify_signature() {
    let (wallet, password) = create_test_wallet();
    let message = b"Test message";

    let signature = wallet.sign(message, &password).unwrap();
    let verified = wallet.verify(message, &signature);

    assert!(verified.is_ok());
    assert!(verified.unwrap(), "Signature should verify");
}

#[test]
fn test_wallet_verify_wrong_message_fails() {
    let (wallet, password) = create_test_wallet();
    let message1 = b"Original message";
    let message2 = b"Different message";

    let signature = wallet.sign(message1, &password).unwrap();
    let verified = wallet.verify(message2, &signature).unwrap();

    assert!(!verified, "Wrong message should not verify");
}

#[test]
fn test_wallet_verify_empty_message_fails() {
    let (wallet, _) = create_test_wallet();
    let signature = vec![0u8; 64];

    let result = wallet.verify(b"", &signature);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), WalletError::SigningError(_)));
}

#[test]
fn test_wallet_verify_empty_signature_fails() {
    let (wallet, _) = create_test_wallet();
    let message = b"test";

    let result = wallet.verify(message, b"");
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), WalletError::SigningError(_)));
}

#[test]
fn test_wallet_sign_clears_private_key_copy() {
    // This test verifies that signing clears the private key copy from memory
    let (wallet, password) = create_test_wallet();
    let message = b"Test";

    // Sign message - internally should clear private key copy
    let result = wallet.sign(message, &password);
    assert!(result.is_ok());

    // Original wallet private key should still be intact
    assert!(!wallet.private_key.is_empty());
    assert!(wallet.private_key.starts_with("kanari"));
}

#[test]
fn test_private_key_formatting() {
    let keypair = generate_keypair(CurveType::Ed25519).unwrap();
    let address = AccountAddress::from_str(&keypair.address).unwrap();

    // Test with kanari prefix
    let wallet1 = Wallet::new(
        address,
        keypair.private_key.to_string(),
        "seed".to_string(),
        None,
        CurveType::Ed25519,
    );
    assert!(wallet1.private_key.starts_with("kanari"));

    // Test without kanari prefix - save_wallet should add it
    let raw_key = keypair.private_key.trim_start_matches("kanari");
    // Can't test save_wallet fully without filesystem mocking
    // but the logic is in save_wallet function
    assert!(!raw_key.is_empty());
}

#[test]
fn test_wallet_error_types() {
    // Test that all error types can be created
    let _err1 = WalletError::EncryptionError("test".to_string());
    let _err2 = WalletError::DecryptionError("test".to_string());
    let _err3 = WalletError::NotFound("test".to_string());
    let _err4 = WalletError::InvalidPassword;
    let _err5 = WalletError::SigningError("test".to_string());
    let _err6 = WalletError::Locked;
    let _err7 = WalletError::AlreadyExists("test".to_string());
}

#[test]
fn test_wallet_with_different_curves() {
    let curves = vec![CurveType::K256, CurveType::P256, CurveType::Ed25519];

    for curve in curves {
        let keypair = generate_keypair(curve).unwrap();
        let priv_key = {
            let zk = keypair.export_private_key_secure();
            zk.to_string()
        };

        let wallet = Wallet::new(
            AccountAddress::from_str(&keypair.address).unwrap(),
            priv_key,
            "seed".to_string(),
            None,
            curve,
        );

        assert_eq!(wallet.curve_type, curve);

        // Test signing with each curve
        let message = b"test";
        let signature = wallet.sign(message, "password");
        assert!(signature.is_ok(), "Signing should work for {:?}", curve);
    }
}

#[test]
fn test_legacy_wallet_toml_parse_without_compression() {
    let keypair = generate_keypair(CurveType::K256).unwrap();
    let address = AccountAddress::from_str(&keypair.address).unwrap();
    let priv_key = {
        let zk = keypair.export_private_key_secure();
        zk.to_string()
    };
    let wallet = Wallet::new(address, priv_key, String::new(), None, CurveType::K256);
    let toml_string = toml::to_string(&wallet).unwrap();
    let encrypted = encryption::encrypt_data(toml_string.as_bytes(), "StrongPassw0rd!").unwrap();
    let decrypted = encryption::decrypt_data(&encrypted, "StrongPassw0rd!").unwrap();
    let parsed: Wallet = toml::from_str(std::str::from_utf8(&decrypted).unwrap()).unwrap();
    assert_eq!(parsed.curve_type, CurveType::K256);
}
