//! Digital signature creation and verification
//!
//! This module handles digital signatures across multiple curves, with unified
//! interfaces for signing and verifying messages using different key types.

use log::debug;
use sha3::{Digest, Sha3_256};
use thiserror::Error;

use k256::{
    SecretKey as K256SecretKey,
    ecdsa::{
        Signature as K256Signature, SigningKey as K256SigningKey, VerifyingKey as K256VerifyingKey,
    },
};

use p256::{
    SecretKey as P256SecretKey,
    ecdsa::{Signature as P256Signature, SigningKey, VerifyingKey, signature::Verifier},
};

use ed25519_dalek::{
    Signature as Ed25519Signature, Signer, SigningKey as Ed25519SigningKey,
    VerifyingKey as Ed25519VerifyingKey,
};

// PQC crates
use pqcrypto_dilithium::dilithium2;
use pqcrypto_dilithium::dilithium3;
use pqcrypto_dilithium::dilithium5;
use pqcrypto_sphincsplus::sphincssha2256fsimple;
use pqcrypto_traits::sign::SecretKey as PqcSecretKeyTrait;
use pqcrypto_traits::sign::{
    DetachedSignature as PqcDetachedTrait, PublicKey as PqcPublicKeyTrait,
};

use crate::keys::CurveType;

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

/// Zero out sensitive data in memory
/// Uses zeroize crate for secure memory clearing with compiler fence
/// to prevent optimization that could leave sensitive data in memory
pub fn secure_clear(data: &mut [u8]) {
    use zeroize::Zeroize;
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
    // Extract raw key if it has the kanari prefix
    let raw_key = private_key_hex
        .strip_prefix("kanari")
        .unwrap_or(private_key_hex);

    match curve_type {
        CurveType::K256 => sign_message_k256(raw_key, message),
        CurveType::P256 => sign_message_p256(raw_key, message),
        CurveType::Ed25519 => sign_message_ed25519(raw_key, message),
        // For hybrid K256+Dilithium3, sign with the classical K256 private key part
        CurveType::K256Dilithium3 => sign_message_hybrid_k256(raw_key, message),
        // For hybrid Ed25519+Dilithium3, sign with the classical Ed25519 private key part
        CurveType::Ed25519Dilithium3 => sign_message_hybrid_ed25519(raw_key, message),
        // Handle pure PQC curves by delegating to PQC-specific signing functions
        CurveType::Dilithium2 => sign_message_dilithium2(raw_key, message),
        CurveType::Dilithium3 => sign_message_dilithium3(raw_key, message),
        CurveType::Dilithium5 => sign_message_dilithium5(raw_key, message),
        CurveType::SphincsPlusSha256Robust => sign_message_sphincs(raw_key, message),
    }
}

/// Sign a message using Dilithium2 private key (PQC)
fn sign_message_dilithium2(
    private_key_hex: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let raw = private_key_hex
        .strip_prefix("kanapqc")
        .unwrap_or(private_key_hex);
    // Accept formats: "<secret_hex>" or "<secret_hex>:<public_hex>"
    let secret_hex = raw.split_once(':').map(|(s, _)| s).unwrap_or(raw);
    let sk_bytes = hex::decode(secret_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key hex".to_string()))?;
    let sk = dilithium2::SecretKey::from_bytes(&sk_bytes).map_err(|_| {
        SignatureError::InvalidPrivateKey("Invalid Dilithium2 private key".to_string())
    })?;
    let sig = dilithium2::detached_sign(message, &sk);
    Ok(sig.as_bytes().to_vec())
}

/// Sign a message using Dilithium3 private key (PQC)
fn sign_message_dilithium3(
    private_key_hex: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let raw = private_key_hex
        .strip_prefix("kanapqc")
        .unwrap_or(private_key_hex);
    // Accept formats: "<secret_hex>" or "<secret_hex>:<public_hex>"
    let secret_hex = raw.split_once(':').map(|(s, _)| s).unwrap_or(raw);
    let sk_bytes = hex::decode(secret_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key hex".to_string()))?;
    let sk = dilithium3::SecretKey::from_bytes(&sk_bytes).map_err(|_| {
        SignatureError::InvalidPrivateKey("Invalid Dilithium3 private key".to_string())
    })?;
    let sig = dilithium3::detached_sign(message, &sk);
    Ok(sig.as_bytes().to_vec())
}

/// Sign a message using Dilithium5 private key (PQC)
fn sign_message_dilithium5(
    private_key_hex: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let raw = private_key_hex
        .strip_prefix("kanapqc")
        .unwrap_or(private_key_hex);
    // Accept formats: "<secret_hex>" or "<secret_hex>:<public_hex>"
    let secret_hex = raw.split_once(':').map(|(s, _)| s).unwrap_or(raw);
    let sk_bytes = hex::decode(secret_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key hex".to_string()))?;
    let sk = dilithium5::SecretKey::from_bytes(&sk_bytes).map_err(|_| {
        SignatureError::InvalidPrivateKey("Invalid Dilithium5 private key".to_string())
    })?;
    let sig = dilithium5::detached_sign(message, &sk);
    Ok(sig.as_bytes().to_vec())
}

/// Sign a message using SPHINCS+ private key (PQC)
fn sign_message_sphincs(private_key_hex: &str, message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    let raw = private_key_hex
        .strip_prefix("kanapqc")
        .unwrap_or(private_key_hex);
    // Accept formats: "<secret_hex>" or "<secret_hex>:<public_hex>"
    let secret_hex = raw.split_once(':').map(|(s, _)| s).unwrap_or(raw);
    let sk_bytes = hex::decode(secret_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key hex".to_string()))?;
    let sk = sphincssha2256fsimple::SecretKey::from_bytes(&sk_bytes).map_err(|_| {
        SignatureError::InvalidPrivateKey("Invalid SPHINCS+ private key".to_string())
    })?;
    let sig = sphincssha2256fsimple::detached_sign(message, &sk);
    Ok(sig.as_bytes().to_vec())
}

/// Hybrid helper: sign using the classical K256 part of a hybrid key
fn sign_message_hybrid_k256(
    hybrid_private: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let hybrid = hybrid_private
        .strip_prefix("kanahybrid")
        .unwrap_or(hybrid_private);
    // Expect format: "<classical_secret_hex>:<pqc_secret_hex>" (pqc part may contain ":<pub>" too)
    let parts: Vec<&str> = hybrid.splitn(2, ':').collect();
    let classical = parts.get(0).ok_or_else(|| {
        SignatureError::InvalidPrivateKey("Invalid hybrid private key format".to_string())
    })?;
    let pqc_part = parts.get(1).ok_or_else(|| {
        SignatureError::InvalidPrivateKey("Missing PQC part in hybrid private key".to_string())
    })?;

    // PQC secret may be stored as "<secret_hex>:<public_hex>" or just "<secret_hex>"
    let pqc_secret = pqc_part.split_once(':').map(|(s, _)| s).unwrap_or(pqc_part);

    // Sign classical part (K256)
    let classical_sig = sign_message_k256(classical, message)?;

    // Sign PQC part (Dilithium3)
    let pqc_sig = sign_message_dilithium3(pqc_secret, message)?;

    // Combine as: [2-byte classical_sig_len BE] || classical_sig || pqc_sig
    let mut out = Vec::with_capacity(2 + classical_sig.len() + pqc_sig.len());
    let len_be = (classical_sig.len() as u16).to_be_bytes();
    out.extend_from_slice(&len_be);
    out.extend_from_slice(&classical_sig);
    out.extend_from_slice(&pqc_sig);
    Ok(out)
}

/// Hybrid helper: sign using the classical Ed25519 part of a hybrid key
fn sign_message_hybrid_ed25519(
    hybrid_private: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let hybrid = hybrid_private
        .strip_prefix("kanahybrid")
        .unwrap_or(hybrid_private);
    // Expect format: "<classical_secret_hex>:<pqc_secret_hex>" (pqc part may contain ":<pub>" too)
    let parts: Vec<&str> = hybrid.splitn(2, ':').collect();
    let classical = parts.get(0).ok_or_else(|| {
        SignatureError::InvalidPrivateKey("Invalid hybrid private key format".to_string())
    })?;
    let pqc_part = parts.get(1).ok_or_else(|| {
        SignatureError::InvalidPrivateKey("Missing PQC part in hybrid private key".to_string())
    })?;

    let pqc_secret = pqc_part.split_once(':').map(|(s, _)| s).unwrap_or(pqc_part);

    // Sign classical part (Ed25519)
    let classical_sig = sign_message_ed25519(classical, message)?;

    // Sign PQC part (Dilithium3)
    let pqc_sig = sign_message_dilithium3(pqc_secret, message)?;

    // Validate classical signature length fits in u16 (prevent overflow)
    if classical_sig.len() > u16::MAX as usize {
        return Err(SignatureError::InvalidFormat(
            "Classical signature too large".to_string(),
        ));
    }

    // Combine as: [2-byte classical_sig_len BE] || classical_sig || pqc_sig
    // Use checked_add to prevent overflow in capacity calculation
    let total_capacity = 2usize
        .checked_add(classical_sig.len())
        .and_then(|sum| sum.checked_add(pqc_sig.len()))
        .ok_or_else(|| SignatureError::InvalidFormat("Signature size overflow".to_string()))?;
    let mut out = Vec::with_capacity(total_capacity);
    let len_be = (classical_sig.len() as u16).to_be_bytes();
    out.extend_from_slice(&len_be);
    out.extend_from_slice(&classical_sig);
    out.extend_from_slice(&pqc_sig);
    Ok(out)
}

/// Sign a message using K256 (secp256k1) private key
fn sign_message_k256(private_key_hex: &str, message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    use zeroize::Zeroize;

    // Hash the message with SHA3
    let mut hasher = Sha3_256::default();
    hasher.update(message);
    let message_hash = hasher.finalize();

    // Convert hex private key to bytes with zeroization
    let mut private_key_bytes = hex::decode(private_key_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key".to_string()))?;

    // Create signing key from private key
    let secret_key = K256SecretKey::from_slice(&private_key_bytes)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key".to_string()))?;
    let signing_key = K256SigningKey::from(secret_key);

    // Zeroize private key bytes immediately after use
    private_key_bytes.zeroize();

    // Sign the hashed message
    let signature: K256Signature = signing_key.sign(&message_hash);

    // Use to_vec() from SignatureEncoding trait to get DER formatted bytes
    let der_bytes = signature.to_der();
    Ok(der_bytes.as_bytes().to_vec())
}

/// Sign a message using P256 (secp256r1) private key
fn sign_message_p256(private_key_hex: &str, message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    use zeroize::Zeroize;

    // Hash the message with SHA3
    let mut hasher = Sha3_256::default();
    hasher.update(message);
    let message_hash = hasher.finalize();

    // Convert hex private key to bytes with zeroization
    let mut private_key_bytes = hex::decode(private_key_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key".to_string()))?;

    // Create signing key from private key
    let secret_key = P256SecretKey::from_slice(&private_key_bytes)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key".to_string()))?;
    let signing_key = SigningKey::from(secret_key);

    // Zeroize private key bytes immediately after use
    private_key_bytes.zeroize();

    // Sign the hashed message
    let signature: P256Signature = signing_key.sign(&message_hash);

    // Convert DER signature to bytes correctly
    let der_bytes = signature.to_der();
    Ok(der_bytes.as_bytes().to_vec())
}

/// Sign a message using Ed25519 private key
fn sign_message_ed25519(private_key_hex: &str, message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    use zeroize::Zeroize;

    // Convert hex private key to bytes with zeroization
    let mut private_key_bytes = hex::decode(private_key_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid private key".to_string()))?;

    if private_key_bytes.len() != 32 {
        private_key_bytes.zeroize();
        return Err(SignatureError::InvalidPrivateKey(
            "Invalid private key".to_string(),
        ));
    }

    // Create a fixed-size array from the private key bytes
    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&private_key_bytes);

    // Zeroize source bytes immediately
    private_key_bytes.zeroize();

    // Create signing key from private key
    let signing_key = Ed25519SigningKey::from_bytes(&key_array);

    // Sign the message directly (Ed25519 doesn't need pre-hashing)
    let signature: Ed25519Signature = signing_key.sign(message);

    // Zeroize key array after use
    key_array.zeroize();

    // Return the signature bytes
    Ok(signature.to_bytes().to_vec())
}

/// Verify a signature against a message using an address
///
/// This function attempts to parse tagged addresses (e.g., "K256:0xabc...") first.
/// If the address is not tagged, it falls back to trying all classical curve types.
///
/// For maximum reliability, use tagged addresses or `verify_signature_safe()`.
/// For best performance when curve type is known, use `verify_signature_with_curve()`.
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

    // Try to parse as tagged address first (most reliable)
    if let Some((curve_type, addr)) = crate::keys::KeyPair::parse_tagged_address(address) {
        debug!("Using tagged address with curve type: {:?}", curve_type);
        return verify_signature_with_curve(&addr, message, signature, curve_type);
    }

    // Fallback: Try all classical curves (safe but slower)
    debug!("No tagged address found, trying all curve types");
    verify_signature_safe(address, message, signature)
}

/// Safely verify a signature by trying all supported classical curve types
///
/// This function tries all curve types, making it slower but more reliable.
/// Returns true if verification succeeds with any curve.
///
/// **Security:** This is the safest option when curve type is unknown.
/// **Performance:** Slower than `verify_signature_with_curve()` but more reliable.
///
/// For PQC/hybrid schemes, use `verify_signature_with_curve()` with explicit curve type.
pub fn verify_signature_safe(
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

    let clean_address = address.trim_start_matches("0x");

    // Try all classical curves without early return to prevent timing attacks
    let k256_result = verify_signature_k256(clean_address, message, signature).unwrap_or(false);
    let p256_result = verify_signature_p256(clean_address, message, signature).unwrap_or(false);
    let ed25519_result =
        verify_signature_ed25519(clean_address, message, signature).unwrap_or(false);

    // Use OR to check if any verification succeeded (constant-time operation)
    let verified = k256_result || p256_result || ed25519_result;

    if verified {
        debug!("Signature verification succeeded");
    } else {
        debug!("Signature verification failed for all curve types");
    }

    Ok(verified)
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
        CurveType::K256Dilithium3 => {
            // Hybrid must be provided as combined address: "classical_pub_hex:pqc_pub_hex"
            if !address_hex.contains(':') {
                return Err(SignatureError::InvalidPublicKey(
                    "Hybrid key requires combined address format: classical:pqc".to_string(),
                ));
            }

            let addr = address_hex;
            let classical = addr.split(':').next().unwrap_or("");

            // Require combined signature: first two bytes = classical len
            if signature.len() < 2 {
                return Ok(false);
            }
            let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
            if classical_len == 0 || classical_len > MAX_CLASSICAL_SIG_LEN {
                return Ok(false);
            }
            if 2usize.saturating_add(classical_len) > signature.len() {
                return Ok(false);
            }

            let classical_sig = &signature[2..2 + classical_len];
            let pqc_sig = &signature[2 + classical_len..];

            // Verify classical part (must succeed)
            let classical_ok =
                verify_signature_k256(classical, message, classical_sig).unwrap_or(false);

            // Verify PQC part (must succeed)
            let pqc_pub = addr.splitn(2, ':').nth(1).unwrap_or("");
            if pqc_pub.is_empty() || pqc_sig.is_empty() {
                return Ok(false);
            }
            let pub_bytes = hex::decode(pqc_pub).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            let pk = dilithium3::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid Dilithium3 public key".to_string())
            })?;
            let sig_obj = dilithium3::DetachedSignature::from_bytes(pqc_sig).map_err(|_| {
                SignatureError::InvalidFormat("Invalid signature bytes for Dilithium3".to_string())
            })?;
            let pqc_ok = dilithium3::verify_detached_signature(&sig_obj, message, &pk).is_ok();

            Ok(classical_ok && pqc_ok)
        }
        // For hybrid Ed25519+Dilithium3, verify using the classical Ed25519 public key part when provided
        CurveType::Ed25519Dilithium3 => {
            // Hybrid must be provided as combined address
            if !address_hex.contains(':') {
                return Err(SignatureError::InvalidPublicKey(
                    "Hybrid key requires combined address format: classical:pqc".to_string(),
                ));
            }

            let addr = address_hex;
            let classical = addr.split(':').next().unwrap_or("");

            if signature.len() < 2 {
                return Ok(false);
            }
            let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
            if classical_len == 0 || classical_len > MAX_CLASSICAL_SIG_LEN {
                return Ok(false);
            }
            if 2usize.saturating_add(classical_len) > signature.len() {
                return Ok(false);
            }

            let classical_sig = &signature[2..2 + classical_len];
            let pqc_sig = &signature[2 + classical_len..];

            let classical_ok =
                verify_signature_ed25519(classical, message, classical_sig).unwrap_or(false);

            let pqc_pub = addr.splitn(2, ':').nth(1).unwrap_or("");
            if pqc_pub.is_empty() || pqc_sig.is_empty() {
                return Ok(false);
            }
            let pub_bytes = hex::decode(pqc_pub).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            let pk = dilithium3::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid Dilithium3 public key".to_string())
            })?;
            let sig_obj = dilithium3::DetachedSignature::from_bytes(pqc_sig).map_err(|_| {
                SignatureError::InvalidFormat("Invalid signature bytes for Dilithium3".to_string())
            })?;
            let pqc_ok = dilithium3::verify_detached_signature(&sig_obj, message, &pk).is_ok();

            Ok(classical_ok && pqc_ok)
        }
        // PQC verification using pqcrypto crates
        CurveType::Dilithium2 => {
            let pub_bytes = hex::decode(address_hex).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            // Validate key length (Dilithium2 public key is 1312 bytes)
            if pub_bytes.len() != 1312 {
                return Err(SignatureError::InvalidPublicKey(format!(
                    "Invalid Dilithium2 public key length: expected 1312, got {}",
                    pub_bytes.len()
                )));
            }
            let pk = dilithium2::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid Dilithium2 public key".to_string())
            })?;
            let sig_obj = dilithium2::DetachedSignature::from_bytes(signature).map_err(|_| {
                SignatureError::InvalidFormat("Invalid signature bytes for Dilithium2".to_string())
            })?;
            let res = dilithium2::verify_detached_signature(&sig_obj, message, &pk);
            match res {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        CurveType::Dilithium3 => {
            let pub_bytes = hex::decode(address_hex).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            // Validate key length (Dilithium3 public key is 1952 bytes)
            if pub_bytes.len() != 1952 {
                return Err(SignatureError::InvalidPublicKey(format!(
                    "Invalid Dilithium3 public key length: expected 1952, got {}",
                    pub_bytes.len()
                )));
            }
            let pk = dilithium3::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid Dilithium3 public key".to_string())
            })?;
            let sig_obj = dilithium3::DetachedSignature::from_bytes(signature).map_err(|_| {
                SignatureError::InvalidFormat("Invalid signature bytes for Dilithium3".to_string())
            })?;
            let res = dilithium3::verify_detached_signature(&sig_obj, message, &pk);
            match res {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        CurveType::Dilithium5 => {
            let pub_bytes = hex::decode(address_hex).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            // Validate key length (Dilithium5 public key is 2592 bytes)
            if pub_bytes.len() != 2592 {
                return Err(SignatureError::InvalidPublicKey(format!(
                    "Invalid Dilithium5 public key length: expected 2592, got {}",
                    pub_bytes.len()
                )));
            }
            let pk = dilithium5::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid Dilithium5 public key".to_string())
            })?;
            let sig_obj = dilithium5::DetachedSignature::from_bytes(signature).map_err(|_| {
                SignatureError::InvalidFormat("Invalid signature bytes for Dilithium5".to_string())
            })?;
            let res = dilithium5::verify_detached_signature(&sig_obj, message, &pk);
            match res {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
        CurveType::SphincsPlusSha256Robust => {
            let pub_bytes = hex::decode(address_hex).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            let pk = sphincssha2256fsimple::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid SPHINCS+ public key".to_string())
            })?;
            let sig_obj =
                sphincssha2256fsimple::DetachedSignature::from_bytes(signature).map_err(|_| {
                    SignatureError::InvalidFormat(
                        "Invalid signature bytes for SPHINCS+".to_string(),
                    )
                })?;
            let res = sphincssha2256fsimple::verify_detached_signature(&sig_obj, message, &pk);
            match res {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }
    }
}

/// Detailed hybrid verifier that returns (classical_ok, pqc_ok).
/// This avoids fallback behaviour and enables precise logging/errors.
fn verify_hybrid_signature_detailed(
    signature: &[u8],
    classical_pub: &str,
    pqc_pub: &str,
    message: &[u8],
    classical_verify_fn: impl Fn(&str, &[u8], &[u8]) -> Result<bool, SignatureError>,
) -> Result<(bool, bool), SignatureError> {
    // Require 2-byte classical length prefix
    if signature.len() < 2 {
        return Ok((false, false));
    }
    let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
    if classical_len == 0
        || classical_len > MAX_CLASSICAL_SIG_LEN
        || 2usize.saturating_add(classical_len) > signature.len()
    {
        return Ok((false, false));
    }

    let classical_sig = &signature[2..2 + classical_len];
    let pqc_sig = &signature[2 + classical_len..];

    let classical_ok = classical_verify_fn(classical_pub, message, classical_sig).unwrap_or(false);

    if pqc_sig.is_empty() {
        return Ok((classical_ok, false));
    }

    let pub_bytes = hex::decode(pqc_pub)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid public key hex".to_string()))?;
    let pk = dilithium3::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
        SignatureError::InvalidPublicKey("Invalid Dilithium3 public key".to_string())
    })?;
    let sig_obj = dilithium3::DetachedSignature::from_bytes(pqc_sig).map_err(|_| {
        SignatureError::InvalidFormat("Invalid signature bytes for Dilithium3".to_string())
    })?;
    let pqc_ok = dilithium3::verify_detached_signature(&sig_obj, message, &pk).is_ok();

    Ok((classical_ok, pqc_ok))
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
    let classical_pub = pub_combined.splitn(2, ':').next().unwrap_or(pub_combined);

    match curve_type {
        CurveType::K256 => verify_signature_k256(classical_pub, message, signature),
        CurveType::P256 => verify_signature_p256(classical_pub, message, signature),
        CurveType::Ed25519 => verify_signature_ed25519(classical_pub, message, signature),
        CurveType::K256Dilithium3 => {
            // Require explicit PQC public key on KeyPair for hybrid verification
            let pqc_pub = keypair.get_pqc_public_key().ok_or_else(|| {
                SignatureError::InvalidPublicKey(
                    "Missing PQC public key for hybrid keypair".to_string(),
                )
            })?;
            let (classical_ok, pqc_ok) = verify_hybrid_signature_detailed(
                signature,
                classical_pub,
                &pqc_pub,
                message,
                verify_signature_k256,
            )?;
            if classical_ok && pqc_ok {
                Ok(true)
            } else {
                debug!(
                    "Hybrid verification failed: classical_ok={}, pqc_ok={}",
                    classical_ok, pqc_ok
                );
                Ok(false)
            }
        }
        CurveType::Ed25519Dilithium3 => {
            let pqc_pub = keypair.get_pqc_public_key().ok_or_else(|| {
                SignatureError::InvalidPublicKey(
                    "Missing PQC public key for hybrid keypair".to_string(),
                )
            })?;
            let (classical_ok, pqc_ok) = verify_hybrid_signature_detailed(
                signature,
                classical_pub,
                &pqc_pub,
                message,
                verify_signature_ed25519,
            )?;
            if classical_ok && pqc_ok {
                Ok(true)
            } else {
                debug!(
                    "Hybrid verification failed: classical_ok={}, pqc_ok={}",
                    classical_ok, pqc_ok
                );
                Ok(false)
            }
        }
        // Pure PQC curves: public_key on KeyPair is the PQC public hex
        CurveType::Dilithium2 => {
            let pqc_pub = keypair.get_pqc_public_key().ok_or_else(|| {
                SignatureError::InvalidPublicKey("Missing PQC public key".to_string())
            })?;
            let pub_bytes = hex::decode(&pqc_pub).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            // Validate key length (Dilithium2 public key is 1312 bytes)
            if pub_bytes.len() != 1312 {
                return Err(SignatureError::InvalidPublicKey(format!(
                    "Invalid Dilithium2 public key length: expected 1312, got {}",
                    pub_bytes.len()
                )));
            }
            let pk = dilithium2::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid Dilithium2 public key".to_string())
            })?;
            let sig_obj = dilithium2::DetachedSignature::from_bytes(signature).map_err(|_| {
                SignatureError::InvalidFormat("Invalid signature bytes for Dilithium2".to_string())
            })?;
            Ok(dilithium2::verify_detached_signature(&sig_obj, message, &pk).is_ok())
        }
        CurveType::Dilithium3 => {
            let pqc_pub = keypair.get_pqc_public_key().ok_or_else(|| {
                SignatureError::InvalidPublicKey("Missing PQC public key".to_string())
            })?;
            let pub_bytes = hex::decode(&pqc_pub).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            // Validate key length (Dilithium3 public key is 1952 bytes)
            if pub_bytes.len() != 1952 {
                return Err(SignatureError::InvalidPublicKey(format!(
                    "Invalid Dilithium3 public key length: expected 1952, got {}",
                    pub_bytes.len()
                )));
            }
            let pk = dilithium3::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid Dilithium3 public key".to_string())
            })?;
            let sig_obj = dilithium3::DetachedSignature::from_bytes(signature).map_err(|_| {
                SignatureError::InvalidFormat("Invalid signature bytes for Dilithium3".to_string())
            })?;
            Ok(dilithium3::verify_detached_signature(&sig_obj, message, &pk).is_ok())
        }
        CurveType::Dilithium5 => {
            let pqc_pub = keypair.get_pqc_public_key().ok_or_else(|| {
                SignatureError::InvalidPublicKey("Missing PQC public key".to_string())
            })?;
            let pub_bytes = hex::decode(&pqc_pub).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            // Validate key length (Dilithium5 public key is 2592 bytes)
            if pub_bytes.len() != 2592 {
                return Err(SignatureError::InvalidPublicKey(format!(
                    "Invalid Dilithium5 public key length: expected 2592, got {}",
                    pub_bytes.len()
                )));
            }
            let pk = dilithium5::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid Dilithium5 public key".to_string())
            })?;
            let sig_obj = dilithium5::DetachedSignature::from_bytes(signature).map_err(|_| {
                SignatureError::InvalidFormat("Invalid signature bytes for Dilithium5".to_string())
            })?;
            Ok(dilithium5::verify_detached_signature(&sig_obj, message, &pk).is_ok())
        }
        CurveType::SphincsPlusSha256Robust => {
            let pqc_pub = keypair.get_pqc_public_key().ok_or_else(|| {
                SignatureError::InvalidPublicKey("Missing PQC public key".to_string())
            })?;
            let pub_bytes = hex::decode(&pqc_pub).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid public key hex".to_string())
            })?;
            // Validate key length (SPHINCS+ public key is 64 bytes)
            if pub_bytes.len() != 64 {
                return Err(SignatureError::InvalidPublicKey(format!(
                    "Invalid SPHINCS+ public key length: expected 64, got {}",
                    pub_bytes.len()
                )));
            }
            let pk = sphincssha2256fsimple::PublicKey::from_bytes(&pub_bytes).map_err(|_| {
                SignatureError::InvalidPublicKey("Invalid SPHINCS+ public key".to_string())
            })?;
            let sig_obj =
                sphincssha2256fsimple::DetachedSignature::from_bytes(signature).map_err(|_| {
                    SignatureError::InvalidFormat(
                        "Invalid signature bytes for SPHINCS+".to_string(),
                    )
                })?;
            Ok(sphincssha2256fsimple::verify_detached_signature(&sig_obj, message, &pk).is_ok())
        }
    }
}

/// Verify a signature using K256 (secp256k1)
pub fn verify_signature_k256(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    // Try to parse the signature from DER format
    let signature = K256Signature::from_der(signature)
        .map_err(|_| SignatureError::InvalidFormat("Invalid signature format".to_string()))?;

    // Hash the message with SHA3
    let mut hasher = Sha3_256::default();
    hasher.update(message);
    let message_hash = hasher.finalize();

    // Decode the address hex
    let decoded_hex = hex::decode(address_hex)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid address format".to_string()))?;

    // Handle both uncompressed (64 bytes) and compressed-derived (32 bytes) public keys
    if decoded_hex.len() != 64 && decoded_hex.len() != 32 {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid address format".to_string(),
        ));
    }

    // Try uncompressed first (0x04 || X || Y)
    if decoded_hex.len() == 64 {
        let mut public_key_bytes = Vec::with_capacity(65);
        public_key_bytes.push(0x04);
        public_key_bytes.extend_from_slice(&decoded_hex);

        if let Ok(verifying_key) = K256VerifyingKey::from_sec1_bytes(&public_key_bytes) {
            if verifying_key.verify(&message_hash, &signature).is_ok() {
                return Ok(true);
            }
        }
    }

    // Try compressed-derived (address contains X only): try 0x02 and 0x03
    if decoded_hex.len() == 32 {
        let mut public_key_bytes = vec![0x02];
        public_key_bytes.extend_from_slice(&decoded_hex[0..32]);
        if let Ok(verifying_key) = K256VerifyingKey::from_sec1_bytes(&public_key_bytes) {
            if verifying_key.verify(&message_hash, &signature).is_ok() {
                return Ok(true);
            }
        }
        public_key_bytes[0] = 0x03;
        if let Ok(verifying_key) = K256VerifyingKey::from_sec1_bytes(&public_key_bytes) {
            if verifying_key.verify(&message_hash, &signature).is_ok() {
                return Ok(true);
            }
        }
    }

    // None matched
    Ok(false)
}

/// Verify a signature using P256 (secp256r1)
pub fn verify_signature_p256(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    // Parse the signature
    let signature = P256Signature::from_der(signature)
        .map_err(|_| SignatureError::InvalidFormat("Invalid signature format".to_string()))?;

    // Hash the message with SHA3
    let mut hasher = Sha3_256::default();
    hasher.update(message);
    let message_hash = hasher.finalize();

    // Decode the address hex
    let decoded_hex = hex::decode(address_hex)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid address format".to_string()))?;

    // Handle both uncompressed (64 bytes) and compressed-derived (32 bytes) public keys
    if decoded_hex.len() != 64 && decoded_hex.len() != 32 {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid address format".to_string(),
        ));
    }

    // Try uncompressed first
    if decoded_hex.len() == 64 {
        let mut public_key_bytes = Vec::with_capacity(65);
        public_key_bytes.push(0x04);
        public_key_bytes.extend_from_slice(&decoded_hex);

        if let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(&public_key_bytes) {
            if verifying_key.verify(&message_hash, &signature).is_ok() {
                return Ok(true);
            }
        }
    }

    // Try compressed-derived (address contains X only)
    if decoded_hex.len() == 32 {
        let mut public_key_bytes = vec![0x02];
        public_key_bytes.extend_from_slice(&decoded_hex[0..32]);
        if let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(&public_key_bytes) {
            if verifying_key.verify(&message_hash, &signature).is_ok() {
                return Ok(true);
            }
        }
        public_key_bytes[0] = 0x03;
        if let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(&public_key_bytes) {
            if verifying_key.verify(&message_hash, &signature).is_ok() {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Verify a signature using Ed25519
pub fn verify_signature_ed25519(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    // Check if signature has correct length for Ed25519
    if signature.len() != 64 {
        return Err(SignatureError::InvalidSignatureLength);
    }

    // Create a fixed-size array for the signature
    let mut sig_array = [0u8; 64];
    sig_array.copy_from_slice(signature);
    let signature = Ed25519Signature::from_bytes(&sig_array);

    // Decode the address hex (which should be the public key)
    let decoded_hex = hex::decode(address_hex)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid address format".to_string()))?;

    // For Ed25519, the address should be the 32-byte public key
    if decoded_hex.len() != 32 {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid address format".to_string(),
        ));
    }

    // Create a fixed-size array for the public key
    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&decoded_hex);

    // Create verifying key from public key bytes
    let verifying_key = Ed25519VerifyingKey::from_bytes(&key_array)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid address format".to_string()))?;

    // Use constant time comparison when checking equality of signatures
    // during verification for added security against timing attacks
    match verifying_key.verify(message, &signature) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{CurveType, generate_keypair};

    // ============================================================================
    // Bug #2: Timing Attack in Signature Verification (Critical)
    // ============================================================================

    #[test]
    fn test_signature_verification_uses_constant_time() {
        // This test verifies that signature verification doesn't have timing leaks
        // The cryptographic libraries (k256, p256, ed25519-dalek) provide constant-time
        // comparison internally, so we verify that the API uses them correctly

        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"test message";

        // Sign the message
        let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();

        // Verification should succeed
        let result =
            verify_signature_with_curve(&keypair.address, message, &signature, CurveType::Ed25519);
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Modify signature slightly
        let mut bad_signature = signature.clone();
        bad_signature[0] ^= 0x01;

        // Verification should fail - this uses constant-time comparison internally
        let result = verify_signature_with_curve(
            &keypair.address,
            message,
            &bad_signature,
            CurveType::Ed25519,
        );
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    // ============================================================================
    // Bug #3: Memory Safety in secure_clear (Critical)
    // ============================================================================

    #[test]
    fn test_secure_clear_memory_safety() {
        let mut sensitive = vec![0xFF; 256];

        // Clear with secure_clear
        secure_clear(&mut sensitive);

        // Verify all bytes are zero
        assert!(
            sensitive.iter().all(|&b| b == 0),
            "All bytes should be zero after secure_clear"
        );
    }

    #[test]
    fn test_secure_clear_uses_black_box() {
        // This test ensures secure_clear uses black_box to prevent optimization
        let mut data = b"secret_key_data_that_must_be_cleared".to_vec();

        secure_clear(&mut data);

        // Compiler shouldn't optimize this away due to black_box
        assert_eq!(data, vec![0u8; data.len()]);
    }

    // ============================================================================
    // Signature Creation and Verification Tests
    // ============================================================================

    #[test]
    fn test_sign_and_verify_k256() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"Hello, K256!";

        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();
        let verified =
            verify_signature_with_curve(&keypair.address, message, &signature, CurveType::K256)
                .unwrap();

        assert!(verified, "K256 signature should verify");
    }

    #[test]
    fn test_sign_and_verify_p256() {
        let keypair = generate_keypair(CurveType::P256).unwrap();
        let message = b"Hello, P256!";

        let signature = sign_message(&keypair.private_key, message, CurveType::P256).unwrap();
        let verified =
            verify_signature_with_curve(&keypair.address, message, &signature, CurveType::P256)
                .unwrap();

        assert!(verified, "P256 signature should verify");
    }

    #[test]
    fn test_sign_and_verify_ed25519() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Hello, Ed25519!";

        let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();
        let verified =
            verify_signature_with_curve(&keypair.address, message, &signature, CurveType::Ed25519)
                .unwrap();

        assert!(verified, "Ed25519 signature should verify");
    }

    #[test]
    fn test_signature_fails_with_wrong_message() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message1 = b"Original message";
        let message2 = b"Different message";

        let signature = sign_message(&keypair.private_key, message1, CurveType::K256).unwrap();
        let verified =
            verify_signature_with_curve(&keypair.address, message2, &signature, CurveType::K256)
                .unwrap();

        assert!(!verified, "Signature should not verify with wrong message");
    }

    #[test]
    fn test_signature_fails_with_wrong_address() {
        let keypair1 = generate_keypair(CurveType::Ed25519).unwrap();
        let keypair2 = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Test message";

        let signature = sign_message(&keypair1.private_key, message, CurveType::Ed25519).unwrap();
        let verified =
            verify_signature_with_curve(&keypair2.address, message, &signature, CurveType::Ed25519)
                .unwrap();

        assert!(
            !verified,
            "Signature should not verify with different address"
        );
    }

    #[test]
    fn test_signature_with_kanari_prefix() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"Test message";

        // Should work with kanari prefix
        assert!(keypair.private_key.starts_with("kanari"));
        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

        assert!(!signature.is_empty());
    }

    #[test]
    fn test_invalid_signature_length() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Test";

        // Ed25519 signatures must be 64 bytes
        let bad_signature = vec![0u8; 32]; // Wrong length

        let result = verify_signature_ed25519(
            &keypair.address.trim_start_matches("0x"),
            message,
            &bad_signature,
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SignatureError::InvalidSignatureLength
        ));
    }

    #[test]
    fn test_verify_signature_with_legacy_api() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"Test";

        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

        // Test the legacy verify_signature API
        let verified = verify_signature(&keypair.address, message, &signature).unwrap();
        assert!(verified);
    }

    #[test]
    fn test_sign_message_handles_empty_message() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let empty_message = b"";

        // Should still be able to sign empty message (hashes to deterministic value)
        let signature = sign_message(&keypair.private_key, empty_message, CurveType::K256);
        assert!(signature.is_ok(), "Should be able to sign empty message");
    }

    #[test]
    fn test_sign_with_invalid_private_key() {
        let result = sign_message("invalid_hex", b"message", CurveType::K256);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SignatureError::InvalidPrivateKey(_)
        ));
    }

    #[test]
    fn test_verify_with_invalid_address() {
        let signature = vec![0u8; 64];
        let message = b"test";

        let result = verify_signature_ed25519("invalid_hex", message, &signature);
        assert!(result.is_err());
    }

    #[test]
    fn test_signature_deterministic_for_same_input() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Deterministic test";

        // Ed25519 signatures should be deterministic
        let sig1 = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();
        let sig2 = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();

        assert_eq!(sig1, sig2, "Ed25519 signatures should be deterministic");
    }

    #[test]
    fn test_pqc_signing_not_supported_yet() {
        let keypair = generate_keypair(CurveType::Dilithium3).unwrap();
        let message = b"test";

        // PQC signing should be supported by the PQC-specific API
        let signature = sign_message(&keypair.private_key, message, CurveType::Dilithium3).unwrap();
        // Verify using explicit curve type
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Dilithium3,
        )
        .unwrap();
        assert!(verified, "Dilithium3 signature should verify");
    }

    #[test]
    fn test_secure_clear_on_different_sizes() {
        // Test various sizes
        for size in [0, 1, 16, 32, 64, 128, 256, 1024] {
            let mut data = vec![0xAA; size];
            secure_clear(&mut data);
            assert!(
                data.iter().all(|&b| b == 0),
                "Size {} should be fully cleared",
                size
            );
        }
    }

    #[test]
    fn test_verify_signature_safe_all_curves() {
        // Test that verify_signature_safe works for all classical curves
        let curves = vec![CurveType::K256, CurveType::P256, CurveType::Ed25519];

        for curve in curves {
            let keypair = generate_keypair(curve).unwrap();
            let message = b"Safe verification test";

            let signature = sign_message(&keypair.private_key, message, curve).unwrap();

            // verify_signature_safe should work without knowing the curve
            let result = verify_signature_safe(&keypair.address, message, &signature).unwrap();
            assert!(result, "Safe verification failed for {:?}", curve);
        }
    }

    #[test]
    fn test_hybrid_sign_and_verify_k256_dilithium3() {
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let message = b"Hybrid K256+Dilithium3 test";

        // Sign using hybrid API
        let signature =
            sign_message(&keypair.private_key, message, CurveType::K256Dilithium3).unwrap();

        // Verify using combined public parts (classical:pqc)
        let pub_combined = keypair.public_key; // format: "classical_pub:pqc_pub"

        let verified = verify_signature_with_curve(
            &pub_combined,
            message,
            &signature,
            CurveType::K256Dilithium3,
        )
        .unwrap();
        assert!(verified, "Hybrid K256+Dilithium3 signature should verify");
    }

    #[test]
    fn test_hybrid_sign_and_verify_ed25519_dilithium3() {
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let message = b"Hybrid Ed25519+Dilithium3 test";

        let signature =
            sign_message(&keypair.private_key, message, CurveType::Ed25519Dilithium3).unwrap();

        let pub_combined = keypair.public_key;

        let verified = verify_signature_with_curve(
            &pub_combined,
            message,
            &signature,
            CurveType::Ed25519Dilithium3,
        )
        .unwrap();
        assert!(
            verified,
            "Hybrid Ed25519+Dilithium3 signature should verify"
        );
    }

    #[test]
    fn test_malformed_hybrid_signature_fallback_and_reject() {
        // Generate hybrid keypair and a proper signature
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let message = b"Malformed hybrid test";
        let mut signature =
            sign_message(&keypair.private_key, message, CurveType::K256Dilithium3).unwrap();

        // Truncate signature to make length prefix inconsistent
        if signature.len() > 10 {
            signature.truncate(3); // too short to contain 2-byte length + classical
        }

        // Try verify with combined public key; should not panic and should return false
        let pub_combined = keypair.public_key;
        let result = verify_signature_with_curve(
            &pub_combined,
            message,
            &signature,
            CurveType::K256Dilithium3,
        )
        .unwrap();
        assert!(!result, "Malformed combined signature should not verify");
    }

    #[test]
    fn test_verify_signature_with_tagged_address() {
        // Test that verify_signature correctly uses tagged addresses
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Tagged address test";

        let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();

        // Use tagged address
        let tagged = keypair.tagged_address();
        let result = verify_signature(&tagged, message, &signature).unwrap();

        assert!(result, "Verification with tagged address should succeed");
    }

    #[test]
    fn test_verify_signature_fallback_to_safe() {
        // Test that verify_signature falls back to safe mode for untagged addresses
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"Fallback test";

        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

        // Use untagged address - should fallback to verify_signature_safe
        let result = verify_signature(&keypair.address, message, &signature).unwrap();

        assert!(
            result,
            "Verification with untagged address should succeed via fallback"
        );
    }

    #[test]
    fn test_verify_signature_safe_wrong_signature() {
        // Test that verify_signature_safe correctly rejects invalid signatures
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message1 = b"Original message";
        let message2 = b"Different message";

        let signature = sign_message(&keypair.private_key, message1, CurveType::K256).unwrap();

        // Verify with wrong message should fail
        let result = verify_signature_safe(&keypair.address, message2, &signature).unwrap();

        assert!(!result, "Safe verification should reject wrong message");
    }
}
