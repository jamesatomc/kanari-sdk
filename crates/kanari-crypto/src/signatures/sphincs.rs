// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "slh-dsa")]
use zeroize::Zeroizing;

use crate::SignatureError;

#[cfg(feature = "slh-dsa")]
use crate::signatures::slh_dsa_provider::{
    self, SLH_DSA_SHA2_256F_PUBLIC_KEY_BYTES, SLH_DSA_SHA2_256F_SIGNATURE_BYTES,
};

/// Verify a signature using SLH-DSA-SHA2-256f (FIPS 205; formerly SPHINCS+).
pub fn verify_signature_sphincs(
    address_hex: &str,
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    #[cfg(feature = "slh-dsa")]
    {
        let pqc_pub_raw = crate::keys::extract_raw_key(address_hex);
        let pub_bytes = hex::decode(pqc_pub_raw)
            .map_err(|_| SignatureError::InvalidPublicKey("Invalid public key hex".to_string()))?;
        if pub_bytes.len() != SLH_DSA_SHA2_256F_PUBLIC_KEY_BYTES {
            return Err(SignatureError::InvalidPublicKey(
                "Invalid SLH-DSA-SHA2-256f public key".to_string(),
            ));
        }
        if signature.len() != SLH_DSA_SHA2_256F_SIGNATURE_BYTES {
            return Err(SignatureError::InvalidFormat(
                "Invalid SLH-DSA-SHA2-256f signature length".to_string(),
            ));
        }
        slh_dsa_provider::verify_slh_dsa_sha2_256f(&pub_bytes, message, signature)
    }

    #[cfg(not(feature = "slh-dsa"))]
    {
        let _ = (address_hex, message, signature);
        Err(SignatureError::InvalidFormat(
            "SphincsPlusSha256Robust requires slh-dsa or pqc feature".to_string(),
        ))
    }
}

/// Sign a message using an SLH-DSA-SHA2-256f private key.
pub fn sign_message_sphincs(
    private_key_hex: &str,
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    #[cfg(feature = "slh-dsa")]
    {
        let raw = crate::keys::extract_raw_key(private_key_hex);
        let secret_hex = raw.split_once(':').map(|(secret, _)| secret).unwrap_or(raw);
        let sk_bytes: Zeroizing<Vec<u8>> =
            Zeroizing::new(hex::decode(secret_hex).map_err(|_| {
                SignatureError::InvalidPrivateKey("Invalid private key hex".to_string())
            })?);
        slh_dsa_provider::sign_slh_dsa_sha2_256f(&sk_bytes, message)
    }

    #[cfg(not(feature = "slh-dsa"))]
    {
        let _ = (private_key_hex, message);
        Err(SignatureError::InvalidPrivateKey(
            "SphincsPlusSha256Robust requires slh-dsa or pqc feature".to_string(),
        ))
    }
}
