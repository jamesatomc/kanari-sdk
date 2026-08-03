// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use ed25519_dalek::{
    Signature as Ed25519Signature, Signer, SigningKey as Ed25519SigningKey, Verifier,
    VerifyingKey as Ed25519VerifyingKey,
};
use std::cell::RefCell;
use std::collections::HashMap;

use zeroize::Zeroize;

use crate::{SignatureError, signatures::ED25519_PUBLIC_KEY_LEN};

thread_local! {
    static ED25519_VERIFYING_KEY_CACHE: RefCell<HashMap<String, Ed25519VerifyingKey>> =
        RefCell::new(HashMap::new());
}

/// Verify a signature using Ed25519
///
/// **✅ RFC-8032 COMPLIANT - Standard Ed25519 Verification (NO Pre-Hashing)**
///
/// This function strictly adheres to RFC-8032 standard Ed25519:
/// - Signatures are verified DIRECTLY against the original message (no pre-hashing)
/// - Compatible with Ed25519 signatures from all standard implementations
/// - Uses constant-time verification to prevent timing attacks
///
/// **⚠️ CRITICAL DIFFERENCE FROM K256/P256:**
/// - K256: Message → SHA3-256 hash → Sign hash ← Kanari-specific
/// - P256: Message → SHA3-256 hash → Sign hash ← Kanari-specific
/// - Ed25519: Message → Sign directly ← RFC-8032 standard
///
/// **DO NOT:**
/// ❌ Pre-hash the message: `sign_message(key, sha256(msg), Ed25519)` ← WRONG!
/// ❌ Try to verify K256-style hashed signatures
///
/// **Interoperability:**
/// - ✅ Verifies Ed25519 signatures from libsodium, NaCl, cryptonote, external Ed25519 tools
/// - ✅ Kanari Ed25519 signatures verify in standard Ed25519 implementations
/// - ✅ No format conversion or special handling needed
///
pub fn verify_signature_ed25519(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    // ✅ RFC-8032 STANDARD: Verify message DIRECTLY (no pre-hashing)
    // This is the key difference from K256/P256 which use pre-hashing
    // Check signature length and construct signature object
    if signature.len() != 64 {
        return Err(SignatureError::InvalidSignatureLength);
    }
    let mut sig_array = [0u8; 64];
    sig_array.copy_from_slice(signature);
    let signature = Ed25519Signature::from_bytes(&sig_array);
    sig_array.zeroize();

    // Normalize and decode public key
    let raw_key = crate::keys::extract_raw_key(address_hex).to_string();
    let verifying_key = ed25519_verifying_key_from_cache(&raw_key)?;

    match verifying_key.verify(message, &signature) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

fn ed25519_verifying_key_from_cache(raw_key: &str) -> Result<Ed25519VerifyingKey, SignatureError> {
    if let Some(key) =
        ED25519_VERIFYING_KEY_CACHE.with(|cache| cache.borrow().get(raw_key).copied())
    {
        return Ok(key);
    }

    let decoded_hex = hex::decode(raw_key)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid address format".to_string()))?;

    if decoded_hex.len() != ED25519_PUBLIC_KEY_LEN {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid address format".to_string(),
        ));
    }

    let mut key_array = [0u8; ED25519_PUBLIC_KEY_LEN];
    key_array.copy_from_slice(&decoded_hex);
    let verifying_key = Ed25519VerifyingKey::from_bytes(&key_array)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid address format".to_string()))?;

    ED25519_VERIFYING_KEY_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .insert(raw_key.to_string(), verifying_key);
    });

    Ok(verifying_key)
}

/// Sign a message using Ed25519 private key
///
/// **✅ RFC-8032 COMPLIANT - Standard Ed25519 Behavior**
///
/// This implementation strictly follows RFC-8032 standard Ed25519:
/// - **Messages are signed DIRECTLY without pre-hashing** (per RFC-8032)
/// - This makes Kanari Ed25519 signatures 100% compatible with standard Ed25519 implementations
/// - Verification by external Ed25519 systems will succeed without modification
///
/// **Why This Differs From K256/P256:**
/// Kanari uses curve-specific strategies:
/// - **K256/P256** (ECC): Hash message with SHA3-256 before signing (Kanari-specific for domain separation)
/// - **Ed25519** (EdDSA): Sign message directly per RFC-8032 (STANDARD - no pre-hashing)
/// - **Hybrid** (Ed25519+Dilithium3): Ed25519 component uses RFC-8032, PQC component uses direct signing
/// - **PQC** (Dilithium, SPHINCS+): Sign message directly (standard PQC behavior)
///
/// **Interoperability Guarantee:**
/// Ed25519 signatures created by Kanari can be verified by:
/// - Standard libraries: dalek (Rust), libsodium (C), tweetnacl, PyNaCl, etc.
/// - Any RFC-8032 compliant implementation
/// - No special handling required in external systems
///
/// This architectural choice provides domain separation between curve types
/// while maintaining compatibility with standard implementations.
pub fn sign_message_ed25519(
    private_key_hex: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    // RFC-8032 COMPLIANT: Sign the message DIRECTLY without pre-hashing
    let mut private_key_bytes = hex::decode(private_key_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key".to_string()))?;

    if private_key_bytes.len() != 32 {
        private_key_bytes.zeroize();
        return Err(SignatureError::InvalidPrivateKey(
            "Invalid private key".to_string(),
        ));
    }

    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&private_key_bytes);
    private_key_bytes.zeroize();

    let signing_key = Ed25519SigningKey::from_bytes(&key_array);

    // Sign the message directly (RFC-8032)
    let signature: Ed25519Signature = signing_key.sign(message);

    key_array.zeroize();

    Ok(signature.to_bytes().to_vec())
}
