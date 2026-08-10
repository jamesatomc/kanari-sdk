// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

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

    let encrypted = encrypt_string(original, password).expect("String encryption should succeed");
    let decrypted = decrypt_string(&encrypted, password).expect("String decryption should succeed");

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
fn test_stream_encrypt_decrypt_roundtrip() {
    let data = (0..2_500_000)
        .map(|value| (value % 251) as u8)
        .collect::<Vec<_>>();
    let password = "stream_password";
    let mut encrypted = Vec::new();
    let header = encrypt_stream(data.as_slice(), &mut encrypted, password, 64 * 1024).unwrap();
    let mut decrypted = Vec::new();

    decrypt_stream(&header, encrypted.as_slice(), &mut decrypted, password).unwrap();

    assert_eq!(decrypted, data);
}

#[test]
fn test_stream_decrypt_rejects_corruption() {
    let data = b"streaming secret";
    let password = "stream_password";
    let mut encrypted = Vec::new();
    let header = encrypt_stream(data.as_slice(), &mut encrypted, password, 8).unwrap();
    let last = encrypted.len() - 1;
    encrypted[last] ^= 0x01;

    let result = decrypt_stream(&header, encrypted.as_slice(), Vec::new(), password);

    assert!(result.is_err());
}

#[test]
fn test_stream_decrypt_rejects_truncation() {
    let data = b"streaming secret";
    let password = "stream_password";
    let mut encrypted = Vec::new();
    let header = encrypt_stream(data.as_slice(), &mut encrypted, password, 8).unwrap();
    encrypted.truncate(encrypted.len() - 2);

    let result = decrypt_stream(&header, encrypted.as_slice(), Vec::new(), password);

    assert!(result.is_err());
}

#[test]
fn test_stream_decrypt_rejects_oversized_chunk_header() {
    let password = "stream_password";
    let header = StreamEncryptionHeader {
        format_version: STREAM_ENCRYPTION_FORMAT_VERSION,
        algorithm: STREAM_ENCRYPTION_ALGORITHM.to_string(),
        salt: SaltString::generate(&mut OsRng).to_string(),
        nonce: general_purpose::STANDARD.encode([0u8; STREAM_NONCE_PREFIX_LEN]),
        chunk_size: (MAX_STREAM_CHUNK_SIZE as u32) + 1,
    };

    let result = StreamDecryptingReader::new(&header, io::empty(), password);

    assert!(matches!(result, Err(EncryptionError::InvalidInput(_))));
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
    assert!(!EncryptionScheme::Aes256Gcm.is_quantum_resistant());
    assert_eq!(EncryptionScheme::Aes256Gcm.security_level(), 4);
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
