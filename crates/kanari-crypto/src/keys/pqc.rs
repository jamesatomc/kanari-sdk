// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use sha3::{Digest, Sha3_256};
use zeroize::Zeroizing;

use crate::signatures::falcon_provider::{
    generate_falcon512_keypair_bytes, generate_falcon1024_keypair_bytes,
    validate_falcon512_secret_public, validate_falcon1024_secret_public,
};
use crate::signatures::ml_dsa_provider::{
    ML_DSA_44_PUBLIC_KEY_BYTES, ML_DSA_65_PUBLIC_KEY_BYTES, ML_DSA_87_PUBLIC_KEY_BYTES,
    derive_mldsa44_public_key, derive_mldsa65_public_key, derive_mldsa87_public_key,
    generate_mldsa44_keypair_bytes, generate_mldsa65_keypair_bytes, generate_mldsa87_keypair_bytes,
};
#[cfg(feature = "slh-dsa")]
use crate::signatures::slh_dsa_provider::{
    generate_slh_dsa_sha2_256f_keypair_bytes, validate_slh_dsa_sha2_256f_secret_public,
};

use super::{
    CurveType, KANAFALCON_PREFIX, KANAMLDSA_PREFIX, KANAPQC_PREFIX, KANASLHDSA_PREFIX, KeyError,
    KeyPair, secure_hex_encode,
};

pub(super) fn generate_dilithium2_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = generate_mldsa44_keypair_bytes();
    pqc_keypair_from_parts(
        &public_key,
        &secret_key,
        CurveType::Dilithium2,
        true,
        KANAMLDSA_PREFIX,
    )
}

pub(super) fn generate_dilithium3_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = generate_mldsa65_keypair_bytes();
    pqc_keypair_from_parts(
        &public_key,
        &secret_key,
        CurveType::Dilithium3,
        true,
        KANAMLDSA_PREFIX,
    )
}

pub(super) fn generate_dilithium5_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = generate_mldsa87_keypair_bytes();
    pqc_keypair_from_parts(
        &public_key,
        &secret_key,
        CurveType::Dilithium5,
        true,
        KANAMLDSA_PREFIX,
    )
}

pub(super) fn generate_sphincs_keypair() -> Result<KeyPair, KeyError> {
    #[cfg(feature = "slh-dsa")]
    {
        let (public_key, secret_key) = generate_slh_dsa_sha2_256f_keypair_bytes();
        pqc_keypair_from_parts(
            &public_key,
            &secret_key,
            CurveType::SphincsPlusSha256Robust,
            false,
            super::KANASLHDSA_PREFIX,
        )
    }

    #[cfg(not(feature = "slh-dsa"))]
    {
        Err(KeyError::GenerationFailed(
            "SphincsPlusSha256Robust requires slh-dsa or pqc feature".to_string(),
        ))
    }
}

pub(super) fn generate_falcon512_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = generate_falcon512_keypair_bytes()
        .map_err(|e| KeyError::GenerationFailed(e.to_string()))?;
    pqc_keypair_from_parts(
        &public_key,
        &secret_key,
        CurveType::Falcon512,
        true,
        KANAFALCON_PREFIX,
    )
}

pub(super) fn generate_falcon1024_keypair() -> Result<KeyPair, KeyError> {
    let (public_key, secret_key) = generate_falcon1024_keypair_bytes()
        .map_err(|e| KeyError::GenerationFailed(e.to_string()))?;
    pqc_keypair_from_parts(
        &public_key,
        &secret_key,
        CurveType::Falcon1024,
        true,
        KANAFALCON_PREFIX,
    )
}

pub(super) fn keypair_from_pqc_private_key(
    private_key: &str,
    raw_private_key: &str,
    curve_type: CurveType,
) -> Result<KeyPair, KeyError> {
    let raw_for_pqc = raw_private_key
        .strip_prefix(KANAMLDSA_PREFIX)
        .unwrap_or(raw_private_key);
    let raw_for_pqc = raw_for_pqc
        .strip_prefix(KANASLHDSA_PREFIX)
        .unwrap_or(raw_for_pqc);
    let raw_for_pqc = raw_for_pqc
        .strip_prefix(KANAFALCON_PREFIX)
        .unwrap_or(raw_for_pqc);
    let raw_for_pqc = raw_for_pqc
        .strip_prefix(KANAPQC_PREFIX)
        .unwrap_or(raw_for_pqc);
    let Some((secret_hex, public_key_hex)) = raw_for_pqc.split_once(':') else {
        return Err(KeyError::InvalidPrivateKey);
    };

    let secret_bytes = hex::decode(secret_hex).map_err(|_| KeyError::InvalidPrivateKey)?;
    let public_key_bytes = hex::decode(public_key_hex).map_err(|_| KeyError::InvalidPrivateKey)?;
    validate_pqc_secret_and_public(&secret_bytes, &public_key_bytes, curve_type)?;
    let address = match curve_type {
        CurveType::Dilithium2 | CurveType::Dilithium3 | CurveType::Dilithium5 => {
            address_from_pqc_public_key_bytes(&public_key_bytes)
        }
        CurveType::SphincsPlusSha256Robust => address_from_pqc_public_key_hex(public_key_hex),
        CurveType::Falcon512 | CurveType::Falcon1024 => {
            address_from_pqc_public_key_bytes(&public_key_bytes)
        }
        _ => return Err(KeyError::InvalidPrivateKey),
    };
    let formatted_private_key = if private_key.starts_with(KANAMLDSA_PREFIX)
        || private_key.starts_with(KANASLHDSA_PREFIX)
        || private_key.starts_with(KANAFALCON_PREFIX)
        || private_key.starts_with(KANAPQC_PREFIX)
    {
        private_key.to_string()
    } else {
        format!("{}{}", KANAPQC_PREFIX, raw_for_pqc)
    };

    Ok(KeyPair {
        private_key: Zeroizing::new(formatted_private_key),
        public_key: public_key_hex.to_string(),
        pqc_public_key: Some(public_key_hex.to_string()),
        address,
        curve_type,
    })
}

pub(super) fn validate_pqc_secret_and_public(
    secret_bytes: &[u8],
    public_key_bytes: &[u8],
    curve_type: CurveType,
) -> Result<(), KeyError> {
    let expected_public_key_len = match curve_type {
        CurveType::Dilithium2 => ML_DSA_44_PUBLIC_KEY_BYTES,
        CurveType::Dilithium3 => ML_DSA_65_PUBLIC_KEY_BYTES,
        CurveType::Dilithium5 => ML_DSA_87_PUBLIC_KEY_BYTES,
        CurveType::SphincsPlusSha256Robust => {
            return validate_slh_dsa_secret_public(secret_bytes, public_key_bytes);
        }
        CurveType::Falcon512 => {
            return validate_falcon512_secret_public(secret_bytes, public_key_bytes)
                .map_err(|_| KeyError::InvalidPrivateKey);
        }
        CurveType::Falcon1024 => {
            return validate_falcon1024_secret_public(secret_bytes, public_key_bytes)
                .map_err(|_| KeyError::InvalidPrivateKey);
        }
        _ => return Err(KeyError::InvalidPrivateKey),
    };

    if public_key_bytes.len() != expected_public_key_len {
        return Err(KeyError::InvalidPrivateKey);
    }

    let derived_public_key = match curve_type {
        CurveType::Dilithium2 => derive_mldsa44_public_key(secret_bytes),
        CurveType::Dilithium3 => derive_mldsa65_public_key(secret_bytes),
        CurveType::Dilithium5 => derive_mldsa87_public_key(secret_bytes),
        _ => return Err(KeyError::InvalidPrivateKey),
    }
    .map_err(|_| KeyError::InvalidPrivateKey)?;

    if derived_public_key.as_slice() != public_key_bytes {
        return Err(KeyError::InvalidPrivateKey);
    }

    Ok(())
}

fn validate_slh_dsa_secret_public(
    secret_bytes: &[u8],
    public_key_bytes: &[u8],
) -> Result<(), KeyError> {
    #[cfg(feature = "slh-dsa")]
    {
        validate_slh_dsa_sha2_256f_secret_public(secret_bytes, public_key_bytes)
            .map_err(|_| KeyError::InvalidPrivateKey)
    }

    #[cfg(not(feature = "slh-dsa"))]
    {
        let _ = (secret_bytes, public_key_bytes);
        Err(KeyError::InvalidPrivateKey)
    }
}

fn pqc_keypair_from_parts(
    public_key: &[u8],
    secret_key: &[u8],
    curve_type: CurveType,
    hash_public_key_bytes: bool,
    private_key_prefix: &str,
) -> Result<KeyPair, KeyError> {
    let public_key_hex = hex::encode(public_key);
    let address = if hash_public_key_bytes {
        address_from_pqc_public_key_bytes(public_key)
    } else {
        address_from_pqc_public_key_hex(&public_key_hex)
    };
    let raw_private_key = secure_hex_encode(secret_key);
    let private_key = format!(
        "{}{}:{}",
        private_key_prefix, *raw_private_key, public_key_hex
    );

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: public_key_hex.clone(),
        pqc_public_key: Some(public_key_hex),
        address,
        curve_type,
    })
}

fn address_from_pqc_public_key_bytes(public_key: &[u8]) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(public_key);
    format!("0x{}", hex::encode(hasher.finalize()))
}

fn address_from_pqc_public_key_hex(public_key_hex: &str) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(public_key_hex.as_bytes());
    format!("0x{}", hex::encode(hasher.finalize()))
}
