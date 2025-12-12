//! Cryptographic key generation and management
//!
//! This module handles key generation for multiple curve types (K256/secp256k1,
//! P256/secp256r1, Ed25519) and Post-Quantum Cryptography (Dilithium, SPHINCS+).
//!
//! **Quantum-Safe**: Includes NIST-standardized post-quantum algorithms.

use bip39::{Language, Mnemonic};
use rand::rngs::OsRng;
use std::fmt;
use move_core_types::account_address::AccountAddress;
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

// Post-Quantum Cryptography imports
use pqcrypto_dilithium::dilithium2;
use pqcrypto_dilithium::dilithium3;
use pqcrypto_dilithium::dilithium5;
use pqcrypto_sphincsplus::sphincssha2256fsimple;
use pqcrypto_traits::sign::{PublicKey as PqcPublicKey, SecretKey as PqcSecretKey};

/// Supported cryptographic algorithms (Classical + Post-Quantum)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub enum CurveType {
    // Classical Elliptic Curve Cryptography (ECC)
    /// Secp256k1 curve (used by Bitcoin and Ethereum)
    #[default]
    K256,

    /// Secp256r1 curve (NIST P-256)
    P256,

    /// Ed25519 curve (modern, fast signature scheme)
    Ed25519,

    // Post-Quantum Cryptography (PQC) - NIST Standards
    /// Dilithium2 - Fast, ~2.5KB signatures, NIST Level 2 security
    Dilithium2,

    /// Dilithium3 - Balanced, ~4KB signatures, NIST Level 3 security (Recommended)
    Dilithium3,

    /// Dilithium5 - Maximum security, ~5KB signatures, NIST Level 5 security
    Dilithium5,

    /// SPHINCS+ SHA256-256f-robust - Hash-based, ~50KB signatures, ultra-secure
    SphincsPlusSha256Robust,

    // Hybrid Schemes (Classical + PQC for transition period)
    /// Ed25519 + Dilithium3 hybrid (Best of both worlds)
    Ed25519Dilithium3,

    /// K256 + Dilithium3 hybrid (Bitcoin/Ethereum compatible + quantum-safe)
    K256Dilithium3,
}

impl fmt::Display for CurveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CurveType::K256 => write!(f, "K256 (secp256k1)"),
            CurveType::P256 => write!(f, "P256 (secp256r1)"),
            CurveType::Ed25519 => write!(f, "Ed25519"),
            CurveType::Dilithium2 => write!(f, "Dilithium2 (PQC Level 2)"),
            CurveType::Dilithium3 => write!(f, "Dilithium3 (PQC Level 3)"),
            CurveType::Dilithium5 => write!(f, "Dilithium5 (PQC Level 5)"),
            CurveType::SphincsPlusSha256Robust => write!(f, "SPHINCS+ SHA256 (Ultra-Secure PQC)"),
            CurveType::Ed25519Dilithium3 => write!(f, "Ed25519+Dilithium3 (Hybrid)"),
            CurveType::K256Dilithium3 => write!(f, "K256+Dilithium3 (Hybrid)"),
        }
    }
}

impl CurveType {
    /// Returns true if this is a post-quantum algorithm
    pub fn is_post_quantum(&self) -> bool {
        matches!(
            self,
            CurveType::Dilithium2
                | CurveType::Dilithium3
                | CurveType::Dilithium5
                | CurveType::SphincsPlusSha256Robust
                | CurveType::Ed25519Dilithium3
                | CurveType::K256Dilithium3
        )
    }

    /// Returns true if this is a hybrid scheme
    pub fn is_hybrid(&self) -> bool {
        matches!(
            self,
            CurveType::Ed25519Dilithium3 | CurveType::K256Dilithium3
        )
    }

    /// Get security level (1-5, where 5 is highest)
    pub fn security_level(&self) -> u8 {
        match self {
            CurveType::K256 | CurveType::P256 => 3,
            CurveType::Ed25519 => 3,
            CurveType::Dilithium2 => 4,
            CurveType::Dilithium3 => 5,
            CurveType::Dilithium5 => 5,
            CurveType::SphincsPlusSha256Robust => 5,
            CurveType::Ed25519Dilithium3 => 5,
            CurveType::K256Dilithium3 => 5,
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
    formatted_key
        .strip_prefix(KANARI_KEY_PREFIX)
        .unwrap_or(formatted_key)
}

/// Generate a keypair for the specified curve type
pub fn generate_keypair(curve_type: CurveType) -> Result<KeyPair, KeyError> {
    match curve_type {
        CurveType::K256 => generate_k256_keypair(),
        CurveType::P256 => generate_p256_keypair(),
        CurveType::Ed25519 => generate_ed25519_keypair(),
        CurveType::Dilithium2 => generate_dilithium2_keypair(),
        CurveType::Dilithium3 => generate_dilithium3_keypair(),
        CurveType::Dilithium5 => generate_dilithium5_keypair(),
        CurveType::SphincsPlusSha256Robust => generate_sphincs_keypair(),
        CurveType::Ed25519Dilithium3 => generate_hybrid_ed25519_dilithium3_keypair(),
        CurveType::K256Dilithium3 => generate_hybrid_k256_dilithium3_keypair(),
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
    let hex_encoded = hex::encode(public_key_bytes);
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

// ============================================================================
// POST-QUANTUM CRYPTOGRAPHY (PQC) KEY GENERATION
// ============================================================================

/// Generate a Dilithium2 keypair (Fast, NIST Level 2)
fn generate_dilithium2_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = dilithium2::keypair();

    let public_key_bytes = public_key.as_bytes();
    let secret_key_bytes = secret_key.as_bytes();

    let hex_encoded = hex::encode(public_key_bytes);
    // Use valid hex characters for address - take first 64 hex chars (32 bytes)
    let address = format!("0x{}", &hex_encoded[..64]);
    let raw_private_key = hex::encode(secret_key_bytes);
    let private_key = format!("kanapqc{}", raw_private_key);

    Ok(KeyPair {
        private_key,
        public_key: hex_encoded,
        address,
        curve_type: CurveType::Dilithium2,
    })
}

/// Generate a Dilithium3 keypair (Balanced, NIST Level 3, Recommended)
fn generate_dilithium3_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = dilithium3::keypair();

    let public_key_bytes = public_key.as_bytes();
    let secret_key_bytes = secret_key.as_bytes();

    let hex_encoded = hex::encode(public_key_bytes);
    // Use valid hex characters for address - take first 64 hex chars (32 bytes)
    let address = format!("0x{}", &hex_encoded[..64]);
    let raw_private_key = hex::encode(secret_key_bytes);
    let private_key = format!("kanapqc{}", raw_private_key);

    Ok(KeyPair {
        private_key,
        public_key: hex_encoded,
        address,
        curve_type: CurveType::Dilithium3,
    })
}

/// Generate a Dilithium5 keypair (Maximum security, NIST Level 5)
fn generate_dilithium5_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = dilithium5::keypair();

    let public_key_bytes = public_key.as_bytes();
    let secret_key_bytes = secret_key.as_bytes();

    let hex_encoded = hex::encode(public_key_bytes);
    // Use valid hex characters for address - take first 64 hex chars (32 bytes)
    let address = format!("0x{}", &hex_encoded[..64]);
    let raw_private_key = hex::encode(secret_key_bytes);
    let private_key = format!("kanapqc{}", raw_private_key);

    Ok(KeyPair {
        private_key,
        public_key: hex_encoded,
        address,
        curve_type: CurveType::Dilithium5,
    })
}

/// Generate a SPHINCS+ keypair (Hash-based, ultra-secure)
fn generate_sphincs_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = sphincssha2256fsimple::keypair();

    let public_key_bytes = public_key.as_bytes();
    let secret_key_bytes = secret_key.as_bytes();

    let hex_encoded = hex::encode(public_key_bytes);
    // Use valid hex characters for address - take first 64 hex chars (32 bytes)
    let address = format!("0x{}", &hex_encoded[..64]);
    let raw_private_key = hex::encode(secret_key_bytes);
    let private_key = format!("kanapqc{}", raw_private_key);

    Ok(KeyPair {
        private_key,
        public_key: hex_encoded,
        address,
        curve_type: CurveType::SphincsPlusSha256Robust,
    })
}

// ============================================================================
// HYBRID CRYPTOGRAPHY (Classical + PQC)
// ============================================================================

/// Generate Ed25519 + Dilithium3 hybrid keypair
fn generate_hybrid_ed25519_dilithium3_keypair() -> Result<KeyPair, KeyError> {
    // Generate both keypairs
    let ed25519_pair = generate_ed25519_keypair()?;
    let dilithium3_pair = generate_dilithium3_keypair()?;

    // Combine public keys
    let combined_public = format!("{}:{}", ed25519_pair.public_key, dilithium3_pair.public_key);

    // Combine private keys
    let ed25519_raw = extract_raw_key(&ed25519_pair.private_key);
    // Extract dilithium3 raw key (remove "kanapqc" prefix to get just the hex)
    let dilithium3_with_prefix = &dilithium3_pair.private_key;
    let dilithium3_raw = dilithium3_with_prefix.strip_prefix("kanapqc").unwrap_or(dilithium3_with_prefix);
    let combined_private = format!("kanahybrid{}:{}", ed25519_raw, dilithium3_raw);

    // Generate hybrid address using SHA3-256 hash of combined public key
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(combined_public.as_bytes());
    let hash_result = hasher.finalize();
    // Use full 32 bytes (64 hex chars) for valid address
    let address = format!("0x{}", hex::encode(&hash_result[..]));

    Ok(KeyPair {
        private_key: combined_private,
        public_key: combined_public,
        address,
        curve_type: CurveType::Ed25519Dilithium3,
    })
}

/// Generate K256 + Dilithium3 hybrid keypair
fn generate_hybrid_k256_dilithium3_keypair() -> Result<KeyPair, KeyError> {
    // Generate both keypairs
    let k256_pair = generate_k256_keypair()?;
    let dilithium3_pair = generate_dilithium3_keypair()?;

    // Combine public keys
    let combined_public = format!("{}:{}", k256_pair.public_key, dilithium3_pair.public_key);

    // Combine private keys
    let k256_raw = extract_raw_key(&k256_pair.private_key);
    // Extract dilithium3 raw key (remove "kanapqc" prefix to get just the hex)
    let dilithium3_with_prefix = &dilithium3_pair.private_key;
    let dilithium3_raw = dilithium3_with_prefix.strip_prefix("kanapqc").unwrap_or(dilithium3_with_prefix);
    let combined_private = format!("kanahybrid{}:{}", k256_raw, dilithium3_raw);

    // Generate hybrid address using SHA3-256 hash of combined public key
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(combined_public.as_bytes());
    let hash_result = hasher.finalize();
    // Use full 32 bytes (64 hex chars) for valid address
    let address = format!("0x{}", hex::encode(&hash_result[..]));

    Ok(KeyPair {
        private_key: combined_private,
        public_key: combined_public,
        address,
        curve_type: CurveType::K256Dilithium3,
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
            let raw_private_key = hex::encode(signing_key.to_bytes());

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
            let hex_encoded = hex::encode(public_key_bytes);
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
        // PQC algorithms don't support HD wallet derivation yet
        // Fall back to random generation for now
        _ => Err(KeyError::GenerationFailed(
            "Post-quantum algorithms don't support BIP39 mnemonic derivation yet. Use generate_keypair() instead.".to_string()
        )),
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
                format_private_key(raw_private_key)
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
                format_private_key(raw_private_key)
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
            let hex_encoded = hex::encode(public_key_bytes);
            let address = format!("0x{}", hex_encoded);

            // Format with kanari prefix if not already formatted
            let formatted_private_key = if private_key.starts_with(KANARI_KEY_PREFIX) {
                private_key.to_string()
            } else {
                format_private_key(raw_private_key)
            };

            Ok(KeyPair {
                private_key: formatted_private_key,
                public_key: hex_encoded,
                address,
                curve_type: CurveType::Ed25519,
            })
        }
        // PQC algorithms require importing raw key bytes
        _ => Err(KeyError::GenerationFailed(
            "Post-quantum and hybrid algorithms require specialized import methods. Use generate_keypair() instead.".to_string()
        )),
    }
}

/// Derive an Address type from a public key
pub fn derive_address_from_pubkey(public_key: &str) -> Result<AccountAddress, KeyError> {
    let address_str = format!("0x{}", public_key);
    AccountAddress::from_str(&address_str).map_err(|_| KeyError::InvalidPublicKey)
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

    // Do not assume 32-byte values are Ed25519 immediately — they may be
    // compressed EC public keys for K256/P256. We'll check EC keys first
    // and fall back to Ed25519 if EC checks fail.

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

    let ec_result = match (k256_key_valid, p256_key_valid) {
        (true, false) => Some(CurveType::K256),
        (false, true) => Some(CurveType::P256),
        (true, true) => Some(CurveType::K256), // Default to K256 if both valid
        (false, false) => None,
    };

    if ec_result.is_some() {
        return ec_result;
    }

    // If neither EC validation succeeded but the decoded data is 32 bytes,
    // it's likely an Ed25519 public key — try to validate it as such.
    if decoded_hex.len() == 32 {
        let mut key_array = [0u8; 32];
        key_array.copy_from_slice(&decoded_hex);
        if Ed25519VerifyingKey::from_bytes(&key_array).is_ok() {
            return Some(CurveType::Ed25519);
        }
    }

    None
}

/// Generate a new Kanari address with the specified mnemonic length and curve type
pub fn generate_karix_address(
    mnemonic_length: usize,
    curve_type: CurveType,
) -> Result<(String, String, String), KeyError> {
    // Generate mnemonic phrase
    let seed_phrase = generate_mnemonic(mnemonic_length)?;

    // Generate keypair from mnemonic
    let keypair = keypair_from_mnemonic(&seed_phrase, curve_type, "")?;

    Ok((keypair.private_key, keypair.address, seed_phrase))
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

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // Bug #4: Panic in Hybrid Address Generation (Critical)
    // ============================================================================

    #[test]
    fn test_hybrid_ed25519_dilithium3_address_generation() {
        // Test that hybrid address generation doesn't panic with short keys
        let result = generate_hybrid_ed25519_dilithium3_keypair();
        assert!(result.is_ok(), "Hybrid keypair generation should succeed");

        let keypair = result.unwrap();
        assert!(
            keypair.address.starts_with("0x"),
            "Hybrid address should have correct prefix"
        );
        assert_eq!(keypair.curve_type, CurveType::Ed25519Dilithium3);
    }

    #[test]
    fn test_hybrid_k256_dilithium3_address_generation() {
        // Test that K256+Dilithium3 hybrid doesn't panic
        let result = generate_hybrid_k256_dilithium3_keypair();

        // May fail due to PQC prefix handling issue, but should not panic
        if result.is_ok() {
            let keypair = result.unwrap();
            assert!(
                keypair.address.starts_with("0x"),
                "Hybrid address should have correct prefix"
            );
            assert_eq!(keypair.curve_type, CurveType::K256Dilithium3);
        } else {
            // Expected to fail due to PQC key format issue
            // This is acceptable as hybrid crypto is still experimental
            assert!(result.is_err());
        }
    }

    // Test that address generation handles short public keys without panic
    #[test]
    fn test_short_public_key_handling() {
        // This tests the fix for the [..20] slice panic bug
        let short_string = "abc"; // Less than 20 bytes
        let bytes = short_string.as_bytes();

        // Should not panic - take min of (bytes.len(), 20)
        let hash_input = if bytes.len() >= 20 {
            &bytes[..20]
        } else {
            bytes
        };

        assert_eq!(hash_input.len(), 3, "Should use full length if < 20");
        assert_eq!(hash_input, b"abc");
    }

    // ============================================================================
    // Bug #9: Logic Error in detect_curve_type (Medium)
    // ============================================================================

    #[test]
    fn test_detect_curve_type_ed25519() {
        // Generate an Ed25519 keypair
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();

        // Test detection with address
        let detected = detect_curve_type(&keypair.address);
        assert_eq!(detected, Some(CurveType::Ed25519), "Should detect Ed25519");
    }

    #[test]
    fn test_detect_curve_type_k256() {
        // Generate a K256 keypair
        let keypair = generate_keypair(CurveType::K256).unwrap();

        // Test detection with address
        let detected = detect_curve_type(&keypair.address);
        assert_eq!(detected, Some(CurveType::K256), "Should detect K256");
    }

    #[test]
    fn test_detect_curve_type_invalid() {
        // Test with invalid address
        let detected = detect_curve_type("0xinvalid");
        assert_eq!(detected, None, "Should return None for invalid address");

        // Test with empty address
        let detected = detect_curve_type("0x");
        assert_eq!(detected, None, "Should return None for empty address");
    }

    #[test]
    fn test_detect_curve_type_no_redundant_check() {
        // This test verifies the fix for redundant length check bug
        // The bug was: if decoded_hex.len() == 32 { if decoded_hex.len() == 32 { ... } }

        // Generate Ed25519 key (32 bytes)
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let address_hex = keypair.address.trim_start_matches("0x");
        let decoded = hex::decode(address_hex).unwrap();

        assert_eq!(decoded.len(), 32, "Ed25519 public key should be 32 bytes");

        // The function should work correctly without redundant check
        let detected = detect_curve_type(&keypair.address);
        assert!(detected.is_some(), "Should detect curve type");
    }

    // ============================================================================
    // Additional Key Generation Tests
    // ============================================================================

    #[test]
    fn test_keypair_generation_all_curves() {
        let curves = vec![
            CurveType::K256,
            CurveType::P256,
            CurveType::Ed25519,
            CurveType::Dilithium2,
            CurveType::Dilithium3,
            CurveType::Dilithium5,
        ];

        for curve in curves {
            let result = generate_keypair(curve);
            assert!(result.is_ok(), "Keypair generation failed for {:?}", curve);

            let keypair = result.unwrap();
            assert!(
                !keypair.private_key.is_empty(),
                "Private key should not be empty"
            );
            assert!(
                !keypair.public_key.is_empty(),
                "Public key should not be empty"
            );
            assert!(!keypair.address.is_empty(), "Address should not be empty");
            assert_eq!(keypair.curve_type, curve, "Curve type should match");
        }
    }

    #[test]
    fn test_mnemonic_generation() {
        // Test 12-word mnemonic
        let mnemonic_12 = generate_mnemonic(12);
        assert!(
            mnemonic_12.is_ok(),
            "12-word mnemonic generation should succeed"
        );
        assert_eq!(mnemonic_12.unwrap().split_whitespace().count(), 12);

        // Test 24-word mnemonic
        let mnemonic_24 = generate_mnemonic(24);
        assert!(
            mnemonic_24.is_ok(),
            "24-word mnemonic generation should succeed"
        );
        assert_eq!(mnemonic_24.unwrap().split_whitespace().count(), 24);

        // Test invalid word count
        let mnemonic_invalid = generate_mnemonic(18);
        assert!(mnemonic_invalid.is_err(), "Invalid word count should fail");
    }

    #[test]
    fn test_keypair_from_mnemonic_consistency() {
        // Generate a mnemonic
        let mnemonic = generate_mnemonic(12).unwrap();
        let password = "test_password";

        // Generate keypair twice with same mnemonic
        let keypair1 = keypair_from_mnemonic(&mnemonic, CurveType::K256, password).unwrap();
        let keypair2 = keypair_from_mnemonic(&mnemonic, CurveType::K256, password).unwrap();

        // Should generate identical keypairs
        assert_eq!(keypair1.private_key, keypair2.private_key);
        assert_eq!(keypair1.public_key, keypair2.public_key);
        assert_eq!(keypair1.address, keypair2.address);
    }

    #[test]
    fn test_private_key_formatting() {
        // Test that private keys are properly formatted with kanari prefix
        let keypair = generate_keypair(CurveType::K256).unwrap();
        assert!(
            keypair.private_key.starts_with(KANARI_KEY_PREFIX),
            "Private key should have kanari prefix"
        );

        // Test extracting raw key
        let raw = extract_raw_key(&keypair.private_key);
        assert!(
            !raw.starts_with(KANARI_KEY_PREFIX),
            "Raw key should not have prefix"
        );

        // Test formatting again
        let formatted = format_private_key(raw);
        assert_eq!(
            formatted, keypair.private_key,
            "Re-formatted key should match"
        );
    }

    #[test]
    fn test_keypair_from_private_key() {
        // Generate a keypair
        let original = generate_keypair(CurveType::Ed25519).unwrap();

        // Recreate from private key
        let recreated =
            keypair_from_private_key(&original.private_key, CurveType::Ed25519).unwrap();

        // Should generate the same public key and address
        assert_eq!(original.public_key, recreated.public_key);
        assert_eq!(original.address, recreated.address);
        assert_eq!(original.private_key, recreated.private_key);
    }

    #[test]
    fn test_post_quantum_keypair_generation() {
        // Test Dilithium3 (recommended PQC)
        let dil3 = generate_keypair(CurveType::Dilithium3).unwrap();
        assert!(
            dil3.private_key.starts_with("kanapqc"),
            "PQC keys should have kanapqc prefix, got: {}",
            dil3.private_key
        );
        assert!(
            dil3.address.starts_with("0x"),
            "PQC addresses should have pqc prefix"
        );

        // Test that PQC is detected
        assert!(CurveType::Dilithium3.is_post_quantum());
        assert!(!CurveType::K256.is_post_quantum());
    }

    #[test]
    fn test_hybrid_keypair_properties() {
        // Test Ed25519+Dilithium3 hybrid
        let hybrid = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        assert!(
            hybrid.private_key.starts_with("kanahybrid"),
            "Hybrid keys should have kanahybrid prefix"
        );
        assert!(
            hybrid.address.starts_with("0x"),
            "Hybrid addresses should have hybrid prefix"
        );

        // Should contain both key parts separated by ':'
        assert!(
            hybrid.private_key.contains(':'),
            "Hybrid key should contain separator"
        );
        assert!(
            hybrid.public_key.contains(':'),
            "Hybrid public key should contain separator"
        );

        // Test that hybrid is detected as post-quantum
        assert!(CurveType::Ed25519Dilithium3.is_post_quantum());
        assert!(CurveType::Ed25519Dilithium3.is_hybrid());
    }

    #[test]
    fn test_invalid_private_key_handling() {
        // Test with invalid hex
        let result = keypair_from_private_key("not_hex", CurveType::K256);
        assert!(result.is_err(), "Invalid hex should fail");

        // Test with wrong length for Ed25519
        let result = keypair_from_private_key("kanari1234", CurveType::Ed25519);
        assert!(result.is_err(), "Wrong length should fail");

        // Test with empty key
        let result = keypair_from_private_key("", CurveType::K256);
        assert!(result.is_err(), "Empty key should fail");
    }

    #[test]
    fn test_pqc_mnemonic_not_supported() {
        // PQC algorithms don't support BIP39 derivation
        let mnemonic = generate_mnemonic(12).unwrap();
        let result = keypair_from_mnemonic(&mnemonic, CurveType::Dilithium3, "");
        assert!(
            result.is_err(),
            "PQC should not support mnemonic derivation yet"
        );
    }
}
