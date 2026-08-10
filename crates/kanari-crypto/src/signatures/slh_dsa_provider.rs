// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use slh_dsa::{
    Sha2_256f, Signature, SigningKey, VerifyingKey,
    signature::{Keypair, Signer, Verifier},
};
use zeroize::Zeroizing;

use crate::SignatureError;

pub const SLH_DSA_SHA2_256F_PUBLIC_KEY_BYTES: usize = 64;
pub const SLH_DSA_SHA2_256F_PRIVATE_KEY_BYTES: usize = 128;
pub const SLH_DSA_SHA2_256F_SIGNATURE_BYTES: usize = 49_856;

pub fn generate_slh_dsa_sha2_256f_keypair_bytes() -> (Vec<u8>, Zeroizing<Vec<u8>>) {
    let mut rng = rand::rng();
    let signing_key = SigningKey::<Sha2_256f>::new(&mut rng);
    let public_key = signing_key.verifying_key().to_bytes().as_slice().to_vec();
    let private_key = Zeroizing::new(signing_key.to_bytes().as_slice().to_vec());
    (public_key, private_key)
}

pub fn sign_slh_dsa_sha2_256f(
    private_key_bytes: &[u8],
    message: &[u8],
) -> Result<Vec<u8>, SignatureError> {
    if private_key_bytes.len() != SLH_DSA_SHA2_256F_PRIVATE_KEY_BYTES {
        return Err(SignatureError::InvalidPrivateKey(
            "Invalid SLH-DSA-SHA2-256f private key length".to_string(),
        ));
    }

    let signing_key = SigningKey::<Sha2_256f>::try_from(private_key_bytes).map_err(|_| {
        SignatureError::InvalidPrivateKey("Invalid SLH-DSA-SHA2-256f private key".to_string())
    })?;
    let signature: Signature<Sha2_256f> = signing_key.sign(message);
    Ok(signature.to_bytes().as_slice().to_vec())
}

pub fn verify_slh_dsa_sha2_256f(
    public_key_bytes: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    if public_key_bytes.len() != SLH_DSA_SHA2_256F_PUBLIC_KEY_BYTES {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid SLH-DSA-SHA2-256f public key length".to_string(),
        ));
    }
    if signature.len() != SLH_DSA_SHA2_256F_SIGNATURE_BYTES {
        return Err(SignatureError::InvalidFormat(
            "Invalid SLH-DSA-SHA2-256f signature length".to_string(),
        ));
    }

    let verifying_key = VerifyingKey::<Sha2_256f>::try_from(public_key_bytes).map_err(|_| {
        SignatureError::InvalidPublicKey("Invalid SLH-DSA-SHA2-256f public key".to_string())
    })?;
    let signature = Signature::<Sha2_256f>::try_from(signature)
        .map_err(|_| SignatureError::InvalidFormat("Invalid SLH-DSA signature".to_string()))?;
    Ok(verifying_key.verify(message, &signature).is_ok())
}

pub fn validate_slh_dsa_sha2_256f_secret_public(
    private_key_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<(), SignatureError> {
    if private_key_bytes.len() != SLH_DSA_SHA2_256F_PRIVATE_KEY_BYTES {
        return Err(SignatureError::InvalidPrivateKey(
            "Invalid SLH-DSA-SHA2-256f private key length".to_string(),
        ));
    }
    if public_key_bytes.len() != SLH_DSA_SHA2_256F_PUBLIC_KEY_BYTES {
        return Err(SignatureError::InvalidPublicKey(
            "Invalid SLH-DSA-SHA2-256f public key length".to_string(),
        ));
    }

    let signing_key = SigningKey::<Sha2_256f>::try_from(private_key_bytes).map_err(|_| {
        SignatureError::InvalidPrivateKey("Invalid SLH-DSA-SHA2-256f private key".to_string())
    })?;
    let derived_public = signing_key.verifying_key().to_bytes();
    if derived_public.as_slice() != public_key_bytes {
        return Err(SignatureError::InvalidPublicKey(
            "SLH-DSA-SHA2-256f public key does not match private key".to_string(),
        ));
    }

    Ok(())
}
