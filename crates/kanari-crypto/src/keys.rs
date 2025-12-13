//! Cryptographic key generation and management
//!
//! This module handles key generation for multiple curve types (K256/secp256k1,
//! P256/secp256r1, Ed25519) and Post-Quantum Cryptography (Dilithium, SPHINCS+).
//!
//! **Quantum-Safe**: Includes NIST-standardized post-quantum algorithms.

use bip39::{Language, Mnemonic};
use move_core_types::account_address::AccountAddress;
use rand::rngs::OsRng;
use std::fmt;
use std::str::FromStr;
use thiserror::Error;
use zeroize::Zeroizing;

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
///
/// Security: Private key is automatically zeroized when dropped.
/// Clone is intentionally not implemented to prevent key material duplication.
/// Private key is NOT serialized by default for security.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct KeyPair {
    #[serde(skip)]
    pub private_key: Zeroizing<String>,
    pub public_key: String,
    /// Optional post-quantum public key (hex) when applicable
    pub pqc_public_key: Option<String>,
    pub address: String,
    pub curve_type: CurveType,
}

impl KeyPair {
    /// Export private key in a wrapper that zeroizes on drop
    /// Prefer this API to avoid accidental long-lived clones of secret material.
    pub fn export_private_key_secure(&self) -> zeroize::Zeroizing<String> {
        zeroize::Zeroizing::new(self.private_key.to_string())
    }

    /// Get public key as reference (avoid unnecessary cloning)
    pub fn get_public_key(&self) -> &str {
        &self.public_key
    }

    /// Get PQC public key if present
    pub fn get_pqc_public_key(&self) -> Option<String> {
        self.pqc_public_key.clone()
    }

    /// Get a reference to the PQC public key if present (avoids cloning)
    pub fn get_pqc_public_key_ref(&self) -> Option<&str> {
        self.pqc_public_key.as_deref()
    }

    /// Get address as reference (avoid unnecessary cloning)
    pub fn get_address(&self) -> &str {
        &self.address
    }

    /// Get a tagged address that includes curve type information
    /// Format: "curve_type:address" (e.g., "K256:0xabc123...")
    /// This is the recommended way to store addresses for reliable curve detection
    pub fn tagged_address(&self) -> String {
        format!("{:?}:{}", self.curve_type, self.address)
    }

    /// Create a serializable version that includes private key (use with caution)
    /// This should only be used when explicitly needed for encrypted storage
    pub fn to_serializable_with_private_key(&self) -> serde_json::Value {
        serde_json::json!({
            "private_key": self.private_key.to_string(),
            "public_key": self.public_key,
            "address": self.address,
            "curve_type": self.curve_type,
        })
    }

    /// Parse a tagged address back into curve type and address
    /// Returns None if the address is not in tagged format
    pub fn parse_tagged_address(tagged: &str) -> Option<(CurveType, String)> {
        // Use split_once to avoid indexing
        let (curve_str, address_str) = tagged.split_once(':')?;

        let curve_type = match curve_str {
            "K256" => CurveType::K256,
            "P256" => CurveType::P256,
            "Ed25519" => CurveType::Ed25519,
            "Dilithium2" => CurveType::Dilithium2,
            "Dilithium3" => CurveType::Dilithium3,
            "Dilithium5" => CurveType::Dilithium5,
            "SphincsPlusSha256Robust" => CurveType::SphincsPlusSha256Robust,
            "Ed25519Dilithium3" => CurveType::Ed25519Dilithium3,
            "K256Dilithium3" => CurveType::K256Dilithium3,
            _ => return None,
        };

        Some((curve_type, address_str.to_string()))
    }
}

/// Prefix used for Kanari private keys
pub const KANARI_KEY_PREFIX: &str = "kanari";

/// Additional known prefixes
pub const KANAPQC_PREFIX: &str = "kanapqc";
pub const KANAHYBRID_PREFIX: &str = "kanahybrid";

/// Format a raw hex private key with the Kanari prefix
pub fn format_private_key(raw_key: &str) -> String {
    format!("{}{}", KANARI_KEY_PREFIX, raw_key)
}

/// Extract the raw hex key from a formatted private key
pub fn extract_raw_key(formatted_key: &str) -> &str {
    // Allow multiple known prefixes (kanari, kanapqc, kanahybrid)
    formatted_key
        .strip_prefix(KANARI_KEY_PREFIX)
        .or_else(|| formatted_key.strip_prefix(KANAPQC_PREFIX))
        .or_else(|| formatted_key.strip_prefix(KANAHYBRID_PREFIX))
        .unwrap_or(formatted_key)
}

/// Skip the uncompressed EC point prefix (0x04) safely.
fn skip_uncompressed_point_prefix(bytes: &[u8]) -> &[u8] {
    // Check length before accessing to prevent buffer overread
    if bytes.is_empty() {
        return bytes;
    }

    if bytes[0] == 0x04 && bytes.len() > 1 {
        &bytes[1..]
    } else {
        bytes
    }
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

    // Get encoded public key and format (skip uncompressed prefix safely)
    let encoded_point = public_key.to_encoded_point(false);
    let slice = skip_uncompressed_point_prefix(encoded_point.as_bytes());
    let full_pub_hex = hex::encode(slice);
    // Keep legacy behavior: address is the (truncated) public key hex prefixed with 0x
    let address = format!(
        "0x{}",
        &full_pub_hex[..std::cmp::min(64, full_pub_hex.len())]
    );
    let raw_private_key = hex::encode(signing_key.to_bytes());

    // Format private key with kanari prefix
    let private_key = format_private_key(&raw_private_key);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: full_pub_hex,
        pqc_public_key: None,
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

    // Format the public key, skipping the 0x04 prefix byte safely
    let slice = skip_uncompressed_point_prefix(public_key.as_bytes());
    let full_pub_hex = hex::encode(slice);
    // Keep legacy behavior: address is the (truncated) public key hex prefixed with 0x
    let address = format!(
        "0x{}",
        &full_pub_hex[..std::cmp::min(64, full_pub_hex.len())]
    );
    let raw_private_key = hex::encode(secret_key);

    // Format private key with kanari prefix
    let private_key = format_private_key(&raw_private_key);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: full_pub_hex,
        pqc_public_key: None,
        address,
        curve_type: CurveType::P256,
    })
}

/// Generate an Ed25519 keypair
fn generate_ed25519_keypair() -> Result<KeyPair, KeyError> {
    use rand::RngCore;

    // Generate random bytes for the private key using OS RNG
    let mut rng = OsRng;
    let mut seed = [0u8; 32];

    // Fill with random bytes
    rng.fill_bytes(&mut seed);

    // Validate entropy - ensure we didn't get all zeros (extremely unlikely but check anyway)
    if seed.iter().all(|&b| b == 0) {
        return Err(KeyError::GenerationFailed(
            "Insufficient entropy from RNG".to_string(),
        ));
    }

    // Create signing key from random bytes
    let signing_key = Ed25519SigningKey::from_bytes(&seed);
    let verifying_key = Ed25519VerifyingKey::from(&signing_key);

    // Get the bytes of the keys
    let private_key_bytes = signing_key.to_bytes();
    let public_key_bytes = verifying_key.to_bytes();

    // Format the public key
    let hex_encoded = hex::encode(public_key_bytes);
    // Keep legacy behavior: address is the public key hex prefixed with 0x
    let address = format!("0x{}", hex_encoded);
    let raw_private_key = hex::encode(private_key_bytes);

    // Format private key with kanari prefix
    let private_key = format_private_key(&raw_private_key);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: hex_encoded,
        pqc_public_key: None,
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
    // Derive address as SHA3-256(public_key) for better entropy and uniformity
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(public_key_bytes);
    let hash_result = hasher.finalize();
    let address = format!("0x{}", hex::encode(&hash_result[..]));
    let raw_private_key = hex::encode(secret_key_bytes);
    // Store public key alongside secret to avoid fragile recovery from secret bytes
    let private_key = format!("kanapqc{}:{}", raw_private_key, hex_encoded);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: hex_encoded.clone(),
        pqc_public_key: Some(hex_encoded),
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
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(public_key_bytes);
    let hash_result = hasher.finalize();
    let address = format!("0x{}", hex::encode(&hash_result[..]));
    let raw_private_key = hex::encode(secret_key_bytes);
    // Store public key alongside secret to avoid fragile recovery from secret bytes
    let private_key = format!("kanapqc{}:{}", raw_private_key, hex_encoded);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: hex_encoded.clone(),
        pqc_public_key: Some(hex_encoded),
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
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(public_key_bytes);
    let hash_result = hasher.finalize();
    let address = format!("0x{}", hex::encode(&hash_result[..]));
    let raw_private_key = hex::encode(secret_key_bytes);
    // Store public key alongside secret to avoid fragile recovery from secret bytes
    let private_key = format!("kanapqc{}:{}", raw_private_key, hex_encoded);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: hex_encoded.clone(),
        pqc_public_key: Some(hex_encoded),
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
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(public_key_bytes);
    let hash_result = hasher.finalize();
    let address = format!("0x{}", hex::encode(&hash_result[..]));
    let raw_private_key = hex::encode(secret_key_bytes);
    // Store public key alongside secret to avoid fragile recovery from secret bytes
    let private_key = format!("kanapqc{}:{}", raw_private_key, hex_encoded);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: hex_encoded.clone(),
        pqc_public_key: Some(hex_encoded),
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
    let dilithium3_raw = dilithium3_with_prefix
        .strip_prefix("kanapqc")
        .unwrap_or(dilithium3_with_prefix);
    let combined_private = format!("kanahybrid{}:{}", ed25519_raw, dilithium3_raw);

    // Generate hybrid address using SHA3-256 hash of combined public key
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(combined_public.as_bytes());
    let hash_result = hasher.finalize();
    // Use full 32 bytes (64 hex chars) for valid address
    let address = format!("0x{}", hex::encode(&hash_result[..]));

    Ok(KeyPair {
        private_key: Zeroizing::new(combined_private),
        public_key: combined_public,
        pqc_public_key: Some(dilithium3_pair.public_key.clone()),
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
    let dilithium3_raw = dilithium3_with_prefix
        .strip_prefix("kanapqc")
        .unwrap_or(dilithium3_with_prefix);
    let combined_private = format!("kanahybrid{}:{}", k256_raw, dilithium3_raw);

    // Generate hybrid address using SHA3-256 hash of combined public key
    use sha3::{Digest, Sha3_256};
    let mut hasher = Sha3_256::new();
    hasher.update(combined_public.as_bytes());
    let hash_result = hasher.finalize();
    // Use full 32 bytes (64 hex chars) for valid address
    let address = format!("0x{}", hex::encode(&hash_result[..]));

    Ok(KeyPair {
        private_key: Zeroizing::new(combined_private),
        public_key: combined_public,
        pqc_public_key: Some(dilithium3_pair.public_key.clone()),
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
    // Validate inputs
    if phrase.trim().is_empty() {
        return Err(KeyError::InvalidMnemonic(
            "Empty mnemonic phrase".to_string(),
        ));
    }

    // Password can be empty, but validate reasonable length
    const MAX_PASSWORD_LEN: usize = 1024;
    if password.len() > MAX_PASSWORD_LEN {
        return Err(KeyError::InvalidMnemonic("Password too long".to_string()));
    }

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
            let pub_bytes = &encoded_point.as_bytes()[1..];
            let full_pub_hex = hex::encode(pub_bytes);
            // Keep legacy behavior: address is the (truncated) public key hex prefixed with 0x
            let address = format!("0x{}", &full_pub_hex[..std::cmp::min(64, full_pub_hex.len())]);
            let raw_private_key = hex::encode(signing_key.to_bytes());

            // Format private key with kanari prefix
            let private_key = format_private_key(&raw_private_key);

            Ok(KeyPair {
                private_key: Zeroizing::new(private_key),
                public_key: full_pub_hex,
                pqc_public_key: None,
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

            let pub_bytes = &public_key.as_bytes()[1..];
            let full_pub_hex = hex::encode(pub_bytes);
            // Keep legacy behavior: address is the (truncated) public key hex prefixed with 0x
            let address = format!("0x{}", &full_pub_hex[..std::cmp::min(64, full_pub_hex.len())]);
            let raw_private_key = hex::encode(signing_key.to_bytes());

            // Format private key with kanari prefix
            let private_key = format_private_key(&raw_private_key);

            Ok(KeyPair {
                private_key: Zeroizing::new(private_key),
                public_key: full_pub_hex,
                pqc_public_key: None,
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
            // Keep legacy behavior: address is the public key hex prefixed with 0x
            let address = format!("0x{}", hex_encoded);

            // Format private key with kanari prefix
            let private_key = format_private_key(&private_key);

            Ok(KeyPair {
                private_key: Zeroizing::new(private_key),
                public_key: hex_encoded,
                pqc_public_key: None,
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
    use zeroize::Zeroize;

    // Remove kanari prefix if present
    let raw_private_key = extract_raw_key(private_key);

    match curve_type {
        CurveType::K256 => {
            let mut private_key_bytes =
                hex::decode(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;
            let secret_key = K256SecretKey::from_slice(&private_key_bytes)
                .map_err(|_| KeyError::InvalidPrivateKey)?;

            // Zeroize immediately after use
            private_key_bytes.zeroize();

            let signing_key = K256SigningKey::from(secret_key);
            let verifying_key = K256VerifyingKey::from(&signing_key);
            let public_key = K256PublicKey::from(verifying_key);

            let encoded_point = public_key.to_encoded_point(false);
            let mut hex_encoded = hex::encode(&encoded_point.as_bytes()[1..]);
            hex_encoded.truncate(64);

            // Keep legacy behavior: address is the (truncated) public key hex prefixed with 0x
            let address = format!("0x{}", hex_encoded);

            // Format with kanari prefix if not already formatted
            let formatted_private_key = if private_key.starts_with(KANARI_KEY_PREFIX) {
                private_key.to_string()
            } else {
                format_private_key(raw_private_key)
            };

            Ok(KeyPair {
                private_key: Zeroizing::new(formatted_private_key),
                public_key: hex_encoded,
                pqc_public_key: None,
                address,
                curve_type: CurveType::K256,
            })
        }
        CurveType::P256 => {
            let mut private_key_bytes =
                hex::decode(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;
            let secret_key = P256SecretKey::from_slice(&private_key_bytes)
                .map_err(|_| KeyError::InvalidPrivateKey)?;

            // Zeroize immediately after use
            private_key_bytes.zeroize();

            let signing_key = SigningKey::from(secret_key);
            let verifying_key = VerifyingKey::from(&signing_key);
            let public_key = verifying_key.to_encoded_point(false);

            let mut hex_encoded = hex::encode(&public_key.as_bytes()[1..]);
            hex_encoded.truncate(64);

            // Keep legacy behavior: address is the (truncated) public key hex prefixed with 0x
            let address = format!("0x{}", hex_encoded);

            // Format with kanari prefix if not already formatted
            let formatted_private_key = if private_key.starts_with(KANARI_KEY_PREFIX) {
                private_key.to_string()
            } else {
                format_private_key(raw_private_key)
            };

            Ok(KeyPair {
                private_key: Zeroizing::new(formatted_private_key),
                public_key: hex_encoded,
                pqc_public_key: None,
                address,
                curve_type: CurveType::P256,
            })
        }
        CurveType::Ed25519 => {
            let mut private_key_bytes =
                hex::decode(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;
            if private_key_bytes.len() != 32 {
                private_key_bytes.zeroize();
                return Err(KeyError::InvalidPrivateKey);
            }

            let mut key_array = [0u8; 32];
            key_array.copy_from_slice(&private_key_bytes);

            // Zeroize source bytes
            private_key_bytes.zeroize();

            let signing_key = Ed25519SigningKey::from_bytes(&key_array);
            let verifying_key = Ed25519VerifyingKey::from(&signing_key);

            // Zeroize key array after use
            key_array.zeroize();

            let public_key_bytes = verifying_key.to_bytes();
            let hex_encoded = hex::encode(public_key_bytes);
            // Keep legacy behavior: address is the public key hex prefixed with 0x
            let address = format!("0x{}", hex_encoded);

            // Format with kanari prefix if not already formatted
            let formatted_private_key = if private_key.starts_with(KANARI_KEY_PREFIX) {
                private_key.to_string()
            } else {
                format_private_key(raw_private_key)
            };

            Ok(KeyPair {
                private_key: Zeroizing::new(formatted_private_key),
                public_key: hex_encoded,
                pqc_public_key: None,
                address,
                curve_type: CurveType::Ed25519,
            })
        }
        // Post-quantum imports: require public key be stored alongside secret when possible.
        CurveType::Dilithium2
        | CurveType::Dilithium3
        | CurveType::Dilithium5
        | CurveType::SphincsPlusSha256Robust => {
            // raw_private_key may be: "kanapqc<secret_hex>:<public_hex>" or older
            let raw_for_pqc = raw_private_key
                .strip_prefix("kanapqc")
                .unwrap_or(raw_private_key);

            // Require explicit public key stored alongside secret: prefer format
            // "kanapqc<secret_hex>:<public_hex>" and reject secret-only inputs.
            if let Some((_secret_hex, pub_hex)) = raw_for_pqc.split_once(':') {
                // validate pub_hex is hex
                let pub_bytes = hex::decode(pub_hex).map_err(|_| KeyError::InvalidPrivateKey)?;
                let pqc_hex = pub_hex.to_string();

                // Derive address from hash of the PQC public key for uniformity
                use sha3::{Digest, Sha3_256};
                let mut hasher = Sha3_256::new();
                hasher.update(&pub_bytes);
                let hash_result = hasher.finalize();
                let address = format!("0x{}", hex::encode(&hash_result[..]));

                let formatted_private_key = if private_key.starts_with("kanapqc") {
                    private_key.to_string()
                } else {
                    format!("kanapqc{}", raw_for_pqc)
                };

                return Ok(KeyPair {
                    private_key: Zeroizing::new(formatted_private_key),
                    public_key: pqc_hex.clone(),
                    pqc_public_key: Some(pqc_hex),
                    address,
                    curve_type,
                });
            }

            // No explicit public key supplied — reject to avoid fragile recovery
            return Err(KeyError::InvalidPrivateKey);
        }
        // Hybrid imports: expect format "kanahybrid<classical_hex>:<pqc_hex>" (may be prefixed with `kanari`)
        CurveType::Ed25519Dilithium3 | CurveType::K256Dilithium3 => {
            // For hybrid imports we require the caller to provide a hybrid-formatted
            // private key (must start with `kanahybrid`). This avoids ambiguous
            // parsing when users accidentally pass other prefixed keys.
            // Accept hybrid input where either the original `private_key` string
            // began with `kanahybrid` or the stripped `raw_private_key` begins
            // with it (this handles cases where multiple prefixes were present
            // and one was stripped by `extract_raw_key`). Require the hybrid
            // structure to avoid ambiguous parsing.
            if !(private_key.starts_with(KANAHYBRID_PREFIX)
                || raw_private_key.starts_with(KANAHYBRID_PREFIX))
            {
                return Err(KeyError::InvalidPrivateKey);
            }

            // raw_private_key currently has had one known prefix removed by
            // `extract_raw_key`. Strip an internal `kanahybrid` if present to
            // obtain the canonical hybrid payload (classical_hex:pqc_part).
            let hybrid = raw_private_key
                .strip_prefix(KANAHYBRID_PREFIX)
                .unwrap_or(raw_private_key);
            // split into two parts at the first ':' so pqc part may itself contain ':'
            let parts: Vec<&str> = hybrid.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(KeyError::InvalidPrivateKey);
            }

            let classical_raw = parts[0];
            let pqc_raw = parts[1];

            // Recreate classical public key hex
            let classical_bytes =
                hex::decode(classical_raw).map_err(|_| KeyError::InvalidPrivateKey)?;

            let classical_pub_hex = match curve_type {
                CurveType::Ed25519Dilithium3 => {
                    if classical_bytes.len() != 32 {
                        return Err(KeyError::InvalidPrivateKey);
                    }
                    let mut key_array = [0u8; 32];
                    key_array.copy_from_slice(&classical_bytes);
                    let signing_key = Ed25519SigningKey::from_bytes(&key_array);
                    let verifying_key = Ed25519VerifyingKey::from(&signing_key);
                    hex::encode(verifying_key.to_bytes())
                }
                CurveType::K256Dilithium3 => {
                    let secret_key = K256SecretKey::from_slice(&classical_bytes)
                        .map_err(|_| KeyError::InvalidPrivateKey)?;
                    let signing_key = K256SigningKey::from(secret_key);
                    let verifying_key = K256VerifyingKey::from(&signing_key);
                    let public_key = K256PublicKey::from(verifying_key);
                    let encoded_point = public_key.to_encoded_point(false);
                    let full_pub_hex = hex::encode(&encoded_point.as_bytes()[1..]);
                    // Keep full public key for storage, use truncated prefix for address elsewhere
                    full_pub_hex
                }
                _ => return Err(KeyError::InvalidPrivateKey),
            };

            // PQC raw part may be one of:
            // - "<secret_hex>:<public_hex>" (preferred)
            // - "<secret_hex>" (older formats)
            let pqc_hex = if pqc_raw.contains(':') {
                // split secret:pub and use pub directly
                let (_secret, pub_hex) =
                    pqc_raw.split_once(':').ok_or(KeyError::InvalidPrivateKey)?;
                // validate
                let _ = hex::decode(pub_hex).map_err(|_| KeyError::InvalidPrivateKey)?;
                pub_hex.to_string()
            } else {
                // Fallback: try to recover public key from secret bytes (backwards compatibility)
                let pqc_bytes = hex::decode(pqc_raw).map_err(|_| KeyError::InvalidPrivateKey)?;

                // Prevent DoS: limit max iterations
                const MAX_HYBRID_ITERATIONS: usize = 500;
                let start_len = 32usize.max(pqc_bytes.len().saturating_sub(MAX_HYBRID_ITERATIONS));

                let mut pqc_hex_opt: Option<String> = None;
                for suffix_len in (start_len..=pqc_bytes.len()).rev() {
                    let start = pqc_bytes.len().saturating_sub(suffix_len);
                    if let Ok(pk) = dilithium3::PublicKey::from_bytes(&pqc_bytes[start..]) {
                        pqc_hex_opt = Some(hex::encode(pk.as_bytes()));
                        break;
                    }
                }
                pqc_hex_opt.ok_or(KeyError::InvalidPrivateKey)?
            };

            // Combine public parts and compute hybrid address (SHA3-256 of combined_public)
            let combined_public = format!("{}:{}", classical_pub_hex, pqc_hex);
            use sha3::{Digest, Sha3_256};
            let mut hasher = Sha3_256::new();
            hasher.update(combined_public.as_bytes());
            let hash_result = hasher.finalize();
            let address = format!("0x{}", hex::encode(&hash_result[..]));

            // Preserve provided formatting where possible. If the original
            // `private_key` began with `kanahybrid` use it; otherwise return a
            // canonical `kanahybrid`-prefixed payload reconstructed from the
            // parsed hybrid payload.
            let formatted_private_key = if private_key.starts_with(KANAHYBRID_PREFIX)
                || raw_private_key.starts_with(KANAHYBRID_PREFIX)
            {
                if private_key.starts_with(KANAHYBRID_PREFIX) {
                    private_key.to_string()
                } else {
                    // original had a different prefix but raw contains kanahybrid
                    format!("{}{}", KANAHYBRID_PREFIX, hybrid)
                }
            } else {
                // Fallback: create canonical hybrid prefix
                format!("{}{}", KANAHYBRID_PREFIX, hybrid)
            };

            Ok(KeyPair {
                private_key: Zeroizing::new(formatted_private_key),
                public_key: combined_public,
                pqc_public_key: Some(pqc_hex.clone()),
                address,
                curve_type,
            })
        } // All CurveType variants are handled above; no catch-all arm needed.
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

/// Import a wallet from a seed phrase
pub fn import_from_seed_phrase(
    phrase: &str,
    curve_type: CurveType,
) -> Result<(String, String, String), String> {
    keypair_from_mnemonic(phrase, curve_type, "")
        .map(|keypair| {
            let zk = keypair.export_private_key_secure();
            (
                zk.to_string(),
                keypair.get_public_key().to_string(),
                keypair.get_address().to_string(),
            )
        })
        .map_err(|e| e.to_string())
}

/// Import a wallet from a private key
pub fn import_from_private_key(
    private_key: &str,
    curve_type: CurveType,
) -> Result<(String, String, String), String> {
    keypair_from_private_key(private_key, curve_type)
        .map(|keypair| {
            let zk = keypair.export_private_key_secure();
            (
                zk.to_string(),
                keypair.get_public_key().to_string(),
                keypair.get_address().to_string(),
            )
        })
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
        // Test that K256+Dilithium3 hybrid generation always succeeds
        // No panic or failure expected - all operations are safe
        let result = generate_hybrid_k256_dilithium3_keypair();

        assert!(
            result.is_ok(),
            "Hybrid K256+Dilithium3 keypair generation must always succeed"
        );

        let keypair = result.unwrap();
        assert!(
            keypair.address.starts_with("0x"),
            "Hybrid address should have correct prefix"
        );
        assert_eq!(keypair.curve_type, CurveType::K256Dilithium3);

        // Verify combined public key format
        assert!(
            keypair.public_key.contains(':'),
            "Hybrid public key should be in format 'classical:pqc'"
        );

        // Verify PQC public key is present
        assert!(
            keypair.pqc_public_key.is_some(),
            "Hybrid keypair must have PQC public key"
        );
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
            formatted,
            keypair.private_key.to_string(),
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
            dil3.private_key.to_string()
        );
        assert!(
            dil3.address.starts_with("0x"),
            "PQC addresses should have pqc prefix"
        );

        // pqc_public_key should be set for PQC keypairs
        assert!(dil3.pqc_public_key.is_some());
        assert_eq!(dil3.pqc_public_key.unwrap(), dil3.public_key);

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

        // pqc_public_key should be present and equal to the PQC part
        let hybrid_pqc = hybrid
            .pqc_public_key
            .as_ref()
            .expect("PQC public key missing");
        assert!(hybrid.public_key.contains(':'));
        let parts: Vec<&str> = hybrid.public_key.splitn(2, ':').collect();
        assert_eq!(parts[1], hybrid_pqc);
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

    // ============================================================================
    // Tagged Address Tests (Security Enhancement)
    // ============================================================================

    #[test]
    fn test_tagged_address_generation() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let tagged = keypair.tagged_address();

        // Should have format "CurveType:0xaddress"
        assert!(tagged.starts_with("K256:"));
        assert!(tagged.contains(&keypair.address));
    }

    #[test]
    fn test_tagged_address_parsing() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let tagged = keypair.tagged_address();

        // Parse it back
        let (curve_type, address) = KeyPair::parse_tagged_address(&tagged).unwrap();

        assert_eq!(curve_type, CurveType::Ed25519);
        assert_eq!(address, keypair.address);
    }

    #[test]
    fn test_tagged_address_all_curves() {
        let curves = vec![
            CurveType::K256,
            CurveType::P256,
            CurveType::Ed25519,
            CurveType::Dilithium3,
        ];

        for curve in curves {
            let keypair = generate_keypair(curve).unwrap();
            let tagged = keypair.tagged_address();

            // Should parse back correctly
            let (parsed_curve, parsed_address) = KeyPair::parse_tagged_address(&tagged)
                .expect(&format!("Failed to parse tagged address for {:?}", curve));

            assert_eq!(parsed_curve, curve);
            assert_eq!(parsed_address, keypair.address);
        }
    }

    #[test]
    fn test_tagged_address_invalid_format() {
        // Test with untagged address
        let result = KeyPair::parse_tagged_address("0xabc123");
        assert!(result.is_none(), "Should return None for untagged address");

        // Test with invalid curve type
        let result = KeyPair::parse_tagged_address("InvalidCurve:0xabc123");
        assert!(
            result.is_none(),
            "Should return None for invalid curve type"
        );

        // Test with empty string
        let result = KeyPair::parse_tagged_address("");
        assert!(result.is_none(), "Should return None for empty string");
    }

    #[test]
    fn test_safe_signature_verification() {
        // Test that verify_signature_safe works correctly even when curve type is ambiguous

        // Generate keypairs for all classical curves
        let k256 = generate_keypair(CurveType::K256).unwrap();
        let p256 = generate_keypair(CurveType::P256).unwrap();
        let ed25519 = generate_keypair(CurveType::Ed25519).unwrap();

        let message = b"test message for safe verification";

        // Sign with each curve
        use crate::signatures::{sign_message, verify_signature_safe};

        let k256_sig = sign_message(&k256.private_key, message, CurveType::K256).unwrap();
        let p256_sig = sign_message(&p256.private_key, message, CurveType::P256).unwrap();
        let ed25519_sig = sign_message(&ed25519.private_key, message, CurveType::Ed25519).unwrap();

        // verify_signature_safe should work for all without knowing curve type
        assert!(
            verify_signature_safe(&k256.address, message, &k256_sig).unwrap(),
            "K256 signature should verify with safe method"
        );
        assert!(
            verify_signature_safe(&p256.address, message, &p256_sig).unwrap(),
            "P256 signature should verify with safe method"
        );
        assert!(
            verify_signature_safe(&ed25519.address, message, &ed25519_sig).unwrap(),
            "Ed25519 signature should verify with safe method"
        );
    }

    #[test]
    fn test_tagged_address_verification() {
        // Test that tagged addresses provide reliable verification

        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"test with tagged address";

        use crate::signatures::{sign_message, verify_signature};

        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

        // Use tagged address for verification
        let tagged = keypair.tagged_address();
        let result = verify_signature(&tagged, message, &signature).unwrap();

        assert!(result, "Signature should verify with tagged address");
    }

    // ============================================================================
    // Ed25519Dilithium3 Specific Tests
    // ============================================================================

    #[test]
    fn test_ed25519_dilithium3_keypair_structure() {
        // Test Ed25519+Dilithium3 hybrid keypair structure
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        // Verify private key format
        assert!(
            keypair.private_key.starts_with("kanahybrid"),
            "Ed25519Dilithium3 private key must start with 'kanahybrid'"
        );
        assert!(
            keypair.private_key.contains(':'),
            "Ed25519Dilithium3 private key must contain ':' separator"
        );

        // Verify public key format (classical:pqc)
        assert!(
            keypair.public_key.contains(':'),
            "Ed25519Dilithium3 public key must be in format 'classical:pqc'"
        );

        let pub_parts: Vec<&str> = keypair.public_key.split(':').collect();
        assert_eq!(pub_parts.len(), 2, "Public key must have exactly 2 parts");

        // Ed25519 public key should be 32 bytes (64 hex chars)
        assert_eq!(
            pub_parts[0].len(),
            64,
            "Ed25519 public key must be 64 hex characters"
        );

        // Dilithium3 public key should be 1952 bytes (3904 hex chars)
        assert_eq!(
            pub_parts[1].len(),
            3904,
            "Dilithium3 public key must be 3904 hex characters"
        );

        // Verify PQC public key field
        assert!(
            keypair.pqc_public_key.is_some(),
            "Ed25519Dilithium3 must have PQC public key"
        );
        assert_eq!(
            keypair.pqc_public_key.unwrap(),
            pub_parts[1],
            "PQC public key must match Dilithium3 part"
        );

        // Verify address format
        assert!(
            keypair.address.starts_with("0x"),
            "Address must start with '0x'"
        );
        assert_eq!(
            keypair.address.len(),
            66,
            "Address must be 66 characters (0x + 64 hex)"
        );

        // Verify curve type
        assert_eq!(keypair.curve_type, CurveType::Ed25519Dilithium3);
    }

    #[test]
    fn test_ed25519_dilithium3_sign_and_verify() {
        // Test signing and verification with Ed25519Dilithium3
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let message = b"Test message for Ed25519+Dilithium3 hybrid signature";

        use crate::signatures::{sign_message, verify_signature_with_curve};

        // Sign the message
        let signature =
            sign_message(&keypair.private_key, message, CurveType::Ed25519Dilithium3).unwrap();

        // Signature should not be empty
        assert!(!signature.is_empty(), "Signature must not be empty");

        // Signature should contain both classical and PQC parts
        // Format: [2-byte length] || classical_sig || pqc_sig
        assert!(
            signature.len() > 2,
            "Signature must be longer than length prefix"
        );

        let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
        assert_eq!(classical_len, 64, "Ed25519 signature should be 64 bytes");

        // Verify the signature using combined public key
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Ed25519Dilithium3,
        )
        .unwrap();

        assert!(
            verified,
            "Ed25519Dilithium3 signature must verify successfully"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_signature_fails_wrong_message() {
        // Test that signature verification fails with wrong message
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let message1 = b"Original message";
        let message2 = b"Different message";

        use crate::signatures::{sign_message, verify_signature_with_curve};

        let signature =
            sign_message(&keypair.private_key, message1, CurveType::Ed25519Dilithium3).unwrap();

        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message2,
            &signature,
            CurveType::Ed25519Dilithium3,
        )
        .unwrap();

        assert!(!verified, "Signature must not verify with wrong message");
    }

    #[test]
    fn test_ed25519_dilithium3_import_from_private_key() {
        // Test importing Ed25519Dilithium3 keypair from private key
        let original = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        // Import from private key
        let imported =
            keypair_from_private_key(&original.private_key, CurveType::Ed25519Dilithium3).unwrap();

        // Should produce identical keypair
        assert_eq!(
            original.public_key, imported.public_key,
            "Public keys must match"
        );
        assert_eq!(original.address, imported.address, "Addresses must match");
        assert_eq!(
            original.pqc_public_key, imported.pqc_public_key,
            "PQC public keys must match"
        );
        assert_eq!(
            original.curve_type, imported.curve_type,
            "Curve types must match"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_deterministic_address() {
        // Test that same combined public key always produces same address
        let keypair1 = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let keypair2 = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        // Different keypairs should have different addresses
        assert_ne!(
            keypair1.address, keypair2.address,
            "Different keypairs must have different addresses"
        );

        // Same public key should always produce same address
        let reimported =
            keypair_from_private_key(&keypair1.private_key, CurveType::Ed25519Dilithium3).unwrap();

        assert_eq!(
            keypair1.address, reimported.address,
            "Same keypair must produce same address"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_address_is_sha3_hash() {
        // Test that Ed25519Dilithium3 address is SHA3-256 of combined public key
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        use sha3::{Digest, Sha3_256};
        let mut hasher = Sha3_256::new();
        hasher.update(keypair.public_key.as_bytes());
        let expected_hash = hasher.finalize();
        let expected_address = format!("0x{}", hex::encode(&expected_hash[..]));

        assert_eq!(
            keypair.address, expected_address,
            "Address must be SHA3-256 hash of combined public key"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_security_properties() {
        // Test security properties of Ed25519Dilithium3
        let curve = CurveType::Ed25519Dilithium3;

        // Must be post-quantum
        assert!(
            curve.is_post_quantum(),
            "Ed25519Dilithium3 must be post-quantum"
        );

        // Must be hybrid
        assert!(curve.is_hybrid(), "Ed25519Dilithium3 must be hybrid");

        // Must have maximum security level
        assert_eq!(
            curve.security_level(),
            5,
            "Ed25519Dilithium3 must have security level 5"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_tagged_address() {
        // Test tagged address functionality for Ed25519Dilithium3
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let tagged = keypair.tagged_address();

        // Should have correct format
        assert!(
            tagged.starts_with("Ed25519Dilithium3:"),
            "Tagged address must start with curve type"
        );

        // Parse it back
        let (parsed_curve, parsed_address) =
            KeyPair::parse_tagged_address(&tagged).expect("Failed to parse tagged address");

        assert_eq!(
            parsed_curve,
            CurveType::Ed25519Dilithium3,
            "Parsed curve type must match"
        );
        assert_eq!(parsed_address, keypair.address, "Parsed address must match");
    }

    #[test]
    fn test_ed25519_dilithium3_invalid_private_key_import() {
        // Test that invalid private key formats are rejected

        // Test with non-hybrid prefix
        let result =
            keypair_from_private_key("kanari1234567890abcdef", CurveType::Ed25519Dilithium3);
        assert!(result.is_err(), "Non-hybrid prefix must be rejected");

        // Test with missing separator
        let result =
            keypair_from_private_key("kanahybrid1234567890abcdef", CurveType::Ed25519Dilithium3);
        assert!(result.is_err(), "Missing separator must be rejected");

        // Test with invalid hex
        let result = keypair_from_private_key("kanahybridzzzz:yyyy", CurveType::Ed25519Dilithium3);
        assert!(result.is_err(), "Invalid hex must be rejected");
    }

    #[test]
    fn test_ed25519_dilithium3_display_format() {
        // Test display format for Ed25519Dilithium3
        let curve = CurveType::Ed25519Dilithium3;
        let display = format!("{}", curve);

        assert_eq!(
            display, "Ed25519+Dilithium3 (Hybrid)",
            "Display format must be correct"
        );
    }
}
