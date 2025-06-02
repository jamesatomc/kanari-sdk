// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

//! Wallet management functionality
//!
//! This module handles wallet operations including creation, encryption,
//! storage, and loading of cryptocurrency wallets.

use std::io;
use key::keys::CurveType;
use thiserror::Error;
use serde::{Serialize, Deserialize};

use mona_types::address::Address;
use common::{load_kanari_config, save_kanari_config};
use serde_yaml::{Mapping, Value};

use crate::encryption;
use crate::compression;
use crate::signatures;
use crate::Keystore;

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
}

/// Structure representing a wallet with private key and address
#[derive(Serialize, Deserialize, Clone)]
pub struct Wallet {
    pub address: Address,
    pub private_key: String,
    pub seed_phrase: String,
    pub curve_type: CurveType,
}

impl Wallet {
    /// Create a new wallet instance
    pub fn new(
        address: Address,
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
    pub fn sign(
        &self,
        message: &[u8],
        password: &str,
    ) -> Result<Vec<u8>, WalletError> {
        // Validate message is not empty
        if message.is_empty() {
            return Err(WalletError::SigningError("Cannot sign empty message".to_string()));
        }

        // Validate password is not empty - this makes the parameter used and required
        if password.is_empty() {
            return Err(WalletError::InvalidPassword);
        }

        // Create a temporary copy of the private key for signing
        let private_key_copy = self.private_key.clone();

        // Sign the message
        let result = signatures::sign_message(
            &private_key_copy,
            message,
            self.curve_type,
        ).map_err(|e| WalletError::SigningError(e.to_string()));

        // Securely clear the private key copy from memory
        let mut private_key_bytes = private_key_copy.into_bytes();
        signatures::secure_clear(&mut private_key_bytes);

        result
    }

    /// Verify a signature made with this wallet against a message
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool, WalletError> {
        // Validate inputs
        if message.is_empty() {
            return Err(WalletError::SigningError("Cannot verify empty message".to_string()));
        }

        if signature.is_empty() {
            return Err(WalletError::SigningError("Cannot verify empty signature".to_string()));
        }

        signatures::verify_signature_with_curve(
            &self.address.to_string(),
            message,
            signature,
            self.curve_type,
        ).map_err(|e| WalletError::SigningError(e.to_string()))
    }
}

/// Save a wallet to the keystore
pub fn save_wallet(
    address: &Address,
    private_key: &str,
    seed_phrase: &str,
    password: &str,
    curve_type: CurveType,
) -> Result<(), WalletError> {
    // Validate inputs
    if password.is_empty() {
        return Err(WalletError::EncryptionError("Empty password not allowed".to_string()));
    }

    if private_key.is_empty() {
        return Err(WalletError::EncryptionError("Empty private key not allowed".to_string()));
    }

    // Ensure private key has kanari prefix
    let formatted_private_key = if private_key.starts_with("kanari") {
        private_key.to_string()
    } else {
        format!("kanari{}", private_key)
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
        .map_err(|e| WalletError::SerializationError(format!("Compression error: {}", e)))?;

    // Encrypt the wallet data
    let encrypted_data = encryption::encrypt_data(
        &compressed_data,
        password
    ).map_err(|e| WalletError::EncryptionError(e.to_string()))?;

    // Load or create the keystore
    let mut keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    // Add the wallet to the keystore with the address as the key
    keystore.add_wallet(&address.to_string(), encrypted_data)
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    // Also update the active_address in kanari.yaml
    update_active_address(&address.to_string())?;

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
    let keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    // Get the encrypted data for this wallet
    let encrypted_data = keystore.get_wallet(address)
        .ok_or_else(|| WalletError::NotFound(address.to_string()))?;

    // Decrypt wallet data
    let decrypted = encryption::decrypt_data(encrypted_data, password)
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
                    return Err(WalletError::DecryptionError(
                        format!("Decompression failed and data isn't valid TOML: {}", e)
                    ));
                }
            } else {
                return Err(WalletError::DecryptionError(
                    format!("Failed to decompress or parse wallet data: {}", e)
                ));
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
                    Err(WalletError::SerializationError(
                        format!("Failed to parse wallet data as TOML: {}. First 50 bytes: {:?}",
                            e,
                            &decompressed_data.get(..50.min(decompressed_data.len()))
                                .unwrap_or(&[])
                        )
                    ))
                }
            }
        },
        Err(e) => {
            Err(WalletError::DecryptionError(
                format!("Decrypted data is not valid UTF-8: {}. First 50 bytes: {:?}",
                    e,
                    &decompressed_data.get(..50.min(decompressed_data.len()))
                        .unwrap_or(&[])
                )
            ))
        }
    }
}

/// Save mnemonic phrase to keystore
pub fn save_mnemonic(
    mnemonic: &str,
    password: &str,
    addresses: Vec<String>,
) -> Result<(), WalletError> {
    // Validate inputs
    if password.is_empty() {
        return Err(WalletError::EncryptionError("Empty password not allowed".to_string()));
    }

    if mnemonic.is_empty() {
        return Err(WalletError::EncryptionError("Empty mnemonic not allowed".to_string()));
    }

    // Compress mnemonic before encryption
    let compressed_data = compression::compress_data(mnemonic.as_bytes())
        .map_err(|e| WalletError::SerializationError(format!("Compression error: {}", e)))?;

    // Encrypt the mnemonic
    let encrypted_data = encryption::encrypt_data(&compressed_data, password)
        .map_err(|e| WalletError::EncryptionError(e.to_string()))?;

    // Load keystore and save mnemonic
    let mut keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore.set_mnemonic(encrypted_data, addresses)
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
    let keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    // Get encrypted mnemonic
    let encrypted_data = keystore.get_mnemonic()
        .ok_or_else(|| WalletError::NotFound("Mnemonic not found".to_string()))?;

    // Decrypt mnemonic
    let decrypted = encryption::decrypt_data(encrypted_data, password)
        .map_err(|_| WalletError::InvalidPassword)?;

    // Decompress the decrypted data
    let decompressed_data = compression::decompress_data(&decrypted)
        .map_err(|e| WalletError::DecryptionError(format!("Failed to decompress mnemonic: {}", e)))?;

    // Convert to string
    String::from_utf8(decompressed_data)
        .map_err(|e| WalletError::DecryptionError(format!("Invalid UTF-8 in mnemonic: {}", e)))
}

/// Get addresses derived from mnemonic
pub fn get_mnemonic_addresses() -> Result<Vec<String>, WalletError> {
    let keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(keystore.get_mnemonic_addresses().clone())
}

/// Check if mnemonic exists in keystore
pub fn check_mnemonic_exists() -> bool {
    if let Ok(keystore) = Keystore::load() {
        keystore.has_mnemonic()
    } else {
        false
    }
}

/// Remove mnemonic from keystore
pub fn remove_mnemonic() -> Result<(), WalletError> {
    let mut keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore.remove_mnemonic()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(())
}

/// Session key management functions

/// Save session key
pub fn save_session_key(key: &str, value: &str) -> Result<(), WalletError> {
    let mut keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore.add_session_key(key, value)
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(())
}

/// Load session key
pub fn load_session_key(key: &str) -> Result<Option<String>, WalletError> {
    let keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(keystore.get_session_key(key).cloned())
}

/// Remove session key
pub fn remove_session_key(key: &str) -> Result<(), WalletError> {
    let mut keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore.remove_session_key(key)
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(())
}

/// Clear all session keys
pub fn clear_session_keys() -> Result<(), WalletError> {
    let mut keystore = Keystore::load()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore.clear_session_keys()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(())
}

/// Check if any wallets exist
pub fn check_wallet_exists() -> bool {
    if let Ok(keystore) = Keystore::load() {
        !keystore.list_wallets().is_empty()
    } else {
        false
    }
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
        },
        Err(e) => {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Failed to load keystore: {}", e)
            ))
        }
    }
}

/// Set the currently selected wallet address in configuration
pub fn set_selected_wallet(wallet_address: &str) -> io::Result<()> {
    // Clean address
    let formatted_address = wallet_address.to_string();

    // Update active_address in kanari.yaml
    update_active_address(&formatted_address)
}

/// Helper function to update active_address in kanari.yaml
fn update_active_address(address: &str) -> io::Result<()> {
    // Try to load kanari config
    match load_kanari_config() {
        Ok(mut kanari_config) => {
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
        },
        Err(_) => {
            // If kanari config doesn't exist or load, create it
            let mut mapping = Mapping::new();
            mapping.insert(
                Value::String("active_address".to_string()),
                Value::String(address.to_string()),
            );
            save_kanari_config(&Value::Mapping(mapping))
        }
    }
}

/// Get the currently selected wallet from configuration
pub fn get_selected_wallet() -> Option<String> {
    // Only use kanari config
    if let Ok(kanari_config) = load_kanari_config() {
        if let Some(active_address) = kanari_config.get("active_address").and_then(|v| v.as_str()) {
            return Some(active_address.to_string());
        }
    }
    None
}
