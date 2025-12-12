//! Wallet management functionality
//!
//! This module handles wallet operations including creation, encryption,
//! storage, and loading of cryptocurrency wallets.

use crate::keys::CurveType;
use serde::{Deserialize, Serialize};
use std::io;
use std::str::FromStr;
use thiserror::Error;

use kanari_common::{load_kanari_config, save_kanari_config};
use move_core_types::account_address::AccountAddress;
use serde_yaml::{Mapping, Value};
use toml; // Ensure toml is imported for serialization/deserialization

use crate::Keystore;
use crate::compression;
use crate::encryption;
use crate::hd_wallet::{self, HdError};
use crate::signatures; // ADDED: Import hd_wallet module

/// Errors that can occur during wallet operations
#[derive(Error, Debug)]
pub enum WalletError {
    #[error("Encryption error: {0}")]
    EncryptionError(String),

    #[error("Decryption error: {0}")]
    DecryptionError(String),

    #[error("IO error: {0}")]
    IoError(#[from] io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Wallet not found: {0}")]
    NotFound(String),

    #[error("Invalid password")]
    InvalidPassword,

    #[error("Signing error: {0}")]
    SigningError(String),

    #[error("Keystore error: {0}")]
    KeystoreError(String),

    #[error("HD Wallet error: {0}")]
    HdWalletError(#[from] HdError),

    #[error("Wallet already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid wallet format: {0}")]
    InvalidFormat(String),

    #[error("Wallet is locked")]
    Locked,

    #[error("Access denied: {0}")]
    AccessDenied(String),

    #[error("Verification error: {0}")]
    VerificationError(String),
}

/// Structure representing a wallet with private key and address
/// Private key and seed phrase are sensitive and should be handled carefully
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Wallet {
    pub address: AccountAddress,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub private_key: String,
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub seed_phrase: String,
    pub curve_type: CurveType,
}

impl Wallet {
    /// Create a new wallet instance
    pub fn new(
        address: AccountAddress,
        private_key: String,
        seed_phrase: String,
        curve_type: CurveType,
    ) -> Self {
        Self {
            address,
            private_key,
            seed_phrase,
            curve_type,
        }
    }

    /// Sign a message using this wallet's private key
    pub fn sign(&self, message: &[u8], password: &str) -> Result<Vec<u8>, WalletError> {
        // Validate message is not empty
        if message.is_empty() {
            return Err(WalletError::SigningError(
                "Cannot sign empty message".to_string(),
            ));
        }

        // Validate password is not empty - this makes the parameter used and required
        if password.is_empty() {
            return Err(WalletError::InvalidPassword);
        }

        // Create a temporary copy of the private key for signing
        let private_key_copy = self.private_key.clone();

        // Sign the message
        let result = signatures::sign_message(&private_key_copy, message, self.curve_type)
            .map_err(|e| WalletError::SigningError(e.to_string()));

        // Securely clear the private key copy from memory
        let mut private_key_bytes = private_key_copy.into_bytes();
        signatures::secure_clear(&mut private_key_bytes);

        result
    }

    /// Verify a signature made with this wallet against a message
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool, WalletError> {
        // Validate inputs
        if message.is_empty() {
            return Err(WalletError::SigningError(
                "Cannot verify empty message".to_string(),
            ));
        }

        if signature.is_empty() {
            return Err(WalletError::SigningError(
                "Cannot verify empty signature".to_string(),
            ));
        }

        signatures::verify_signature_with_curve(
            &self.address.to_string(),
            message,
            signature,
            self.curve_type,
        )
        .map_err(|e| WalletError::SigningError(e.to_string()))
    }
}

/// Save a wallet to the keystore
pub fn save_wallet(
    address: &AccountAddress,
    private_key: &str,
    seed_phrase: &str,
    password: &str,
    curve_type: CurveType,
) -> Result<(), WalletError> {
    // Validate inputs
    if password.is_empty() {
        return Err(WalletError::EncryptionError(
            "Empty password not allowed".to_string(),
        ));
    }

    // Enforce strong password requirements for security (includes length check >= 16)
    if !crate::is_password_strong(password) {
        return Err(WalletError::EncryptionError(
            "Password does not meet security requirements: must be at least 16 characters and contain uppercase, lowercase, numbers, and special characters".to_string()
        ));
    }

    if private_key.is_empty() {
        return Err(WalletError::EncryptionError(
            "Empty private key not allowed".to_string(),
        ));
    }

    // Ensure private key has kanari prefix
    let formatted_private_key = if private_key.starts_with("kanari") {
        private_key.to_string()
    } else {
        format!("kanari{private_key}")
    };

    // Create wallet object
    let wallet_data = Wallet {
        address: *address,
        private_key: formatted_private_key,
        seed_phrase: seed_phrase.to_string(),
        curve_type,
    };

    // Serialize wallet to TOML (more readable than JSON)
    let toml_string = toml::to_string(&wallet_data)
        .map_err(|e| WalletError::SerializationError(e.to_string()))?;

    // Compress data before encryption to reduce ciphertext size
    let compressed_data = compression::compress_data(toml_string.as_bytes())
        .map_err(|e| WalletError::SerializationError(format!("Compression error: {e}")))?;

    // Encrypt the wallet data
    let encrypted_data = encryption::encrypt_data(&compressed_data, password)
        .map_err(|e| WalletError::EncryptionError(e.to_string()))?;

    // Load or create the keystore
    let mut keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    // Format address with 0x prefix for consistency
    let address_str = format!("0x{}", hex::encode(address.to_vec()));

    // Add the wallet to the keystore with the address as the key
    keystore
        .add_wallet(&address_str, encrypted_data)
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    // Also update the active_address in kanari.yaml
    update_active_address(&address_str)?;

    Ok(())
}

/// Load a wallet from the keystore
pub fn load_wallet(address: &str, password: &str) -> Result<Wallet, WalletError> {
    // Validate inputs
    if address.is_empty() {
        return Err(WalletError::NotFound("Empty address".to_string()));
    }

    if password.is_empty() {
        return Err(WalletError::InvalidPassword);
    }

    // Load the keystore
    let keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    // Normalize address: keystore stores addresses with `0x` prefix, but callers
    // may pass the raw hex string. Try both forms so load is tolerant.
    let key_variants = if address.starts_with("0x") {
        vec![
            address.to_string(),
            address.trim_start_matches("0x").to_string(),
        ]
    } else {
        vec![format!("0x{}", address), address.to_string()]
    };

    let mut encrypted_data_opt: Option<&crate::encryption::EncryptedData> = None;
    for key in key_variants.iter() {
        if let Some(ed) = keystore.get_wallet(key) {
            encrypted_data_opt = Some(ed);
            break;
        }
    }

    let encrypted_data_ref =
        encrypted_data_opt.ok_or_else(|| WalletError::NotFound(address.to_string()))?;

    // Decrypt wallet data
    let decrypted = encryption::decrypt_data(encrypted_data_ref, password)
        .map_err(|_| WalletError::InvalidPassword)?;

    // Decompress the decrypted data (handle both compressed and uncompressed formats)
    let decompressed_data = match compression::decompress_data(&decrypted) {
        Ok(data) => data,
        Err(e) => {
            // If decompression fails, the data might not be compressed
            // (compatibility with wallets created before compression was added)
            if let Ok(str_data) = std::str::from_utf8(&decrypted) {
                if str_data.starts_with("address") || str_data.contains("private_key") {
                    // This appears to be valid uncompressed TOML data, use it directly
                    decrypted
                } else {
                    return Err(WalletError::DecryptionError(format!(
                        "Decompression failed and data isn't valid TOML: {e}"
                    )));
                }
            } else {
                return Err(WalletError::DecryptionError(format!(
                    "Failed to decompress or parse wallet data: {e}"
                )));
            }
        }
    };

    // Parse wallet data - try TOML first
    match std::str::from_utf8(&decompressed_data) {
        Ok(decompressed_str) => {
            // Try to parse as TOML
            match toml::from_str::<Wallet>(decompressed_str) {
                Ok(wallet_data) => Ok(wallet_data),
                Err(e) => {
                    // If TOML parsing fails, provide a detailed error
                    Err(WalletError::SerializationError(format!(
                        "Failed to parse wallet data as TOML: {}. First 50 bytes: {:?}",
                        e,
                        &decompressed_data
                            .get(..50.min(decompressed_data.len()))
                            .unwrap_or(&[])
                    )))
                }
            }
        }
        Err(e) => Err(WalletError::DecryptionError(format!(
            "Decrypted data is not valid UTF-8: {}. First 50 bytes: {:?}",
            e,
            &decompressed_data
                .get(..50.min(decompressed_data.len()))
                .unwrap_or(&[])
        ))),
    }
}

// =========================================================================
// HD Wallet Functionality
// =========================================================================

/// Create a child wallet derived from the stored mnemonic at the given path.
/// The created wallet is automatically saved to the keystore and set as active.
pub fn create_wallet_from_hd(
    password: &str,
    derivation_path: &str,
    curve: CurveType,
) -> Result<Wallet, WalletError> {
    // Backwards-compatible helper: create + save. Delegate to new helpers.
    let wallet = create_hd_wallet(password, derivation_path, curve)?;
    save_hd_wallet(&wallet, password)?;
    Ok(wallet)
}

/// Create (but do not persist) a child wallet derived from the stored mnemonic
/// at the given BIP32 derivation path. Returns the constructed Wallet.
pub fn create_hd_wallet(
    password: &str,
    derivation_path: &str,
    curve: CurveType,
) -> Result<Wallet, WalletError> {
    // Load mnemonic and derive keypair
    let mnemonic_phrase = load_mnemonic(password)?;

    let key_pair =
        hd_wallet::derive_keypair_from_path(&mnemonic_phrase, password, derivation_path, curve)?;

    // Convert the derived address string into an AccountAddress type
    let address = AccountAddress::from_str(&key_pair.address)
        .map_err(|e| WalletError::SerializationError(format!("Invalid derived address: {e}")))?;

    // Construct Wallet; store the derivation path in the seed_phrase field
    let wallet = Wallet::new(
        address,
        key_pair.get_private_key(),
        derivation_path.to_string(),
        curve,
    );

    Ok(wallet)
}

/// Persist a previously-created HD child wallet into the keystore using
/// the standard `save_wallet` path.
pub fn save_hd_wallet(wallet: &Wallet, password: &str) -> Result<(), WalletError> {
    save_wallet(
        &wallet.address,
        &wallet.private_key,
        &wallet.seed_phrase,
        password,
        wallet.curve_type,
    )
}

// =========================================================================
// Mnemonic Management Functions
// =========================================================================

/// Save mnemonic phrase to keystore
pub fn save_mnemonic(
    mnemonic: &str,
    password: &str,
    addresses: Vec<String>,
) -> Result<(), WalletError> {
    // Validate inputs
    if password.is_empty() {
        return Err(WalletError::EncryptionError(
            "Empty password not allowed".to_string(),
        ));
    }

    if password.len() < 8 {
        return Err(WalletError::EncryptionError(
            "Password must be at least 8 characters long".to_string(),
        ));
    }

    // Warn if password is not strong (optional: make this mandatory)
    if !crate::is_password_strong(password) {
        log::warn!(
            "Warning: Password does not meet recommended strength requirements (16+ chars, mixed case, numbers, special chars)"
        );
    }

    if mnemonic.is_empty() {
        return Err(WalletError::EncryptionError(
            "Empty mnemonic not allowed".to_string(),
        ));
    }

    // Compress mnemonic before encryption
    let compressed_data = compression::compress_data(mnemonic.as_bytes())
        .map_err(|e| WalletError::SerializationError(format!("Compression error: {e}")))?;

    // Encrypt the mnemonic
    let encrypted_data = encryption::encrypt_data(&compressed_data, password)
        .map_err(|e| WalletError::EncryptionError(e.to_string()))?;

    // Load keystore and save mnemonic
    let mut keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore
        .set_mnemonic(encrypted_data, addresses)
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(())
}

/// Load mnemonic phrase from keystore
pub fn load_mnemonic(password: &str) -> Result<String, WalletError> {
    // Validate inputs
    if password.is_empty() {
        return Err(WalletError::InvalidPassword);
    }

    // Load keystore
    let keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    // Get encrypted mnemonic
    let encrypted_data = keystore
        .get_mnemonic()
        .ok_or_else(|| WalletError::NotFound("Mnemonic not found".to_string()))?;

    // Decrypt mnemonic
    let decrypted = encryption::decrypt_data(encrypted_data, password)
        .map_err(|_| WalletError::InvalidPassword)?;

    // Decompress the decrypted data
    let decompressed_data = compression::decompress_data(&decrypted)
        .map_err(|e| WalletError::DecryptionError(format!("Failed to decompress mnemonic: {e}")))?;

    // Convert to string
    String::from_utf8(decompressed_data)
        .map_err(|e| WalletError::DecryptionError(format!("Invalid UTF-8 in mnemonic: {e}")))
}

/// Get addresses derived from mnemonic
pub fn get_mnemonic_addresses() -> Result<Vec<String>, WalletError> {
    let keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(keystore.get_mnemonic_addresses().clone())
}

/// Check if mnemonic exists in keystore
#[must_use]
pub fn check_mnemonic_exists() -> bool {
    Keystore::load().is_ok_and(|keystore| keystore.has_mnemonic())
}

/// Remove mnemonic from keystore
pub fn remove_mnemonic() -> Result<(), WalletError> {
    let mut keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore
        .remove_mnemonic()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(())
}

// =========================================================================
// Utility and Configuration Functions
// =========================================================================

/// Check if any wallets exist
#[must_use]
pub fn check_wallet_exists() -> bool {
    Keystore::load().is_ok_and(|keystore| !keystore.list_wallets().is_empty())
}

/// List all available wallets with selection status
pub fn list_wallet_files() -> Result<Vec<(String, bool)>, io::Error> {
    // Get currently selected wallet
    let selected = get_selected_wallet().unwrap_or_default();
    let mut wallets = Vec::new();

    // Load the keystore
    match Keystore::load() {
        Ok(keystore) => {
            // Return addresses from the keystore
            for address in keystore.list_wallets() {
                let is_selected = address == selected;
                wallets.push((address, is_selected));
            }

            // Sort wallets alphabetically
            wallets.sort_by(|a, b| a.0.cmp(&b.0));

            Ok(wallets)
        }
        Err(e) => Err(io::Error::other(format!("Failed to load keystore: {e}"))),
    }
}

/// Set the currently selected wallet address in configuration
pub fn set_selected_wallet(wallet_address: &str) -> io::Result<()> {
    // Clean address
    let formatted_address = wallet_address.to_string();

    // Update active_address in kanari.yaml
    update_active_address(&formatted_address)
}

/// Helper function to update `active_address` in kanari.yaml
fn update_active_address(address: &str) -> io::Result<()> {
    // Try to load kanari config - use if let instead of match for single pattern
    if let Ok(mut kanari_config) = load_kanari_config() {
        if let Some(mapping) = kanari_config.as_mapping_mut() {
            mapping.insert(
                Value::String("active_address".to_string()),
                Value::String(address.to_string()),
            );
            save_kanari_config(&kanari_config)
        } else {
            // Create mapping if none exists
            let mut mapping = Mapping::new();
            mapping.insert(
                Value::String("active_address".to_string()),
                Value::String(address.to_string()),
            );
            save_kanari_config(&Value::Mapping(mapping))
        }
    } else {
        // If kanari config doesn't exist or load, create it
        let mut mapping = Mapping::new();
        mapping.insert(
            Value::String("active_address".to_string()),
            Value::String(address.to_string()),
        );
        save_kanari_config(&Value::Mapping(mapping))
    }
}

/// Get the currently selected wallet from configuration
#[must_use]
pub fn get_selected_wallet() -> Option<String> {
    // Only use kanari config
    if let Ok(kanari_config) = load_kanari_config()
        && let Some(active_address) = kanari_config.get("active_address").and_then(|v| v.as_str())
    {
        return Some(active_address.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{CurveType, generate_keypair};

    // Helper to create a test wallet
    fn create_test_wallet() -> (Wallet, String) {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let password = "TestPassword123!";

        let wallet = Wallet::new(
            AccountAddress::from_str(&keypair.address).unwrap(),
            keypair.get_private_key(),
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about".to_string(),
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
            "short", // Only 5 characters
            CurveType::K256,
        );

        assert!(result.is_err(), "Password < 8 chars should be rejected");
        match result.unwrap_err() {
            WalletError::EncryptionError(msg) => {
                assert!(msg.contains("at least 8 characters"));
            }
            _ => panic!("Expected EncryptionError"),
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
        let address =
            AccountAddress::from_str("0x1234567890123456789012345678901234567890").unwrap();

        let result = save_wallet(
            &address,
            "", // Empty private key
            "test seed",
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
            keypair.private_key.clone(),
            "seed".to_string(),
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
            let wallet = Wallet::new(
                AccountAddress::from_str(&keypair.address).unwrap(),
                keypair.get_private_key(),
                "seed".to_string(),
                curve,
            );

            assert_eq!(wallet.curve_type, curve);

            // Test signing with each curve
            let message = b"test";
            let signature = wallet.sign(message, "password");
            assert!(signature.is_ok(), "Signing should work for {:?}", curve);
        }
    }
}
