// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Shared verification helpers for Move native crypto functions.
//!
//! These helpers preserve Move native semantics. That matters because the Move
//! ECDSA natives expose SHA-256/Keccak verification for external compatibility,
//! while Kanari account K256/P256 signing uses SHA3-256 domain separation.

use ed25519_dalek::{Signature as Ed25519Signature, Verifier, VerifyingKey as Ed25519VerifyingKey};
use k256::{
    PublicKey as K256PublicKey,
    ecdsa::{
        Signature as K256Signature, VerifyingKey as K256VerifyingKey,
        signature::hazmat::PrehashVerifier as K256PrehashVerifier,
    },
    elliptic_curve::sec1::ToSec1Point,
};
use p256::ecdsa::{Signature as P256Signature, VerifyingKey as P256VerifyingKey};
use rsa::{
    BigUint, RsaPublicKey,
    pkcs1v15::{Signature as RsaPkcs1v15Signature, VerifyingKey as RsaPkcs1v15VerifyingKey},
};
use secp256k1::{
    Message as SecpMessage, Secp256k1, XOnlyPublicKey,
    ecdsa::RecoverableSignature as SecpRecoverableSignature, ecdsa::RecoveryId as SecpRecoveryId,
    schnorr::Signature as SchnorrSignature,
};
use sha2::{Digest as Sha2Digest, Sha256};
use sha3::Keccak256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeEcdsaHash {
    Keccak256,
    Sha256,
}

impl NativeEcdsaHash {
    #[must_use]
    pub const fn from_move_selector(hash_type: u8) -> Self {
        if hash_type == 0 {
            Self::Keccak256
        } else {
            Self::Sha256
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCryptoError {
    InvalidRecovery,
    InvalidSignature,
    InvalidPublicKey,
    InvalidXOnlyPublicKey,
    InvalidMessage,
    InvalidSchnorrSignature,
}

#[must_use]
pub fn native_ecdsa_message_hash(msg: &[u8], hash: NativeEcdsaHash) -> [u8; 32] {
    match hash {
        NativeEcdsaHash::Keccak256 => Keccak256::digest(msg).into(),
        NativeEcdsaHash::Sha256 => Sha256::digest(msg).into(),
    }
}

#[must_use]
pub fn verify_ed25519_native(public_key: &[u8], signature: &[u8], msg: &[u8]) -> bool {
    let Ok(public_key) = <&[u8; 32]>::try_from(public_key) else {
        return false;
    };
    let Ok(signature) = <&[u8; 64]>::try_from(signature) else {
        return false;
    };

    let Ok(verifying_key) = Ed25519VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let signature = Ed25519Signature::from_bytes(signature);
    verifying_key.verify(msg, &signature).is_ok()
}

pub fn recover_secp256k1_public_key(
    signature: &[u8],
    msg_hash: &[u8],
) -> Result<Vec<u8>, NativeCryptoError> {
    if signature.len() != 65 {
        return Err(NativeCryptoError::InvalidSignature);
    }
    if msg_hash.len() != 32 {
        return Err(NativeCryptoError::InvalidMessage);
    }

    let mut sig64 = [0u8; 64];
    sig64.copy_from_slice(&signature[..64]);

    let v = signature[64];
    let rec_id = if v <= 3 {
        SecpRecoveryId::try_from(v as i32)
    } else if v == 27 || v == 28 {
        SecpRecoveryId::try_from((v - 27) as i32)
    } else {
        return Err(NativeCryptoError::InvalidRecovery);
    }
    .map_err(|_| NativeCryptoError::InvalidRecovery)?;

    let recoverable_sig = SecpRecoverableSignature::from_compact(&sig64, rec_id)
        .map_err(|_| NativeCryptoError::InvalidRecovery)?;
    let msg32: [u8; 32] = msg_hash
        .try_into()
        .map_err(|_| NativeCryptoError::InvalidMessage)?;
    let message = SecpMessage::from_digest(msg32);

    let secp = Secp256k1::new();
    let public_key = secp
        .recover_ecdsa(message, &recoverable_sig)
        .map_err(|_| NativeCryptoError::InvalidRecovery)?;
    Ok(public_key.serialize().to_vec())
}

pub fn decompress_secp256k1_pubkey(public_key: &[u8]) -> Result<Vec<u8>, NativeCryptoError> {
    let normalized = normalize_sec1_pubkey(public_key);
    let public_key = K256PublicKey::from_sec1_bytes(&normalized)
        .map_err(|_| NativeCryptoError::InvalidPublicKey)?;
    Ok(public_key.to_sec1_point(false).as_bytes().to_vec())
}

pub fn verify_secp256k1_ecdsa_native(
    public_key: &[u8],
    signature: &[u8],
    msg: &[u8],
    hash: NativeEcdsaHash,
) -> Result<bool, NativeCryptoError> {
    let public_key = normalize_sec1_pubkey(public_key);
    let verifying_key = K256VerifyingKey::from_sec1_bytes(&public_key)
        .map_err(|_| NativeCryptoError::InvalidPublicKey)?;
    let signature = parse_k256_signature(signature)?;
    let msg_hash = native_ecdsa_message_hash(msg, hash);
    Ok(verifying_key.verify_prehash(&msg_hash, &signature).is_ok())
}

pub fn verify_secp256k1_schnorr_native(
    public_key: &[u8],
    signature: &[u8],
    msg: &[u8],
) -> Result<bool, NativeCryptoError> {
    let msg32: [u8; 32] = msg
        .try_into()
        .map_err(|_| NativeCryptoError::InvalidMessage)?;
    let public_key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| NativeCryptoError::InvalidXOnlyPublicKey)?;
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| NativeCryptoError::InvalidSchnorrSignature)?;

    let public_key = XOnlyPublicKey::from_byte_array(public_key)
        .map_err(|_| NativeCryptoError::InvalidXOnlyPublicKey)?;
    let signature = SchnorrSignature::from_byte_array(signature);
    let secp = Secp256k1::new();
    Ok(secp.verify_schnorr(&signature, &msg32, &public_key).is_ok())
}

pub fn verify_p256_sha256_native(
    public_key: &[u8],
    signature: &[u8],
    msg: &[u8],
) -> Result<bool, NativeCryptoError> {
    let public_key = normalize_sec1_pubkey(public_key);
    let verifying_key = P256VerifyingKey::from_sec1_bytes(&public_key)
        .map_err(|_| NativeCryptoError::InvalidPublicKey)?;
    let signature = parse_p256_signature(signature)?;
    let msg_hash = Sha256::digest(msg);
    Ok(verifying_key
        .verify_prehash(msg_hash.as_slice(), &signature)
        .is_ok())
}

#[must_use]
/// Verify an RS256/PKCS#1 v1.5 SHA-256 signature for Move compatibility.
///
/// This helper performs public-key verification only. It must not be extended
/// to perform RSA private-key decryption or signing with the current `rsa`
/// provider while RustSec RUSTSEC-2023-0071 remains unresolved upstream.
pub fn verify_rs256_prehash_native(
    modulus_n: &[u8],
    exponent_e: &[u8],
    msg_hash: &[u8],
    signature: &[u8],
) -> bool {
    let Ok(public_key) = RsaPublicKey::new(
        BigUint::from_bytes_be(modulus_n),
        BigUint::from_bytes_be(exponent_e),
    ) else {
        return false;
    };

    let verifying_key = RsaPkcs1v15VerifyingKey::<rsa::sha2::Sha256>::new(public_key);
    let Ok(signature) = RsaPkcs1v15Signature::try_from(signature) else {
        return false;
    };

    use rsa::signature::hazmat::PrehashVerifier;
    verifying_key.verify_prehash(msg_hash, &signature).is_ok()
}

fn normalize_sec1_pubkey(public_key: &[u8]) -> Vec<u8> {
    if public_key.len() == 64 {
        let mut prefixed = Vec::with_capacity(65);
        prefixed.push(0x04);
        prefixed.extend_from_slice(public_key);
        prefixed
    } else {
        public_key.to_vec()
    }
}

fn parse_k256_signature(signature: &[u8]) -> Result<K256Signature, NativeCryptoError> {
    if let Ok(signature) = K256Signature::from_der(signature) {
        return Ok(signature);
    }

    let sig_bytes = if signature.len() == 65 {
        &signature[..64]
    } else {
        signature
    };
    K256Signature::try_from(sig_bytes).map_err(|_| NativeCryptoError::InvalidSignature)
}

fn parse_p256_signature(signature: &[u8]) -> Result<P256Signature, NativeCryptoError> {
    if let Ok(signature) = P256Signature::from_der(signature) {
        return Ok(signature);
    }

    let sig_bytes = if signature.len() == 65 {
        &signature[..64]
    } else {
        signature
    };
    P256Signature::try_from(sig_bytes).map_err(|_| NativeCryptoError::InvalidSignature)
}
