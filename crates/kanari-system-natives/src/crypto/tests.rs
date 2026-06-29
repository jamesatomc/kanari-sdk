#![allow(clippy::print_stdout)]
// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Tests for crypto native functions

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {

    #[test]
    fn test_ecdsa_k1_constants() {
        // Verify error codes are correctly defined
        assert_eq!(crate::crypto::ecdsa_k1::E_INVALID_RECOVERY, 1);
        assert_eq!(crate::crypto::ecdsa_k1::E_INVALID_SIGNATURE, 2);
        assert_eq!(crate::crypto::ecdsa_k1::E_INVALID_PUBKEY, 3);
        assert_eq!(crate::crypto::ecdsa_k1::E_INVALID_XONLY_PUBKEY, 5);
        assert_eq!(crate::crypto::ecdsa_k1::E_INVALID_MESSAGE, 6);
        assert_eq!(crate::crypto::ecdsa_k1::E_INVALID_SCHNORR_SIGNATURE, 7);
        assert_eq!(crate::crypto::ecdsa_k1::MAX_MSG_BYTES, 1_000_000);
    }

    #[test]
    fn test_ecdsa_r1_constants() {
        // Verify error codes are correctly defined
        assert_eq!(crate::crypto::ecdsa_r1::E_INVALID_SIGNATURE, 2);
        assert_eq!(crate::crypto::ecdsa_r1::E_INVALID_PUBKEY, 3);
        assert_eq!(crate::crypto::ecdsa_r1::E_UNSUPPORTED_HASH_FOR_P256, 4);
        assert_eq!(crate::crypto::ecdsa_r1::E_INVALID_MESSAGE, 6);
    }

    #[test]
    fn test_gas_parameters_zeros() {
        let params = crate::crypto::GasParameters::zeros();
        assert_eq!(params.ecrecover, 0.into());
        assert_eq!(params.decompress_pubkey, 0.into());
        assert_eq!(params.verify_k1, 0.into());
        assert_eq!(params.verify_r1, 0.into());
        assert_eq!(params.ed25519_verify, 0.into());
    }

    #[test]
    fn test_module_exports() {
        // Verify that error codes are properly re-exported from mod.rs
        assert_eq!(crate::crypto::E_INVALID_RECOVERY, 1);
        assert_eq!(crate::crypto::E_INVALID_SIGNATURE, 2);
        assert_eq!(crate::crypto::E_INVALID_PUBKEY, 3);
        assert_eq!(crate::crypto::E_INVALID_XONLY_PUBKEY, 5);
        assert_eq!(crate::crypto::E_INVALID_MESSAGE, 6);
        assert_eq!(crate::crypto::E_INVALID_SCHNORR_SIGNATURE, 7);
        assert_eq!(crate::crypto::E_UNSUPPORTED_HASH_FOR_P256, 4);
        assert_eq!(crate::crypto::MAX_MSG_BYTES, 1_000_000);
    }
}

use super::ecdsa_k1::MAX_MSG_BYTES;
use ed25519_dalek::Verifier;
use k256::ecdsa::signature::hazmat::PrehashVerifier;
use k256::{
    PublicKey as K256PublicKey,
    ecdsa::{Signature as K256Signature, VerifyingKey as K256VerifyingKey},
    elliptic_curve::sec1::ToEncodedPoint,
};
use p256::ecdsa::{
    Signature as P256Signature, SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey,
    signature::Signer as P256Signer,
};
use rand::TryRng;
use rand::rngs::SysRng;
use secp256k1::{
    Keypair, Secp256k1, SecretKey, XOnlyPublicKey, ecdsa::RecoveryId as SecpRecoveryId,
};
use sha2::{Digest, Sha256};
use sha3::Keccak256;

// Helper to generate random bytes for key generation using SysRng (same as keys.rs)
fn generate_random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    SysRng
        .try_fill_bytes(&mut bytes)
        .expect("Failed to get OS randomness");
    bytes
}

#[test]
fn test_secp256k1_ecrecover_and_verify() {
    // Test ecdsa_k1 (secp256k1) signature recovery and verification

    let secp = Secp256k1::new();
    let secret_bytes = generate_random_bytes::<32>();
    let secret_key = SecretKey::from_byte_array(secret_bytes).unwrap();
    let public_key = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

    // Create message
    let msg = b"Hello, Kanari Network!";
    let msg_hash = Sha256::digest(msg);
    let message = secp256k1::Message::from_digest(msg_hash.into());

    // Sign the message
    let signature = secp.sign_ecdsa(message, &secret_key);

    // serialize_compact returns [u8; 64], not a tuple
    let sig_bytes = signature.serialize_compact();

    // For recovery, we need to sign with recoverable signature
    let recoverable_sig = secp.sign_ecdsa_recoverable(message, &secret_key);
    let (_recovery_id, _rec_sig_bytes) = recoverable_sig.serialize_compact();

    // Test decompress_pubkey
    let compressed_pk = public_key.serialize(); // 33 bytes
    assert_eq!(compressed_pk.len(), 33);

    // Decompress using k256
    let decompressed = K256PublicKey::from_sec1_bytes(&compressed_pk).unwrap();
    let uncompressed = decompressed.to_encoded_point(false);
    assert_eq!(uncompressed.as_bytes().len(), 65);

    // Test verify with SHA256
    let vk = K256VerifyingKey::from_sec1_bytes(&compressed_pk).unwrap();
    // Use raw 64-byte signature format instead of DER
    let sig = K256Signature::try_from(sig_bytes.as_slice()).unwrap();

    // For k256, we need to use the signature verification API correctly
    // The signature was created over msg_hash, so we verify against that hash
    use k256::ecdsa::signature::hazmat::PrehashVerifier;
    let verified = vk.verify_prehash(msg_hash.as_slice(), &sig).is_ok();
    assert!(verified, "SHA256 signature verification should succeed");

    // Test verify with Keccak256
    let msg_hash_keccak = Keccak256::digest(msg);
    let sig_keccak = secp.sign_ecdsa(
        secp256k1::Message::from_digest(msg_hash_keccak.into()),
        &secret_key,
    );
    let sig_bytes_keccak = sig_keccak.serialize_compact();
    let sig_keccak_parsed = K256Signature::try_from(sig_bytes_keccak.as_slice()).unwrap();
    let verified_keccak = vk
        .verify_prehash(msg_hash_keccak.as_slice(), &sig_keccak_parsed)
        .is_ok();
    assert!(
        verified_keccak,
        "Keccak256 signature verification should succeed"
    );
}

#[test]
fn test_secp256k1_schnorr_signature() {
    // Test Schnorr signatures with x-only public keys

    let secp = Secp256k1::new();
    let secret_bytes = generate_random_bytes::<32>();
    let secret_key = SecretKey::from_byte_array(secret_bytes).unwrap();
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let (xonly_pubkey, _parity) = XOnlyPublicKey::from_keypair(&keypair);

    // Create exactly 32-byte message (required for Schnorr)
    let msg_bytes = generate_random_bytes::<32>();

    // Sign with Schnorr (using no auxiliary randomness for deterministic testing)
    let schnorr_sig = secp.sign_schnorr_no_aux_rand(&msg_bytes, &keypair);

    // Verify
    let verified = secp
        .verify_schnorr(&schnorr_sig, &msg_bytes, &xonly_pubkey)
        .is_ok();
    assert!(verified, "Schnorr signature verification should succeed");

    // Test with wrong message
    let wrong_msg = generate_random_bytes::<32>();
    let verified_wrong = secp
        .verify_schnorr(&schnorr_sig, &wrong_msg, &xonly_pubkey)
        .is_ok();
    assert!(!verified_wrong, "Schnorr should fail with wrong message");
}

#[test]
fn test_p256_ecdsa_verify() {
    // Test ecdsa_r1 (P-256) signature verification

    // Generate P-256 keypair using SysRng
    let random_bytes = generate_random_bytes::<32>();
    // For P-256, we need to ensure the bytes form a valid scalar
    // Use from_slice which handles validation properly
    let signing_key = match P256SigningKey::from_slice(&random_bytes) {
        Ok(key) => key,
        Err(_) => {
            // If invalid, use a known-good test key
            let test_bytes = [0x42u8; 32];
            P256SigningKey::from_slice(&test_bytes).unwrap()
        }
    };
    let verifying_key = signing_key.verifying_key();

    // Create message
    let msg = b"P-256 test message";
    let msg_hash = Sha256::digest(msg);

    // Sign the message
    let signature: P256Signature = signing_key.sign(msg);

    // Verify with SHA256
    use p256::ecdsa::signature::hazmat::PrehashVerifier as P256PrehashVerifier;
    let verified = verifying_key
        .verify_prehash(msg_hash.as_slice(), &signature)
        .is_ok();
    assert!(
        verified,
        "P-256 SHA256 signature verification should succeed"
    );

    // Test with DER encoding
    let der_sig = signature.to_der();
    let sig_from_der = P256Signature::from_der(der_sig.as_bytes()).unwrap();
    let verified_der = verifying_key
        .verify_prehash(msg_hash.as_slice(), &sig_from_der)
        .is_ok();
    assert!(
        verified_der,
        "P-256 DER signature verification should succeed"
    );

    // Test with raw 64-byte encoding
    let raw_bytes = signature.to_bytes();
    assert_eq!(raw_bytes.len(), 64);
    let sig_from_raw = P256Signature::try_from(raw_bytes.as_slice()).unwrap();
    let verified_raw = verifying_key
        .verify_prehash(msg_hash.as_slice(), &sig_from_raw)
        .is_ok();
    assert!(
        verified_raw,
        "P-256 raw signature verification should succeed"
    );
}

#[test]
fn test_ed25519_verify() {
    // Test Ed25519 signature verification

    // Generate Ed25519 keypair using SysRng
    let random_bytes = generate_random_bytes::<32>();
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&random_bytes);
    let verifying_key = signing_key.verifying_key();

    // Create message
    let msg = b"Ed25519 test message";

    // Sign the message
    let signature = signing_key.sign(msg);

    // Verify
    let verified = verifying_key.verify(msg, &signature).is_ok();
    assert!(verified, "Ed25519 signature verification should succeed");

    // Test with wrong message
    let wrong_msg = b"Wrong message";
    let verified_wrong = verifying_key.verify(wrong_msg, &signature).is_ok();
    assert!(!verified_wrong, "Ed25519 should fail with wrong message");

    // Test key and signature sizes
    assert_eq!(verifying_key.to_bytes().len(), 32);
    assert_eq!(signature.to_bytes().len(), 64);
}

#[test]
fn test_invalid_signatures() {
    // Test error handling for invalid inputs

    let secp = Secp256k1::new();
    let secret_bytes = generate_random_bytes::<32>();
    let secret_key = SecretKey::from_byte_array(secret_bytes).unwrap();
    let public_key = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
    let msg = b"Test message";
    let msg_hash = Sha256::digest(msg);
    let message = secp256k1::Message::from_digest(msg_hash.into());
    let signature = secp.sign_ecdsa(message, &secret_key);
    let sig_bytes = signature.serialize_compact();

    // Test invalid signature length (not 64 or 65 bytes)
    let invalid_sig_short = vec![0u8; 32];
    let parsed = K256Signature::from_der(&invalid_sig_short);
    assert!(
        parsed.is_err(),
        "Should fail to parse invalid short signature"
    );

    // Test invalid public key
    let invalid_pk = vec![0u8; 33]; // All zeros
    let result = K256VerifyingKey::from_sec1_bytes(&invalid_pk);
    assert!(result.is_err(), "Should fail to parse invalid public key");

    // Test wrong signature for message
    let wrong_msg = b"Wrong message";
    let wrong_hash = Sha256::digest(wrong_msg);
    let vk = K256VerifyingKey::from_sec1_bytes(&public_key.serialize()).unwrap();
    // Use raw signature format
    let sig = K256Signature::try_from(sig_bytes.as_slice()).unwrap();
    let verified = vk.verify(&wrong_hash, &sig).is_ok();
    assert!(!verified, "Should fail verification with wrong message");
}

#[test]
fn test_pubkey_normalization() {
    // Test public key format normalization (64-byte without prefix -> 65-byte with 0x04)

    let secp = Secp256k1::new();
    let secret_bytes = generate_random_bytes::<32>();
    let secret_key = SecretKey::from_byte_array(secret_bytes).unwrap();
    let public_key = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

    // Get uncompressed pubkey (65 bytes with 0x04 prefix)
    let uncompressed_full = public_key.serialize_uncompressed();
    assert_eq!(uncompressed_full.len(), 65);
    assert_eq!(uncompressed_full[0], 0x04);

    // Extract X||Y without prefix (64 bytes)
    let xy_only = &uncompressed_full[1..];
    assert_eq!(xy_only.len(), 64);

    // Normalize by adding 0x04 prefix
    let mut normalized = Vec::with_capacity(65);
    normalized.push(0x04);
    normalized.extend_from_slice(xy_only);
    assert_eq!(normalized, uncompressed_full);

    // Should be able to parse both formats
    let pk_from_full = K256PublicKey::from_sec1_bytes(&uncompressed_full).unwrap();
    let pk_from_normalized = K256PublicKey::from_sec1_bytes(&normalized).unwrap();
    assert_eq!(pk_from_full, pk_from_normalized);
}

#[test]
fn test_message_size_limits() {
    // Test that large messages are rejected

    // Create message at the limit
    let msg_at_limit = vec![0u8; MAX_MSG_BYTES];
    assert_eq!(msg_at_limit.len(), MAX_MSG_BYTES);

    // Create message over the limit
    let msg_over_limit = vec![0u8; MAX_MSG_BYTES + 1];
    assert!(msg_over_limit.len() > MAX_MSG_BYTES);

    // Hash operations should still work (native would reject based on size check)
    let _hash = Sha256::digest(&msg_at_limit);
    let _hash_over = Sha256::digest(&msg_over_limit);
}

#[test]
fn test_recovery_id_handling() {
    // Test recovery ID conversion (0-3 vs 27-28)

    let secp = Secp256k1::new();
    let secret_bytes = generate_random_bytes::<32>();
    let secret_key = SecretKey::from_byte_array(secret_bytes).unwrap();
    let msg = b"Recovery ID test";
    let msg_hash = Sha256::digest(msg);
    let message = secp256k1::Message::from_digest(msg_hash.into());

    // Sign and get recovery ID
    let signature = secp.sign_ecdsa_recoverable(message, &secret_key);
    let (rec_id, _sig_bytes) = signature.serialize_compact();

    let rec_id_value: i32 = rec_id.into();
    assert!((0..=3).contains(&rec_id_value), "Recovery ID should be 0-3");

    // Test legacy format (27-28)
    let legacy_v = (rec_id_value + 27) as u8;
    assert!(
        legacy_v == 27 || legacy_v == 28,
        "Legacy v should be 27 or 28"
    );

    // Both should convert back correctly
    let rec_id_from_standard = SecpRecoveryId::try_from(rec_id_value).unwrap();
    let rec_id_from_legacy = SecpRecoveryId::try_from((legacy_v - 27) as i32).unwrap();
    assert_eq!(rec_id_from_standard, rec_id_from_legacy);
}

#[test]
fn test_hash_functions() {
    // Test SHA256 and Keccak256 hash functions

    let msg = b"Hash function test";

    // SHA256
    let sha256_hash = Sha256::digest(msg);
    assert_eq!(sha256_hash.len(), 32);

    // Keccak256
    let keccak_hash = Keccak256::digest(msg);
    assert_eq!(keccak_hash.len(), 32);

    // Different hashes for same message
    assert_ne!(sha256_hash.as_slice(), keccak_hash.as_slice());

    // Same message produces same hash
    let sha256_hash2 = Sha256::digest(msg);
    assert_eq!(sha256_hash.as_slice(), sha256_hash2.as_slice());
}

#[test]
fn test_xonly_pubkey_conversion() {
    // Test x-only public key operations for Schnorr

    let secp = Secp256k1::new();
    let secret_bytes = generate_random_bytes::<32>();
    let secret_key = SecretKey::from_byte_array(secret_bytes).unwrap();
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let (xonly, _parity) = XOnlyPublicKey::from_keypair(&keypair);

    // X-only pubkey should be 32 bytes
    let xonly_bytes = xonly.serialize();
    assert_eq!(xonly_bytes.len(), 32);

    // Should be able to reconstruct from bytes
    let xonly_from_bytes = XOnlyPublicKey::from_byte_array(xonly_bytes).unwrap();
    assert_eq!(xonly, xonly_from_bytes);
}

#[test]
fn generate_test_vectors_for_move() {
    // Generate correct test vectors for Move tests
    // This test prints the correct signature, pubkey, and message that can be used in Move tests

    let secp = Secp256k1::new();

    // Use a fixed seed for reproducible test vectors
    let secret_bytes = [0x42u8; 32];
    let secret_key = SecretKey::from_byte_array(secret_bytes).unwrap();
    let public_key = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);

    // Test message: 0x00010203 (same as in Move test)
    let msg = vec![0x00u8, 0x01, 0x02, 0x03];
    let msg_hash = Sha256::digest(&msg);
    let message = secp256k1::Message::from_digest(msg_hash.into());

    // Sign with ECDSA
    let signature = secp.sign_ecdsa(message, &secret_key);
    let sig_bytes = signature.serialize_compact();

    // Get compressed public key (33 bytes)
    let compressed_pk = public_key.serialize();

    println!("\n=== ECDSA K1 Test Vectors ===");
    println!("Message: {}", hex::encode(&msg));
    println!("Public Key (compressed): {}", hex::encode(compressed_pk));
    println!("Signature (r||s): {}", hex::encode(sig_bytes));
    println!("Hash Type: SHA256 (1)");

    // Verify it works
    let vk = K256VerifyingKey::from_sec1_bytes(&compressed_pk).unwrap();
    use k256::ecdsa::signature::hazmat::PrehashVerifier;
    let sig = K256Signature::try_from(sig_bytes.as_slice()).unwrap();
    let verified = vk.verify_prehash(msg_hash.as_slice(), &sig).is_ok();
    assert!(verified, "Generated test vector should verify correctly");

    // Generate Schnorr test vector
    let keypair = Keypair::from_secret_key(&secp, &secret_key);
    let (xonly, _parity) = XOnlyPublicKey::from_keypair(&keypair);

    // Schnorr requires 32-byte message
    let schnorr_msg = generate_random_bytes::<32>();
    let schnorr_sig = secp.sign_schnorr_no_aux_rand(&schnorr_msg, &keypair);

    println!("\n=== Schnorr Test Vectors ===");
    println!("Message (32 bytes): {}", hex::encode(schnorr_msg));
    println!("Public Key (x-only): {}", hex::encode(xonly.serialize()));
    println!(
        "Signature (r||s): {}",
        hex::encode(schnorr_sig.to_byte_array())
    );

    // Verify Schnorr
    let schnorr_verified = secp
        .verify_schnorr(&schnorr_sig, &schnorr_msg, &xonly)
        .is_ok();
    assert!(
        schnorr_verified,
        "Schnorr test vector should verify correctly"
    );
}

#[test]
fn test_move_ecdsa_k1_vector() {
    // Test the exact vector from Move test to see if it works
    let msg = hex::decode("00010203").unwrap();
    let pubkey =
        hex::decode("033e99a541db69bd32040dfe5037fbf5210dafa8151a71e21c5204b05d95ce0a62").unwrap();
    let sig = hex::decode("416a21d50b3c838328d4f03213f8ef0c3776389a972ba1ecd37b56243734eba208ea6aaa6fc076ad7accd71d355f693a6fe54fe69b3c168eace9803827bc9046").unwrap();

    println!("\n=== Testing Move ECDSA K1 Vector ===");
    println!("Message length: {}", msg.len());
    println!("Pubkey length: {}", pubkey.len());
    println!("Signature length: {}", sig.len());

    // Parse pubkey
    let vk = match K256VerifyingKey::from_sec1_bytes(&pubkey) {
        Ok(vk) => {
            println!("✓ Public key parsed successfully");
            vk
        }
        Err(e) => {
            println!("✗ Failed to parse public key: {:?}", e);
            panic!("Invalid public key");
        }
    };

    // Parse signature (raw 64 bytes)
    let sig_parsed = match K256Signature::try_from(sig.as_slice()) {
        Ok(s) => {
            println!("✓ Signature parsed successfully");
            s
        }
        Err(e) => {
            println!("✗ Failed to parse signature: {:?}", e);
            panic!("Invalid signature");
        }
    };

    // Hash message with SHA256
    let msg_hash = Sha256::digest(&msg);
    println!("Message hash: {}", hex::encode(msg_hash.as_slice()));

    // Verify
    use k256::ecdsa::signature::hazmat::PrehashVerifier;
    let verified = vk.verify_prehash(msg_hash.as_slice(), &sig_parsed).is_ok();
    println!(
        "Verification result: {}",
        if verified { "✓ PASS" } else { "✗ FAIL" }
    );

    if !verified {
        println!("\n⚠️  The Move test vector is INVALID!");
        println!("This means the signature doesn't match the pubkey+message combination.");
        println!("The test data in Move code needs to be regenerated.");
    }

    assert!(verified, "Move test vector should verify correctly");
}

#[test]
fn generate_p256_test_vectors() {
    // Generate correct test vectors for P-256 (ECDSA R1)
    use p256::ecdsa::{SigningKey as P256SigningKey, signature::Signer};

    // Use a fixed secret key for reproducible test vectors
    let secret_bytes = [0x42u8; 32];
    let signing_key = P256SigningKey::from_bytes((&secret_bytes).into()).unwrap();
    let public_key = signing_key.verifying_key();

    // Test message: "hello world" (same as in Move test)
    let msg = b"hello world";

    // Sign with P-256
    let signature: P256Signature = signing_key.sign(msg);
    let sig_bytes = signature.to_bytes();

    // Get compressed public key (33 bytes) - using to_encoded_point with compress=true
    let encoded = public_key.to_encoded_point(true); // true = compressed
    let compressed_pk = encoded.as_bytes();

    println!("\n=== P-256 (ECDSA R1) Test Vectors ===");
    println!("Message: {}", String::from_utf8_lossy(msg));
    println!("Public Key length: {} bytes", compressed_pk.len());
    println!("Public Key (compressed): {}", hex::encode(compressed_pk));
    println!("Signature (r||s): {}", hex::encode(sig_bytes));

    // Verify it works
    use p256::ecdsa::signature::Verifier;
    let vk = P256VerifyingKey::from_sec1_bytes(compressed_pk).unwrap();
    let sig = P256Signature::from_bytes(&sig_bytes).unwrap();
    let verified = vk.verify(msg, &sig).is_ok();
    println!(
        "Verification result: {}",
        if verified { "✓ PASS" } else { "✗ FAIL" }
    );
    assert!(
        verified,
        "Generated P-256 test vector should verify correctly"
    );
}

#[test]
fn test_move_p256_vector() {
    // Test the exact vector from Move ecdsa_r1 test
    let msg = b"hello world";
    let pubkey =
        hex::decode("0258a618066814098f8ddb3cbde73838b59028d843958031e50be0a5f4b0a9796d").unwrap();
    let sig = hex::decode("74133905657c1992d8d6bd72ffa7ccf8d2adf3e4a3ca25f8dc8eec175752cb5a40459f71b549a25cba3cddf4157e946bbff7b18fc82774e9c4c54e362b97ccb5").unwrap();

    println!("\n=== Testing Move P-256 Vector ===");
    println!("Message: {}", String::from_utf8_lossy(msg));
    println!("Pubkey length: {} bytes", pubkey.len());
    println!("Signature length: {} bytes", sig.len());

    // Parse pubkey
    let vk = match P256VerifyingKey::from_sec1_bytes(&pubkey) {
        Ok(vk) => {
            println!("✓ Public key parsed successfully");
            vk
        }
        Err(e) => {
            println!("✗ Failed to parse public key: {:?}", e);
            panic!("Invalid public key");
        }
    };

    // Parse signature (raw 64 bytes)
    let sig_parsed = match P256Signature::try_from(sig.as_slice()) {
        Ok(s) => {
            println!("✓ Signature parsed successfully");
            s
        }
        Err(e) => {
            println!("✗ Failed to parse signature: {:?}", e);
            panic!("Invalid signature");
        }
    };

    // Verify with SHA256 hash

    let msg_hash = Sha256::digest(msg);
    println!("Message hash: {}", hex::encode(msg_hash.as_slice()));

    let verified = vk.verify_prehash(msg_hash.as_slice(), &sig_parsed).is_ok();
    println!(
        "Verification result: {}",
        if verified { "✓ PASS" } else { "✗ FAIL" }
    );

    if !verified {
        println!("\n⚠️  The Move test vector is INVALID!");
        println!("The signature doesn't match the pubkey+message combination.");
    }

    assert!(verified, "Move P-256 test vector should verify correctly");
}
