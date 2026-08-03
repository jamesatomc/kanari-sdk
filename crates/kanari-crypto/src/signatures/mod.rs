// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use log::debug;

use thiserror::Error;

use zeroize::Zeroize;

use crate::{
    keys::CurveType,
    signatures::{
        dilithium2::{sign_message_dilithium2, verify_signature_dilithium2},
        dilithium3::{sign_message_dilithium3, verify_signature_dilithium3},
        dilithium5::{sign_message_dilithium5, verify_signature_dilithium5},
        ed25519::{sign_message_ed25519, verify_signature_ed25519},
        hybrid::{
            sign_message_hybrid_ed25519, sign_message_hybrid_k256, verify_ed25519dilithium3,
            verify_hybrid_signature_detailed, verify_k256dilithium3,
        },
        k256::{sign_message_k256, verify_signature_k256},
        p256::{sign_message_p256, verify_signature_p256},
        sphincs::{sign_message_sphincs, verify_signature_sphincs},
    },
};
pub mod dilithium2;
pub mod dilithium3;
pub mod dilithium5;
pub mod ed25519;
pub mod hybrid;
pub mod k256;
pub(crate) mod ml_dsa_provider;
pub mod p256;
#[cfg(feature = "experimental-slh-dsa")]
pub(crate) mod slh_dsa_provider;
pub mod sphincs;

/// Digital signature errors
#[derive(Error, Debug)]
pub enum SignatureError {
    #[error("Invalid private key: {0}")]
    InvalidPrivateKey(String),

    #[error("Invalid signature format: {0}")]
    InvalidFormat(String),

    #[error("Invalid public key or address: {0}")]
    InvalidPublicKey(String),

    #[error("Signature verification failed")]
    VerificationFailed,

    #[error("Invalid signature length")]
    InvalidSignatureLength,
}

/// Maximum allowed signature bytes to guard against resource exhaustion in parsing
const MAX_SIGNATURE_SIZE: usize = 64 * 1024; // 64 KiB
/// Maximum classical signature length we accept inside a hybrid combined signature
const MAX_CLASSICAL_SIG_LEN: usize = 1024; // limit to 1 KiB to avoid DoS

// Common EC public key lengths
// These constants are used for flexible address parsing but should not be
// exposed in error messages to prevent information leakage
const SEC1_UNCOMPRESSED_LEN: usize = 65;
const SEC1_COMPRESSED_LEN: usize = 33;
const RAW_XY_LEN: usize = 64;
const X_ONLY_LEN: usize = 32;
// Ed25519 public key length (32 bytes)
const ED25519_PUBLIC_KEY_LEN: usize = 32;

/// Zero out sensitive data in memory
/// Uses zeroize crate for secure memory clearing with compiler fence
/// to prevent optimization that could leave sensitive data in memory
pub fn secure_clear(data: &mut [u8]) {
    data.zeroize();
    // Add a black_box to prevent compiler from optimizing away the zeroization
    std::hint::black_box(data);
}

/// Sign a message with a given private key and curve type
pub fn sign_message(
    private_key_hex: &str,
    message: &[u8],
    curve_type: CurveType,
) -> Result<Vec<u8>, SignatureError> {
    // Extract raw key if it has any known Kanari prefix
    let raw_key = crate::keys::extract_raw_key(private_key_hex);

    match curve_type {
        CurveType::K256 => sign_message_k256(raw_key, message),
        CurveType::P256 => sign_message_p256(raw_key, message),
        CurveType::Ed25519 => sign_message_ed25519(raw_key, message),
        // Preserve full formatted keys for hybrid/PQC curves. Their prefixes carry
        // provider and packing metadata that must not be stripped by this dispatcher.
        CurveType::K256Dilithium3 => sign_message_hybrid_k256(private_key_hex, message),
        CurveType::Ed25519Dilithium3 => sign_message_hybrid_ed25519(private_key_hex, message),
        CurveType::Dilithium2 => sign_message_dilithium2(private_key_hex, message),
        CurveType::Dilithium3 => sign_message_dilithium3(private_key_hex, message),
        CurveType::Dilithium5 => sign_message_dilithium5(private_key_hex, message),
        CurveType::SphincsPlusSha256Robust => sign_message_sphincs(private_key_hex, message),
    }
}

/// Verify a signature against a message using an address
///
/// This function requires tagged addresses (e.g., "K256:0xabc...") so the
/// verifier never has to guess which curve should be used.
pub fn verify_signature(
    address: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    if signature.is_empty() {
        return Err(SignatureError::InvalidFormat("Empty signature".to_string()));
    }
    if signature.len() > MAX_SIGNATURE_SIZE {
        return Err(SignatureError::InvalidFormat(
            "Signature too large".to_string(),
        ));
    }

    if let Some((curve_type, addr)) = crate::keys::KeyPair::parse_tagged_address(address) {
        debug!("Using tagged address with curve type: {:?}", curve_type);
        return verify_signature_with_curve(&addr, message, signature, curve_type);
    }

    Err(SignatureError::InvalidFormat(
        "Tagged address required for signature verification".to_string(),
    ))
}

/// Verify a signature with the known curve type
pub fn verify_signature_with_curve(
    address: &str,
    message: &[u8],
    signature: &[u8],
    curve_type: CurveType,
) -> Result<bool, SignatureError> {
    let address_hex = address.trim_start_matches("0x");

    if signature.is_empty() {
        return Err(SignatureError::InvalidFormat("Empty signature".to_string()));
    }
    if signature.len() > MAX_SIGNATURE_SIZE {
        return Err(SignatureError::InvalidFormat(
            "Signature too large".to_string(),
        ));
    }

    match curve_type {
        CurveType::K256 => verify_signature_k256(address_hex, message, signature),
        CurveType::P256 => verify_signature_p256(address_hex, message, signature),
        CurveType::Ed25519 => verify_signature_ed25519(address_hex, message, signature),
        // For hybrid K256+Dilithium3, verify using the classical K256 public key part when provided
        CurveType::K256Dilithium3 => verify_k256dilithium3(address_hex, message, signature),
        // For hybrid Ed25519+Dilithium3, verify using the classical Ed25519 public key part when provided
        CurveType::Ed25519Dilithium3 => verify_ed25519dilithium3(address_hex, message, signature),
        // Pure PQC curves: address_hex contains the PQC public key
        CurveType::Dilithium2 => verify_signature_dilithium2(address_hex, message, signature),
        CurveType::Dilithium3 => verify_signature_dilithium3(address_hex, message, signature),
        CurveType::Dilithium5 => verify_signature_dilithium5(address_hex, message, signature),
        CurveType::SphincsPlusSha256Robust => {
            verify_signature_sphincs(address_hex, message, signature)
        }
    }
}

/// Verify a signature using a `KeyPair` directly (avoids parsing combined public_key strings)
///
/// This function prefers the explicit `pqc_public_key` field on `KeyPair` when
/// verifying hybrid signatures, avoiding repeated parsing of `public_key` values
/// that may be stored as "classical:pqc" for backward compatibility.
pub fn verify_signature_with_keypair(
    keypair: &crate::keys::KeyPair,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    let curve_type = keypair.curve_type;

    // `public_key` may be stored as "classical" or "classical:pqc" for legacy reasons.
    // Use the explicit `pqc_public_key` field when present.
    let pub_combined = keypair.public_key.as_str();
    let classical_pub = pub_combined.split(':').next().unwrap_or(pub_combined);

    match curve_type {
        CurveType::K256 => verify_signature_k256(classical_pub, message, signature),
        CurveType::P256 => verify_signature_p256(classical_pub, message, signature),
        CurveType::Ed25519 => verify_signature_ed25519(classical_pub, message, signature),

        CurveType::K256Dilithium3 => {
            let pqc_pub = keypair
                .get_pqc_public_key()
                .unwrap_or_else(|| pub_combined.split(':').nth(1).unwrap_or("").to_string());
            if pqc_pub.is_empty() {
                return Err(SignatureError::InvalidPublicKey(
                    "Missing PQC public key for hybrid keypair".to_string(),
                ));
            }

            let (classical_ok, pqc_ok) = verify_hybrid_signature_detailed(
                signature,
                classical_pub,
                &pqc_pub,
                message,
                verify_signature_k256,
            )?;
            Ok(classical_ok && pqc_ok)
        }

        CurveType::Ed25519Dilithium3 => {
            let pqc_pub = keypair
                .get_pqc_public_key()
                .unwrap_or_else(|| pub_combined.split(':').nth(1).unwrap_or("").to_string());
            if pqc_pub.is_empty() {
                return Err(SignatureError::InvalidPublicKey(
                    "Missing PQC public key for hybrid keypair".to_string(),
                ));
            }

            let (classical_ok, pqc_ok) = verify_hybrid_signature_detailed(
                signature,
                classical_pub,
                &pqc_pub,
                message,
                verify_signature_ed25519,
            )?;
            Ok(classical_ok && pqc_ok)
        }

        CurveType::Dilithium2 => {
            let pqc_pub = keypair
                .get_pqc_public_key_ref()
                .unwrap_or(&keypair.public_key);
            verify_signature_dilithium2(pqc_pub, message, signature)
        }
        CurveType::Dilithium3 => {
            let pqc_pub = keypair
                .get_pqc_public_key_ref()
                .unwrap_or(&keypair.public_key);
            verify_signature_dilithium3(pqc_pub, message, signature)
        }
        CurveType::Dilithium5 => {
            let pqc_pub = keypair
                .get_pqc_public_key_ref()
                .unwrap_or(&keypair.public_key);
            verify_signature_dilithium5(pqc_pub, message, signature)
        }
        CurveType::SphincsPlusSha256Robust => {
            let pqc_pub = keypair
                .get_pqc_public_key_ref()
                .unwrap_or(&keypair.public_key);
            verify_signature_sphincs(pqc_pub, message, signature)
        }
    }
}
