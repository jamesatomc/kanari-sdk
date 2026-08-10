// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use base64::{Engine as _, engine::general_purpose};
use kanari_crypto::{
    CurveType, MAX_PASSWORD_LEN, RateLimiter, SignatureError, StreamEncryptionHeader, decrypt_data,
    decrypt_stream, encrypt_data, encrypt_stream, generate_keypair,
    hd_wallet::{HdError, derive_multiple_addresses},
    is_password_strong,
    keys::{KeyError, generate_mnemonic, keypair_from_private_key},
    keystore::{Keystore, KeystoreError},
    signatures::{sign_message, verify_signature_with_curve},
    verify_signature,
};
use serde_json::Value;
use std::io;

fn assert_verification_rejected(result: Result<bool, SignatureError>) {
    match result {
        Ok(false) => {}
        Err(
            SignatureError::InvalidFormat(_)
            | SignatureError::InvalidPublicKey(_)
            | SignatureError::InvalidSignatureLength
            | SignatureError::VerificationFailed,
        ) => {}
        Ok(true) => panic!("tampered signature unexpectedly verified"),
        Err(err) => panic!("unexpected signature error: {err:?}"),
    }
}

#[test]
fn attack_simulation_rejects_signature_tampering_across_classical_curves() {
    for curve in [CurveType::K256, CurveType::P256, CurveType::Ed25519] {
        let keypair = generate_keypair(curve).unwrap();
        let message = b"kanari attack simulation: original payload";
        let signature = sign_message(&keypair.private_key, message, curve).unwrap();

        assert!(
            verify_signature_with_curve(&keypair.public_key, message, &signature, curve).unwrap()
        );

        let mut tampered_signature = signature.clone();
        let last = tampered_signature.len() - 1;
        tampered_signature[last] ^= 0x80;
        assert_verification_rejected(verify_signature_with_curve(
            &keypair.public_key,
            message,
            &tampered_signature,
            curve,
        ));

        assert_verification_rejected(verify_signature_with_curve(
            &keypair.public_key,
            b"kanari attack simulation: modified payload",
            &signature,
            curve,
        ));

        let wrong_keypair = generate_keypair(curve).unwrap();
        assert_verification_rejected(verify_signature_with_curve(
            &wrong_keypair.public_key,
            message,
            &signature,
            curve,
        ));
    }
}

#[test]
fn attack_simulation_rejects_untagged_and_cross_curve_signature_replay() {
    let keypair = generate_keypair(CurveType::K256).unwrap();
    let message = b"kanari attack simulation: replay payload";
    let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

    assert!(matches!(
        verify_signature(&keypair.address, message, &signature),
        Err(SignatureError::InvalidFormat(_))
    ));

    assert!(
        verify_signature(&keypair.tagged_address(), message, &signature).unwrap(),
        "tagged K256 address must verify its own signature"
    );

    assert_verification_rejected(verify_signature_with_curve(
        &keypair.public_key,
        message,
        &signature,
        CurveType::P256,
    ));
    assert_verification_rejected(verify_signature_with_curve(
        &keypair.public_key,
        message,
        &signature,
        CurveType::Ed25519,
    ));
}

#[test]
fn attack_simulation_rejects_oversized_signature_before_curve_parsing() {
    let keypair = generate_keypair(CurveType::Ed25519).unwrap();
    let oversized_signature = vec![0xA5; 64 * 1024 + 1];

    assert!(matches!(
        verify_signature(&keypair.tagged_address(), b"payload", &oversized_signature),
        Err(SignatureError::InvalidFormat(message)) if message.contains("too large")
    ));

    assert!(matches!(
        verify_signature_with_curve(
            &keypair.public_key,
            b"payload",
            &oversized_signature,
            CurveType::Ed25519,
        ),
        Err(SignatureError::InvalidFormat(message)) if message.contains("too large")
    ));
}

#[test]
fn attack_simulation_rejects_hybrid_signature_component_tampering() {
    let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
    let message = b"kanari attack simulation: hybrid payload";
    let signature = sign_message(&keypair.private_key, message, CurveType::K256Dilithium3).unwrap();

    assert!(
        verify_signature(&keypair.tagged_address(), message, &signature).unwrap(),
        "hybrid signature must verify before tampering"
    );

    let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
    assert!(classical_len > 0);
    assert!(signature.len() > 2 + classical_len);

    let mut tampered_classical = signature.clone();
    tampered_classical[2] ^= 0x01;
    assert_verification_rejected(verify_signature(
        &keypair.tagged_address(),
        message,
        &tampered_classical,
    ));

    let mut tampered_pqc = signature.clone();
    tampered_pqc[2 + classical_len] ^= 0x01;
    assert_verification_rejected(verify_signature(
        &keypair.tagged_address(),
        message,
        &tampered_pqc,
    ));
}

#[test]
fn attack_simulation_rejects_pqc_signature_and_message_tampering() {
    let keypair = generate_keypair(CurveType::Dilithium3).unwrap();
    let message = b"kanari attack simulation: pqc payload";
    let signature = sign_message(&keypair.private_key, message, CurveType::Dilithium3).unwrap();

    assert!(
        verify_signature(&keypair.tagged_address(), message, &signature).unwrap(),
        "PQC signature must verify before tampering"
    );

    let mut tampered_signature = signature.clone();
    let middle = tampered_signature.len() / 2;
    tampered_signature[middle] ^= 0x20;
    assert_verification_rejected(verify_signature(
        &keypair.tagged_address(),
        message,
        &tampered_signature,
    ));

    assert_verification_rejected(verify_signature(
        &keypair.tagged_address(),
        b"kanari attack simulation: pqc payload with attacker edit",
        &signature,
    ));
}

#[test]
fn attack_simulation_rejects_hybrid_public_key_substitution() {
    let legitimate = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
    let attacker = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
    let message = b"kanari attack simulation: hybrid key substitution";
    let signature = sign_message(
        &legitimate.private_key,
        message,
        CurveType::Ed25519Dilithium3,
    )
    .unwrap();

    assert!(
        verify_signature(&legitimate.tagged_address(), message, &signature).unwrap(),
        "legitimate hybrid key must verify before substitution"
    );

    let legitimate_classical = legitimate.public_key.split(':').next().unwrap();
    let attacker_pqc = attacker.get_pqc_public_key().unwrap();
    let substituted_address = format!("Ed25519Dilithium3:{legitimate_classical}:{attacker_pqc}");

    assert_verification_rejected(verify_signature(&substituted_address, message, &signature));
}

#[test]
fn attack_simulation_rejects_encrypted_payload_tampering_and_wrong_password() {
    let encrypted = encrypt_data(
        b"kanari attack simulation: wallet secret",
        "CorrectHorseBatteryStaple!2026",
    )
    .unwrap();

    assert!(decrypt_data(&encrypted, "WrongHorseBatteryStaple!2026").is_err());

    let mut json = serde_json::to_value(&encrypted).unwrap();
    let ciphertext = json
        .get("ciphertext")
        .and_then(Value::as_str)
        .expect("encrypted data should serialize ciphertext");
    let mut ciphertext_bytes = general_purpose::STANDARD.decode(ciphertext).unwrap();
    ciphertext_bytes[0] ^= 0x40;
    json["ciphertext"] =
        Value::String(general_purpose::STANDARD.encode(ciphertext_bytes.as_slice()));
    let tampered = serde_json::from_value(json).unwrap();

    assert!(decrypt_data(&tampered, "CorrectHorseBatteryStaple!2026").is_err());
}

#[test]
fn attack_simulation_rejects_nonce_and_salt_tampering() {
    let encrypted = encrypt_data(
        b"kanari attack simulation: nonce and salt protected secret",
        "CorrectHorseBatteryStaple!2026",
    )
    .unwrap();
    let password = "CorrectHorseBatteryStaple!2026";

    let mut nonce_json = serde_json::to_value(&encrypted).unwrap();
    let nonce = nonce_json
        .get("nonce")
        .and_then(Value::as_str)
        .expect("encrypted data should serialize nonce");
    let mut nonce_bytes = general_purpose::STANDARD.decode(nonce).unwrap();
    nonce_bytes[0] ^= 0x01;
    nonce_json["nonce"] = Value::String(general_purpose::STANDARD.encode(nonce_bytes.as_slice()));
    let tampered_nonce = serde_json::from_value(nonce_json).unwrap();
    assert!(decrypt_data(&tampered_nonce, password).is_err());

    let mut salt_json = serde_json::to_value(&encrypted).unwrap();
    let mut salt = salt_json
        .get("salt")
        .and_then(Value::as_str)
        .expect("encrypted data should serialize salt")
        .to_string();
    salt.push('A');
    salt_json["salt"] = Value::String(salt);
    let tampered_salt = serde_json::from_value(salt_json).unwrap();
    assert!(decrypt_data(&tampered_salt, password).is_err());
}

#[test]
fn attack_simulation_rejects_malformed_encrypted_payload_encoding() {
    let encrypted = encrypt_data(
        b"kanari attack simulation: malformed encoding secret",
        "CorrectHorseBatteryStaple!2026",
    )
    .unwrap();
    let password = "CorrectHorseBatteryStaple!2026";

    let mut bad_ciphertext_json = serde_json::to_value(&encrypted).unwrap();
    bad_ciphertext_json["ciphertext"] = Value::String("%%%not-base64%%%".to_string());
    let bad_ciphertext = serde_json::from_value(bad_ciphertext_json).unwrap();
    assert!(decrypt_data(&bad_ciphertext, password).is_err());

    let mut bad_nonce_json = serde_json::to_value(&encrypted).unwrap();
    bad_nonce_json["nonce"] = Value::String("%%%not-base64%%%".to_string());
    let bad_nonce = serde_json::from_value(bad_nonce_json).unwrap();
    assert!(decrypt_data(&bad_nonce, password).is_err());
}

#[test]
fn attack_simulation_rejects_stream_ciphertext_truncation_and_bitflip() {
    let plaintext = b"kanari stream attack simulation payload repeated".repeat(256);
    let password = "CorrectHorseBatteryStaple!2026";
    let mut encrypted = Vec::new();
    let header = encrypt_stream(plaintext.as_slice(), &mut encrypted, password, 257).unwrap();

    let mut decrypted = Vec::new();
    decrypt_stream(&header, encrypted.as_slice(), &mut decrypted, password).unwrap();
    assert_eq!(decrypted, plaintext);

    let truncated = &encrypted[..encrypted.len() - 1];
    assert!(decrypt_stream(&header, truncated, Vec::new(), password).is_err());

    let mut bitflipped = encrypted.clone();
    let middle = bitflipped.len() / 2;
    bitflipped[middle] ^= 0x08;
    assert!(decrypt_stream(&header, bitflipped.as_slice(), Vec::new(), password).is_err());
}

#[test]
fn attack_simulation_rejects_stream_header_downgrade_and_trailing_frames() {
    let plaintext = b"kanari stream header attack simulation".repeat(64);
    let password = "CorrectHorseBatteryStaple!2026";
    let mut encrypted = Vec::new();
    let header = encrypt_stream(plaintext.as_slice(), &mut encrypted, password, 128).unwrap();

    let mut bad_version = header.clone();
    bad_version.format_version = bad_version.format_version.saturating_add(1);
    assert!(decrypt_stream(&bad_version, encrypted.as_slice(), Vec::new(), password).is_err());

    let mut bad_algorithm = header.clone();
    bad_algorithm.algorithm = "AES-256-GCM-LEGACY".to_string();
    assert!(decrypt_stream(&bad_algorithm, encrypted.as_slice(), Vec::new(), password).is_err());

    let mut bad_nonce = header.clone();
    bad_nonce.nonce = general_purpose::STANDARD.encode([0u8; 3]);
    assert!(decrypt_stream(&bad_nonce, encrypted.as_slice(), Vec::new(), password).is_err());

    let mut with_trailing_frame = encrypted.clone();
    with_trailing_frame.extend_from_slice(&[1, 0, 0, 0, 16]);
    with_trailing_frame.extend_from_slice(&[0xAA; 16]);
    assert!(
        decrypt_stream(
            &header,
            with_trailing_frame.as_slice(),
            Vec::new(),
            password
        )
        .is_err()
    );
}

#[test]
fn attack_simulation_rejects_stream_invalid_chunk_sizes_and_passwords() {
    let header = StreamEncryptionHeader {
        format_version: 1,
        algorithm: "AES-256-GCM-FRAMED-ARGON2ID".to_string(),
        salt: "not-valid-salt".to_string(),
        nonce: general_purpose::STANDARD.encode([0u8; 4]),
        chunk_size: 0,
    };

    assert!(decrypt_stream(&header, io::empty(), Vec::new(), "valid-password").is_err());
    assert!(encrypt_stream(io::empty(), Vec::new(), "", 1024).is_err());
    assert!(encrypt_data(b"secret", &"x".repeat(MAX_PASSWORD_LEN + 1)).is_err());
}

#[test]
fn attack_simulation_rejects_private_key_import_resource_abuse() {
    let oversized_private_key = format!("kana{}", "AA".repeat(128 * 1024));
    assert!(matches!(
        keypair_from_private_key(&oversized_private_key, CurveType::K256),
        Err(KeyError::InvalidPrivateKey)
    ));

    for malformed in [
        "",
        "kana",
        "kana-not-hex",
        "K256:0x1234",
        "kanamldsa65:public-only",
        "kana00010203",
    ] {
        assert!(
            keypair_from_private_key(malformed, CurveType::K256).is_err(),
            "malformed private key unexpectedly imported: {malformed}"
        );
    }
}

#[test]
fn attack_simulation_rejects_keystore_corruption_and_key_count_dos() {
    let encrypted = encrypt_data(b"wallet secret", "CorrectHorseBatteryStaple!2026").unwrap();
    let mut keystore = Keystore::new();
    keystore
        .keys
        .insert("0xgood".to_string(), encrypted.clone());
    assert!(keystore.validate().is_ok());

    let mut corrupted_json = serde_json::to_value(&encrypted).unwrap();
    corrupted_json["ciphertext"] = Value::String("%%%not-base64%%%".to_string());
    let corrupted = serde_json::from_value(corrupted_json).unwrap();
    keystore.keys.insert("0xbad".to_string(), corrupted);
    assert!(matches!(
        keystore.validate(),
        Err(KeystoreError::Corrupted(message)) if message.contains("Invalid ciphertext")
    ));

    let mut huge_keystore = Keystore::new();
    for index in 0..10_001 {
        huge_keystore
            .keys
            .insert(format!("0x{index:064x}"), encrypted.clone());
    }
    assert!(matches!(
        huge_keystore.validate(),
        Err(KeystoreError::Corrupted(message)) if message.contains("too many keys")
    ));
}

#[test]
fn attack_simulation_rejects_hd_wallet_derivation_abuse() {
    let mnemonic = generate_mnemonic(12).unwrap();

    assert!(matches!(
        derive_multiple_addresses(&mnemonic, "", "m/44'/784'/0'/0'", CurveType::Ed25519, 1,),
        Err(HdError::InvalidDerivationPath(_))
    ));

    assert!(matches!(
        derive_multiple_addresses(
            &mnemonic,
            "",
            "m/44'/784'/0'/0'/{index}",
            CurveType::Dilithium3,
            1,
        ),
        Err(HdError::DerivationFailed(_))
    ));

    assert!(matches!(
        derive_multiple_addresses(
            &mnemonic,
            "",
            "m/44'/784'/0'/0'/{index}",
            CurveType::Ed25519,
            1001,
        ),
        Err(HdError::RateLimitExceeded(_))
    ));
}

#[test]
fn attack_simulation_password_policy_and_lockout_behave_defensively() {
    for weak in [
        "password",
        "Password123!",
        "aaaaaaaaaaaaaaaaA1!",
        "abcabcABC123!!!abcabc",
        "NoSpecialCharacter12345",
    ] {
        assert!(!is_password_strong(weak), "weak password accepted: {weak}");
    }
    assert!(is_password_strong("Kanari!Strong#Password2026"));

    let mut limiter = RateLimiter::new(3, 60);
    let actor = "attacker-ip:203.0.113.7";
    assert!(limiter.check_allowed(actor));
    limiter.record_failure(actor);
    limiter.record_failure(actor);
    limiter.record_failure(actor);
    assert!(!limiter.check_allowed(actor));
    assert!(limiter.get_lockout_remaining(actor).is_some());
    limiter.record_success(actor);
    assert!(limiter.check_allowed(actor));
}
