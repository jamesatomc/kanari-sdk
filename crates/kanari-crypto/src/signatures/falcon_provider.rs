// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use zeroize::Zeroizing;

use crate::SignatureError;

pub const FALCON512_PUBLIC_KEY_BYTES: usize = 897;
pub const FALCON512_PRIVATE_KEY_BYTES: usize = 1_281;
pub const FALCON512_MAX_SIGNATURE_BYTES: usize = 1_024;
pub const FALCON1024_PUBLIC_KEY_BYTES: usize = 1_793;
pub const FALCON1024_PRIVATE_KEY_BYTES: usize = 2_305;
pub const FALCON1024_MAX_SIGNATURE_BYTES: usize = 2_048;

#[cfg(feature = "falcon")]
use falcon::prelude::{DomainSeparation, FnDsaKeyPair, FnDsaSignature};

pub fn generate_falcon512_keypair_bytes() -> Result<(Vec<u8>, Zeroizing<Vec<u8>>), SignatureError> {
    generate_keypair_bytes(9, "FN-DSA-512")
}

pub fn generate_falcon1024_keypair_bytes() -> Result<(Vec<u8>, Zeroizing<Vec<u8>>), SignatureError>
{
    generate_keypair_bytes(10, "FN-DSA-1024")
}

pub fn sign_falcon512(secret_key_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    sign(
        secret_key_bytes,
        message,
        FALCON512_PRIVATE_KEY_BYTES,
        "FN-DSA-512",
    )
}

pub fn sign_falcon1024(secret_key_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    sign(
        secret_key_bytes,
        message,
        FALCON1024_PRIVATE_KEY_BYTES,
        "FN-DSA-1024",
    )
}

pub fn verify_falcon512(
    public_key_bytes: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    verify(
        public_key_bytes,
        message,
        signature,
        FALCON512_PUBLIC_KEY_BYTES,
        FALCON512_MAX_SIGNATURE_BYTES,
        "FN-DSA-512",
    )
}

pub fn verify_falcon1024(
    public_key_bytes: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    verify(
        public_key_bytes,
        message,
        signature,
        FALCON1024_PUBLIC_KEY_BYTES,
        FALCON1024_MAX_SIGNATURE_BYTES,
        "FN-DSA-1024",
    )
}

pub fn validate_falcon512_secret_public(
    secret_key_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<(), SignatureError> {
    validate_secret_public(
        secret_key_bytes,
        public_key_bytes,
        FALCON512_PRIVATE_KEY_BYTES,
        "FN-DSA-512",
    )
}

pub fn validate_falcon1024_secret_public(
    secret_key_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<(), SignatureError> {
    validate_secret_public(
        secret_key_bytes,
        public_key_bytes,
        FALCON1024_PRIVATE_KEY_BYTES,
        "FN-DSA-1024",
    )
}

#[cfg(feature = "falcon")]
fn generate_keypair_bytes(
    logn: u32,
    label: &str,
) -> Result<(Vec<u8>, Zeroizing<Vec<u8>>), SignatureError> {
    let keypair = FnDsaKeyPair::generate(logn)
        .map_err(|e| SignatureError::InvalidPrivateKey(format!("{label} keygen failed: {e}")))?;
    Ok((
        keypair.public_key().to_vec(),
        Zeroizing::new(keypair.private_key().to_vec()),
    ))
}

#[cfg(not(feature = "falcon"))]
fn generate_keypair_bytes(
    _logn: u32,
    label: &str,
) -> Result<(Vec<u8>, Zeroizing<Vec<u8>>), SignatureError> {
    Err(SignatureError::InvalidPrivateKey(format!(
        "{label} requires falcon or pqc feature"
    )))
}

#[cfg(feature = "falcon")]
fn sign(
    secret_key_bytes: &[u8],
    message: &[u8],
    expected_secret_len: usize,
    label: &str,
) -> Result<Vec<u8>, SignatureError> {
    if secret_key_bytes.len() != expected_secret_len {
        return Err(SignatureError::InvalidPrivateKey(format!(
            "Invalid {label} private key length"
        )));
    }
    let keypair = FnDsaKeyPair::from_private_key(secret_key_bytes)
        .map_err(|e| SignatureError::InvalidPrivateKey(format!("Invalid {label} key: {e}")))?;
    let signature = keypair
        .sign(message, &DomainSeparation::None)
        .map_err(|e| SignatureError::InvalidFormat(format!("{label} signing failed: {e}")))?;
    Ok(signature.to_bytes().to_vec())
}

#[cfg(not(feature = "falcon"))]
fn sign(
    _secret_key_bytes: &[u8],
    _message: &[u8],
    _expected_secret_len: usize,
    label: &str,
) -> Result<Vec<u8>, SignatureError> {
    Err(SignatureError::InvalidPrivateKey(format!(
        "{label} requires falcon or pqc feature"
    )))
}

#[cfg(feature = "falcon")]
fn verify(
    public_key_bytes: &[u8],
    message: &[u8],
    signature: &[u8],
    expected_public_len: usize,
    max_signature_len: usize,
    label: &str,
) -> Result<bool, SignatureError> {
    if public_key_bytes.len() != expected_public_len {
        return Err(SignatureError::InvalidPublicKey(format!(
            "Invalid {label} public key length"
        )));
    }
    if signature.is_empty() || signature.len() > max_signature_len {
        return Err(SignatureError::InvalidFormat(format!(
            "Invalid {label} signature length"
        )));
    }
    Ok(FnDsaSignature::verify(
        signature,
        public_key_bytes,
        message,
        &DomainSeparation::None,
    )
    .is_ok())
}

#[cfg(not(feature = "falcon"))]
fn verify(
    _public_key_bytes: &[u8],
    _message: &[u8],
    _signature: &[u8],
    _expected_public_len: usize,
    _expected_signature_len: usize,
    label: &str,
) -> Result<bool, SignatureError> {
    Err(SignatureError::InvalidFormat(format!(
        "{label} requires falcon or pqc feature"
    )))
}

#[cfg(feature = "falcon")]
fn validate_secret_public(
    secret_key_bytes: &[u8],
    public_key_bytes: &[u8],
    expected_secret_len: usize,
    label: &str,
) -> Result<(), SignatureError> {
    if secret_key_bytes.len() != expected_secret_len {
        return Err(SignatureError::InvalidPrivateKey(format!(
            "Invalid {label} private key length"
        )));
    }
    let keypair = FnDsaKeyPair::from_private_key(secret_key_bytes)
        .map_err(|e| SignatureError::InvalidPrivateKey(format!("Invalid {label} key: {e}")))?;
    if keypair.public_key() != public_key_bytes {
        return Err(SignatureError::InvalidPublicKey(format!(
            "{label} public key does not match private key"
        )));
    }
    Ok(())
}

#[cfg(not(feature = "falcon"))]
fn validate_secret_public(
    _secret_key_bytes: &[u8],
    _public_key_bytes: &[u8],
    _expected_secret_len: usize,
    label: &str,
) -> Result<(), SignatureError> {
    Err(SignatureError::InvalidPrivateKey(format!(
        "{label} requires falcon or pqc feature"
    )))
}
