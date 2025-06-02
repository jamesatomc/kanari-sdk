// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

//! Secure data encryption and decryption
//!
//! This module provides functions for encrypting and decrypting data using
//! modern secure algorithms (AES-256-GCM with Argon2 key derivation).

use aes_gcm::{
    Aes256Gcm, Key,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use argon2::{
    Algorithm,
    Argon2,
    Version, // Remove Variant as it doesn't exist
    password_hash::{PasswordHasher, SaltString},
};
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::string::ToString;
use thiserror::Error;

/// Error types for encryption operations
#[derive(Error, Debug)]
pub enum EncryptionError {
    #[error("Encryption error: {0}")]
    AeadError(String),

    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),

    #[error("Invalid format error: {0}")]
    InvalidFormat(String),

    #[error("Decryption error")]
    DecryptionError,
}

/// Structure representing encrypted data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ciphertext_array: Vec<u8>,

    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    ciphertext: String,

    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nonce_array: Vec<u8>,

    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    nonce: String,

    salt: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

impl EncryptedData {
    /// Get the ciphertext bytes, regardless of format
    pub fn get_ciphertext(&self) -> Vec<u8> {
        if !self.ciphertext.is_empty() {
            general_purpose::STANDARD
                .decode(&self.ciphertext)
                .unwrap_or_default()
        } else {
            self.ciphertext_array.clone()
        }
    }

    /// Get the nonce bytes, regardless of format
    pub fn get_nonce(&self) -> Vec<u8> {
        if !self.nonce.is_empty() {
            general_purpose::STANDARD
                .decode(&self.nonce)
                .unwrap_or_default()
        } else {
            self.nonce_array.clone()
        }
    }
}

impl fmt::Display for EncryptedData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cipher_len = if !self.ciphertext.is_empty() {
            self.ciphertext.len()
        } else {
            self.ciphertext_array.len()
        };

        let nonce_len = if !self.nonce.is_empty() {
            self.nonce.len()
        } else {
            self.nonce_array.len()
        };

        write!(
            f,
            "EncryptedData {{ ciphertext: [{}], nonce: [{}], salt: {}, tag: {:?} }}",
            cipher_len, nonce_len, self.salt, self.tag
        )
    }
}

/// Encrypt data with a password
pub fn encrypt_data(data: &[u8], password: &str) -> Result<EncryptedData, EncryptionError> {
    // Generate a random salt for key derivation
    let salt = SaltString::generate(&mut OsRng);

    // Derive a cryptographic key from the password
    let params = argon2_params();
    let password_hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| EncryptionError::KeyDerivationError(e.to_string()))?;

    // Fix for the temporary value dropped error - bind to variable first
    let hash = password_hash.hash.unwrap();
    let key_bytes = hash.as_bytes();
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);

    // Generate a random nonce for AES-GCM
    let nonce_bytes = Aes256Gcm::generate_nonce(&mut OsRng).to_vec();

    // Create the cipher for encryption
    let cipher = Aes256Gcm::new(key);

    // Encrypt the data
    let ciphertext = cipher
        .encrypt(aes_gcm::Nonce::from_slice(&nonce_bytes), data)
        .map_err(|e| EncryptionError::AeadError(e.to_string()))?;

    // Store values in a more compact base64 representation
    let ciphertext_b64 = general_purpose::STANDARD.encode(&ciphertext);
    let nonce_b64 = general_purpose::STANDARD.encode(&nonce_bytes);

    Ok(EncryptedData {
        ciphertext_array: Vec::new(),
        ciphertext: ciphertext_b64,
        nonce_array: Vec::new(),
        nonce: nonce_b64,
        salt: salt.to_string(),
        tag: None,
    })
}

/// Decrypt data with a password
pub fn decrypt_data(encrypted: &EncryptedData, password: &str) -> Result<Vec<u8>, EncryptionError> {
    // Get salt from the encrypted data
    let salt = SaltString::from_b64(&encrypted.salt)
        .map_err(|e| EncryptionError::InvalidFormat(e.to_string()))?;

    // Derive key from password and salt
    let params = argon2_params();
    let password_hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| EncryptionError::KeyDerivationError(e.to_string()))?;

    // Fix for the temporary value dropped error
    let hash = password_hash.hash.unwrap();
    let key_bytes = hash.as_bytes();
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);

    // Get ciphertext and nonce from the encrypted data
    let ciphertext = encrypted.get_ciphertext();
    let nonce_bytes = encrypted.get_nonce();

    // Create nonce for decryption - fix the Nonce usage
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);

    // Create cipher for decryption
    let cipher = Aes256Gcm::new(key);

    // Decrypt the data
    cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| EncryptionError::DecryptionError)
}

// Helper function to get consistent argon2 parameters
fn argon2_params() -> argon2::Params {
    argon2::Params::new(
        19456, // Memory cost (19 MB)
        2,     // Time cost (2 iterations)
        1,     // Parallelism (1 thread)
        None,  // No Output::BLOCK_SIZE in this version
    )
    .expect("Failed to create Argon2 parameters")
}

/// Upgrade legacy encrypted data to new base64 format
pub fn upgrade_encrypted_data(old_data: EncryptedData) -> EncryptedData {
    // Only upgrade if using older array format
    if !old_data.ciphertext_array.is_empty() && old_data.ciphertext.is_empty() {
        EncryptedData {
            ciphertext: general_purpose::STANDARD.encode(&old_data.ciphertext_array),
            ciphertext_array: Vec::new(),
            nonce: general_purpose::STANDARD.encode(&old_data.nonce_array),
            nonce_array: Vec::new(),
            salt: old_data.salt,
            tag: old_data.tag,
        }
    } else {
        old_data
    }
}

/// Encrypt a string with a password
pub fn encrypt_string(data: &str, password: &str) -> Result<EncryptedData, EncryptionError> {
    encrypt_data(data.as_bytes(), password)
}

/// Decrypt a string with a password
pub fn decrypt_string(
    encrypted: &EncryptedData,
    password: &str,
) -> Result<String, EncryptionError> {
    let bytes = decrypt_data(encrypted, password)?;
    String::from_utf8(bytes).map_err(|e| EncryptionError::InvalidFormat(e.to_string()))
}

/// Securely erase sensitive data from memory
pub fn secure_erase(data: &mut [u8]) {
    for byte in data.iter_mut() {
        *byte = 0;
    }
}
