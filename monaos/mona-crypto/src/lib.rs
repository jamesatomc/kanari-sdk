// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

//! Secure cryptographic primitives for the Mona blockchain platform
//!
//! This crate provides cryptographic operations including:
//! - Key generation and management
//! - Digital signatures
//! - Encryption and decryption
//! - Wallet operations

pub mod compression;
pub mod encryption;
pub mod keystore;
pub mod signatures;
pub mod wallet;

// Re-export signature functionality
pub use signatures::{
    SignatureError, secure_clear, sign_message, verify_signature, verify_signature_with_curve,
};

// Re-export encryption functionality - now using actual functions from the module
pub use encryption::{
    EncryptedData, EncryptionError, decrypt_data, decrypt_string, encrypt_data, encrypt_string,
    secure_erase,
};

// Re-export wallet functionality
pub use wallet::{
    Wallet, WalletError, check_mnemonic_exists, check_wallet_exists, clear_session_keys,
    get_mnemonic_addresses, get_selected_wallet, list_wallet_files, load_mnemonic,
    load_session_key, load_wallet, remove_mnemonic, remove_session_key, save_mnemonic,
    save_session_key, save_wallet, set_selected_wallet,
};

// Re-export keystore functionality
pub use keystore::{Keystore, get_keystore_path, keystore_exists};

// Re-export compression functionality
pub use compression::{compress_data, decompress_data};

/// Hash algorithm options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    /// SHA3-256 algorithm (default)
    Sha3_256,
    /// Blake3 algorithm (faster, equally secure)
    Blake3,
}

impl Default for HashAlgorithm {
    fn default() -> Self {
        HashAlgorithm::Sha3_256
    }
}

/// Cryptographic hash using SHA3-256
pub fn hash_data(data: &[u8]) -> Vec<u8> {
    hash_data_with_algorithm(data, HashAlgorithm::Sha3_256)
}

/// Cryptographic hash using Blake3 (faster and secure alternative to SHA3)
pub fn hash_data_blake3(data: &[u8]) -> Vec<u8> {
    hash_data_with_algorithm(data, HashAlgorithm::Blake3)
}

/// Cryptographic hash using the specified algorithm
pub fn hash_data_with_algorithm(data: &[u8], algorithm: HashAlgorithm) -> Vec<u8> {
    match algorithm {
        HashAlgorithm::Sha3_256 => {
            use sha3::{Digest, Sha3_256};
            let mut hasher = Sha3_256::new();
            hasher.update(data);
            hasher.finalize().to_vec()
        }
        HashAlgorithm::Blake3 => {
            let mut hasher = blake3::Hasher::new();
            hasher.update(data);
            hasher.finalize().as_bytes().to_vec()
        }
    }
}

// Add constant for recommended password length
pub const MIN_RECOMMENDED_PASSWORD_LENGTH: usize = 12;

/// Security level used by this library
pub const SECURITY_LEVEL: &str = "High - AES-256-GCM with Argon2id key derivation";

/// Version information for the crypto library
pub fn version() -> &'static str {
    "1.0.0"
}

/// Returns security information about the library
pub fn security_info() -> &'static str {
    "This library uses Argon2id for password hashing, AES-256-GCM for encryption,
    and constant-time comparisons for secure signature verification.
    Always keep your private keys secure and use strong, unique passwords."
}

/// Checks if a password meets minimum security requirements
pub fn is_password_strong(password: &str) -> bool {
    if password.len() < MIN_RECOMMENDED_PASSWORD_LENGTH {
        return false;
    }

    let has_uppercase = password.chars().any(|c| c.is_uppercase());
    let has_lowercase = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_digit(10));
    let has_special = password.chars().any(|c| !c.is_alphanumeric());

    has_uppercase && has_lowercase && has_digit && has_special
}
