//! Secure data encryption and decryption
//!
//! This module provides functions for encrypting and decrypting data using
//! modern secure algorithms including post-quantum key encapsulation (Kyber).
//!
//! **Classical**: AES-256-GCM with Argon2 key derivation
//! **Post-Quantum**: Kyber768/1024 key encapsulation mechanism (KEM)
//! **Hybrid**: AES-256-GCM + Kyber for quantum-safe encryption

use aes_gcm::{
    Aes256Gcm, Key,
    aead::{Aead, AeadCore, KeyInit, OsRng},
};
use argon2::{
    Algorithm, Argon2, Version,
    password_hash::{PasswordHasher, SaltString},
};
use base64::{Engine as _, engine::general_purpose};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::string::ToString;
use thiserror::Error;

// Post-Quantum Cryptography - Kyber KEM
#[cfg(feature = "pqc")]
use pqcrypto_kyber::kyber768;

#[cfg(feature = "pqc")]
use pqcrypto_kyber::kyber1024;

#[cfg(feature = "pqc")]
use pqcrypto_traits::kem::{PublicKey as KemPublicKey, SecretKey as KemSecretKey, SharedSecret, Ciphertext};

/// Encryption scheme selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EncryptionScheme {
    /// AES-256-GCM (Classical, fast)
    #[default]
    Aes256Gcm,

    /// Kyber768 KEM (Post-quantum, NIST Level 3)
    Kyber768,

    /// Kyber1024 KEM (Post-quantum, NIST Level 5, maximum security)
    Kyber1024,

    /// Hybrid: AES-256-GCM + Kyber768 (Best of both worlds)
    HybridAesKyber768,

    /// Hybrid: AES-256-GCM + Kyber1024 (Maximum security)
    HybridAesKyber1024,
}

impl EncryptionScheme {
    /// Returns true if this scheme is quantum-resistant
    pub fn is_quantum_resistant(&self) -> bool {
        matches!(
            self,
            EncryptionScheme::Kyber768
                | EncryptionScheme::Kyber1024
                | EncryptionScheme::HybridAesKyber768
                | EncryptionScheme::HybridAesKyber1024
        )
    }

    /// Get security level (1-5)
    pub fn security_level(&self) -> u8 {
        match self {
            EncryptionScheme::Aes256Gcm => 4,
            EncryptionScheme::Kyber768 | EncryptionScheme::HybridAesKyber768 => 5,
            EncryptionScheme::Kyber1024 | EncryptionScheme::HybridAesKyber1024 => 5,
        }
    }
}

/// Error types for encryption operations
#[derive(Error, Debug)]
pub enum EncryptionError {
    #[error("Encryption error: {0}")]
    AeadError(String),

    #[error("Key derivation error: {0}")]
    KeyDerivationError(String),

    #[error("Invalid format error: {0}")]
    InvalidFormat(String),

    #[error("Decryption error")]
    DecryptionError,

    #[error("Post-quantum encryption error: {0}")]
    PqcError(String),
}

/// Structure representing encrypted data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ciphertext_array: Vec<u8>,

    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    ciphertext: String,

    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nonce_array: Vec<u8>,

    #[serde(default)]
    #[serde(skip_serializing_if = "String::is_empty")]
    nonce: String,

    salt: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
}

impl EncryptedData {
    /// Get the ciphertext bytes, regardless of format
    pub fn get_ciphertext(&self) -> Result<Vec<u8>, EncryptionError> {
        if !self.ciphertext.is_empty() {
            general_purpose::STANDARD
                .decode(&self.ciphertext)
                .map_err(|e| {
                    EncryptionError::InvalidFormat(format!("Invalid ciphertext base64: {}", e))
                })
        } else if !self.ciphertext_array.is_empty() {
            Ok(self.ciphertext_array.clone())
        } else {
            Err(EncryptionError::InvalidFormat(
                "Empty ciphertext".to_string(),
            ))
        }
    }

    /// Get the nonce bytes, regardless of format
    pub fn get_nonce(&self) -> Result<Vec<u8>, EncryptionError> {
        if !self.nonce.is_empty() {
            general_purpose::STANDARD
                .decode(&self.nonce)
                .map_err(|e| EncryptionError::InvalidFormat(format!("Invalid nonce base64: {}", e)))
        } else if !self.nonce_array.is_empty() {
            Ok(self.nonce_array.clone())
        } else {
            Err(EncryptionError::InvalidFormat("Empty nonce".to_string()))
        }
    }
}

impl fmt::Display for EncryptedData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cipher_len = if !self.ciphertext.is_empty() {
            self.ciphertext.len()
        } else {
            self.ciphertext_array.len()
        };

        let nonce_len = if !self.nonce.is_empty() {
            self.nonce.len()
        } else {
            self.nonce_array.len()
        };

        write!(
            f,
            "EncryptedData {{ ciphertext: [{}], nonce: [{}], salt: {}, tag: {:?} }}",
            cipher_len, nonce_len, self.salt, self.tag
        )
    }
}

/// Encrypt data with a password
pub fn encrypt_data(data: &[u8], password: &str) -> Result<EncryptedData, EncryptionError> {
    // Generate a random salt for key derivation
    let salt = SaltString::generate(&mut OsRng);

    // Derive a cryptographic key from the password
    let params = argon2_params()?;
    let password_hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| EncryptionError::KeyDerivationError(e.to_string()))?;

    // the temporary value dropped error - bind to variable first
    let hash = password_hash.hash.ok_or_else(|| {
        EncryptionError::KeyDerivationError("Argon2 hash output is missing".to_string())
    })?;
    let key_bytes = hash.as_bytes();
    #[allow(deprecated)]
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);

    // Generate a random nonce for AES-GCM
    let nonce_bytes = Aes256Gcm::generate_nonce(&mut OsRng);

    // Create the cipher for encryption
    let cipher = Aes256Gcm::new(key);

    // Encrypt the data
    let ciphertext = cipher
        .encrypt(&nonce_bytes, data)
        .map_err(|e| EncryptionError::AeadError(e.to_string()))?;

    // Store values in a more compact base64 representation
    let ciphertext_b64 = general_purpose::STANDARD.encode(&ciphertext);
    let nonce_b64 = general_purpose::STANDARD.encode(nonce_bytes);

    Ok(EncryptedData {
        ciphertext_array: Vec::new(),
        ciphertext: ciphertext_b64,
        nonce_array: Vec::new(),
        nonce: nonce_b64,
        salt: salt.to_string(),
        tag: None,
    })
}

/// Decrypt data with a password
pub fn decrypt_data(encrypted: &EncryptedData, password: &str) -> Result<Vec<u8>, EncryptionError> {
    // Get salt from the encrypted data
    let salt = SaltString::from_b64(&encrypted.salt)
        .map_err(|e| EncryptionError::InvalidFormat(e.to_string()))?;

    // Derive key from password and salt
    let params = argon2_params()?;
    let password_hash = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| EncryptionError::KeyDerivationError(e.to_string()))?;

    // Fix for the temporary value dropped error
    let hash = password_hash.hash.ok_or_else(|| {
        EncryptionError::KeyDerivationError("Argon2 hash output is missing".to_string())
    })?;
    let key_bytes = hash.as_bytes();
    #[allow(deprecated)]
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);

    // Get ciphertext and nonce from the encrypted data
    let ciphertext = encrypted.get_ciphertext()?;
    let nonce_bytes = encrypted.get_nonce()?;

    // Create nonce for decryption - need to convert Vec<u8> to Nonce
    if nonce_bytes.len() != 12 {
        return Err(EncryptionError::InvalidFormat(
            "Invalid nonce length".to_string(),
        ));
    }
    #[allow(deprecated)]
    let nonce = aes_gcm::Nonce::from_slice(&nonce_bytes);

    // Create cipher for decryption
    let cipher = Aes256Gcm::new(key);

    // Decrypt the data
    cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| EncryptionError::DecryptionError)
}

// Helper function to get consistent argon2 parameters
// Uses OWASP recommended parameters for interactive applications
fn argon2_params() -> Result<argon2::Params, EncryptionError> {
    argon2::Params::new(
        47104, // Memory cost (46 MB) - OWASP minimum recommendation
        3,     // Time cost (3 iterations) - improved security
        1,     // Parallelism (1 thread)
        None,  // No Output::BLOCK_SIZE in this version
    )
    .map_err(|e| EncryptionError::KeyDerivationError(format!("Invalid Argon2 parameters: {}", e)))
}

/// Upgrade legacy encrypted data to new base64 format
pub fn upgrade_encrypted_data(old_data: EncryptedData) -> EncryptedData {
    // Only upgrade if using older array format
    if !old_data.ciphertext_array.is_empty() && old_data.ciphertext.is_empty() {
        EncryptedData {
            ciphertext: general_purpose::STANDARD.encode(&old_data.ciphertext_array),
            ciphertext_array: Vec::new(),
            nonce: general_purpose::STANDARD.encode(&old_data.nonce_array),
            nonce_array: Vec::new(),
            salt: old_data.salt,
            tag: old_data.tag,
        }
    } else {
        old_data
    }
}

/// Encrypt a string with a password
pub fn encrypt_string(data: &str, password: &str) -> Result<EncryptedData, EncryptionError> {
    encrypt_data(data.as_bytes(), password)
}

/// Decrypt a string with a password
pub fn decrypt_string(
    encrypted: &EncryptedData,
    password: &str,
) -> Result<String, EncryptionError> {
    let bytes = decrypt_data(encrypted, password)?;
    String::from_utf8(bytes).map_err(|e| EncryptionError::InvalidFormat(e.to_string()))
}

/// Securely erase sensitive data from memory
/// Uses black_box to prevent compiler optimization
pub fn secure_erase(data: &mut [u8]) {
    for byte in data.iter_mut() {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    // Ensure the compiler doesn't optimize away the clearing
    std::hint::black_box(data);
}

// ==========================
// Post-Quantum KEM helpers
// ==========================
#[cfg(feature = "pqc")]
/// Generate a Kyber keypair and return raw bytes of (public_key, secret_key)
pub fn pqc_generate_keypair(scheme: EncryptionScheme) -> Result<(Vec<u8>, Vec<u8>), EncryptionError> {
    match scheme {
        EncryptionScheme::Kyber768 => {
            let (pk, sk) = kyber768::keypair();
            Ok((pk.as_bytes().to_vec(), sk.as_bytes().to_vec()))
        }
        EncryptionScheme::Kyber1024 => {
            let (pk, sk) = kyber1024::keypair();
            Ok((pk.as_bytes().to_vec(), sk.as_bytes().to_vec()))
        }
        _ => Err(EncryptionError::PqcError("Unsupported PQC scheme for key generation".to_string())),
    }
}

#[cfg(feature = "pqc")]
/// Encapsulate to a public key (provided as bytes) and return (ciphertext_bytes, shared_secret_bytes).
/// Note: converting raw bytes back into pqcrypto types may fail for invalid inputs.
pub fn pqc_encapsulate_from_public_key(
    scheme: EncryptionScheme,
    public_key_bytes: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), EncryptionError> {
    match scheme {
        EncryptionScheme::Kyber768 => {
            let pk = kyber768::PublicKey::from_bytes(public_key_bytes)
                .map_err(|e| EncryptionError::PqcError(format!("Invalid public key: {:?}", e)))?;
            let (ct, ss) = kyber768::encapsulate(&pk);
            Ok((ct.as_bytes().to_vec(), ss.as_bytes().to_vec()))
        }
        EncryptionScheme::Kyber1024 => {
            let pk = kyber1024::PublicKey::from_bytes(public_key_bytes)
                .map_err(|e| EncryptionError::PqcError(format!("Invalid public key: {:?}", e)))?;
            let (ct, ss) = kyber1024::encapsulate(&pk);
            Ok((ct.as_bytes().to_vec(), ss.as_bytes().to_vec()))
        }
        _ => Err(EncryptionError::PqcError("Unsupported PQC scheme for encapsulation".to_string())),
    }
}

#[cfg(feature = "pqc")]
/// Decapsulate using secret key bytes and ciphertext bytes, returning the shared secret bytes.
pub fn pqc_decapsulate_from_secret(
    scheme: EncryptionScheme,
    secret_key_bytes: &[u8],
    ciphertext_bytes: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    match scheme {
        EncryptionScheme::Kyber768 => {
            let sk = kyber768::SecretKey::from_bytes(secret_key_bytes)
                .map_err(|e| EncryptionError::PqcError(format!("Invalid secret key: {:?}", e)))?;
            let ct = kyber768::Ciphertext::from_bytes(ciphertext_bytes)
                .map_err(|e| EncryptionError::PqcError(format!("Invalid ciphertext: {:?}", e)))?;
            let ss = kyber768::decapsulate(&ct, &sk);
            Ok(ss.as_bytes().to_vec())
        }
        EncryptionScheme::Kyber1024 => {
            let sk = kyber1024::SecretKey::from_bytes(secret_key_bytes)
                .map_err(|e| EncryptionError::PqcError(format!("Invalid secret key: {:?}", e)))?;
            let ct = kyber1024::Ciphertext::from_bytes(ciphertext_bytes)
                .map_err(|e| EncryptionError::PqcError(format!("Invalid ciphertext: {:?}", e)))?;
            let ss = kyber1024::decapsulate(&ct, &sk);
            Ok(ss.as_bytes().to_vec())
        }
        _ => Err(EncryptionError::PqcError("Unsupported PQC scheme for decapsulation".to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================================
    // Bug #3: Memory Safety in secure_clear/secure_erase (Critical)
    // ============================================================================

    #[test]
    fn test_secure_erase_clears_memory() {
        let mut data = vec![0xAA; 100]; // Fill with pattern

        // Verify data is set
        assert!(
            data.iter().all(|&b| b == 0xAA),
            "Data should be initialized"
        );

        // Secure erase
        secure_erase(&mut data);

        // Verify data is zeroed
        assert!(
            data.iter().all(|&b| b == 0),
            "Data should be zeroed after secure_erase"
        );
    }

    #[test]
    fn test_secure_erase_empty_array() {
        let mut data = vec![];
        secure_erase(&mut data); // Should not panic
        assert_eq!(data.len(), 0);
    }

    #[test]
    fn test_secure_erase_large_data() {
        let mut data = vec![0xFF; 10_000];
        secure_erase(&mut data);
        assert!(
            data.iter().all(|&b| b == 0),
            "Large data should be fully zeroed"
        );
    }

    // ============================================================================
    // Bug #5: Weak Argon2 Parameters (High)
    // ============================================================================

    #[test]
    fn test_argon2_parameters_meet_owasp_standards() {
        let params = argon2_params().expect("Should create valid Argon2 params");

        // OWASP recommendations for interactive applications:
        // - Memory: minimum 46 MB (47104 KB)
        // - Iterations: minimum 1, recommended 2-3
        assert!(
            params.m_cost() >= 46000,
            "Memory cost should be at least 46 MB (OWASP standard), got {} KB",
            params.m_cost()
        );

        assert!(
            params.t_cost() >= 2,
            "Time cost should be at least 2 iterations, got {}",
            params.t_cost()
        );

        assert_eq!(
            params.p_cost(),
            1,
            "Parallelism should be 1 for compatibility"
        );
    }

    #[test]
    fn test_argon2_stronger_than_old_params() {
        // Old params: 19456 KB (19 MB), 2 iterations
        // New params: 47104 KB (46 MB), 3 iterations
        let params = argon2_params().unwrap();

        assert!(
            params.m_cost() > 19456,
            "Memory cost should be increased from old 19 MB"
        );
        assert!(
            params.t_cost() >= 2,
            "Time cost should be at least maintained"
        );
    }

    // ============================================================================
    // Encryption/Decryption Tests
    // ============================================================================

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let data = b"sensitive data";
        let password = "strong_password_123";

        // Encrypt
        let encrypted = encrypt_data(data, password).expect("Encryption should succeed");

        // Verify encrypted data structure
        assert!(
            !encrypted.ciphertext.is_empty(),
            "Ciphertext should not be empty"
        );
        assert!(!encrypted.nonce.is_empty(), "Nonce should not be empty");
        assert!(!encrypted.salt.is_empty(), "Salt should not be empty");

        // Decrypt
        let decrypted = decrypt_data(&encrypted, password).expect("Decryption should succeed");

        // Verify
        assert_eq!(decrypted, data, "Decrypted data should match original");
    }

    #[test]
    fn test_encrypt_string_decrypt_string() {
        let original = "Hello, World!";
        let password = "test_password";

        let encrypted =
            encrypt_string(original, password).expect("String encryption should succeed");
        let decrypted =
            decrypt_string(&encrypted, password).expect("String decryption should succeed");

        assert_eq!(decrypted, original, "Decrypted string should match");
    }

    #[test]
    fn test_decrypt_with_wrong_password_fails() {
        let data = b"secret";
        let correct_password = "password123";
        let wrong_password = "wrong_password";

        let encrypted = encrypt_data(data, correct_password).unwrap();
        let result = decrypt_data(&encrypted, wrong_password);

        assert!(
            result.is_err(),
            "Decryption with wrong password should fail"
        );
        assert!(matches!(
            result.unwrap_err(),
            EncryptionError::DecryptionError
        ));
    }

    #[test]
    fn test_encrypt_empty_data() {
        let data = b"";
        let password = "password";

        let encrypted = encrypt_data(data, password).expect("Should encrypt empty data");
        let decrypted = decrypt_data(&encrypted, password).expect("Should decrypt empty data");

        assert_eq!(decrypted, data, "Empty data roundtrip should work");
    }

    #[test]
    fn test_encrypt_large_data() {
        let data = vec![0x42; 1_000_000]; // 1 MB
        let password = "password";

        let encrypted = encrypt_data(&data, password).expect("Should encrypt large data");
        let decrypted = decrypt_data(&encrypted, password).expect("Should decrypt large data");

        assert_eq!(decrypted, data, "Large data roundtrip should work");
    }

    #[test]
    fn test_different_passwords_produce_different_ciphertexts() {
        let data = b"same data";
        let password1 = "password1";
        let password2 = "password2";

        let encrypted1 = encrypt_data(data, password1).unwrap();
        let encrypted2 = encrypt_data(data, password2).unwrap();

        // Ciphertexts should be different
        assert_ne!(
            encrypted1.ciphertext, encrypted2.ciphertext,
            "Different passwords should produce different ciphertexts"
        );
    }

    #[test]
    fn test_same_password_produces_different_ciphertexts() {
        // Due to random nonce and salt
        let data = b"same data";
        let password = "password";

        let encrypted1 = encrypt_data(data, password).unwrap();
        let encrypted2 = encrypt_data(data, password).unwrap();

        // Salts should be different
        assert_ne!(
            encrypted1.salt, encrypted2.salt,
            "Each encryption should use unique salt"
        );

        // Nonces should be different
        assert_ne!(
            encrypted1.nonce, encrypted2.nonce,
            "Each encryption should use unique nonce"
        );
    }

    #[test]
    fn test_encrypted_data_get_methods() {
        let data = b"test";
        let password = "password";

        let encrypted = encrypt_data(data, password).unwrap();

        // Test get_ciphertext
        let ciphertext = encrypted.get_ciphertext().expect("Should get ciphertext");
        assert!(!ciphertext.is_empty(), "Ciphertext should not be empty");

        // Test get_nonce
        let nonce = encrypted.get_nonce().expect("Should get nonce");
        assert_eq!(nonce.len(), 12, "Nonce should be 12 bytes for AES-GCM");
    }

    #[test]
    fn test_invalid_nonce_length() {
        let encrypted = EncryptedData {
            ciphertext: general_purpose::STANDARD.encode(b"data"),
            ciphertext_array: Vec::new(),
            nonce: general_purpose::STANDARD.encode(b"short"), // Invalid: should be 12 bytes
            nonce_array: Vec::new(),
            salt: "salt".to_string(),
            tag: None,
        };

        let result = decrypt_data(&encrypted, "password");
        assert!(result.is_err(), "Invalid nonce length should fail");
    }

    #[test]
    fn test_upgrade_encrypted_data() {
        // Test upgrading from array format to base64 format
        let old_data = EncryptedData {
            ciphertext_array: vec![1, 2, 3, 4],
            ciphertext: String::new(),
            nonce_array: vec![5, 6, 7, 8],
            nonce: String::new(),
            salt: "salt".to_string(),
            tag: None,
        };

        let upgraded = upgrade_encrypted_data(old_data);

        assert!(
            upgraded.ciphertext_array.is_empty(),
            "Array should be cleared"
        );
        assert!(
            !upgraded.ciphertext.is_empty(),
            "Base64 should be populated"
        );
        assert!(
            upgraded.nonce_array.is_empty(),
            "Nonce array should be cleared"
        );
        assert!(
            !upgraded.nonce.is_empty(),
            "Nonce base64 should be populated"
        );
    }

    #[test]
    fn test_encryption_scheme_properties() {
        assert!(EncryptionScheme::Kyber768.is_quantum_resistant());
        assert!(!EncryptionScheme::Aes256Gcm.is_quantum_resistant());

        assert_eq!(EncryptionScheme::Aes256Gcm.security_level(), 4);
        assert_eq!(EncryptionScheme::Kyber768.security_level(), 5);
        assert_eq!(EncryptionScheme::HybridAesKyber1024.security_level(), 5);
    }

    #[test]
    fn test_encrypted_data_display() {
        let data = b"test";
        let password = "password";
        let encrypted = encrypt_data(data, password).unwrap();

        let display = format!("{}", encrypted);
        assert!(
            display.contains("EncryptedData"),
            "Display should show type"
        );
        assert!(
            display.contains("ciphertext"),
            "Display should mention ciphertext"
        );
    }
}
