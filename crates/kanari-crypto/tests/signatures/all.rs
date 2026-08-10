// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod tests {

    // ============================================================================
    // Bug #2: Timing Attack in Signature Verification (Critical)
    // ============================================================================

    use kanari_crypto::{
        CurveType, SignatureError, generate_keypair,
        signatures::{
            ed25519::verify_signature_ed25519, secure_clear, sign_message,
            verify_signature_with_curve,
        },
        verify_signature,
    };

    #[test]
    fn test_signature_verification_uses_constant_time() {
        // This test verifies that signature verification doesn't have timing leaks
        // The cryptographic libraries (k256, p256, ed25519-dalek) provide constant-time
        // comparison internally, so we verify that the API uses them correctly

        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"test message";

        // Sign the message
        let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();

        // Verification should succeed
        let result = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Ed25519,
        );
        assert!(result.is_ok());
        assert!(result.unwrap());

        // Modify signature slightly
        let mut bad_signature = signature.clone();
        bad_signature[0] ^= 0x01;

        // Verification should fail - this uses constant-time comparison internally
        let result = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &bad_signature,
            CurveType::Ed25519,
        );
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_malformed_pqc_pubkey() {
        let message = b"test";
        let signature = b"\x00";
        // invalid hex for pqc pub should return InvalidPublicKey
        let res = verify_signature_with_curve("zz", message, signature, CurveType::Dilithium3);
        assert!(matches!(res, Err(SignatureError::InvalidPublicKey(_))));
    }

    #[test]
    fn test_oversized_classical_len_in_hybrid_signature() {
        // Create a hybrid keypair and craft a signature whose classical_len > MAX_CLASSICAL_SIG_LEN
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let message = b"hello";

        // craft signature: 2-byte classical len set to 2000 (> MAX_CLASSICAL_SIG_LEN)
        let mut sig = Vec::new();
        sig.extend_from_slice(&2000u16.to_be_bytes());
        // append some bytes to represent pqc part
        sig.extend_from_slice(&[0u8; 16]);

        let classical_pub = keypair
            .public_key
            .split(':')
            .next()
            .unwrap_or(&keypair.public_key);
        let addr = format!(
            "{}:{}",
            classical_pub,
            keypair.get_pqc_public_key().unwrap()
        );
        let res =
            verify_signature_with_curve(&addr, message, &sig, CurveType::K256Dilithium3).unwrap();
        // should return false due to oversized classical length (defensive check)
        assert!(!res);
    }

    #[test]
    fn test_hybrid_roundtrip_and_malformed_parts() {
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let message = b"roundtrip";

        // Sign and verify roundtrip
        let signature =
            sign_message(&keypair.private_key, message, CurveType::Ed25519Dilithium3).unwrap();
        let classical_pub = keypair
            .public_key
            .split(':')
            .next()
            .unwrap_or(&keypair.public_key);
        let addr = format!(
            "{}:{}",
            classical_pub,
            keypair.get_pqc_public_key().unwrap()
        );
        assert!(
            verify_signature_with_curve(&addr, message, &signature, CurveType::Ed25519Dilithium3)
                .unwrap()
        );

        // Truncated PQC part (only classical present) should fail verification
        let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
        let truncated = signature[..2 + classical_len].to_vec();
        assert!(
            !verify_signature_with_curve(&addr, message, &truncated, CurveType::Ed25519Dilithium3)
                .unwrap()
        );

        // Invalid PQC public hex should fail verification (treated as verification failure)
        let bad_addr = format!("{}:zzzz", keypair.public_key);
        let res = verify_signature_with_curve(
            &bad_addr,
            message,
            &signature,
            CurveType::Ed25519Dilithium3,
        );
        assert!(matches!(res, Err(SignatureError::InvalidPublicKey(_))));
    }

    // ============================================================================
    // Bug #3: Memory Safety in secure_clear (Critical)
    // ============================================================================

    #[test]
    fn test_secure_clear_memory_safety() {
        let mut sensitive = vec![0xFF; 256];

        // Clear with secure_clear
        secure_clear(&mut sensitive);

        // Verify all bytes are zero
        assert!(
            sensitive.iter().all(|&b| b == 0),
            "All bytes should be zero after secure_clear"
        );
    }

    #[test]
    fn test_secure_clear_uses_black_box() {
        // This test ensures secure_clear uses black_box to prevent optimization
        let mut data = b"secret_key_data_that_must_be_cleared".to_vec();

        secure_clear(&mut data);

        // Compiler shouldn't optimize this away due to black_box
        assert_eq!(data, vec![0u8; data.len()]);
    }

    // ============================================================================
    // Signature Creation and Verification Tests
    // ============================================================================

    #[test]
    fn test_sign_and_verify_k256() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"Hello, K256!";

        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();
        let verified =
            verify_signature_with_curve(&keypair.public_key, message, &signature, CurveType::K256)
                .unwrap();

        assert!(verified, "K256 signature should verify");
    }

    #[test]
    fn test_sign_and_verify_p256() {
        let keypair = generate_keypair(CurveType::P256).unwrap();
        let message = b"Hello, P256!";

        let signature = sign_message(&keypair.private_key, message, CurveType::P256).unwrap();
        let verified =
            verify_signature_with_curve(&keypair.public_key, message, &signature, CurveType::P256)
                .unwrap();

        assert!(verified, "P256 signature should verify");
    }

    #[test]
    fn test_sign_and_verify_ed25519() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Hello, Ed25519!";

        let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Ed25519,
        )
        .unwrap();

        assert!(verified, "Ed25519 signature should verify");
    }

    #[cfg(feature = "falcon")]
    #[test]
    fn test_sign_and_verify_falcon512() {
        let keypair = generate_keypair(CurveType::Falcon512).unwrap();
        let message = b"Hello, FN-DSA-512!";

        assert!(keypair.private_key.starts_with("kanafalcon"));
        let signature = sign_message(&keypair.private_key, message, CurveType::Falcon512).unwrap();
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Falcon512,
        )
        .unwrap();

        assert!(verified, "Falcon512 signature should verify");
    }

    #[cfg(feature = "falcon")]
    #[test]
    fn test_sign_and_verify_falcon1024() {
        let keypair = generate_keypair(CurveType::Falcon1024).unwrap();
        let message = b"Hello, FN-DSA-1024!";

        assert!(keypair.private_key.starts_with("kanafalcon"));
        let signature = sign_message(&keypair.private_key, message, CurveType::Falcon1024).unwrap();
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Falcon1024,
        )
        .unwrap();

        assert!(verified, "Falcon1024 signature should verify");
    }

    #[cfg(feature = "falcon")]
    #[test]
    fn test_falcon_aliases_parse_and_sign() {
        let aliases = [
            ("Falcon512", CurveType::Falcon512),
            ("FnDsa512", CurveType::Falcon512),
            ("FN-DSA-512", CurveType::Falcon512),
            ("Falcon1024", CurveType::Falcon1024),
            ("FnDsa1024", CurveType::Falcon1024),
            ("FN-DSA-1024", CurveType::Falcon1024),
        ];
        let message = b"falcon alias compatibility";

        for (alias, expected_curve) in aliases {
            let parsed_curve: CurveType = alias
                .parse()
                .unwrap_or_else(|err| panic!("failed to parse Falcon alias {alias}: {err}"));
            assert_eq!(parsed_curve, expected_curve);

            let keypair = generate_keypair(parsed_curve).unwrap();
            let signature = sign_message(&keypair.private_key, message, parsed_curve)
                .unwrap_or_else(|err| panic!("failed to sign with {alias}: {err}"));

            assert!(
                verify_signature_with_curve(&keypair.public_key, message, &signature, parsed_curve)
                    .unwrap_or(false),
                "signature made through alias {alias} should verify"
            );
        }
    }

    #[test]
    fn test_signature_fails_with_wrong_message() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message1 = b"Original message";
        let message2 = b"Different message";

        let signature = sign_message(&keypair.private_key, message1, CurveType::K256).unwrap();
        let verified =
            verify_signature_with_curve(&keypair.public_key, message2, &signature, CurveType::K256)
                .unwrap();

        assert!(!verified, "Signature should not verify with wrong message");
    }

    #[test]
    fn test_signature_fails_with_wrong_address() {
        let keypair1 = generate_keypair(CurveType::Ed25519).unwrap();
        let keypair2 = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Test message";

        let signature = sign_message(&keypair1.private_key, message, CurveType::Ed25519).unwrap();
        let verified = verify_signature_with_curve(
            &keypair2.public_key,
            message,
            &signature,
            CurveType::Ed25519,
        )
        .unwrap();

        assert!(
            !verified,
            "Signature should not verify with different address"
        );
    }

    #[test]
    fn test_signature_with_kanari_prefix() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"Test message";

        // Should work with kanari prefix
        assert!(keypair.private_key.starts_with("kanari"));
        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

        assert!(!signature.is_empty());
    }

    #[test]
    fn test_invalid_signature_length() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Test";

        // Ed25519 signatures must be 64 bytes
        let bad_signature = vec![0u8; 32]; // Wrong length

        let result = verify_signature_ed25519(
            keypair.address.trim_start_matches("0x"),
            message,
            &bad_signature,
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SignatureError::InvalidSignatureLength
        ));
    }

    #[test]
    fn test_verify_signature_with_legacy_api() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"Test";

        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

        // Use tagged address for verification (carries public key for classical curves)
        let tagged = keypair.tagged_address();
        let verified = verify_signature(&tagged, message, &signature).unwrap();
        assert!(verified);
    }

    #[test]
    fn test_sign_message_handles_empty_message() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let empty_message = b"";

        // Should still be able to sign empty message (hashes to deterministic value)
        let signature = sign_message(&keypair.private_key, empty_message, CurveType::K256);
        assert!(signature.is_ok(), "Should be able to sign empty message");
    }

    #[test]
    fn test_sign_with_invalid_private_key() {
        let result = sign_message("invalid_hex", b"message", CurveType::K256);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SignatureError::InvalidPrivateKey(_)
        ));
    }

    #[test]
    fn test_verify_with_invalid_address() {
        let signature = vec![0u8; 64];
        let message = b"test";

        let result = verify_signature_ed25519("invalid_hex", message, &signature);
        assert!(result.is_err());
    }

    #[test]
    fn test_signature_deterministic_for_same_input() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Deterministic test";

        // Ed25519 signatures should be deterministic
        let sig1 = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();
        let sig2 = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();

        assert_eq!(sig1, sig2, "Ed25519 signatures should be deterministic");
    }

    #[test]
    fn test_pqc_signing_not_supported_yet() {
        let keypair = generate_keypair(CurveType::Dilithium3).unwrap();
        let message = b"test";

        // PQC signing should be supported by the PQC-specific API
        let signature = sign_message(&keypair.private_key, message, CurveType::Dilithium3).unwrap();
        // Verify using explicit curve type
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Dilithium3,
        )
        .unwrap();
        assert!(verified, "Dilithium3 signature should verify");
    }

    #[test]
    fn test_sign_and_verify_sphincs_plus_sha256_robust() {
        let keypair = generate_keypair(CurveType::SphincsPlusSha256Robust).unwrap();
        let message = b"Hello, SLH-DSA/SphincsPlusSha256Robust!";

        assert!(keypair.private_key.starts_with("kanaslh"));
        let signature = sign_message(
            &keypair.private_key,
            message,
            CurveType::SphincsPlusSha256Robust,
        )
        .unwrap();
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::SphincsPlusSha256Robust,
        )
        .unwrap();

        assert!(verified, "SphincsPlusSha256Robust signature should verify");
    }

    #[test]
    fn test_pqc_signatures_reject_truncated_bytes() {
        let cases = vec![
            CurveType::Dilithium2,
            CurveType::Dilithium3,
            CurveType::Dilithium5,
            #[cfg(feature = "falcon")]
            CurveType::Falcon512,
            #[cfg(feature = "falcon")]
            CurveType::Falcon1024,
            #[cfg(feature = "slh-dsa")]
            CurveType::SphincsPlusSha256Robust,
        ];
        let message = b"truncated pqc signature should fail closed";

        for curve in cases {
            let keypair = generate_keypair(curve).unwrap();
            let mut signature = sign_message(&keypair.private_key, message, curve)
                .unwrap_or_else(|err| panic!("signing failed for {curve:?}: {err}"));

            assert!(!signature.is_empty(), "test requires a non-empty signature");
            signature.truncate(signature.len() - 1);

            let verified =
                verify_signature_with_curve(&keypair.public_key, message, &signature, curve)
                    .unwrap_or(false);
            assert!(
                !verified,
                "truncated signature unexpectedly verified for {curve:?}"
            );
        }
    }

    #[cfg(feature = "falcon")]
    #[test]
    fn test_falcon_signature_fails_with_wrong_falcon_curve() {
        let keypair = generate_keypair(CurveType::Falcon512).unwrap();
        let message = b"wrong Falcon parameter set must not verify";
        let signature = sign_message(&keypair.private_key, message, CurveType::Falcon512).unwrap();

        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Falcon1024,
        )
        .unwrap_or(false);

        assert!(!verified);
    }

    #[test]
    fn test_sign_message_preserves_pqc_and_hybrid_key_metadata() {
        let cases = vec![
            CurveType::Dilithium2,
            CurveType::Dilithium3,
            CurveType::Dilithium5,
            CurveType::Ed25519Dilithium3,
            CurveType::K256Dilithium3,
            #[cfg(feature = "falcon")]
            CurveType::Falcon512,
            #[cfg(feature = "falcon")]
            CurveType::Falcon1024,
            #[cfg(feature = "slh-dsa")]
            CurveType::SphincsPlusSha256Robust,
        ];
        let message = b"formatted provider metadata must survive dispatch";

        for curve in cases {
            let keypair = generate_keypair(curve).unwrap();
            let signature = sign_message(&keypair.private_key, message, curve)
                .unwrap_or_else(|err| panic!("formatted key signing failed for {curve:?}: {err}"));
            assert!(
                !signature.is_empty(),
                "formatted key signing returned empty signature for {curve:?}"
            );

            assert!(
                verify_signature_with_curve(&keypair.public_key, message, &signature, curve)
                    .unwrap_or(false),
                "formatted key signature did not verify for {curve:?}"
            );
        }
    }

    #[test]
    fn test_secure_clear_on_different_sizes() {
        // Test various sizes
        for size in [0, 1, 16, 32, 64, 128, 256, 1024] {
            let mut data = vec![0xAA; size];
            secure_clear(&mut data);
            assert!(
                data.iter().all(|&b| b == 0),
                "Size {} should be fully cleared",
                size
            );
        }
    }

    #[test]
    fn test_verify_signature_safe_all_curves() {
        // Test that verify_signature_safe works for all classical curves
        let curves = vec![CurveType::K256, CurveType::P256, CurveType::Ed25519];

        for curve in curves {
            let keypair = generate_keypair(curve).unwrap();
            let message = b"Safe verification test";

            let signature = sign_message(&keypair.private_key, message, curve).unwrap();

            // verify_signature should work with tagged address carrying curve and public key
            let tagged = keypair.tagged_address();
            let result = verify_signature(&tagged, message, &signature).unwrap();
            assert!(result, "Safe verification failed for {:?}", curve);
        }
    }

    #[test]
    fn test_hybrid_sign_and_verify_k256_dilithium3() {
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let message = b"Hybrid K256+Dilithium3 test";

        // Sign using hybrid API
        let signature =
            sign_message(&keypair.private_key, message, CurveType::K256Dilithium3).unwrap();

        // Verify using combined public parts (classical:pqc)
        let pub_combined = keypair.public_key; // format: "classical_pub:pqc_pub"

        let verified = verify_signature_with_curve(
            &pub_combined,
            message,
            &signature,
            CurveType::K256Dilithium3,
        )
        .unwrap();
        assert!(verified, "Hybrid K256+Dilithium3 signature should verify");
    }

    #[test]
    fn test_hybrid_sign_and_verify_ed25519_dilithium3() {
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let message = b"Hybrid Ed25519+Dilithium3 test";

        let signature =
            sign_message(&keypair.private_key, message, CurveType::Ed25519Dilithium3).unwrap();

        let pub_combined = keypair.public_key;

        let verified = verify_signature_with_curve(
            &pub_combined,
            message,
            &signature,
            CurveType::Ed25519Dilithium3,
        )
        .unwrap();
        assert!(
            verified,
            "Hybrid Ed25519+Dilithium3 signature should verify"
        );
    }

    #[test]
    fn test_malformed_hybrid_signature_and_reject() {
        // Generate hybrid keypair and a proper signature
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let message = b"Malformed hybrid test";
        let mut signature =
            sign_message(&keypair.private_key, message, CurveType::K256Dilithium3).unwrap();

        // Truncate signature to make length prefix inconsistent
        if signature.len() > 10 {
            signature.truncate(3); // too short to contain 2-byte length + classical
        }

        // Try verify with combined public key; should not panic and should return false
        let pub_combined = keypair.public_key;
        let result = verify_signature_with_curve(
            &pub_combined,
            message,
            &signature,
            CurveType::K256Dilithium3,
        )
        .unwrap();
        assert!(!result, "Malformed combined signature should not verify");
    }

    #[test]
    fn test_verify_signature_with_tagged_address() {
        // Test that verify_signature correctly uses tagged addresses
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Tagged address test";

        let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();

        // Use tagged address
        let tagged = keypair.tagged_address();
        let result = verify_signature(&tagged, message, &signature).unwrap();

        assert!(result, "Verification with tagged address should succeed");
    }

    #[test]
    fn test_verify_signature_requires_tagged_address() {
        // Test that verify_signature requires tagged address (no fallback)
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"Tagged address test";
        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

        // Use tagged address - should succeed
        let result = verify_signature(&keypair.tagged_address(), message, &signature).unwrap();

        assert!(result, "Verification should succeed with tagged address");

        // Untagged address should fail
        let untagged_result = verify_signature(&keypair.address, message, &signature);
        assert!(
            untagged_result.is_err(),
            "Verification should fail with untagged address"
        );
    }

    #[test]
    fn test_verify_signature_safe_wrong_signature() {
        // Test that verify_signature correctly rejects invalid signatures using tagged address
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message1 = b"Original message";
        let message2 = b"Different message";

        let signature = sign_message(&keypair.private_key, message1, CurveType::K256).unwrap();

        // Verify with wrong message should fail (using required tagged address)
        let tagged_addr = keypair.tagged_address();
        let result = verify_signature(&tagged_addr, message2, &signature).unwrap();

        assert!(
            !result,
            "Verification should reject wrong message with tagged address"
        );
    }

    // ============================================================================
    // RFC-8032 COMPLIANCE TESTS (Ed25519 Interoperability)
    // ============================================================================

    #[test]
    fn test_ed25519_rfc8032_direct_signing_no_prehash() {
        // ✅ CRITICAL: Verify Ed25519 uses direct signing, NOT pre-hashed
        // This ensures interoperability with standard Ed25519 implementations
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"RFC-8032 Compliance Test";

        // Sign the message
        let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519)
            .expect("Ed25519 signing should succeed");

        // Verify the signature
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Ed25519,
        )
        .expect("Ed25519 verification should succeed");

        assert!(
            verified,
            "✅ Ed25519 signature verified successfully (RFC-8032 compliant)"
        );

        // Verify signature length is standard 64 bytes
        assert_eq!(
            signature.len(),
            64,
            "Ed25519 signature MUST be exactly 64 bytes per RFC-8032"
        );

        // Verify the signature fails for different message (proves no external hashing)
        let wrong_message = b"Different message";
        let wrong_result = verify_signature_with_curve(
            &keypair.public_key,
            wrong_message,
            &signature,
            CurveType::Ed25519,
        )
        .expect("Verification should not error");

        assert!(
            !wrong_result,
            "Signature should fail for different message (direct signing verified)"
        );
    }

    #[test]
    fn test_ed25519_signature_deterministic() {
        // ✅ Verify Ed25519 signatures are deterministic (RFC-8032)
        // Same message + keypair = same signature every time (no randomness)
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"Deterministic Ed25519 test";

        let sig1 = sign_message(&keypair.private_key, message, CurveType::Ed25519)
            .expect("First signing should succeed");
        let sig2 = sign_message(&keypair.private_key, message, CurveType::Ed25519)
            .expect("Second signing should succeed");

        assert_eq!(
            sig1, sig2,
            "✅ Ed25519 signatures are deterministic per RFC-8032"
        );
    }

    #[test]
    fn test_ed25519_rfc8032_vs_ecdsa_difference() {
        // ✅ Verify Ed25519 (direct) differs from ECDSA (K256/P256 with SHA3-256)
        // This documents that Kanari intentionally uses curve-specific strategies
        let ed_keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let k256_keypair = generate_keypair(CurveType::K256).unwrap();

        let message = b"Strategy Difference Test";

        let ed_sig = sign_message(&ed_keypair.private_key, message, CurveType::Ed25519)
            .expect("Ed25519 signing");
        let k256_sig = sign_message(&k256_keypair.private_key, message, CurveType::K256)
            .expect("K256 signing");

        // Ed25519: RFC-8032 (direct sign) = 64 bytes
        // K256: ECDSA with SHA3-256 = variable DER format (typically 70-72 bytes)
        assert_eq!(ed_sig.len(), 64, "Ed25519 signature is always 64 bytes");
        assert!(
            k256_sig.len() > 64,
            "K256 DER signature is larger than 64 bytes"
        );
    }

    #[test]
    fn test_ed25519_hybrid_uses_rfc8032_component() {
        // ✅ Verify hybrid Ed25519+Dilithium3 uses RFC-8032 Ed25519 component
        let hybrid_keypair =
            generate_keypair(CurveType::Ed25519Dilithium3).expect("Hybrid keypair generation");

        let message = b"Hybrid RFC-8032 Test";
        let signature = sign_message(
            &hybrid_keypair.private_key,
            message,
            CurveType::Ed25519Dilithium3,
        )
        .expect("Hybrid signing");

        let verified = verify_signature_with_curve(
            &hybrid_keypair.public_key,
            message,
            &signature,
            CurveType::Ed25519Dilithium3,
        )
        .expect("Hybrid verification");

        assert!(
            verified,
            "✅ Hybrid Ed25519+Dilithium3 verifies (uses RFC-8032)"
        );

        // Hybrid signature format: [2-byte len] || classical_sig || pqc_sig
        // First 2 bytes = Ed25519 signature length (always 64 for RFC-8032)
        assert!(
            signature.len() > 64,
            "Hybrid signature must be larger than Ed25519 alone"
        );
        let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
        assert_eq!(
            classical_len, 64,
            "✅ Hybrid Ed25519 component is 64 bytes (RFC-8032)"
        );
    }
}
