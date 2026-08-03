// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use zeroize::Zeroizing;

use crate::{SignatureError, signatures::ml_dsa_provider};

/// Verify a signature using Dilithium5-compatible ML-DSA-87 (FIPS 204).
pub fn verify_signature_dilithium5(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    let pqc_pub_raw = crate::keys::extract_raw_key(address_hex);
    let pub_bytes = hex::decode(pqc_pub_raw)
        .map_err(|_| SignatureError::InvalidPublicKey("Invalid public key hex".to_string()))?;
    if pub_bytes.len() != ml_dsa_provider::ML_DSA_87_PUBLIC_KEY_BYTES {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid ML-DSA-87 public key".to_string(),
        ));
    }
    if signature.len() != ml_dsa_provider::ML_DSA_87_SIGNATURE_BYTES {
        return Err(SignatureError::InvalidFormat(
            "Invalid ML-DSA-87 signature length".to_string(),
        ));
    }
    ml_dsa_provider::verify_mldsa87(&pub_bytes, message, signature)
}

/// Sign a message using an ML-DSA-87 seed private key.
pub fn sign_message_dilithium5(
    private_key_hex: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    let raw = crate::keys::extract_raw_key(private_key_hex);
    let secret_hex = raw.split_once(':').map(|(secret, _)| secret).unwrap_or(raw);
    let sk_bytes: Zeroizing<Vec<u8>> =
        Zeroizing::new(hex::decode(secret_hex).map_err(|_| {
            SignatureError::InvalidPrivateKey("Invalid private key hex".to_string())
        })?);
    ml_dsa_provider::sign_mldsa87(&sk_bytes, message)
}
