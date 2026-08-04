#![no_main]

use kanari_crypto::{decrypt_data, encrypt_data, secure_erase};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Skip if data is too small
    if data.len() < 10 {
        return;
    }

    // Split data into password and plaintext
    let split_point = data.len() / 2;
    let password_bytes = &data[..split_point];
    let plaintext = &data[split_point..];

    // Convert password to string (skip invalid UTF-8)
    let Ok(password) = std::str::from_utf8(password_bytes) else {
        return;
    };

    // Skip empty or too long passwords
    if password.is_empty() || password.len() > 1024 {
        return;
    }

    // Test encryption roundtrip
    if let Ok(encrypted) = encrypt_data(plaintext, password) {
        // Verify decryption with correct password
        if let Ok(decrypted) = decrypt_data(&encrypted, password) {
            // Decrypted data should match original
            assert_eq!(
                decrypted, plaintext,
                "Decryption with correct password should recover original data"
            );
        } else {
            panic!("Decryption with correct password should succeed");
        }

        // Test decryption with wrong password (should fail)
        let wrong_password = format!("{}_wrong", password);
        if let Ok(_) = decrypt_data(&encrypted, &wrong_password) {
            panic!("Decryption with wrong password should fail");
        }
    }

    // Test secure_erase
    if !plaintext.is_empty() {
        let mut test_buffer = plaintext.to_vec();
        let original_len = test_buffer.len();

        // Secure erase should zero out the buffer
        secure_erase(&mut test_buffer);
        assert!(
            test_buffer.iter().all(|&b| b == 0),
            "secure_erase should zero all bytes"
        );
        assert_eq!(
            test_buffer.len(),
            original_len,
            "Length should not change after secure_erase"
        );
    }

    // Test with empty data
    if let Ok(encrypted_empty) = encrypt_data(&[], password) {
        if let Ok(decrypted_empty) = decrypt_data(&encrypted_empty, password) {
            assert_eq!(
                decrypted_empty.len(),
                0,
                "Empty data should decrypt to empty"
            );
        }
    }
});
