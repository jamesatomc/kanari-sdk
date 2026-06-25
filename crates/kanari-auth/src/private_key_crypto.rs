use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{AuthError, AuthResult};

const PAYLOAD_VERSION: u8 = 2;
const KDF_MEMORY_KIB: u32 = 64 * 1024;
const KDF_TIME_COST: u32 = 3;
const KDF_PARALLELISM: u32 = 1;
const LEGACY_MIN_ITERATIONS: u32 = 100_000;
const LEGACY_MAX_ITERATIONS: u32 = 1_000_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const MAX_ENCODED_FIELD_LEN: usize = 128 * 1024;
const MAX_CIPHERTEXT_LEN: usize = 64 * 1024;
const AAD: &[u8] = b"kanari-auth:encrypted-secret:v2";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedPrivateKeyPayload {
    #[serde(default = "legacy_version")]
    pub version: u8,
    pub ciphertext: String,
    pub nonce: String,
    pub salt: String,
    #[serde(default)]
    pub iterations: u32,
    #[serde(default)]
    pub memory_kib: u32,
    #[serde(default)]
    pub time_cost: u32,
    #[serde(default)]
    pub parallelism: u32,
}

fn legacy_version() -> u8 {
    1
}

pub fn payload_needs_upgrade(encoded: &str) -> bool {
    serde_json::from_str::<EncryptedPrivateKeyPayload>(encoded)
        .map(|payload| payload.version != PAYLOAD_VERSION)
        .unwrap_or(true)
}

pub fn encrypt_private_key(private_key: &str, password: &str) -> AuthResult<String> {
    validate_password(password)?;
    let salt = random_bytes(SALT_LEN);
    let key = derive_argon2id_key(password.as_bytes(), &salt)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|e| AuthError::CryptoError(format!("Failed to create cipher: {e}")))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: private_key.as_bytes(),
                aad: AAD,
            },
        )
        .map_err(|e| AuthError::CryptoError(format!("Private key encryption failed: {e}")))?;
    let payload = EncryptedPrivateKeyPayload {
        version: PAYLOAD_VERSION,
        ciphertext: general_purpose::STANDARD.encode(ciphertext),
        nonce: general_purpose::STANDARD.encode(nonce),
        salt: general_purpose::STANDARD.encode(salt),
        iterations: 0,
        memory_kib: KDF_MEMORY_KIB,
        time_cost: KDF_TIME_COST,
        parallelism: KDF_PARALLELISM,
    };
    serde_json::to_string(&payload).map_err(|e| {
        AuthError::SerializationError(format!("Encryption payload serialize failed: {e}"))
    })
}

pub fn decrypt_private_key(encrypted_payload: &str, password: &str) -> AuthResult<String> {
    validate_password(password)?;
    if encrypted_payload.len() > MAX_ENCODED_FIELD_LEN * 3 {
        return Err(AuthError::CryptoError(
            "Encrypted private key payload is too large".to_string(),
        ));
    }
    let payload: EncryptedPrivateKeyPayload =
        serde_json::from_str(encrypted_payload).map_err(|e| {
            AuthError::SerializationError(format!("Invalid encrypted key payload: {e}"))
        })?;
    validate_encoded_field("ciphertext", &payload.ciphertext)?;
    validate_encoded_field("nonce", &payload.nonce)?;
    validate_encoded_field("salt", &payload.salt)?;
    let ciphertext = general_purpose::STANDARD
        .decode(payload.ciphertext)
        .map_err(|e| AuthError::CryptoError(format!("Invalid ciphertext encoding: {e}")))?;
    let nonce = general_purpose::STANDARD
        .decode(payload.nonce)
        .map_err(|e| AuthError::CryptoError(format!("Invalid nonce encoding: {e}")))?;
    let salt = general_purpose::STANDARD
        .decode(payload.salt)
        .map_err(|e| AuthError::CryptoError(format!("Invalid salt encoding: {e}")))?;
    if ciphertext.len() > MAX_CIPHERTEXT_LEN || nonce.len() != NONCE_LEN || salt.len() != SALT_LEN {
        return Err(AuthError::CryptoError(
            "Invalid encrypted key payload dimensions".to_string(),
        ));
    }

    let (key, aad): (Zeroizing<[u8; 32]>, &[u8]) = if payload.version == PAYLOAD_VERSION {
        if payload.memory_kib != KDF_MEMORY_KIB
            || payload.time_cost != KDF_TIME_COST
            || payload.parallelism != KDF_PARALLELISM
        {
            return Err(AuthError::CryptoError(
                "Unsupported Argon2id parameters".to_string(),
            ));
        }
        (derive_argon2id_key(password.as_bytes(), &salt)?, AAD)
    } else {
        validate_legacy_iterations(payload.iterations)?;
        (
            derive_legacy_key(password.as_bytes(), &salt, payload.iterations)?,
            b"",
        )
    };

    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|e| AuthError::CryptoError(format!("Failed to create cipher: {e}")))?;
    let decrypted = cipher
        .decrypt(
            aes_gcm::Nonce::from_slice(&nonce),
            Payload {
                msg: ciphertext.as_ref(),
                aad,
            },
        )
        .map_err(|_| AuthError::AuthenticationFailed)?;
    String::from_utf8(decrypted)
        .map_err(|e| AuthError::SerializationError(format!("Invalid UTF-8 private key: {e}")))
}

fn derive_argon2id_key(password: &[u8], salt: &[u8]) -> AuthResult<Zeroizing<[u8; 32]>> {
    let params = Params::new(KDF_MEMORY_KIB, KDF_TIME_COST, KDF_PARALLELISM, Some(32))
        .map_err(|e| AuthError::CryptoError(format!("Invalid Argon2id parameters: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|e| AuthError::CryptoError(format!("Argon2id derivation failed: {e}")))?;
    Ok(key)
}

fn validate_legacy_iterations(iterations: u32) -> AuthResult<()> {
    if !(LEGACY_MIN_ITERATIONS..=LEGACY_MAX_ITERATIONS).contains(&iterations) {
        return Err(AuthError::CryptoError(format!(
            "KDF iteration count {iterations} is outside the accepted range"
        )));
    }
    Ok(())
}

fn derive_legacy_key(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
) -> AuthResult<Zeroizing<[u8; 32]>> {
    validate_legacy_iterations(iterations)?;
    let mut hasher = Sha256::new();
    hasher.update(password);
    hasher.update(salt);
    let mut block = Zeroizing::new([0u8; 32]);
    block.copy_from_slice(&hasher.finalize());
    for _ in 1..iterations {
        let mut round = Sha256::new();
        round.update(*block);
        round.update(password);
        round.update(salt);
        block.copy_from_slice(&round.finalize());
    }
    Ok(block)
}

fn validate_encoded_field(name: &str, value: &str) -> AuthResult<()> {
    if value.len() > MAX_ENCODED_FIELD_LEN {
        return Err(AuthError::CryptoError(format!(
            "Encrypted private key {name} field is too large"
        )));
    }
    Ok(())
}

fn random_bytes(len: usize) -> Vec<u8> {
    use aes_gcm::aead::rand_core::RngCore;
    let mut bytes = vec![0u8; len];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn validate_password(password: &str) -> AuthResult<()> {
    if password.is_empty() {
        return Err(AuthError::InvalidPassword(
            "Password cannot be empty".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_key_round_trip_uses_argon2id_v2() {
        let encrypted = encrypt_private_key("secret", "password").unwrap();
        assert!(!payload_needs_upgrade(&encrypted));
        assert_eq!(
            decrypt_private_key(&encrypted, "password").unwrap(),
            "secret"
        );
    }
}
