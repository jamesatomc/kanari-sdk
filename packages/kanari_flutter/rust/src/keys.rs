// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Cryptographic key generation and management
//!
//! This module handles key generation for multiple curve types (K256/secp256k1,
//! P256/secp256r1, Ed25519) and Post-Quantum Cryptography (Dilithium, SPHINCS+).
//!
//! **Quantum-Safe**: Includes NIST-standardized post-quantum algorithms.
//!
//! # ⚠️ Security Considerations
//!
//! ## Private Key Storage
//! - Private keys are stored in `Zeroizing<String>` which automatically clears memory on drop
//! - `Clone` is NOT implemented on `KeyPair` to prevent accidental key duplication
//! - Use `export_private_key_secure()` to explicitly handle private key export
//! - Private keys are **NOT serialized** by default; use `to_serializable_with_private_key()`
//!   only for encrypted storage scenarios
//!
//! ## Tagged Addresses
//! For enhanced security and reliability, use tagged addresses:
//! - Format: `\"CurveType:address\"` (e.g., `\"K256:0xabc...\"`)
//! - Prevents address ambiguity when multiple curves could apply
//! - Required for secure signature verification without timing leaks
//! - Use `tagged_address()` method to generate; use `parse_tagged_address()` to extract
//!
//! ## Hybrid Keys
//! Hybrid keys (e.g., Ed25519+Dilithium3) combine classical and post-quantum algorithms:
//! - Format: `\"CurveType:classical_pub:pqc_pub\"`
//! - Private key format: `\"kanahybrid<classical_hex>:<pqc_secret_hex>:<pqc_pub_hex>\"`
//! - Both classical and PQC signatures required for full verification
//! - Use for transition period during quantum computing threat emergence
//!
//! ## Post-Quantum Cryptography Providers
//! PQC keys use explicit provider prefixes (`kanamldsa`, `kanaslh`) so provider
//! metadata is not lost during signing.
//! - Monitor security advisories regularly
//! - Dilithium3 / ML-DSA-65 (NIST Level 3) is recommended for most use cases
//!
//! ## Mnemonic Derivation Limitations
//! - Only classical curves (K256, P256, Ed25519) support BIP39 mnemonic derivation
//! - PQC algorithms generate fresh keys without HD wallet derivation
//! - For PQC keys, use `generate_keypair()` for fresh key generation

use bip39::Mnemonic;
use rand::TryRng;
use rand::rngs::SysRng;
use std::fmt;
use std::str::FromStr;
use subtle::ConstantTimeEq;
use thiserror::Error;
use zeroize::Zeroizing;

mod classical;
mod hybrid;
mod pqc;

pub use classical::generate_ed25519_keypair;
pub use hybrid::{
    generate_hybrid_ed25519_dilithium3_keypair, generate_hybrid_k256_dilithium3_keypair,
};

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
            CurveType::K256 => write!(f, "K256"),
            CurveType::P256 => write!(f, "P256"),
            CurveType::Ed25519 => write!(f, "Ed25519"),
            CurveType::Dilithium2 => write!(f, "Dilithium2"),
            CurveType::Dilithium3 => write!(f, "Dilithium3"),
            CurveType::Dilithium5 => write!(f, "Dilithium5"),
            CurveType::SphincsPlusSha256Robust => write!(f, "SphincsPlusSha256Robust"),
            CurveType::Ed25519Dilithium3 => write!(f, "Ed25519Dilithium3"),
            CurveType::K256Dilithium3 => write!(f, "K256Dilithium3"),
        }
    }
}

impl std::str::FromStr for CurveType {
    type Err = KeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "K256" => Ok(CurveType::K256),
            "P256" => Ok(CurveType::P256),
            "Ed25519" => Ok(CurveType::Ed25519),
            "Dilithium2" => Ok(CurveType::Dilithium2),
            "Dilithium3" => Ok(CurveType::Dilithium3),
            "Dilithium5" => Ok(CurveType::Dilithium5),
            "SphincsPlusSha256Robust" => Ok(CurveType::SphincsPlusSha256Robust),
            "Ed25519Dilithium3" => Ok(CurveType::Ed25519Dilithium3),
            "K256Dilithium3" => Ok(CurveType::K256Dilithium3),
            _ => Err(KeyError::InvalidPublicKey),
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
#[derive(serde::Serialize, serde::Deserialize)]
pub struct KeyPair {
    #[serde(skip)]
    pub private_key: Zeroizing<String>,
    pub public_key: String,
    /// Optional post-quantum public key (hex) when applicable
    pub pqc_public_key: Option<String>,
    pub address: String,
    pub curve_type: CurveType,
}

impl fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyPair")
            .field("private_key", &"**REDACTED**")
            .field("public_key", &self.public_key)
            .field("address", &self.address)
            .field("curve_type", &self.curve_type)
            .finish()
    }
}

impl KeyPair {
    /// Export private key in a wrapper that zeroizes on drop
    /// Prefer this API to avoid accidental long-lived clones of secret material.
    pub fn export_private_key_secure(&self) -> Zeroizing<String> {
        Zeroizing::new(self.private_key.to_string())
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
    /// For hybrid keys, format is "curve_type:classical_pub:pqc_pub"
    /// This is the recommended way to store addresses for reliable curve detection
    pub fn tagged_address(&self) -> String {
        // For hybrid keys, include both classical and PQC public keys in the tag
        format!("{}:{}", self.curve_type, self.public_key)
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
        // Find the first colon to separate curve type from address
        // For hybrid keys, address contains ':' (format: "classical:pqc")
        // so we must use find() not split_once() to preserve the full address
        let colon_pos = tagged.find(':')?;
        let curve_str = &tagged[..colon_pos];
        let address_str = &tagged[colon_pos + 1..]; // Take everything after first colon

        let curve_type = CurveType::from_str(curve_str).ok()?;

        Some((curve_type, address_str.to_string()))
    }
}

/// Prefix used for Kanari private keys
pub const KANARI_KEY_PREFIX: &str = "kanari";

/// Additional known prefixes
pub const KANAPQC_PREFIX: &str = "kanapqc";
pub const KANAMLDSA_PREFIX: &str = "kanamldsa";
pub const KANASLHDSA_PREFIX: &str = "kanaslh";
pub const KANAHYBRID_PREFIX: &str = "kanahybrid";
const MAX_FORMATTED_PRIVATE_KEY_LEN: usize = 128 * 1024;

// ============================================================================
// SECURITY HELPER FUNCTIONS (Timing Attack Prevention & Memory Safety)
// ============================================================================

/// Securely encode bytes to hex string using a zeroizing buffer
/// This prevents intermediate allocations from leaking sensitive data in memory dumps
fn secure_hex_encode(bytes: &[u8]) -> Zeroizing<String> {
    // Pre-allocate with exact capacity needed (2 chars per byte)
    let mut result = String::with_capacity(bytes.len() * 2);

    // Use lookup table approach for constant-time-ish encoding
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    for &byte in bytes {
        result.push(HEX_CHARS[(byte >> 4) as usize] as char);
        result.push(HEX_CHARS[(byte & 0x0F) as usize] as char);
    }

    Zeroizing::new(result)
}

/// Constant-time check if a string starts with a given prefix
/// Prevents timing attacks that could leak information about key formats
fn constant_time_starts_with(s: &str, prefix: &str) -> bool {
    // Get bytes for comparison
    let s_bytes = s.as_bytes();
    let prefix_bytes = prefix.as_bytes();

    // If prefix is longer than the string, it can't match
    if prefix_bytes.len() > s_bytes.len() {
        return false;
    }

    // Compare only the relevant portion in constant time
    let s_prefix = &s_bytes[..prefix_bytes.len()];

    // Use subtle crate's constant-time equality check
    s_prefix.ct_eq(prefix_bytes).into()
}

/// Format a raw hex private key with the Kanari prefix
pub fn format_private_key(raw_key: &str) -> String {
    format!("{}{}", KANARI_KEY_PREFIX, raw_key)
}

/// Extract the raw hex key from a formatted private key using constant-time comparison
pub fn extract_raw_key(formatted_key: &str) -> &str {
    // Use constant-time checks to prevent timing leaks
    if constant_time_starts_with(formatted_key, KANAHYBRID_PREFIX) {
        &formatted_key[KANAHYBRID_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANAMLDSA_PREFIX) {
        &formatted_key[KANAMLDSA_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANASLHDSA_PREFIX) {
        &formatted_key[KANASLHDSA_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANAPQC_PREFIX) {
        &formatted_key[KANAPQC_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANARI_KEY_PREFIX) {
        &formatted_key[KANARI_KEY_PREFIX.len()..]
    } else {
        formatted_key
    }
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
        CurveType::K256 => classical::generate_k256_keypair(),
        CurveType::P256 => classical::generate_p256_keypair(),
        CurveType::Ed25519 => classical::generate_ed25519_keypair(),
        CurveType::Dilithium2 => pqc::generate_dilithium2_keypair(),
        CurveType::Dilithium3 => pqc::generate_dilithium3_keypair(),
        CurveType::Dilithium5 => pqc::generate_dilithium5_keypair(),
        CurveType::SphincsPlusSha256Robust => pqc::generate_sphincs_keypair(),
        CurveType::Ed25519Dilithium3 => hybrid::generate_hybrid_ed25519_dilithium3_keypair(),
        CurveType::K256Dilithium3 => hybrid::generate_hybrid_k256_dilithium3_keypair(),
    }
}

/// Generate a keypair from a mnemonic phrase
pub fn keypair_from_mnemonic(phrase: &str, curve_type: CurveType) -> Result<KeyPair, KeyError> {
    classical::keypair_from_mnemonic(phrase, curve_type)
}

/// Generate a keypair from a private key
pub fn keypair_from_private_key(
    private_key: &str,
    curve_type: CurveType,
) -> Result<KeyPair, KeyError> {
    if private_key.len() > MAX_FORMATTED_PRIVATE_KEY_LEN {
        return Err(KeyError::InvalidPrivateKey);
    }

    // Remove kanari prefix if present
    let raw_private_key = extract_raw_key(private_key);

    match curve_type {
        CurveType::K256 => classical::keypair_from_k256_private_key(private_key, raw_private_key),
        CurveType::P256 => classical::keypair_from_p256_private_key(private_key, raw_private_key),
        CurveType::Ed25519 => {
            classical::keypair_from_ed25519_private_key(private_key, raw_private_key)
        }
        // Post-quantum imports: require public key be stored alongside secret when possible.
        CurveType::Dilithium2
        | CurveType::Dilithium3
        | CurveType::Dilithium5
        | CurveType::SphincsPlusSha256Robust => {
            pqc::keypair_from_pqc_private_key(private_key, raw_private_key, curve_type)
        }
        CurveType::Ed25519Dilithium3 | CurveType::K256Dilithium3 => {
            hybrid::keypair_from_hybrid_private_key(private_key, raw_private_key, curve_type)
        }
    }
}

/// Generate a mnemonic phrase with the specified word count
pub fn generate_mnemonic(word_count: usize) -> Result<String, KeyError> {
    let entropy_bits = match word_count {
        12 => 128,
        24 => 256,
        _ => {
            return Err(KeyError::GenerationFailed(format!(
                "Unsupported word count: {}",
                word_count
            )));
        }
    };
    let mut entropy = Zeroizing::new(vec![0u8; entropy_bits / 8]);

    SysRng
        .try_fill_bytes(&mut entropy)
        .expect("Failed to get OS randomness");

    let mnemonic =
        Mnemonic::from_entropy(&entropy).map_err(|e| KeyError::GenerationFailed(e.to_string()))?;
    Ok(mnemonic.to_string())
}

/// Struct representing an imported wallet with private key, public key, and address
pub struct ImportedWallet {
    pub private_key: Zeroizing<String>,
    pub public_key: String,
    pub address: String,
}

/// Import a wallet from a seed phrase
pub fn import_from_seed_phrase(
    phrase: &str,
    curve_type: CurveType,
) -> Result<ImportedWallet, String> {
    match keypair_from_mnemonic(phrase, curve_type) {
        Ok(keypair) => Ok(ImportedWallet {
            private_key: keypair.export_private_key_secure(),
            public_key: keypair.get_public_key().to_string(),
            address: keypair.get_address().to_string(),
        }),
        Err(e) => Err(e.to_string()),
    }
}

/// Import a wallet from a private key
pub fn import_from_private_key(
    private_key: &str,
    curve_type: CurveType,
) -> Result<ImportedWallet, String> {
    match keypair_from_private_key(private_key, curve_type) {
        Ok(keypair) => Ok(ImportedWallet {
            private_key: keypair.export_private_key_secure(),
            public_key: keypair.get_public_key().to_string(),
            address: keypair.get_address().to_string(),
        }),
        Err(e) => Err(e.to_string()),
    }
}
