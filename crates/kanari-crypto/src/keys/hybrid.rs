// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use sha3::{Digest, Sha3_256};
use zeroize::Zeroizing;

use super::{
    CurveType, KANAHYBRID_PREFIX, KeyError, KeyPair, classical, constant_time_starts_with,
    extract_raw_key, pqc,
};

pub fn generate_hybrid_ed25519_dilithium3_keypair() -> Result<KeyPair, KeyError> {
    let ed25519_pair = classical::generate_ed25519_keypair()?;
    let dilithium3_pair = pqc::generate_dilithium3_keypair()?;
    build_hybrid_keypair(
        ed25519_pair.public_key,
        extract_raw_key(&ed25519_pair.private_key),
        &dilithium3_pair,
        CurveType::Ed25519Dilithium3,
    )
}

pub fn generate_hybrid_k256_dilithium3_keypair() -> Result<KeyPair, KeyError> {
    let k256_pair = classical::generate_k256_keypair()?;
    let dilithium3_pair = pqc::generate_dilithium3_keypair()?;
    build_hybrid_keypair(
        k256_pair.public_key,
        extract_raw_key(&k256_pair.private_key),
        &dilithium3_pair,
        CurveType::K256Dilithium3,
    )
}

pub(super) fn keypair_from_hybrid_private_key(
    private_key: &str,
    raw_private_key: &str,
    curve_type: CurveType,
) -> Result<KeyPair, KeyError> {
    if !constant_time_starts_with(private_key, KANAHYBRID_PREFIX)
        && !constant_time_starts_with(raw_private_key, KANAHYBRID_PREFIX)
    {
        return Err(KeyError::InvalidPrivateKey);
    }

    let hybrid = raw_private_key
        .strip_prefix(KANAHYBRID_PREFIX)
        .unwrap_or(raw_private_key);
    let Some((classical_raw, pqc_raw)) = hybrid.split_once(':') else {
        return Err(KeyError::InvalidPrivateKey);
    };

    let classical_public_key = match curve_type {
        CurveType::Ed25519Dilithium3 => {
            classical::keypair_from_ed25519_private_key(classical_raw, classical_raw)?.public_key
        }
        CurveType::K256Dilithium3 => {
            classical::keypair_from_k256_private_key(classical_raw, classical_raw)?.public_key
        }
        _ => return Err(KeyError::InvalidPrivateKey),
    };

    let Some((pqc_secret, pqc_public_key)) = pqc_raw.split_once(':') else {
        return Err(KeyError::InvalidPrivateKey);
    };
    let pqc_secret_bytes =
        hex::decode(extract_raw_key(pqc_secret)).map_err(|_| KeyError::InvalidPrivateKey)?;
    let pqc_public_key_bytes =
        hex::decode(pqc_public_key).map_err(|_| KeyError::InvalidPrivateKey)?;
    pqc::validate_pqc_secret_and_public(
        &pqc_secret_bytes,
        &pqc_public_key_bytes,
        CurveType::Dilithium3,
    )?;

    let combined_public = format!("{}:{}", classical_public_key, pqc_public_key);
    let formatted_private_key = if private_key.starts_with(KANAHYBRID_PREFIX) {
        private_key.to_string()
    } else {
        format!("{}{}", KANAHYBRID_PREFIX, hybrid)
    };

    Ok(KeyPair {
        private_key: Zeroizing::new(formatted_private_key),
        public_key: combined_public.clone(),
        pqc_public_key: Some(pqc_public_key.to_string()),
        address: address_from_combined_public(&combined_public),
        curve_type,
    })
}

fn build_hybrid_keypair(
    classical_public_key: String,
    classical_private_key: &str,
    pqc_pair: &KeyPair,
    curve_type: CurveType,
) -> Result<KeyPair, KeyError> {
    let combined_public = format!("{}:{}", classical_public_key, pqc_pair.public_key);
    let combined_private = format!(
        "{}{}:{}",
        KANAHYBRID_PREFIX,
        classical_private_key,
        pqc_pair.private_key.as_str()
    );

    Ok(KeyPair {
        private_key: Zeroizing::new(combined_private),
        public_key: combined_public.clone(),
        pqc_public_key: Some(pqc_pair.public_key.clone()),
        address: address_from_combined_public(&combined_public),
        curve_type,
    })
}

fn address_from_combined_public(combined_public: &str) -> String {
    let mut hasher = Sha3_256::new();
    hasher.update(combined_public.as_bytes());
    format!("0x{}", hex::encode(hasher.finalize()))
}
