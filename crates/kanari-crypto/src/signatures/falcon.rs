// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use zeroize::Zeroizing;

use crate::{SignatureError, signatures::falcon_provider};

pub fn verify_signature_falcon512(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    let public_key_bytes = decode_public_key(address_hex)?;
    if public_key_bytes.len() != falcon_provider::FALCON512_PUBLIC_KEY_BYTES {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid FN-DSA-512 public key".to_string(),
        ));
    }
    falcon_provider::verify_falcon512(&public_key_bytes, message, signature)
}

pub fn verify_signature_falcon1024(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    let public_key_bytes = decode_public_key(address_hex)?;
    if public_key_bytes.len() != falcon_provider::FALCON1024_PUBLIC_KEY_BYTES {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid FN-DSA-1024 public key".to_string(),
        ));
    }
    falcon_provider::verify_falcon1024(&public_key_bytes, message, signature)
}

pub fn sign_message_falcon512(
    private_key_hex: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let secret_key = decode_secret_key(private_key_hex)?;
    falcon_provider::sign_falcon512(&secret_key, message)
}

pub fn sign_message_falcon1024(
    private_key_hex: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let secret_key = decode_secret_key(private_key_hex)?;
    falcon_provider::sign_falcon1024(&secret_key, message)
}

fn decode_public_key(public_key_hex: &str) -> Result<Vec<u8>, SignatureError> {
    let raw = crate::keys::extract_raw_key(public_key_hex);
    hex::decode(raw)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid public key hex".to_string()))
}

fn decode_secret_key(private_key_hex: &str) -> Result<Zeroizing<Vec<u8>>, SignatureError> {
    let raw = crate::keys::extract_raw_key(private_key_hex);
    let secret_hex = raw.split_once(':').map(|(secret, _)| secret).unwrap_or(raw);
    Ok(Zeroizing::new(hex::decode(secret_hex).map_err(|_| {
        SignatureError::InvalidPrivateKey("Invalid private key hex".to_string())
    })?))
}
