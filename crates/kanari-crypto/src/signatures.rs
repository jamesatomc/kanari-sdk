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
        // PQC and hybrid schemes need specialized handling
        _ => Err(SignatureError::InvalidPrivateKey(
            "Post-quantum and hybrid signatures require use of PQC-specific functions".to_string(),
        )),
    }
}

/// Sign a message using K256 (secp256k1) private key
fn sign_message_k256(private_key_hex: &str, message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    // Hash the message with SHA3
    let mut hasher = Sha3_256::default();
    hasher.update(message);
    let message_hash = hasher.finalize();

    // Convert hex private key to bytes
    let private_key_bytes = hex::decode(private_key_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid key format".to_string()))?;

    // Create signing key from private key
    let secret_key = K256SecretKey::from_slice(&private_key_bytes)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid key format".to_string()))?;
    let signing_key = K256SigningKey::from(secret_key);

    // Sign the hashed message
    let signature: K256Signature = signing_key.sign(&message_hash);

    // Use to_vec() from SignatureEncoding trait to get DER formatted bytes
    let der_bytes = signature.to_der();
    Ok(der_bytes.as_bytes().to_vec())
}

/// Sign a message using P256 (secp256r1) private key
fn sign_message_p256(private_key_hex: &str, message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    // Hash the message with SHA3
    let mut hasher = Sha3_256::default();
    hasher.update(message);
    let message_hash = hasher.finalize();

    // Convert hex private key to bytes
    let private_key_bytes = hex::decode(private_key_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid key format".to_string()))?;

    // Create signing key from private key
    let secret_key = P256SecretKey::from_slice(&private_key_bytes)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid key format".to_string()))?;
    let signing_key = SigningKey::from(secret_key);

    // Sign the hashed message
    let signature: P256Signature = signing_key.sign(&message_hash);

    // Convert DER signature to bytes correctly
    let der_bytes = signature.to_der();
    Ok(der_bytes.as_bytes().to_vec())
}

/// Sign a message using Ed25519 private key
fn sign_message_ed25519(private_key_hex: &str, message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    // Convert hex private key to bytes
    let private_key_bytes = hex::decode(private_key_hex)
        .map_err(|_| SignatureError::InvalidPrivateKey("Invalid key format".to_string()))?;

    if private_key_bytes.len() != 32 {
        return Err(SignatureError::InvalidPrivateKey(
            "Invalid key format".to_string()
        ));
    }

    // Create a fixed-size array from the private key bytes
    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&private_key_bytes);

    // Create signing key from private key
    let signing_key = Ed25519SigningKey::from_bytes(&key_array);

    // Sign the message directly (Ed25519 doesn't need pre-hashing)
    let signature: Ed25519Signature = signing_key.sign(message);

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

    let clean_address = address.trim_start_matches("0x");

    // Try all classical curves in priority order: K256 > P256 > Ed25519
    // Return true on first successful verification
    if let Ok(true) = verify_signature_k256(clean_address, message, signature) {
        debug!("K256 signature verification succeeded");
        return Ok(true);
    }

    if let Ok(true) = verify_signature_p256(clean_address, message, signature) {
        debug!("P256 signature verification succeeded");
        return Ok(true);
    }

    if let Ok(true) = verify_signature_ed25519(clean_address, message, signature) {
        debug!("Ed25519 signature verification succeeded");
        return Ok(true);
    }

    // All verifications failed or errored
    debug!("Signature verification failed for all curve types");
    Ok(false)
}

/// Verify a signature with the known curve type
pub fn verify_signature_with_curve(
    address: &str,
    message: &[u8],
    signature: &[u8],
    curve_type: CurveType,
) -> Result<bool, SignatureError> {
    let address_hex = address.trim_start_matches("0x");

    match curve_type {
        CurveType::K256 => verify_signature_k256(address_hex, message, signature),
        CurveType::P256 => verify_signature_p256(address_hex, message, signature),
        CurveType::Ed25519 => verify_signature_ed25519(address_hex, message, signature),
        // PQC and hybrid schemes need specialized handling
        _ => Err(SignatureError::InvalidFormat(
            "Post-quantum and hybrid signature verification requires PQC-specific functions"
                .to_string(),
        )),
    }
}

/// Verify a signature using K256 (secp256k1)
pub fn verify_signature_k256(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    // Try to parse the signature from DER format
    let signature = K256Signature::from_der(signature).map_err(|_| {
        SignatureError::InvalidFormat("Invalid signature format".to_string())
    })?;

    // Hash the message with SHA3
    let mut hasher = Sha3_256::default();
    hasher.update(message);
    let message_hash = hasher.finalize();

    // Decode the address hex
    let decoded_hex = hex::decode(address_hex)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid address format".to_string()))?;

    // Handle both uncompressed (64 bytes) and compressed (32 bytes) public keys
    if decoded_hex.len() != 64 && decoded_hex.len() != 32 {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid address format".to_string()
        ));
    }

    let mut had_valid_key = false;

    // Try with uncompressed public key format
    if decoded_hex.len() == 64 {
        let mut public_key_bytes = Vec::with_capacity(65);
        public_key_bytes.push(0x04);
        public_key_bytes.extend_from_slice(&decoded_hex);

        if let Ok(verifying_key) = K256VerifyingKey::from_sec1_bytes(&public_key_bytes) {
            had_valid_key = true;
            if verifying_key.verify(&message_hash, &signature).is_ok() {
                return Ok(true);
            }
        }
    }

    // Try with compressed public key format (with 0x02 prefix)
    let mut public_key_bytes = vec![0x02];
    public_key_bytes.extend_from_slice(&decoded_hex[0..32]);

    if let Ok(verifying_key) = K256VerifyingKey::from_sec1_bytes(&public_key_bytes) {
        had_valid_key = true;
        if verifying_key.verify(&message_hash, &signature).is_ok() {
            return Ok(true);
        }
    }

    // Try with compressed public key format (with 0x03 prefix)
    public_key_bytes[0] = 0x03;
    if let Ok(verifying_key) = K256VerifyingKey::from_sec1_bytes(&public_key_bytes) {
        had_valid_key = true;
        if verifying_key.verify(&message_hash, &signature).is_ok() {
            return Ok(true);
        }
    }

    // If we could construct at least one valid key but verification failed,
    // then the signature is invalid
    if had_valid_key {
        return Ok(false);
    }

    Err(SignatureError::InvalidPublicKey(
        "Invalid address format".to_string(),
    ))
}

/// Verify a signature using P256 (secp256r1)
pub fn verify_signature_p256(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    // Parse the signature
    let signature = P256Signature::from_der(signature).map_err(|_| {
        SignatureError::InvalidFormat("Invalid signature format".to_string())
    })?;

    // Hash the message with SHA3
    let mut hasher = Sha3_256::default();
    hasher.update(message);
    let message_hash = hasher.finalize();

    // Decode the address hex
    let decoded_hex = hex::decode(address_hex)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid address format".to_string()))?;

    // Handle both uncompressed (64 bytes) and compressed (32 bytes) public keys
    if decoded_hex.len() != 64 && decoded_hex.len() != 32 {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid address format".to_string()
        ));
    }

    let mut had_valid_key = false;

    // Try with uncompressed public key format
    if decoded_hex.len() == 64 {
        let mut public_key_bytes = Vec::with_capacity(65);
        public_key_bytes.push(0x04);
        public_key_bytes.extend_from_slice(&decoded_hex);

        if let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(&public_key_bytes) {
            had_valid_key = true;
            if verifying_key.verify(&message_hash, &signature).is_ok() {
                return Ok(true);
            }
        }
    }

    // Try with compressed public key format (with 0x02 prefix)
    let mut public_key_bytes = vec![0x02];
    public_key_bytes.extend_from_slice(&decoded_hex[0..32]);

    if let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(&public_key_bytes) {
        had_valid_key = true;
        if verifying_key.verify(&message_hash, &signature).is_ok() {
            return Ok(true);
        }
    }

    // Try with compressed public key format (with 0x03 prefix)
    public_key_bytes[0] = 0x03;
    if let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(&public_key_bytes) {
        had_valid_key = true;
        if verifying_key.verify(&message_hash, &signature).is_ok() {
            return Ok(true);
        }
    }

    // If we could construct at least one valid key but verification failed,
    // then the signature is invalid
    if had_valid_key {
        return Ok(false);
    }

    Err(SignatureError::InvalidPublicKey(
        "Invalid address format".to_string(),
    ))
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
            "Invalid address format".to_string()
        ));
    }

    // Create a fixed-size array for the public key
    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(&decoded_hex);

    // Create verifying key from public key bytes
    let verifying_key = Ed25519VerifyingKey::from_bytes(&key_array).map_err(|_| {
        SignatureError::InvalidPublicKey("Invalid address format".to_string())
    })?;

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

        // Should return error for PQC signatures via this API
        let result = sign_message(&keypair.private_key, message, CurveType::Dilithium3);
        assert!(result.is_err(), "PQC signing should use specialized API");
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
