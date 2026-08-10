// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Keystore management functionality
//!
//! This module handles the kanari.keystore format for secure storage of wallet information.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use thiserror::Error;

/// Maximum number of keys allowed in keystore to prevent DoS
const MAX_KEYSTORE_KEYS: usize = 10_000;

use crate::encryption::EncryptedData;

mod path;
mod statistics;
mod storage;
pub use path::{get_keystore_path, keystore_exists};
pub use statistics::KeystoreStatistics;

/// Errors related to keystore operations
#[derive(Error, Debug)]
pub enum KeystoreError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Key not found: {0}")]
    KeyNotFound(String),

    #[error("Invalid keystore format")]
    InvalidFormat,

    #[error("Password verification failed")]
    PasswordVerificationFailed,

    #[error("Keystore is locked")]
    Locked,

    #[error("Keystore is corrupted: {0}")]
    Corrupted(String),

    #[error("Access denied: {0}")]
    AccessDenied(String),

    #[error("Backup error: {0}")]
    BackupError(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),
}

/// Structure representing the keystore file
/// Note: Default trait is derived but should not be used directly for creating keystores
/// Always use Keystore::new() or Keystore::load() to ensure proper initialization
#[derive(Serialize, Deserialize)]
pub struct Keystore {
    /// Individual wallet keys by address
    pub keys: HashMap<String, EncryptedData>,

    /// Mnemonic phrase information
    pub mnemonic: MnemonicStore,

    /// Whether the password is empty
    #[serde(default)]
    pub is_password_empty: bool,

    /// Version of the keystore format
    #[serde(default = "default_keystore_version")]
    pub version: String,

    /// Last modified timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<u64>,

    /// Disk location bound to this instance. It is deliberately not serialized:
    /// a keystore file is portable, while its local storage location is not.
    #[serde(skip)]
    storage_path: Option<PathBuf>,
}

fn default_keystore_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Structure for storing mnemonic phrases
#[derive(Serialize, Deserialize, Default)]
pub struct MnemonicStore {
    /// List of addresses derived from the mnemonic
    pub addresses: Vec<String>,

    /// Encrypted mnemonic phrase
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mnemonic_phrase_encryption: Option<EncryptedData>,
}

impl Keystore {
    /// Create a new empty keystore with proper initialization
    #[must_use]
    pub fn new() -> Self {
        Self::with_storage_path(None)
    }

    fn with_storage_path(storage_path: Option<PathBuf>) -> Self {
        Self {
            keys: HashMap::new(),
            mnemonic: MnemonicStore::default(),
            is_password_empty: false,
            version: default_keystore_version(),
            last_modified: None,
            storage_path,
        }
    }

    /// Load keystore from disk
    pub fn load() -> Result<Self, KeystoreError> {
        Self::load_from_path(get_keystore_path())
    }

    /// Load a keystore from an explicit path.
    ///
    /// This keeps tests and embedded callers isolated from the user's default
    /// wallet while preserving `load()` as the production convenience API.
    pub fn load_from_path(keystore_path: impl Into<PathBuf>) -> Result<Self, KeystoreError> {
        let keystore_path = keystore_path.into();

        if !keystore_path.exists() {
            return Ok(Self::with_storage_path(Some(keystore_path)));
        }

        let guard = storage::acquire_shared_lock(&keystore_path)?;

        // Load the keystore data
        let keystore_data = storage::read_to_string(&keystore_path)?;
        let mut keystore: Keystore = serde_json::from_str(&keystore_data)?;
        keystore.storage_path = Some(keystore_path);

        // Upgrade any keys that might be using the old format
        for encrypted_data in keystore.keys.values_mut() {
            *encrypted_data = crate::encryption::upgrade_encrypted_data(encrypted_data.clone());
        }

        // Save if any changes were made (conversion from array to base64)
        drop(guard); // Release read lock before attempting write
        keystore.save()?;

        Ok(keystore)
    }

    /// Save keystore to disk with atomic write
    pub fn save(&mut self) -> Result<(), KeystoreError> {
        let keystore_path = self.storage_path.clone().unwrap_or_else(get_keystore_path);
        storage::ensure_parent_dir(&keystore_path)?;

        // Update last modified timestamp
        self.last_modified = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| KeystoreError::InvalidPath(format!("System time error: {}", e)))?
                .as_secs(),
        );

        let keystore_data = serde_json::to_string_pretty(self)?;
        let _guard = storage::acquire_exclusive_lock(&keystore_path)?;
        storage::atomic_write_string(&keystore_path, &keystore_data)?;

        // Lock is released when `_guard` is dropped at function exit

        Ok(())
    }

    /// Add a wallet to the keystore
    pub fn add_wallet(
        &mut self,
        address: &str,
        encrypted_data: EncryptedData,
    ) -> Result<(), KeystoreError> {
        // Prevent DoS via excessive key count
        if !self.keys.contains_key(address) && self.keys.len() >= MAX_KEYSTORE_KEYS {
            return Err(KeystoreError::InvalidFormat);
        }
        self.keys.insert(address.to_string(), encrypted_data);
        self.save()?;
        Ok(())
    }

    /// Get a wallet from the keystore
    pub fn get_wallet(&self, address: &str) -> Option<&EncryptedData> {
        self.keys.get(address)
    }

    /// Remove a wallet from the keystore
    pub fn remove_wallet(&mut self, address: &str) -> Result<(), KeystoreError> {
        if self.keys.remove(address).is_none() {
            return Err(KeystoreError::KeyNotFound(address.to_string()));
        }

        // Also remove from mnemonic addresses if present
        self.mnemonic.addresses.retain(|addr| addr != address);

        self.save()?;
        Ok(())
    }

    /// Check if a wallet exists in the keystore
    pub fn wallet_exists(&self, address: &str) -> bool {
        self.keys.contains_key(address)
    }

    /// List all wallets in the keystore
    pub fn list_wallets(&self) -> Vec<String> {
        self.keys.keys().cloned().collect()
    }

    /// Set encrypted mnemonic phrase
    pub fn set_mnemonic(
        &mut self,
        encrypted_mnemonic: EncryptedData,
        addresses: Vec<String>,
    ) -> Result<(), KeystoreError> {
        self.mnemonic.mnemonic_phrase_encryption = Some(encrypted_mnemonic);
        self.mnemonic.addresses = addresses;
        self.save()?;
        Ok(())
    }

    /// Get encrypted mnemonic phrase
    pub fn get_mnemonic(&self) -> Option<&EncryptedData> {
        self.mnemonic.mnemonic_phrase_encryption.as_ref()
    }

    /// Get addresses derived from mnemonic
    pub fn get_mnemonic_addresses(&self) -> &Vec<String> {
        &self.mnemonic.addresses
    }

    /// Add address to mnemonic-derived addresses
    pub fn add_mnemonic_address(&mut self, address: &str) -> Result<(), KeystoreError> {
        if !self.mnemonic.addresses.contains(&address.to_string()) {
            self.mnemonic.addresses.push(address.to_string());
            self.save()?;
        }
        Ok(())
    }

    /// Remove mnemonic and all associated data
    pub fn remove_mnemonic(&mut self) -> Result<(), KeystoreError> {
        self.mnemonic.mnemonic_phrase_encryption = None;
        self.mnemonic.addresses.clear();
        self.save()?;
        Ok(())
    }

    /// Check if mnemonic exists
    pub fn has_mnemonic(&self) -> bool {
        self.mnemonic.mnemonic_phrase_encryption.is_some()
    }

    /// Validate keystore integrity
    pub fn validate(&self) -> Result<(), KeystoreError> {
        // Prevent DoS: limit maximum keys to validate
        const MAX_KEYS_TO_VALIDATE: usize = 10_000;

        // Check version compatibility
        if self.version.is_empty() {
            return Err(KeystoreError::InvalidFormat);
        }

        // Check key count limit
        if self.keys.len() > MAX_KEYS_TO_VALIDATE {
            return Err(KeystoreError::Corrupted(format!(
                "Keystore contains too many keys: {} (max: {})",
                self.keys.len(),
                MAX_KEYS_TO_VALIDATE
            )));
        }

        // Validate all encrypted data entries
        for (address, encrypted_data) in &self.keys {
            match encrypted_data.get_ciphertext() {
                Ok(ciphertext) if ciphertext.is_empty() => {
                    return Err(KeystoreError::Corrupted(format!(
                        "Empty ciphertext for address: {}",
                        address
                    )));
                }
                Err(e) => {
                    return Err(KeystoreError::Corrupted(format!(
                        "Invalid ciphertext for address {}: {}",
                        address, e
                    )));
                }
                _ => {}
            }

            match encrypted_data.get_nonce() {
                Ok(nonce) if nonce.is_empty() => {
                    return Err(KeystoreError::Corrupted(format!(
                        "Empty nonce for address: {}",
                        address
                    )));
                }
                Err(e) => {
                    return Err(KeystoreError::Corrupted(format!(
                        "Invalid nonce for address {}: {}",
                        address, e
                    )));
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Get keystore statistics
    pub fn statistics(&self) -> KeystoreStatistics {
        KeystoreStatistics {
            total_keys: self.keys.len(),
            has_mnemonic: self.has_mnemonic(),
            mnemonic_addresses: self.mnemonic.addresses.len(),
            version: self.version.clone(),
            last_modified: self.last_modified,
        }
    }
}

impl Default for Keystore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../tests/unit/keystore_test.rs"]
mod tests;
