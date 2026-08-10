// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Wallet management functionality
//!
//! This module handles wallet operations including creation, encryption,
//! storage, and loading of cryptocurrency wallets.

use crate::keys::{
    CurveType, KANAFALCON_PREFIX, KANAHYBRID_PREFIX, KANAMLDSA_PREFIX, KANAPQC_PREFIX,
    KANARI_KEY_PREFIX, KANASLHDSA_PREFIX, keypair_from_private_key,
};
use serde::{Deserialize, Serialize};
use std::io;
use std::str::FromStr;
use thiserror::Error;

use move_core_types::account_address::AccountAddress;
use toml; // Ensure toml is imported for serialization/deserialization

use crate::Keystore;
use crate::compression;
use crate::encryption;
use crate::hd_wallet::HdError;
use crate::signatures; // ADDED: Import hd_wallet module
use zeroize;

mod config;
mod hd;
mod serde_zeroizing;
mod validation;

pub use config::{
    check_wallet_exists, get_selected_wallet, list_wallet_files, set_selected_wallet,
};
pub use hd::{
    check_mnemonic_exists, create_hd_wallet, create_wallet_from_hd, get_mnemonic_addresses,
    load_mnemonic, remove_mnemonic, save_hd_wallet, save_mnemonic,
};

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

    #[error("Wallet key/curve mismatch: {0}")]
    KeyCurveMismatch(String),
}

/// Structure representing a wallet with private key and address
/// Private key and seed phrase are sensitive and should be handled carefully
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Wallet {
    pub address: AccountAddress,
    /// Private key wrapped in Zeroizing to clear memory on drop
    #[serde(skip_serializing_if = "String::is_empty", default)]
    #[serde(
        serialize_with = "serde_zeroizing::serialize",
        deserialize_with = "serde_zeroizing::deserialize"
    )]
    pub private_key: zeroize::Zeroizing<String>,
    /// Seed phrase wrapped in Zeroizing to clear memory on drop
    #[serde(skip_serializing_if = "String::is_empty", default)]
    #[serde(
        serialize_with = "serde_zeroizing::serialize",
        deserialize_with = "serde_zeroizing::deserialize"
    )]
    pub seed_phrase: zeroize::Zeroizing<String>,
    /// Optional derivation path (e.g. "m/44'/637'/0'/0/0") for HD wallets
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<String>,
    pub curve_type: CurveType,
}

impl Wallet {
    /// Create a new wallet instance
    pub fn new(
        address: AccountAddress,
        private_key: String,
        seed_phrase: String,
        derivation_path: Option<String>,
        curve_type: CurveType,
    ) -> Self {
        Self {
            address,
            private_key: zeroize::Zeroizing::new(private_key),
            seed_phrase: zeroize::Zeroizing::new(seed_phrase),
            derivation_path,
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

        // Sign the message - use reference to avoid unnecessary clone
        // Zeroizing wrapper already protects the private_key field
        let curve_type = self.validated_signing_curve()?;
        signatures::sign_message(&self.private_key, message, curve_type)
            .map_err(|e| WalletError::SigningError(e.to_string()))
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
        // Recreate a KeyPair from the stored private key so we can use the
        // KeyPair-aware verifier which prefers the explicit `pqc_public_key`
        // field (avoids parsing combined public_key strings).
        let curve_type = self.validated_signing_curve()?;
        let keypair = keypair_from_private_key(&self.private_key, curve_type)
            .map_err(|e| WalletError::VerificationError(e.to_string()))?;

        signatures::verify_signature_with_keypair(&keypair, message, signature)
            .map_err(|e| WalletError::VerificationError(e.to_string()))
    }

    /// Return a signing curve that is proven to match this wallet's private key
    /// and stored address.
    ///
    /// Older wallet files can contain stale `curve_type` metadata after PQC/hybrid
    /// migrations. We do not guess blindly: if the declared curve cannot import
    /// the private key, only accept an inferred curve when deriving it from the
    /// private key produces exactly this wallet address. Ambiguous or mismatched
    /// keys fail closed.
    pub fn validated_signing_curve(&self) -> Result<CurveType, WalletError> {
        if self.private_key.is_empty() {
            return Err(WalletError::KeyCurveMismatch(
                "wallet has no private key".to_string(),
            ));
        }

        if keypair_matches_wallet(&self.private_key, self.curve_type, &self.address) {
            return Ok(self.curve_type);
        }

        let matching_curves: Vec<CurveType> = candidate_curves_for_private_key(&self.private_key)
            .into_iter()
            .filter(|curve| *curve != self.curve_type)
            .filter(|curve| keypair_matches_wallet(&self.private_key, *curve, &self.address))
            .collect();

        match matching_curves.as_slice() {
            [curve] => Ok(*curve),
            [] => Err(WalletError::KeyCurveMismatch(format!(
                "stored curve {} does not match the wallet private key/address",
                self.curve_type
            ))),
            curves => Err(WalletError::KeyCurveMismatch(format!(
                "private key matches multiple curves for this address: {:?}",
                curves
            ))),
        }
    }
}

fn keypair_matches_wallet(
    private_key: &str,
    curve_type: CurveType,
    address: &AccountAddress,
) -> bool {
    keypair_from_private_key(private_key, curve_type)
        .map(|keypair| account_address_matches(&keypair.address, address))
        .unwrap_or(false)
}

fn account_address_matches(derived_address: &str, wallet_address: &AccountAddress) -> bool {
    let normalized = derived_address.trim_start_matches("0x");
    AccountAddress::from_str(normalized)
        .map(|derived| &derived == wallet_address)
        .unwrap_or(false)
}

fn candidate_curves_for_private_key(private_key: &str) -> Vec<CurveType> {
    if private_key.starts_with(KANAHYBRID_PREFIX) {
        return vec![CurveType::Ed25519Dilithium3, CurveType::K256Dilithium3];
    }

    if private_key.starts_with(KANAMLDSA_PREFIX) || private_key.starts_with(KANAPQC_PREFIX) {
        return vec![
            CurveType::Dilithium2,
            CurveType::Dilithium3,
            CurveType::Dilithium5,
        ];
    }

    if private_key.starts_with(KANASLHDSA_PREFIX) {
        return vec![CurveType::SphincsPlusSha256Robust];
    }

    if private_key.starts_with(KANAFALCON_PREFIX) {
        return vec![CurveType::Falcon512, CurveType::Falcon1024];
    }

    if private_key.starts_with(KANARI_KEY_PREFIX) || !private_key.contains(':') {
        return vec![CurveType::K256, CurveType::P256, CurveType::Ed25519];
    }

    Vec::new()
}

/// Save a wallet to the keystore
pub fn save_wallet(
    address: &AccountAddress,
    private_key: &str,
    seed_phrase: &str,
    derivation_path: Option<&str>,
    password: &str,
    curve_type: CurveType,
) -> Result<(), WalletError> {
    validation::validate_wallet_secret_inputs(private_key, password)?;
    let formatted_private_key = validation::format_wallet_private_key(private_key);

    // Create wallet object
    let wallet_data = Wallet {
        address: *address,
        private_key: zeroize::Zeroizing::new(formatted_private_key),
        seed_phrase: zeroize::Zeroizing::new(seed_phrase.to_string()),
        derivation_path: derivation_path.map(|s| s.to_string()),
        curve_type,
    };

    // Serialize wallet to TOML (more readable than JSON)
    let toml_string = toml::to_string(&wallet_data)
        .map_err(|e| WalletError::SerializationError(e.to_string()))?;

    // Validate data size before compression to prevent DoS
    const MAX_WALLET_SIZE: usize = 1024 * 1024; // 1MB should be more than enough for wallet data
    if toml_string.len() > MAX_WALLET_SIZE {
        return Err(WalletError::SerializationError(format!(
            "Wallet data too large: {} bytes (max: {})",
            toml_string.len(),
            MAX_WALLET_SIZE
        )));
    }

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
    set_selected_wallet(&address_str)?;

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

    // Normalize address: keystore may store addresses with or without `0x` prefix.
    // Use central helper to produce candidate variants.
    fn address_variants(addr: &str) -> Vec<String> {
        if addr.starts_with("0x") {
            vec![addr.to_string(), addr.trim_start_matches("0x").to_string()]
        } else {
            vec![format!("0x{}", addr), addr.to_string()]
        }
    }

    let key_variants = address_variants(address);

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

    // Validate decrypted data integrity - should be valid UTF-8 or compressed data
    if decrypted.is_empty() {
        return Err(WalletError::DecryptionError(
            "Decrypted data is empty".to_string(),
        ));
    }

    // Additional integrity check: verify data structure before decompression
    const MAX_WALLET_SIZE: usize = 10 * 1024 * 1024; // 10MB max
    if decrypted.len() > MAX_WALLET_SIZE {
        return Err(WalletError::DecryptionError(
            "Decrypted wallet data exceeds maximum size".to_string(),
        ));
    };

    // Decompress the decrypted data (handle both compressed and uncompressed formats)
    let decompressed_data = match compression::decompress_data(&decrypted) {
        Ok(data) => data,
        Err(_e) => {
            // If decompression fails, attempt to parse the raw decrypted bytes as TOML
            // (compatibility with wallets created before compression was added)
            match std::str::from_utf8(&decrypted) {
                Ok(s) => match toml::from_str::<Wallet>(s) {
                    Ok(_) => decrypted,
                    Err(err) => {
                        // Don't expose potentially sensitive data in error messages
                        return Err(WalletError::DecryptionError(format!(
                            "Decompression failed and parsing as TOML failed: {}. Data length: {} bytes",
                            err,
                            decrypted.len()
                        )));
                    }
                },
                Err(_) => {
                    return Err(WalletError::DecryptionError(
                        "Failed to decompress or parse wallet data: non-UTF8 content".to_string(),
                    ));
                }
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
                    // Don't include raw data in error to prevent sensitive data leakage
                    Err(WalletError::SerializationError(format!(
                        "Failed to parse wallet data as TOML: {}",
                        e
                    )))
                }
            }
        }
        Err(e) => Err(WalletError::DecryptionError(format!(
            "Decrypted data is not valid UTF-8: {}",
            e
        ))),
    }
}

#[cfg(test)]
#[path = "../tests/unit/wallet_test.rs"]
mod tests;
