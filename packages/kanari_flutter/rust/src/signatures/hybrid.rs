// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::{
    SignatureError,
    signatures::{
        MAX_CLASSICAL_SIG_LEN,
        dilithium3::{sign_message_dilithium3, verify_signature_dilithium3},
        ed25519::verify_signature_ed25519,
        k256::sign_message_k256,
        k256::verify_signature_k256,
        sign_message_ed25519,
    },
};

/// Detailed hybrid verifier that returns (classical_ok, pqc_ok).
/// This avoids fallback behaviour and enables precise logging/errors.
pub fn verify_hybrid_signature_detailed(
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

    let pqc_ok = verify_signature_dilithium3(pqc_pub, message, pqc_sig)?;

    Ok((classical_ok, pqc_ok))
}

/// Hybrid helper: sign using the classical K256 part of a hybrid key
pub fn sign_message_hybrid_k256(
    hybrid_private: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let hybrid = crate::keys::extract_raw_key(hybrid_private);
    // Expect format: "<classical_secret_hex>:<pqc_secret_hex>" (pqc part may contain ":<pub>" too)
    let parts: Vec<&str> = hybrid.splitn(2, ':').collect();
    let classical = parts.first().ok_or_else(|| {
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

    // Validate classical signature length before encoding into u16 and combining
    if classical_sig.len() > MAX_CLASSICAL_SIG_LEN || classical_sig.len() > u16::MAX as usize {
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

/// Hybrid helper: sign using the classical Ed25519 part of a hybrid key
pub fn sign_message_hybrid_ed25519(
    hybrid_private: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let hybrid = crate::keys::extract_raw_key(hybrid_private);
    // Expect format: "<classical_secret_hex>:<pqc_secret_hex>" (pqc part may contain ":<pub>" too)
    let parts: Vec<&str> = hybrid.splitn(2, ':').collect();
    let classical = parts.first().ok_or_else(|| {
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

    // Validate classical signature length fits in u16 and within configured limits (prevent overflow / DoS)
    if classical_sig.len() > MAX_CLASSICAL_SIG_LEN || classical_sig.len() > u16::MAX as usize {
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

/// Verify K256+Dilithium3 hybrid signature
pub fn verify_k256dilithium3(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
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
    if classical_len == 0
        || classical_len > MAX_CLASSICAL_SIG_LEN
        || 2usize.saturating_add(classical_len) > signature.len()
    {
        return Ok(false);
    }

    let classical_sig = &signature[2..2 + classical_len];
    let pqc_sig = &signature[2 + classical_len..];

    let classical_ok = verify_signature_k256(classical, message, classical_sig).unwrap_or(false);
    if pqc_sig.is_empty() {
        return Ok(false);
    }

    let parts: Vec<&str> = addr.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Ok(false);
    }
    let pqc_pub = parts[1];
    let pqc_ok = verify_signature_dilithium3(pqc_pub, message, pqc_sig)?;

    Ok(classical_ok && pqc_ok)
}

/// Verify Ed25519+Dilithium3 hybrid signature
pub fn verify_ed25519dilithium3(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
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
    if classical_len == 0
        || classical_len > MAX_CLASSICAL_SIG_LEN
        || 2usize.saturating_add(classical_len) > signature.len()
    {
        return Ok(false);
    }

    let classical_sig = &signature[2..2 + classical_len];
    let pqc_sig = &signature[2 + classical_len..];

    let classical_ok = verify_signature_ed25519(classical, message, classical_sig).unwrap_or(false);
    if pqc_sig.is_empty() {
        return Ok(false);
    }

    let parts: Vec<&str> = addr.splitn(2, ':').collect();
    if parts.len() != 2 {
        return Ok(false);
    }
    let pqc_pub = parts[1];
    let pqc_ok = verify_signature_dilithium3(pqc_pub, message, pqc_sig)?;

    Ok(classical_ok && pqc_ok)
}
