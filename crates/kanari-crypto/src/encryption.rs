// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Secure data encryption and decryption
//!
//! This module provides functions for encrypting and decrypting data using
//!
//! **Classical**: AES-256-GCM with Argon2 key derivation

use aes_gcm::{
    Aes256Gcm, Key, Nonce,
    aead::{Aead, KeyInit},
};
use argon2::{
    Algorithm, Argon2, Version,
    password_hash::{PasswordHasher, SaltString},
};
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
// OsRng from bip39's rand (v0.8) is compatible with password-hash's rand_core v0.6
use bip39::rand::rngs::OsRng;
use std::fmt;
use std::io::{self, ErrorKind};
use std::string::ToString;
use thiserror::Error;
use zeroize::Zeroize;

pub mod stream;
pub use stream::{
    DEFAULT_STREAM_CHUNK_SIZE, MAX_STREAM_CHUNK_SIZE, StreamDecryptingReader,
    StreamEncryptingWriter, StreamEncryptionHeader, decrypt_stream, encrypt_stream,
    stream_encrypting_writer,
};

/// Encryption scheme selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EncryptionScheme {
    /// AES-256-GCM (Classical, fast)
    #[default]
    Aes256Gcm,
}

impl EncryptionScheme {
    /// Returns true if this scheme is quantum-resistant
    pub fn is_quantum_resistant(&self) -> bool {
        false
    }

    /// Get security level (1-5)
    pub fn security_level(&self) -> u8 {
        match self {
            EncryptionScheme::Aes256Gcm => 4,
        }
    }

    /// Check if this scheme is available (compile-time feature check)
    pub fn is_available(&self) -> bool {
        match self {
            EncryptionScheme::Aes256Gcm => true,
        }
    }
}

/// Error types for encryption operations
#[derive(Error, Debug)]
pub enum EncryptionError {
    #[error("Encryption error: {0}")]
    AeadError(String),

    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),

    #[error("Invalid format error: {0}")]
    InvalidFormat(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Decryption error")]
    DecryptionError,

    #[error("Post-quantum encryption error: {0}")]
    PqcError(String),

    #[error("Feature not available: {0} requires 'pqc' feature to be enabled")]
    FeatureNotAvailable(String),
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
}

pub(super) const STREAM_ENCRYPTION_FORMAT_VERSION: u32 = 1;
pub(super) const STREAM_ENCRYPTION_ALGORITHM: &str = "AES-256-GCM-FRAMED-ARGON2ID";
pub(super) const STREAM_NONCE_PREFIX_LEN: usize = 4;
pub(super) const AEAD_TAG_LEN: usize = 16;
pub(super) const STREAM_FRAME_AAD_LEN: usize =
    STREAM_ENCRYPTION_ALGORITHM.len() + STREAM_NONCE_PREFIX_LEN + 8 + 1 + 4;

impl EncryptedData {
    /// Get the ciphertext bytes, regardless of format
    pub fn get_ciphertext(&self) -> Result<Vec<u8>, EncryptionError> {
        if !self.ciphertext.is_empty() {
            general_purpose::STANDARD
                .decode(&self.ciphertext)
                .map_err(|e| {
                    EncryptionError::InvalidFormat(format!("Invalid ciphertext base64: {}", e))
                })
        } else if !self.ciphertext_array.is_empty() {
            Ok(self.ciphertext_array.clone())
        } else {
            Err(EncryptionError::InvalidFormat(
                "Empty ciphertext".to_string(),
            ))
        }
    }

    /// Get the nonce bytes, regardless of format
    pub fn get_nonce(&self) -> Result<Vec<u8>, EncryptionError> {
        if !self.nonce.is_empty() {
            general_purpose::STANDARD
                .decode(&self.nonce)
                .map_err(|e| EncryptionError::InvalidFormat(format!("Invalid nonce base64: {}", e)))
        } else if !self.nonce_array.is_empty() {
            Ok(self.nonce_array.clone())
        } else {
            Err(EncryptionError::InvalidFormat("Empty nonce".to_string()))
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
            "EncryptedData {{ ciphertext: [{}], nonce: [{}], salt: {} }}",
            cipher_len, nonce_len, self.salt
        )
    }
}

/// Encrypt data with a password
pub fn encrypt_data(data: &[u8], password: &str) -> Result<EncryptedData, EncryptionError> {
    validate_password(password)?;

    // Generate a random salt for key derivation
    let salt = SaltString::generate(&mut OsRng);

    let key_bytes_vec = derive_key(password, &salt)?;
    let cipher = cipher_from_derived(key_bytes_vec.as_slice())?;

    // Generate a random nonce for AES-GCM
    let nonce_arr: [u8; 12] = rand::random();
    let nonce = Nonce::try_from(&nonce_arr[..])
        .map_err(|_| EncryptionError::InvalidInput("Invalid nonce length".to_string()))?;

    // Encrypt the data
    let ciphertext = cipher
        .encrypt(&nonce, data)
        .map_err(|e| EncryptionError::AeadError(e.to_string()))?;

    // Store values in a more compact base64 representation
    let ciphertext_b64 = general_purpose::STANDARD.encode(&ciphertext);
    let nonce_b64 = general_purpose::STANDARD.encode(nonce_arr);

    // Zeroize intermediate derived key material as soon as possible
    drop(key_bytes_vec);

    Ok(EncryptedData {
        ciphertext_array: Vec::new(),
        ciphertext: ciphertext_b64,
        nonce_array: Vec::new(),
        nonce: nonce_b64,
        salt: salt.to_string(),
    })
}

/// Decrypt data with a password
pub fn decrypt_data(encrypted: &EncryptedData, password: &str) -> Result<Vec<u8>, EncryptionError> {
    validate_password(password)?;

    // Validate ciphertext size to prevent memory exhaustion attacks
    const MAX_CIPHERTEXT_SIZE: usize = 100 * 1024 * 1024; // 100MB
    // Decode ciphertext first (handles base64 or raw array) then check size in bytes
    let ciphertext = encrypted.get_ciphertext()?;
    if ciphertext.len() > MAX_CIPHERTEXT_SIZE {
        return Err(EncryptionError::InvalidFormat(
            "Ciphertext size exceeds maximum allowed".to_string(),
        ));
    }

    // Get salt from the encrypted data
    let salt = SaltString::from_b64(&encrypted.salt)
        .map_err(|_| EncryptionError::InvalidFormat("Invalid salt format".to_string()))?;

    let key_bytes_vec = derive_key(password, &salt)?;
    let cipher = cipher_from_derived(key_bytes_vec.as_slice())?;

    // We already decoded ciphertext above; get the nonce bytes now
    let nonce_bytes = encrypted.get_nonce()?;

    // Create nonce for decryption - need to convert Vec<u8> to Nonce
    if nonce_bytes.len() != 12 {
        return Err(EncryptionError::InvalidFormat(
            "Invalid nonce length".to_string(),
        ));
    }
    let nonce = aes_gcm::Nonce::try_from(nonce_bytes.as_slice())
        .map_err(|_| EncryptionError::InvalidFormat("Invalid nonce length".to_string()))?;

    // Zeroize intermediate derived key material before decryption
    drop(key_bytes_vec);

    // Decrypt the data
    cipher
        .decrypt(&nonce, ciphertext.as_ref())
        .map_err(|_| EncryptionError::DecryptionError)
}

fn validate_password(password: &str) -> Result<(), EncryptionError> {
    if password.is_empty() {
        return Err(EncryptionError::InvalidInput(
            "Password cannot be empty".to_string(),
        ));
    }
    if password.len() > crate::MAX_PASSWORD_LEN {
        return Err(EncryptionError::InvalidInput(format!(
            "Password exceeds maximum length of {} bytes",
            crate::MAX_PASSWORD_LEN
        )));
    }
    Ok(())
}

fn derive_key(
    password: &str,
    salt: &SaltString,
) -> Result<zeroize::Zeroizing<Vec<u8>>, EncryptionError> {
    let password_zero = zeroize::Zeroizing::new(password.as_bytes().to_vec());
    let params = argon2_params()?;
    let password_hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(&password_zero, salt)
        .map_err(|e| EncryptionError::KeyDerivationError(e.to_string()))?;

    let hash = password_hash.hash.ok_or_else(|| {
        EncryptionError::KeyDerivationError("Argon2 hash output is missing".to_string())
    })?;
    Ok(zeroize::Zeroizing::new(hash.as_bytes().to_vec()))
}

fn cipher_from_derived(key_bytes: &[u8]) -> Result<Aes256Gcm, EncryptionError> {
    let key = Key::<Aes256Gcm>::try_from(key_bytes).map_err(|_| {
        EncryptionError::KeyDerivationError("Argon2 produced invalid key length".to_string())
    })?;
    Ok(Aes256Gcm::new(&key))
}

fn encryption_error_to_io(error: EncryptionError) -> io::Error {
    let kind = match error {
        EncryptionError::InvalidFormat(_) | EncryptionError::DecryptionError => {
            ErrorKind::InvalidData
        }
        EncryptionError::InvalidInput(_) => ErrorKind::InvalidInput,
        _ => ErrorKind::Other,
    };
    io::Error::new(kind, error)
}

// Helper function to get consistent argon2 parameters
// Uses OWASP recommended parameters for interactive applications
fn argon2_params() -> Result<argon2::Params, EncryptionError> {
    argon2::Params::new(
        47104,    // Memory cost (46 MB) - OWASP minimum recommendation
        3,        // Time cost (3 iterations) - improved security
        1,        // Parallelism (1 thread)
        Some(32), // Produce 32-byte output to use directly as AES-256 key
    )
    .map_err(|e| EncryptionError::KeyDerivationError(format!("Invalid Argon2 parameters: {}", e)))
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
/// Uses zeroize crate for secure memory clearing
pub fn secure_erase(data: &mut [u8]) {
    data.zeroize();
}
