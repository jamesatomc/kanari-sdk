// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

//! Cryptographic key generation and management
//!
//! This module handles key generation for multiple curve types (K256/secp256k1,
//! P256/secp256r1, and Ed25519), as well as key derivation from mnemonics.

use bip39::{Language, Mnemonic, rand, rand::rngs::OsRng};
use mona_types::address::Address;
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

use k256::{
    PublicKey as K256PublicKey, SecretKey as K256SecretKey,
    ecdsa::{SigningKey as K256SigningKey, VerifyingKey as K256VerifyingKey},
    elliptic_curve::sec1::ToEncodedPoint,
};

use p256::{
    SecretKey as P256SecretKey,
    ecdsa::{SigningKey, VerifyingKey},
};

use ed25519_dalek::{SigningKey as Ed25519SigningKey, VerifyingKey as Ed25519VerifyingKey};

/// Supported elliptic curve types
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CurveType {
    /// Secp256k1 curve (used by Bitcoin and Ethereum)
    K256,

    /// Secp256r1 curve (NIST P-256)
    P256,

    /// Ed25519 curve (modern, fast signature scheme)
    Ed25519,
}

impl Default for CurveType {
    fn default() -> Self {
        CurveType::K256
    }
}

impl fmt::Display for CurveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurveType::K256 => write!(f, "K256 (secp256k1)"),
            CurveType::P256 => write!(f, "P256 (secp256r1)"),
            CurveType::Ed25519 => write!(f, "Ed25519"),
        }
    }
}

/// Key generation errors
#[derive(Error, Debug)]
pub enum KeyError {
    #[error("Invalid private key format")]
    InvalidPrivateKey,

    #[error("Invalid public key format")]
    InvalidPublicKey,

    #[error("Invalid mnemonic phrase: {0}")]
    InvalidMnemonic(String),

    #[error("Key generation failed: {0}")]
    GenerationFailed(String),
}

/// Result of key generation containing private key, public key, and address
pub struct KeyPair {
    pub private_key: String,
    pub public_key: String,
    pub address: String,
    pub curve_type: CurveType,
}

/// Prefix used for Kanari private keys
pub const KANARI_KEY_PREFIX: &str = "kanari";

/// Format a raw hex private key with the Kanari prefix
pub fn format_private_key(raw_key: &str) -> String {
    format!("{}{}", KANARI_KEY_PREFIX, raw_key)
}

/// Extract the raw hex key from a formatted private key
pub fn extract_raw_key(formatted_key: &str) -> &str {
    if formatted_key.starts_with(KANARI_KEY_PREFIX) {
        &formatted_key[KANARI_KEY_PREFIX.len()..]
    } else {
        formatted_key // Return as-is if it doesn't have the prefix
    }
}

/// Generate a keypair for the specified curve type
pub fn generate_keypair(curve_type: CurveType) -> Result<KeyPair, KeyError> {
    match curve_type {
        CurveType::K256 => generate_k256_keypair(),
        CurveType::P256 => generate_p256_keypair(),
        CurveType::Ed25519 => generate_ed25519_keypair(),
    }
}

/// Generate a K256 (secp256k1) keypair
fn generate_k256_keypair() -> Result<KeyPair, KeyError> {
    // Generate secret key using k256
    let secret_key = K256SecretKey::random(&mut OsRng);
    // Convert to signing key first
    let signing_key = K256SigningKey::from(secret_key);
    // Then get verifying key
    let verifying_key = K256VerifyingKey::from(&signing_key);
    // Finally get public key
    let public_key = K256PublicKey::from(verifying_key);

    // Get encoded public key and format
    let encoded_point = public_key.to_encoded_point(false);
    let mut hex_encoded = hex::encode(&encoded_point.as_bytes()[1..]);
    hex_encoded.truncate(64); // Keep consistent with the existing approach

    let address = format!("0x{}", hex_encoded);
    let raw_private_key = hex::encode(signing_key.to_bytes());

    // Format private key with kanari prefix
    let private_key = format_private_key(&raw_private_key);

    Ok(KeyPair {
        private_key,
        public_key: hex_encoded,
        address,
        curve_type: CurveType::K256,
    })
}

/// Generate a P256 (secp256r1) keypair
fn generate_p256_keypair() -> Result<KeyPair, KeyError> {
    // Generate a random P-256 private key
    let signing_key = SigningKey::random(&mut OsRng);
    let secret_key = signing_key.to_bytes();

    // Get the corresponding public key
    let verifying_key = VerifyingKey::from(&signing_key);
    let public_key = verifying_key.to_encoded_point(false);

    // Format the public key, skipping the 0x04 prefix byte
    let mut hex_encoded = hex::encode(&public_key.as_bytes()[1..]);
    hex_encoded.truncate(64); // Keep consistent with secp256k1 method

    let address = format!("0x{}", hex_encoded);
    let raw_private_key = hex::encode(secret_key);

    // Format private key with kanari prefix
    let private_key = format_private_key(&raw_private_key);

    Ok(KeyPair {
        private_key,
        public_key: hex_encoded,
        address,
        curve_type: CurveType::P256,
    })
}

/// Generate an Ed25519 keypair
fn generate_ed25519_keypair() -> Result<KeyPair, KeyError> {
    // Generate random bytes for the private key
    let mut rng = rand::thread_rng();
    let mut seed = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rng, &mut seed);

    // Create signing key from random bytes
    let signing_key = Ed25519SigningKey::from_bytes(&seed);
    let verifying_key = Ed25519VerifyingKey::from(&signing_key);

    // Get the bytes of the keys
    let private_key_bytes = signing_key.to_bytes();
    let public_key_bytes = verifying_key.to_bytes();

    // Format the public key
    let hex_encoded = hex::encode(&public_key_bytes);
    let address = format!("0x{}", hex_encoded);
    let raw_private_key = hex::encode(private_key_bytes);

    // Format private key with kanari prefix
    let private_key = format_private_key(&raw_private_key);

    Ok(KeyPair {
        private_key,
        public_key: hex_encoded,
        address,
        curve_type: CurveType::Ed25519,
    })
}

/// Generate a keypair from a mnemonic phrase
pub fn keypair_from_mnemonic(
    phrase: &str,
    curve_type: CurveType,
    password: &str, // Add optional password parameter
) -> Result<KeyPair, KeyError> {
    // Validate and create mnemonic
    let mnemonic = Mnemonic::parse_in(Language::English, phrase)
        .map_err(|e| KeyError::InvalidMnemonic(e.to_string()))?;

    // Generate seed from mnemonic with password
    let seed = mnemonic.to_seed(password); // Use password instead of empty string
    let bytes = &seed[0..32];

    match curve_type {
        CurveType::K256 => {
            let secret_key =
                K256SecretKey::from_slice(bytes).map_err(|_e| KeyError::InvalidPrivateKey)?;

            let signing_key = K256SigningKey::from(secret_key);
            let verifying_key = K256VerifyingKey::from(&signing_key);
            let public_key = K256PublicKey::from(verifying_key);

            let encoded_point = public_key.to_encoded_point(false);
            let mut hex_encoded = hex::encode(&encoded_point.as_bytes()[1..]);
            hex_encoded.truncate(64);

            let address = format!("0x{}", hex_encoded);
            let raw_private_key = hex::encode(signing_key.to_bytes());

            // Format private key with kanari prefix
            let private_key = format_private_key(&raw_private_key);

            Ok(KeyPair {
                private_key,
                public_key: hex_encoded,
                address,
                curve_type: CurveType::K256,
            })
        }
        CurveType::P256 => {
            let secret_key =
                P256SecretKey::from_slice(bytes).map_err(|_e| KeyError::InvalidPrivateKey)?;

            let signing_key = SigningKey::from(secret_key);
            let verifying_key = VerifyingKey::from(&signing_key);
            let public_key = verifying_key.to_encoded_point(false);

            let mut hex_encoded = hex::encode(&public_key.as_bytes()[1..]);
            hex_encoded.truncate(64);

            let address = format!("0x{}", hex_encoded);
            let raw_private_key = hex::encode(&signing_key.to_bytes());

            // Format private key with kanari prefix
            let private_key = format_private_key(&raw_private_key);

            Ok(KeyPair {
                private_key,
                public_key: hex_encoded,
                address,
                curve_type: CurveType::P256,
            })
        }
        CurveType::Ed25519 => {
            let mut seed_array = [0u8; 32];
            seed_array.copy_from_slice(bytes);

            let signing_key = Ed25519SigningKey::from_bytes(&seed_array);
            let verifying_key = Ed25519VerifyingKey::from(&signing_key);

            let private_key = hex::encode(signing_key.to_bytes());
            let public_key_bytes = verifying_key.to_bytes();
            let hex_encoded = hex::encode(&public_key_bytes);
            let address = format!("0x{}", hex_encoded);

            // Format private key with kanari prefix
            let private_key = format_private_key(&private_key);

            Ok(KeyPair {
                private_key,
                public_key: hex_encoded,
                address,
                curve_type: CurveType::Ed25519,
            })
        }
    }
}

/// Generate a keypair from a private key
pub fn keypair_from_private_key(
    private_key: &str,
    curve_type: CurveType,
) -> Result<KeyPair, KeyError> {
    // Remove kanari prefix if present
    let raw_private_key = extract_raw_key(private_key);

    let private_key_bytes =
        hex::decode(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;

    match curve_type {
        CurveType::K256 => {
            let secret_key = K256SecretKey::from_slice(&private_key_bytes)
                .map_err(|_| KeyError::InvalidPrivateKey)?;

            let signing_key = K256SigningKey::from(secret_key);
            let verifying_key = K256VerifyingKey::from(&signing_key);
            let public_key = K256PublicKey::from(verifying_key);

            let encoded_point = public_key.to_encoded_point(false);
            let mut hex_encoded = hex::encode(&encoded_point.as_bytes()[1..]);
            hex_encoded.truncate(64);

            let address = format!("0x{}", hex_encoded);

            // Format with kanari prefix if not already formatted
            let formatted_private_key = if private_key.starts_with(KANARI_KEY_PREFIX) {
                private_key.to_string()
            } else {
                format_private_key(private_key)
            };

            Ok(KeyPair {
                private_key: formatted_private_key,
                public_key: hex_encoded,
                address,
                curve_type: CurveType::K256,
            })
        }
        CurveType::P256 => {
            let secret_key = P256SecretKey::from_slice(&private_key_bytes)
                .map_err(|_| KeyError::InvalidPrivateKey)?;

            let signing_key = SigningKey::from(secret_key);
            let verifying_key = VerifyingKey::from(&signing_key);
            let public_key = verifying_key.to_encoded_point(false);

            let mut hex_encoded = hex::encode(&public_key.as_bytes()[1..]);
            hex_encoded.truncate(64);

            let address = format!("0x{}", hex_encoded);

            // Format with kanari prefix if not already formatted
            let formatted_private_key = if private_key.starts_with(KANARI_KEY_PREFIX) {
                private_key.to_string()
            } else {
                format_private_key(private_key)
            };

            Ok(KeyPair {
                private_key: formatted_private_key,
                public_key: hex_encoded,
                address,
                curve_type: CurveType::P256,
            })
        }
        CurveType::Ed25519 => {
            if private_key_bytes.len() != 32 {
                return Err(KeyError::InvalidPrivateKey);
            }

            let mut key_array = [0u8; 32];
            key_array.copy_from_slice(&private_key_bytes);

            let signing_key = Ed25519SigningKey::from_bytes(&key_array);
            let verifying_key = Ed25519VerifyingKey::from(&signing_key);

            let public_key_bytes = verifying_key.to_bytes();
            let hex_encoded = hex::encode(&public_key_bytes);
            let address = format!("0x{}", hex_encoded);

            // Format private key with kanari prefix
            let private_key = format_private_key(private_key);

            Ok(KeyPair {
                private_key: private_key.to_string(),
                public_key: hex_encoded,
                address,
                curve_type: CurveType::Ed25519,
            })
        }
    }
}

/// Derive an Address type from a public key
pub fn derive_address_from_pubkey(public_key: &str) -> Result<Address, KeyError> {
    let address_str = format!("0x{}", public_key);
    Address::from_str(&address_str).map_err(|_| KeyError::InvalidPublicKey)
}

/// Generate a mnemonic phrase with the specified word count
pub fn generate_mnemonic(word_count: usize) -> Result<String, KeyError> {
    let mnemonic_result = match word_count {
        12 => Mnemonic::generate(12),
        24 => Mnemonic::generate(24),
        _ => {
            return Err(KeyError::GenerationFailed(format!(
                "Unsupported word count: {}",
                word_count
            )));
        }
    };

    let mnemonic = mnemonic_result.map_err(|e| KeyError::GenerationFailed(e.to_string()))?;

    Ok(mnemonic.to_string())
}

/// Detect likely curve type for a given address
pub fn detect_curve_type(address: &str) -> Option<CurveType> {
    let address_hex = address.trim_start_matches("0x");
    let decoded_hex = match hex::decode(address_hex) {
        Ok(hex) => hex,
        Err(_) => return None,
    };

    // For Ed25519, public keys are always 32 bytes exactly
    if decoded_hex.len() == 32 {
        // Try to construct an Ed25519 key
        let mut key_array = [0u8; 32];
        if decoded_hex.len() == 32 {
            key_array.copy_from_slice(&decoded_hex);
            if Ed25519VerifyingKey::from_bytes(&key_array).is_ok() {
                return Some(CurveType::Ed25519);
            }
        }
    }

    if decoded_hex.len() != 64 && decoded_hex.len() != 32 {
        return None;
    }

    let k256_key_valid = if decoded_hex.len() == 64 {
        let mut public_key_bytes = Vec::with_capacity(65);
        public_key_bytes.push(0x04);
        public_key_bytes.extend_from_slice(&decoded_hex);
        K256VerifyingKey::from_sec1_bytes(&public_key_bytes).is_ok()
    } else {
        let mut compressed_bytes = vec![0x02];
        compressed_bytes.extend_from_slice(&decoded_hex[0..32]);
        K256VerifyingKey::from_sec1_bytes(&compressed_bytes).is_ok() || {
            compressed_bytes[0] = 0x03;
            K256VerifyingKey::from_sec1_bytes(&compressed_bytes).is_ok()
        }
    };

    let p256_key_valid = if decoded_hex.len() == 64 {
        let mut public_key_bytes = Vec::with_capacity(65);
        public_key_bytes.push(0x04);
        public_key_bytes.extend_from_slice(&decoded_hex);
        VerifyingKey::from_sec1_bytes(&public_key_bytes).is_ok()
    } else {
        let mut compressed_bytes = vec![0x02];
        compressed_bytes.extend_from_slice(&decoded_hex[0..32]);
        VerifyingKey::from_sec1_bytes(&compressed_bytes).is_ok() || {
            compressed_bytes[0] = 0x03;
            VerifyingKey::from_sec1_bytes(&compressed_bytes).is_ok()
        }
    };

    match (k256_key_valid, p256_key_valid) {
        (true, false) => Some(CurveType::K256),
        (false, true) => Some(CurveType::P256),
        (true, true) => Some(CurveType::K256), // Default to K256 if both valid
        (false, false) => None,
    }
}

/// Generate a new Kanari address with the specified mnemonic length and curve type
pub fn generate_karix_address(
    mnemonic_length: usize,
    curve_type: CurveType,
) -> (String, String, String) {
    // Generate mnemonic phrase
    let seed_phrase = generate_mnemonic(mnemonic_length).expect("Failed to generate mnemonic");

    // Generate keypair from mnemonic
    let keypair = keypair_from_mnemonic(&seed_phrase, curve_type, "")
        .expect("Failed to generate keypair from mnemonic");

    (keypair.private_key, keypair.address, seed_phrase)
}

/// Import a wallet from a seed phrase
pub fn import_from_seed_phrase(
    phrase: &str,
    curve_type: CurveType,
) -> Result<(String, String, String), String> {
    keypair_from_mnemonic(phrase, curve_type, "")
        .map(|keypair| (keypair.private_key, keypair.public_key, keypair.address))
        .map_err(|e| e.to_string())
}

/// Import a wallet from a private key
pub fn import_from_private_key(
    private_key: &str,
    curve_type: CurveType,
) -> Result<(String, String, String), String> {
    keypair_from_private_key(private_key, curve_type)
        .map(|keypair| (keypair.private_key, keypair.public_key, keypair.address))
        .map_err(|e| e.to_string())
}
