// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::{BatchVerificationItem, SignatureError};

/// Maximum allowed signature bytes to guard against resource exhaustion in parsing.
pub(crate) const MAX_SIGNATURE_SIZE: usize = 64 * 1024;

/// Maximum public-key/address text accepted by verification APIs.
///
/// This comfortably covers the largest supported PQC public keys encoded as hex
/// plus Kanari provider prefixes, while rejecting adversarial multi-megabyte
/// inputs before curve-specific parsers allocate.
pub(crate) const MAX_PUBLIC_KEY_OR_ADDRESS_SIZE: usize = 8 * 1024;

/// Maximum message size accepted by direct signature verification APIs.
///
/// Kanari transaction signatures are expected to sign compact transaction bytes.
/// Large payload signing should hash externally into a domain-separated digest.
pub(crate) const MAX_SIGNED_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Maximum number of items accepted by batch verification APIs.
pub(crate) const MAX_BATCH_SIZE: usize = 16_384;

pub(crate) fn validate_batch_items(
    items: &[BatchVerificationItem<'_>],
) -> Result<(), SignatureError> {
    if items.is_empty() {
        return Err(SignatureError::InvalidFormat(
            "Empty batch verification is not allowed".to_string(),
        ));
    }
    if items.len() > MAX_BATCH_SIZE {
        return Err(SignatureError::InvalidFormat(
            "Batch verification input too large".to_string(),
        ));
    }

    for item in items {
        validate_verification_text(item.public_key_or_address)?;
        validate_message_size(item.message)?;
        validate_signature_bytes(item.signature)?;
    }

    Ok(())
}

pub(crate) fn validate_signature_bytes(signature: &[u8]) -> Result<(), SignatureError> {
    if signature.is_empty() {
        return Err(SignatureError::InvalidFormat("Empty signature".to_string()));
    }
    if signature.len() > MAX_SIGNATURE_SIZE {
        return Err(SignatureError::InvalidFormat(
            "Signature too large".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_verification_text(
    public_key_or_address: &str,
) -> Result<(), SignatureError> {
    if public_key_or_address.is_empty() {
        return Err(SignatureError::InvalidPublicKey(
            "Empty public key or address".to_string(),
        ));
    }
    if public_key_or_address.len() > MAX_PUBLIC_KEY_OR_ADDRESS_SIZE {
        return Err(SignatureError::InvalidPublicKey(
            "Public key or address too large".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_message_size(message: &[u8]) -> Result<(), SignatureError> {
    if message.len() > MAX_SIGNED_MESSAGE_SIZE {
        return Err(SignatureError::InvalidFormat(
            "Signed message too large".to_string(),
        ));
    }
    Ok(())
}
