// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use bip39::{Language, Mnemonic};
use ed25519_dalek::{SigningKey as Ed25519SigningKey, VerifyingKey as Ed25519VerifyingKey};
use k256::{
    PublicKey as K256PublicKey, SecretKey as K256SecretKey,
    ecdsa::{SigningKey as K256SigningKey, VerifyingKey as K256VerifyingKey},
    elliptic_curve::sec1::ToSec1Point,
};
use p256::{
    SecretKey as P256SecretKey,
    ecdsa::{SigningKey, VerifyingKey},
};
use rand::TryRng;
use rand::rngs::SysRng;
use sha3::{Digest, Sha3_256};
use zeroize::{Zeroize, Zeroizing};

use super::{
    CurveType, KANARI_KEY_PREFIX, KeyError, KeyPair, format_private_key, secure_hex_encode,
    skip_uncompressed_point_prefix,
};

pub(super) fn generate_k256_keypair() -> Result<KeyPair, KeyError> {
    let mut seed = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut seed)
        .map_err(|e| KeyError::GenerationFailed(format!("Failed to get OS randomness: {e}")))?;

    let secret_key = K256SecretKey::from_slice(&seed)
        .map_err(|_| KeyError::GenerationFailed("Invalid K256 seed".to_string()))?;

    seed.zeroize();

    let signing_key = K256SigningKey::from(&secret_key);
    let public_key = K256PublicKey::from(K256VerifyingKey::from(&signing_key));
    let encoded_point = public_key.to_sec1_point(false);
    let public_key_hex = hex::encode(skip_uncompressed_point_prefix(encoded_point.as_bytes()));
    let address = address_from_public_key_hex(&public_key_hex);
    let secret_bytes = signing_key.to_bytes();
    let raw_private_key = secure_hex_encode(&secret_bytes);

    let mut secret_bytes_mut = secret_bytes.to_vec();
    secret_bytes_mut.zeroize();

    Ok(KeyPair {
        private_key: Zeroizing::new(format!("{}{}", KANARI_KEY_PREFIX, *raw_private_key)),
        public_key: public_key_hex,
        pqc_public_key: None,
        address,
        curve_type: CurveType::K256,
    })
}

pub(super) fn generate_p256_keypair() -> Result<KeyPair, KeyError> {
    let mut seed = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut seed)
        .map_err(|e| KeyError::GenerationFailed(format!("Failed to get OS randomness: {e}")))?;

    let secret_key = P256SecretKey::from_slice(&seed)
        .map_err(|_| KeyError::GenerationFailed("Invalid P256 seed".to_string()))?;

    seed.zeroize();

    let signing_key = SigningKey::from(&secret_key);
    let public_key = VerifyingKey::from(&signing_key).to_sec1_point(false);
    let public_key_hex = hex::encode(skip_uncompressed_point_prefix(public_key.as_bytes()));
    let address = address_from_public_key_hex(&public_key_hex);
    let secret_bytes = secret_key.to_bytes();
    let raw_private_key = secure_hex_encode(&secret_bytes);

    let mut secret_bytes_mut = secret_bytes.to_vec();
    secret_bytes_mut.zeroize();

    Ok(KeyPair {
        private_key: Zeroizing::new(format!("{}{}", KANARI_KEY_PREFIX, *raw_private_key)),
        public_key: public_key_hex,
        pqc_public_key: None,
        address,
        curve_type: CurveType::P256,
    })
}

pub fn generate_ed25519_keypair() -> Result<KeyPair, KeyError> {
    let mut seed = [0u8; 32];
    SysRng
        .try_fill_bytes(&mut seed)
        .map_err(|e| KeyError::GenerationFailed(format!("Failed to get OS randomness: {e}")))?;

    if seed.iter().all(|&b| b == 0) {
        return Err(KeyError::GenerationFailed(
            "Insufficient entropy from RNG".to_string(),
        ));
    }

    let signing_key = Ed25519SigningKey::from_bytes(&seed);
    let verifying_key = Ed25519VerifyingKey::from(&signing_key);
    let private_key_bytes = signing_key.to_bytes();
    let raw_private_key = secure_hex_encode(&private_key_bytes);
    let public_key_hex = hex::encode(verifying_key.to_bytes());
    let address = address_from_public_key_hex(&public_key_hex);

    seed.zeroize();
    let mut private_key_bytes_mut = private_key_bytes.to_vec();
    private_key_bytes_mut.zeroize();

    Ok(KeyPair {
        private_key: Zeroizing::new(format!("{}{}", KANARI_KEY_PREFIX, *raw_private_key)),
        public_key: public_key_hex,
        pqc_public_key: None,
        address,
        curve_type: CurveType::Ed25519,
    })
}

pub(super) fn keypair_from_mnemonic(
    phrase: &str,
    curve_type: CurveType,
) -> Result<KeyPair, KeyError> {
    if phrase.trim().is_empty() {
        return Err(KeyError::InvalidMnemonic(
            "Empty mnemonic phrase".to_string(),
        ));
    }

    let mnemonic = Mnemonic::parse_in(Language::English, phrase)
        .map_err(|e| KeyError::InvalidMnemonic(e.to_string()))?;
    let seed = Zeroizing::new(mnemonic.to_seed(""));
    let bytes = &seed[0..32];

    match curve_type {
        CurveType::K256 => keypair_from_k256_raw(bytes, false),
        CurveType::P256 => keypair_from_p256_raw(bytes, false),
        CurveType::Ed25519 => keypair_from_ed25519_raw(bytes, false),
        _ => Err(KeyError::GenerationFailed(
            "Post-quantum algorithms don't support BIP39 mnemonic derivation yet. Use generate_keypair() instead.".to_string(),
        )),
    }
}

pub(super) fn keypair_from_k256_private_key(
    private_key: &str,
    raw_private_key: &str,
) -> Result<KeyPair, KeyError> {
    keypair_from_k256_raw_with_format(raw_private_key, Some(private_key))
}

pub(super) fn keypair_from_p256_private_key(
    private_key: &str,
    raw_private_key: &str,
) -> Result<KeyPair, KeyError> {
    keypair_from_p256_raw_with_format(raw_private_key, Some(private_key))
}

pub(super) fn keypair_from_ed25519_private_key(
    private_key: &str,
    raw_private_key: &str,
) -> Result<KeyPair, KeyError> {
    keypair_from_ed25519_raw_with_format(raw_private_key, Some(private_key))
}

fn keypair_from_k256_raw(
    raw_private_key: &[u8],
    canonical_only: bool,
) -> Result<KeyPair, KeyError> {
    let secret_key =
        K256SecretKey::from_slice(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;
    let signing_key = K256SigningKey::from(secret_key);
    let public_key = K256PublicKey::from(K256VerifyingKey::from(&signing_key));
    let encoded_point = public_key.to_sec1_point(false);
    let public_key_hex = hex::encode(skip_uncompressed_point_prefix(encoded_point.as_bytes()));
    let private_key_hex = secure_hex_encode(&signing_key.to_bytes());
    let private_key = format_private_key(&private_key_hex);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: public_key_hex.clone(),
        pqc_public_key: None,
        address: address_from_public_key_hex(&public_key_hex),
        curve_type: CurveType::K256,
    })
    .map(|mut keypair| {
        if canonical_only {
            keypair.private_key = Zeroizing::new(format_private_key(&private_key_hex));
        }
        keypair
    })
}

fn keypair_from_p256_raw(
    raw_private_key: &[u8],
    canonical_only: bool,
) -> Result<KeyPair, KeyError> {
    let secret_key =
        P256SecretKey::from_slice(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;
    let signing_key = SigningKey::from(secret_key);
    let public_key = VerifyingKey::from(&signing_key).to_sec1_point(false);
    let public_key_hex = hex::encode(skip_uncompressed_point_prefix(public_key.as_bytes()));
    let private_key_hex = secure_hex_encode(&signing_key.to_bytes());
    let private_key = format_private_key(&private_key_hex);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: public_key_hex.clone(),
        pqc_public_key: None,
        address: address_from_public_key_hex(&public_key_hex),
        curve_type: CurveType::P256,
    })
    .map(|mut keypair| {
        if canonical_only {
            keypair.private_key = Zeroizing::new(format_private_key(&private_key_hex));
        }
        keypair
    })
}

fn keypair_from_ed25519_raw(
    raw_private_key: &[u8],
    canonical_only: bool,
) -> Result<KeyPair, KeyError> {
    if raw_private_key.len() != 32 {
        return Err(KeyError::InvalidPrivateKey);
    }

    let mut key_array = [0u8; 32];
    key_array.copy_from_slice(raw_private_key);
    let signing_key = Ed25519SigningKey::from_bytes(&key_array);
    key_array.zeroize();

    let verifying_key = Ed25519VerifyingKey::from(&signing_key);
    let public_key_hex = hex::encode(verifying_key.to_bytes());
    let private_key_hex = secure_hex_encode(&signing_key.to_bytes());
    let private_key = format_private_key(&private_key_hex);

    Ok(KeyPair {
        private_key: Zeroizing::new(private_key),
        public_key: public_key_hex.clone(),
        pqc_public_key: None,
        address: address_from_public_key_hex(&public_key_hex),
        curve_type: CurveType::Ed25519,
    })
    .map(|mut keypair| {
        if canonical_only {
            keypair.private_key = Zeroizing::new(format_private_key(&private_key_hex));
        }
        keypair
    })
}

fn keypair_from_k256_raw_with_format(
    raw_private_key: &str,
    original_private_key: Option<&str>,
) -> Result<KeyPair, KeyError> {
    let mut private_key_bytes =
        hex::decode(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;
    let mut keypair = keypair_from_k256_raw(&private_key_bytes, false)?;
    private_key_bytes.zeroize();
    keypair.private_key = formatted_private_key(original_private_key, raw_private_key);
    Ok(keypair)
}

fn keypair_from_p256_raw_with_format(
    raw_private_key: &str,
    original_private_key: Option<&str>,
) -> Result<KeyPair, KeyError> {
    let mut private_key_bytes =
        hex::decode(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;
    let mut keypair = keypair_from_p256_raw(&private_key_bytes, false)?;
    private_key_bytes.zeroize();
    keypair.private_key = formatted_private_key(original_private_key, raw_private_key);
    Ok(keypair)
}

fn keypair_from_ed25519_raw_with_format(
    raw_private_key: &str,
    original_private_key: Option<&str>,
) -> Result<KeyPair, KeyError> {
    let mut private_key_bytes =
        hex::decode(raw_private_key).map_err(|_| KeyError::InvalidPrivateKey)?;
    let mut keypair = keypair_from_ed25519_raw(&private_key_bytes, false)?;
    private_key_bytes.zeroize();
    keypair.private_key = formatted_private_key(original_private_key, raw_private_key);
    Ok(keypair)
}

fn formatted_private_key(
    original_private_key: Option<&str>,
    raw_private_key: &str,
) -> Zeroizing<String> {
    Zeroizing::new(match original_private_key {
        Some(private_key) if private_key.starts_with(KANARI_KEY_PREFIX) => private_key.to_string(),
        _ => format_private_key(raw_private_key),
    })
}

fn address_from_public_key_hex(public_key_hex: &str) -> String {
    let mut hasher = Sha3_256::default();
    hasher.update(public_key_hex.as_bytes());
    format!("0x{}", hex::encode(hasher.finalize()))
}
