use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, AeadCore, OsRng},
};
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{AuthError, AuthResult};

const KDF_ITERATIONS: u32 = 120_000;
const MIN_KDF_ITERATIONS: u32 = 100_000;
const MAX_KDF_ITERATIONS: u32 = 1_000_000;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const MAX_ENCODED_FIELD_LEN: usize = 128 * 1024;
const MAX_CIPHERTEXT_LEN: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedPrivateKeyPayload {
    pub ciphertext: String,
    pub nonce: String,
    pub salt: String,
    pub iterations: u32,
}

pub fn encrypt_private_key(private_key: &str, password: &str) -> AuthResult<String> {
    validate_password(password)?;

    let salt = random_bytes(SALT_LEN);
    let key = derive_key(password.as_bytes(), &salt, KDF_ITERATIONS)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|e| AuthError::CryptoError(format!("Failed to create cipher: {e}")))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, private_key.as_bytes())
        .map_err(|e| AuthError::CryptoError(format!("Private key encryption failed: {e}")))?;

    let payload = EncryptedPrivateKeyPayload {
        ciphertext: general_purpose::STANDARD.encode(ciphertext),
        nonce: general_purpose::STANDARD.encode(nonce),
        salt: general_purpose::STANDARD.encode(salt),
        iterations: KDF_ITERATIONS,
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

    validate_iterations(payload.iterations)?;
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

    let key = derive_key(password.as_bytes(), &salt, payload.iterations)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|e| AuthError::CryptoError(format!("Failed to create cipher: {e}")))?;

    let decrypted = cipher
        .decrypt(aes_gcm::Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|_| AuthError::AuthenticationFailed)?;

    String::from_utf8(decrypted)
        .map_err(|e| AuthError::SerializationError(format!("Invalid UTF-8 private key: {e}")))
}

fn validate_iterations(iterations: u32) -> AuthResult<()> {
    if !(MIN_KDF_ITERATIONS..=MAX_KDF_ITERATIONS).contains(&iterations) {
        return Err(AuthError::CryptoError(format!(
            "KDF iteration count {iterations} is outside the accepted range"
        )));
    }
    Ok(())
}

fn validate_encoded_field(name: &str, value: &str) -> AuthResult<()> {
    if value.len() > MAX_ENCODED_FIELD_LEN {
        return Err(AuthError::CryptoError(format!(
            "Encrypted private key {name} field is too large"
        )));
    }
    Ok(())
}

fn derive_key(password: &[u8], salt: &[u8], iterations: u32) -> AuthResult<Zeroizing<[u8; 32]>> {
    validate_iterations(iterations)?;

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
    fn rejects_excessive_iterations_before_kdf_work() {
        let payload = EncryptedPrivateKeyPayload {
            ciphertext: general_purpose::STANDARD.encode([0u8; 16]),
            nonce: general_purpose::STANDARD.encode([0u8; NONCE_LEN]),
            salt: general_purpose::STANDARD.encode([0u8; SALT_LEN]),
            iterations: u32::MAX,
        };
        let encoded = serde_json::to_string(&payload).unwrap();
        let error = decrypt_private_key(&encoded, "password").unwrap_err();
        assert!(error.to_string().contains("outside the accepted range"));
    }

    #[test]
    fn encrypted_key_round_trip_still_works() {
        let encrypted = encrypt_private_key("secret", "password").unwrap();
        assert_eq!(
            decrypt_private_key(&encrypted, "password").unwrap(),
            "secret"
        );
    }
}
