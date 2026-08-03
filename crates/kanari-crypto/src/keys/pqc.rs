use sha3::{Digest, Sha3_256};
use zeroize::Zeroizing;

use crate::signatures::ml_dsa_provider::{
    generate_mldsa44_keypair_bytes, generate_mldsa65_keypair_bytes, generate_mldsa87_keypair_bytes,
};
#[cfg(feature = "experimental-slh-dsa")]
use crate::signatures::slh_dsa_provider::generate_slh_dsa_sha2_256f_keypair_bytes;

use super::{
    CurveType, KANAMLDSA_PREFIX, KANAPQC_PREFIX, KANASLHDSA_PREFIX, KeyError, KeyPair,
    secure_hex_encode,
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
    #[cfg(feature = "experimental-slh-dsa")]
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

    #[cfg(not(feature = "experimental-slh-dsa"))]
    {
        Err(KeyError::GenerationFailed(
            "SphincsPlusSha256Robust requires experimental-slh-dsa feature".to_string(),
        ))
    }
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
        .strip_prefix(KANAPQC_PREFIX)
        .unwrap_or(raw_for_pqc);
    let Some((_secret_hex, public_key_hex)) = raw_for_pqc.split_once(':') else {
        return Err(KeyError::InvalidPrivateKey);
    };

    let public_key_bytes = hex::decode(public_key_hex).map_err(|_| KeyError::InvalidPrivateKey)?;
    let address = match curve_type {
        CurveType::Dilithium2 | CurveType::Dilithium3 | CurveType::Dilithium5 => {
            address_from_pqc_public_key_bytes(&public_key_bytes)
        }
        CurveType::SphincsPlusSha256Robust => address_from_pqc_public_key_hex(public_key_hex),
        _ => return Err(KeyError::InvalidPrivateKey),
    };
    let formatted_private_key = if private_key.starts_with(KANAMLDSA_PREFIX)
        || private_key.starts_with(KANASLHDSA_PREFIX)
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
