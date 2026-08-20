// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_crypto::keys::{
    CurveType, generate_keypair, keypair_from_mnemonic, keypair_from_private_key,
};
use kanari_crypto::signatures::{sign_message, verify_signature_with_curve};
use kanari_crypto::{hash_data_blake3, hd_wallet, keys};
use serde::{Deserialize, Serialize};

uniffi::setup_scaffolding!();

/// Expose curve names safely for Kotlin/FFI
#[derive(Serialize, Deserialize, Debug, Clone, uniffi::Record)]
pub struct CurveInfo {
    pub name: String,
    pub is_post_quantum: bool,
    pub is_hybrid: bool,
    pub security_level: u8,
}

/// KeyPair data structure safe for FFI transfer
#[derive(Serialize, Deserialize, Debug, Clone, uniffi::Record)]
pub struct KeyPairData {
    pub private_key: String, // format "kanari...", "kanapqc...", "kanahybrid..."
    pub public_key: String,  // hex
    pub address: String,     // 0x...
    pub tagged_address: String, // Curve:0x...
    pub raw_public_key: Vec<u8>, // raw bytes of public key (for PQC verification)
    pub curve_type: String,  // e.g., "K256", "Ed25519Dilithium3"
}

/// Generate a keypair for the specified curve type
#[uniffi::export]
pub fn generate_keypair_api(curve_name: String) -> Result<KeyPairData, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    let kp = generate_keypair(curve).map_err(|e| format!("Key generation failed: {}", e))?;

    // Clone the pieces we need to avoid moving out of `kp` more than once
    let public_key_clone = kp.public_key.clone();
    let address_clone = kp.address.clone();
    let tagged_address = kp.tagged_address();
    let pqc_clone = kp.pqc_public_key.clone();
    let raw_public_key =
        hex::decode(pqc_clone.unwrap_or_else(|| public_key_clone.clone())).unwrap_or_default();

    Ok(KeyPairData {
        private_key: kp.private_key.to_string(),
        public_key: public_key_clone,
        address: address_clone,
        tagged_address,
        raw_public_key,
        curve_type: format!("{:?}", curve),
    })
}

/// Derive a keypair from a mnemonic (BIP39)
#[uniffi::export]
pub fn derive_keypair_from_mnemonic(
    mnemonic: String,
    curve_name: String,
) -> Result<KeyPairData, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    if curve.is_post_quantum() {
        return Err("Post-quantum curves do not support BIP39 derivation".to_string());
    }

    let kp = keypair_from_mnemonic(&mnemonic, curve)
        .map_err(|e| format!("Mnemonic derivation failed: {}", e))?;

    let public_key_clone = kp.public_key.clone();
    let address_clone = kp.address.clone();
    let tagged_address = kp.tagged_address();
    let pqc_clone = kp.pqc_public_key.clone();
    let raw_public_key =
        hex::decode(pqc_clone.unwrap_or_else(|| public_key_clone.clone())).unwrap_or_default();

    Ok(KeyPairData {
        private_key: kp.private_key.to_string(),
        public_key: public_key_clone,
        address: address_clone,
        tagged_address,
        raw_public_key,
        curve_type: format!("{:?}", curve),
    })
}

/// Import a keypair from a provided private key
#[uniffi::export]
pub fn import_keypair_from_private_key(
    private_key: String,
    curve_name: String,
) -> Result<KeyPairData, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    let kp = keypair_from_private_key(&private_key, curve)
        .map_err(|e| format!("Private key import failed: {}", e))?;

    let public_key_clone = kp.public_key.clone();
    let address_clone = kp.address.clone();
    let tagged_address = kp.tagged_address();
    let pqc_clone = kp.pqc_public_key.clone();
    let raw_public_key =
        hex::decode(pqc_clone.unwrap_or_else(|| public_key_clone.clone())).unwrap_or_default();

    Ok(KeyPairData {
        private_key: kp.private_key.to_string(),
        public_key: public_key_clone,
        address: address_clone,
        tagged_address,
        raw_public_key,
        curve_type: format!("{:?}", curve),
    })
}

/// Sign a message
#[uniffi::export]
pub fn sign_message_api(
    private_key: String,
    message: Vec<u8>,
    curve_name: String,
) -> Result<Vec<u8>, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    sign_message(&private_key, &message, curve).map_err(|e| format!("Signing failed: {}", e))
}

/// Hash data using Blake3
#[uniffi::export]
pub fn blake3_hash_api(data: Vec<u8>) -> Vec<u8> {
    hash_data_blake3(&data)
}

/// Verify a signature
#[uniffi::export]
pub fn verify_signature_api(
    address: String,
    message: Vec<u8>,
    signature: Vec<u8>,
    curve_name: String,
) -> Result<bool, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    verify_signature_with_curve(&address, &message, &signature, curve)
        .map_err(|e| format!("Verification failed: {}", e))
}

/// Generate a random mnemonic
#[uniffi::export]
pub fn generate_mnemonic_api(word_count: u32) -> Result<String, String> {
    if word_count != 12 && word_count != 24 {
        return Err("Only 12 or 24-word mnemonics are supported".to_string());
    }

    keys::generate_mnemonic(word_count as usize)
        .map_err(|e| format!("Mnemonic generation failed: {}", e))
}

/// Derive a keypair from a mnemonic at a specific derivation path
#[uniffi::export]
pub fn derive_keypair_from_path_api(
    mnemonic: String,
    derivation_path: String,
    curve_name: String,
) -> Result<KeyPairData, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    let kp = hd_wallet::derive_keypair_from_path(&mnemonic, "", &derivation_path, curve)
        .map_err(|e| format!("Path derivation failed: {}", e))?;

    let public_key_clone = kp.public_key.clone();
    let tagged_address = kp.tagged_address();
    let raw_public_key = hex::decode(
        kp.pqc_public_key
            .clone()
            .unwrap_or_else(|| public_key_clone.clone()),
    )
    .unwrap_or_default();

    Ok(KeyPairData {
        private_key: kp.private_key.to_string(),
        public_key: public_key_clone,
        address: kp.address,
        tagged_address,
        raw_public_key,
        curve_type: format!("{:?}", curve),
    })
}

/// Derive multiple addresses from a mnemonic using a path template
#[uniffi::export]
pub fn derive_multiple_addresses_api(
    mnemonic: String,
    path_template: String,
    curve_name: String,
    count: u32,
) -> Result<Vec<KeyPairData>, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    let keypairs =
        hd_wallet::derive_multiple_addresses(&mnemonic, "", &path_template, curve, count as usize)
            .map_err(|e| format!("HD derivation failed: {}", e))?;

    Ok(keypairs
        .into_iter()
        .map(|kp| {
            let public_key_clone = kp.public_key.clone();
            let tagged_address = kp.tagged_address();
            let raw_public_key = hex::decode(
                kp.pqc_public_key
                    .clone()
                    .unwrap_or_else(|| public_key_clone.clone()),
            )
            .unwrap_or_default();

            KeyPairData {
                private_key: kp.private_key.to_string(),
                public_key: public_key_clone,
                address: kp.address,
                tagged_address,
                raw_public_key,
                curve_type: format!("{:?}", curve),
            }
        })
        .collect())
}

/// List all supported curves
#[uniffi::export]
pub fn list_supported_curves() -> Vec<CurveInfo> {
    use CurveType::*;
    let curves = [
        K256,
        P256,
        Ed25519,
        Dilithium2,
        Dilithium3,
        Dilithium5,
        SphincsPlusSha256Robust,
        Falcon512,
        Falcon1024,
        Ed25519Dilithium3,
        K256Dilithium3,
    ];

    curves
        .iter()
        .map(|&c| CurveInfo {
            name: format!("{:?}", c),
            is_post_quantum: c.is_post_quantum(),
            is_hybrid: c.is_hybrid(),
            security_level: c.security_level(),
        })
        .collect()
}

// --- Helper Internals ---
fn parse_curve_type(name: &str) -> Option<CurveType> {
    match name {
        "K256" => Some(CurveType::K256),
        "P256" => Some(CurveType::P256),
        "Ed25519" => Some(CurveType::Ed25519),
        "Dilithium2" => Some(CurveType::Dilithium2),
        "Dilithium3" => Some(CurveType::Dilithium3),
        "Dilithium5" => Some(CurveType::Dilithium5),
        "SphincsPlusSha256Robust" => Some(CurveType::SphincsPlusSha256Robust),
        "Falcon512" | "FnDsa512" | "FN-DSA-512" => Some(CurveType::Falcon512),
        "Falcon1024" | "FnDsa1024" | "FN-DSA-1024" => Some(CurveType::Falcon1024),
        "Ed25519Dilithium3" => Some(CurveType::Ed25519Dilithium3),
        "K256Dilithium3" => Some(CurveType::K256Dilithium3),
        _ => None,
    }
}
