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

const PAYLOAD_VERSION_ARGON2ID: u8 = 2;
const ARGON2_MEMORY_KIB: u32 = 64 * 1024;
const ARGON2_TIME_COST: u32 = 3;
const ARGON2_PARALLELISM: u32 = 1;
const MIN_ARGON2_MEMORY_KIB: u32 = 32 * 1024;
const MAX_ARGON2_MEMORY_KIB: u32 = 256 * 1024;
const MIN_ARGON2_TIME_COST: u32 = 2;
const MAX_ARGON2_TIME_COST: u32 = 8;
const MIN_ARGON2_PARALLELISM: u32 = 1;
const MAX_ARGON2_PARALLELISM: u32 = 4;

const LEGACY_MIN_KDF_ITERATIONS: u32 = 100_000;
const LEGACY_MAX_KDF_ITERATIONS: u32 = 1_000_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const MAX_ENCODED_FIELD_LEN: usize = 128 * 1024;
const MAX_CIPHERTEXT_LEN: usize = 64 * 1024;

fn legacy_payload_version() -> u8 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedPrivateKeyPayload {
    #[serde(default = "legacy_payload_version")]
    pub version: u8,
    #[serde(default)]
    pub kdf: String,
    pub ciphertext: String,
    pub nonce: String,
    pub salt: String,
    /// Legacy SHA-256-loop parameter. New payloads leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_kib: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_cost: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct Argon2Policy {
    memory_kib: u32,
    time_cost: u32,
    parallelism: u32,
}

impl Argon2Policy {
    fn production() -> Self {
        Self {
            memory_kib: ARGON2_MEMORY_KIB,
            time_cost: ARGON2_TIME_COST,
            parallelism: ARGON2_PARALLELISM,
        }
    }

    fn from_payload(payload: &EncryptedPrivateKeyPayload) -> AuthResult<Self> {
        let policy = Self {
            memory_kib: payload.memory_kib.ok_or_else(|| {
                AuthError::CryptoError("Missing Argon2 memory parameter".to_string())
            })?,
            time_cost: payload.time_cost.ok_or_else(|| {
                AuthError::CryptoError("Missing Argon2 time parameter".to_string())
            })?,
            parallelism: payload.parallelism.ok_or_else(|| {
                AuthError::CryptoError("Missing Argon2 parallelism parameter".to_string())
            })?,
        };
        policy.validate()?;
        Ok(policy)
    }

    fn validate(self) -> AuthResult<()> {
        if !(MIN_ARGON2_MEMORY_KIB..=MAX_ARGON2_MEMORY_KIB).contains(&self.memory_kib)
            || !(MIN_ARGON2_TIME_COST..=MAX_ARGON2_TIME_COST).contains(&self.time_cost)
            || !(MIN_ARGON2_PARALLELISM..=MAX_ARGON2_PARALLELISM)
                .contains(&self.parallelism)
        {
            return Err(AuthError::CryptoError(
                "Argon2 parameters are outside the accepted policy".to_string(),
            ));
        }
        Ok(())
    }
}

pub fn encrypt_private_key(private_key: &str, password: &str) -> AuthResult<String> {
    validate_password(password)?;

    let salt = random_bytes(SALT_LEN)?;
    let policy = Argon2Policy::production();
    let key = derive_argon2id_key(password.as_bytes(), &salt, policy)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|e| AuthError::CryptoError(format!("Failed to create cipher: {e}")))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let aad = argon2_aad(policy);

    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: private_key.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|e| AuthError::CryptoError(format!("Private key encryption failed: {e}")))?;

    let payload = EncryptedPrivateKeyPayload {
        version: PAYLOAD_VERSION_ARGON2ID,
        kdf: "argon2id".to_string(),
        ciphertext: general_purpose::STANDARD.encode(ciphertext),
        nonce: general_purpose::STANDARD.encode(nonce),
        salt: general_purpose::STANDARD.encode(salt),
        iterations: None,
        memory_kib: Some(policy.memory_kib),
        time_cost: Some(policy.time_cost),
        parallelism: Some(policy.parallelism),
    };

    serde_json::to_string(&payload).map_err(|e| {
        AuthError::SerializationError(format!("Encryption payload serialize failed: {e}"))
    })
}

pub fn decrypt_private_key(encrypted_payload: &str, password: &str) -> AuthResult<String> {
    decrypt_private_key_with_migration(encrypted_payload, password).map(|(key, _)| key)
}

/// Decrypt a key and return a replacement Argon2id payload when a bounded legacy
/// payload was encountered. Callers should persist the replacement atomically after
/// successful authentication.
pub fn decrypt_private_key_with_migration(
    encrypted_payload: &str,
    password: &str,
) -> AuthResult<(String, Option<String>)> {
    validate_password(password)?;
    let payload = parse_and_validate_payload(encrypted_payload)?;
    let ciphertext = decode_field("ciphertext", &payload.ciphertext)?;
    let nonce = decode_field("nonce", &payload.nonce)?;
    let salt = decode_field("salt", &payload.salt)?;

    if ciphertext.len() > MAX_CIPHERTEXT_LEN {
        return Err(AuthError::CryptoError(
            "Encrypted private key ciphertext is too large".to_string(),
        ));
    }
    if nonce.len() != NONCE_LEN {
        return Err(AuthError::CryptoError("Invalid nonce length".to_string()));
    }
    if salt.len() != SALT_LEN {
        return Err(AuthError::CryptoError("Invalid salt length".to_string()));
    }

    let (key, aad, legacy) = match payload.version {
        PAYLOAD_VERSION_ARGON2ID => {
            if payload.kdf != "argon2id" {
                return Err(AuthError::CryptoError(
                    "Unsupported encrypted-key KDF".to_string(),
                ));
            }
            let policy = Argon2Policy::from_payload(&payload)?;
            (
                derive_argon2id_key(password.as_bytes(), &salt, policy)?,
                argon2_aad(policy),
                false,
            )
        }
        1 => {
            let iterations = payload.iterations.ok_or_else(|| {
                AuthError::CryptoError("Missing legacy KDF iteration count".to_string())
            })?;
            validate_legacy_iterations(iterations)?;
            (
                derive_legacy_key(password.as_bytes(), &salt, iterations)?,
                Vec::new(),
                true,
            )
        }
        version => {
            return Err(AuthError::CryptoError(format!(
                "Unsupported encrypted-key payload version {version}"
            )));
        }
    };

    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|e| AuthError::CryptoError(format!("Failed to create cipher: {e}")))?;
    let decrypted = cipher
        .decrypt(
            aes_gcm::Nonce::from_slice(&nonce),
            Payload {
                msg: ciphertext.as_ref(),
                aad: &aad,
            },
        )
        .map_err(|_| AuthError::AuthenticationFailed)?;
    let plaintext = String::from_utf8(decrypted)
        .map_err(|e| AuthError::SerializationError(format!("Invalid UTF-8 private key: {e}")))?;

    let migrated = if legacy {
        Some(encrypt_private_key(&plaintext, password)?)
    } else {
        None
    };
    Ok((plaintext, migrated))
}

fn parse_and_validate_payload(encrypted_payload: &str) -> AuthResult<EncryptedPrivateKeyPayload> {
    if encrypted_payload.len() > MAX_ENCODED_FIELD_LEN * 3 {
        return Err(AuthError::CryptoError(
            "Encrypted private key payload is too large".to_string(),
        ));
    }
    let payload: EncryptedPrivateKeyPayload = serde_json::from_str(encrypted_payload).map_err(|e| {
        AuthError::SerializationError(format!("Invalid encrypted key payload: {e}"))
    })?;
    validate_encoded_field("ciphertext", &payload.ciphertext)?;
    validate_encoded_field("nonce", &payload.nonce)?;
    validate_encoded_field("salt", &payload.salt)?;
    Ok(payload)
}

fn decode_field(name: &str, value: &str) -> AuthResult<Vec<u8>> {
    general_purpose::STANDARD
        .decode(value)
        .map_err(|e| AuthError::CryptoError(format!("Invalid {name} encoding: {e}")))
}

fn derive_argon2id_key(
    password: &[u8],
    salt: &[u8],
    policy: Argon2Policy,
) -> AuthResult<Zeroizing<[u8; 32]>> {
    policy.validate()?;
    let params = Params::new(
        policy.memory_kib,
        policy.time_cost,
        policy.parallelism,
        Some(32),
    )
    .map_err(|e| AuthError::CryptoError(format!("Invalid Argon2 parameters: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut output = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(password, salt, output.as_mut())
        .map_err(|e| AuthError::CryptoError(format!("Argon2id derivation failed: {e}")))?;
    Ok(output)
}

fn argon2_aad(policy: Argon2Policy) -> Vec<u8> {
    format!(
        "kanari-auth-key-v{}:argon2id:m={}:t={}:p={}",
        PAYLOAD_VERSION_ARGON2ID,
        policy.memory_kib,
        policy.time_cost,
        policy.parallelism
    )
    .into_bytes()
}

fn validate_legacy_iterations(iterations: u32) -> AuthResult<()> {
    if !(LEGACY_MIN_KDF_ITERATIONS..=LEGACY_MAX_KDF_ITERATIONS).contains(&iterations) {
        return Err(AuthError::CryptoError(format!(
            "Legacy KDF iteration count {iterations} is outside the accepted range"
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

fn random_bytes(len: usize) -> AuthResult<Vec<u8>> {
    use aes_gcm::aead::rand_core::RngCore;
    let mut bytes = vec![0u8; len];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|e| AuthError::CryptoError(format!("OS randomness unavailable: {e}")))?;
    Ok(bytes)
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
    fn rejects_excessive_legacy_iterations_before_kdf_work() {
        let payload = EncryptedPrivateKeyPayload {
            version: 1,
            kdf: String::new(),
            ciphertext: general_purpose::STANDARD.encode([0u8; 16]),
            nonce: general_purpose::STANDARD.encode([0u8; NONCE_LEN]),
            salt: general_purpose::STANDARD.encode([0u8; SALT_LEN]),
            iterations: Some(u32::MAX),
            memory_kib: None,
            time_cost: None,
            parallelism: None,
        };
        let encoded = serde_json::to_string(&payload).unwrap();
        let error = decrypt_private_key(&encoded, "password").unwrap_err();
        assert!(error.to_string().contains("outside the accepted range"));
    }

    #[test]
    fn rejects_excessive_argon2_parameters_before_kdf_work() {
        let payload = EncryptedPrivateKeyPayload {
            version: PAYLOAD_VERSION_ARGON2ID,
            kdf: "argon2id".to_string(),
            ciphertext: general_purpose::STANDARD.encode([0u8; 16]),
            nonce: general_purpose::STANDARD.encode([0u8; NONCE_LEN]),
            salt: general_purpose::STANDARD.encode([0u8; SALT_LEN]),
            iterations: None,
            memory_kib: Some(u32::MAX),
            time_cost: Some(ARGON2_TIME_COST),
            parallelism: Some(ARGON2_PARALLELISM),
        };
        let encoded = serde_json::to_string(&payload).unwrap();
        let error = decrypt_private_key(&encoded, "password").unwrap_err();
        assert!(error.to_string().contains("outside the accepted policy"));
    }

    #[test]
    fn encrypted_key_round_trip_uses_argon2id() {
        let encrypted = encrypt_private_key("secret", "password").unwrap();
        let payload: EncryptedPrivateKeyPayload = serde_json::from_str(&encrypted).unwrap();
        assert_eq!(payload.version, PAYLOAD_VERSION_ARGON2ID);
        assert_eq!(payload.kdf, "argon2id");
        assert_eq!(
            decrypt_private_key(&encrypted, "password").unwrap(),
            "secret"
        );
    }
}
