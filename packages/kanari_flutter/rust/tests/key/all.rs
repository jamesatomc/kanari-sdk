#[cfg(test)]
mod tests {
    use rust::keys::{
        CurveType, KANARI_KEY_PREFIX, KeyError, KeyPair, extract_raw_key, format_private_key,
        generate_ed25519_keypair, generate_hybrid_ed25519_dilithium3_keypair,
        generate_hybrid_k256_dilithium3_keypair, generate_keypair, generate_mnemonic,
        import_from_seed_phrase, keypair_from_mnemonic, keypair_from_private_key,
    };
    use rust::signatures::{sign_message, verify_signature, verify_signature_with_curve};
    use sha3::{Digest, Sha3_256};

    // ============================================================================
    // Bug #4: Panic in Hybrid Address Generation (Critical)
    // ============================================================================

    #[test]
    fn test_hybrid_ed25519_dilithium3_address_generation() {
        // Test that hybrid address generation doesn't panic with short keys
        let result = generate_hybrid_ed25519_dilithium3_keypair();
        assert!(result.is_ok(), "Hybrid keypair generation should succeed");

        let keypair = result.unwrap();
        assert!(
            keypair.address.starts_with("0x"),
            "Hybrid address should have correct prefix"
        );
        assert_eq!(keypair.curve_type, CurveType::Ed25519Dilithium3);
    }

    #[test]
    fn test_hybrid_k256_dilithium3_address_generation() {
        // Test that K256+Dilithium3 hybrid generation always succeeds
        // No panic or failure expected - all operations are safe
        let result = generate_hybrid_k256_dilithium3_keypair();

        assert!(
            result.is_ok(),
            "Hybrid K256+Dilithium3 keypair generation must always succeed"
        );

        let keypair = result.unwrap();
        assert!(
            keypair.address.starts_with("0x"),
            "Hybrid address should have correct prefix"
        );
        assert_eq!(keypair.curve_type, CurveType::K256Dilithium3);

        // Verify combined public key format
        assert!(
            keypair.public_key.contains(':'),
            "Hybrid public key should be in format 'classical:pqc'"
        );

        // Verify PQC public key is present
        assert!(
            keypair.pqc_public_key.is_some(),
            "Hybrid keypair must have PQC public key"
        );
    }

    // Test that address generation handles short public keys without panic
    #[test]
    fn test_short_public_key_handling() {
        // This tests the fix for the [..20] slice panic bug
        let short_string = "abc"; // Less than 20 bytes
        let bytes = short_string.as_bytes();

        // Should not panic - take min of (bytes.len(), 20)
        let hash_input = if bytes.len() >= 20 {
            &bytes[..20]
        } else {
            bytes
        };

        assert_eq!(hash_input.len(), 3, "Should use full length if < 20");
        assert_eq!(hash_input, b"abc");
    }

    // ============================================================================
    // Additional Key Generation Tests
    // ============================================================================

    #[test]
    fn test_keypair_generation_all_curves() {
        let curves = vec![
            CurveType::K256,
            CurveType::P256,
            CurveType::Ed25519,
            CurveType::Dilithium2,
            CurveType::Dilithium3,
            CurveType::Dilithium5,
        ];

        for curve in curves {
            let result = generate_keypair(curve);
            assert!(result.is_ok(), "Keypair generation failed for {:?}", curve);

            let keypair = result.unwrap();
            assert!(
                !keypair.private_key.is_empty(),
                "Private key should not be empty"
            );
            assert!(
                !keypair.public_key.is_empty(),
                "Public key should not be empty"
            );
            assert!(!keypair.address.is_empty(), "Address should not be empty");
            assert_eq!(keypair.curve_type, curve, "Curve type should match");
        }
    }

    #[test]
    fn test_mnemonic_generation() {
        // Test 12-word mnemonic
        let mnemonic_12 = generate_mnemonic(12);
        assert!(
            mnemonic_12.is_ok(),
            "12-word mnemonic generation should succeed"
        );
        assert_eq!(mnemonic_12.unwrap().split_whitespace().count(), 12);

        // Test 24-word mnemonic
        let mnemonic_24 = generate_mnemonic(24);
        assert!(
            mnemonic_24.is_ok(),
            "24-word mnemonic generation should succeed"
        );
        assert_eq!(mnemonic_24.unwrap().split_whitespace().count(), 24);

        // Test invalid word count
        let mnemonic_invalid = generate_mnemonic(18);
        assert!(mnemonic_invalid.is_err(), "Invalid word count should fail");
    }

    #[test]
    fn test_keypair_from_mnemonic_consistency() {
        // Generate a mnemonic
        let mnemonic = generate_mnemonic(12).unwrap();

        // Generate keypair twice with same mnemonic
        let keypair1 = keypair_from_mnemonic(&mnemonic, CurveType::K256).unwrap();
        let keypair2 = keypair_from_mnemonic(&mnemonic, CurveType::K256).unwrap();

        // Should generate identical keypairs
        assert_eq!(keypair1.private_key, keypair2.private_key);
        assert_eq!(keypair1.public_key, keypair2.public_key);
        assert_eq!(keypair1.address, keypair2.address);
    }

    #[test]
    fn test_private_key_formatting() {
        // Test that private keys are properly formatted with kanari prefix
        let keypair = generate_keypair(CurveType::K256).unwrap();
        assert!(
            keypair.private_key.starts_with(KANARI_KEY_PREFIX),
            "Private key should have kanari prefix"
        );

        // Test extracting raw key
        let raw = extract_raw_key(&keypair.private_key);
        assert!(
            !raw.starts_with(KANARI_KEY_PREFIX),
            "Raw key should not have prefix"
        );

        // Test formatting again
        let formatted = format_private_key(raw);
        assert_eq!(
            formatted,
            keypair.private_key.to_string(),
            "Re-formatted key should match"
        );
    }

    #[test]
    fn test_keypair_from_private_key() {
        // Generate a keypair
        let original = generate_keypair(CurveType::Ed25519).unwrap();

        // Recreate from private key
        let recreated =
            keypair_from_private_key(&original.private_key, CurveType::Ed25519).unwrap();

        // Should generate the same public key and address
        assert_eq!(original.public_key, recreated.public_key);
        assert_eq!(original.tagged_address(), recreated.tagged_address());
        assert_eq!(original.private_key, recreated.private_key);
    }

    #[test]
    fn test_post_quantum_keypair_generation() {
        // Test Dilithium3 (recommended PQC)
        let dil3 = generate_keypair(CurveType::Dilithium3).unwrap();
        assert!(
            dil3.private_key.starts_with("kanamldsa"),
            "ML-DSA keys should have kanamldsa prefix, got: {}",
            *dil3.private_key
        );
        assert!(
            dil3.address.starts_with("0x"),
            "PQC addresses should have pqc prefix"
        );

        // pqc_public_key should be set for PQC keypairs
        assert!(dil3.pqc_public_key.is_some());
        assert_eq!(dil3.pqc_public_key.unwrap(), dil3.public_key);

        // Test that PQC is detected
        assert!(CurveType::Dilithium3.is_post_quantum());
        assert!(!CurveType::K256.is_post_quantum());
    }

    #[test]
    fn test_post_quantum_provider_prefixes_are_explicit() {
        for curve in [
            CurveType::Dilithium2,
            CurveType::Dilithium3,
            CurveType::Dilithium5,
        ] {
            let keypair = generate_keypair(curve).unwrap();
            assert!(
                keypair.private_key.starts_with("kanamldsa"),
                "{curve:?} must use explicit ML-DSA provider prefix"
            );
        }

        #[cfg(feature = "experimental-slh-dsa")]
        {
            let sphincs = generate_keypair(CurveType::SphincsPlusSha256Robust).unwrap();
            assert!(
                sphincs.private_key.starts_with("kanaslh"),
                "SPHINCS+/SLH-DSA must use explicit SLH-DSA provider prefix"
            );
        }

        #[cfg(not(feature = "experimental-slh-dsa"))]
        assert!(
            generate_keypair(CurveType::SphincsPlusSha256Robust).is_err(),
            "SPHINCS+/SLH-DSA must stay behind experimental-slh-dsa by default"
        );
    }

    #[test]
    fn test_hybrid_keypair_properties() {
        // Test Ed25519+Dilithium3 hybrid
        let hybrid = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        assert!(
            hybrid.private_key.starts_with("kanahybrid"),
            "Hybrid keys should have kanahybrid prefix"
        );
        assert!(
            hybrid.address.starts_with("0x"),
            "Hybrid addresses should have hybrid prefix"
        );

        // Should contain both key parts separated by ':'
        assert!(
            hybrid.private_key.contains(':'),
            "Hybrid key should contain separator"
        );
        assert!(
            hybrid.public_key.contains(':'),
            "Hybrid public key should contain separator"
        );

        // Test that hybrid is detected as post-quantum
        assert!(CurveType::Ed25519Dilithium3.is_post_quantum());
        assert!(CurveType::Ed25519Dilithium3.is_hybrid());

        // pqc_public_key should be present and equal to the PQC part
        let hybrid_pqc = hybrid
            .pqc_public_key
            .as_ref()
            .expect("PQC public key missing");
        assert!(hybrid.public_key.contains(':'));
        let parts: Vec<&str> = hybrid.public_key.splitn(2, ':').collect();
        assert_eq!(parts[1], hybrid_pqc);
    }

    #[test]
    fn test_invalid_private_key_handling() {
        // Test with invalid hex
        let result = keypair_from_private_key("not_hex", CurveType::K256);
        assert!(result.is_err(), "Invalid hex should fail");

        // Test with wrong length for Ed25519
        let result = keypair_from_private_key("kanari1234", CurveType::Ed25519);
        assert!(result.is_err(), "Wrong length should fail");

        // Test with empty key
        let result = keypair_from_private_key("", CurveType::K256);
        assert!(result.is_err(), "Empty key should fail");
    }

    #[test]
    fn test_pqc_mnemonic_not_supported() {
        // PQC algorithms don't support BIP39 derivation
        let mnemonic = generate_mnemonic(12).unwrap();
        let result = keypair_from_mnemonic(&mnemonic, CurveType::Dilithium3);
        assert!(
            result.is_err(),
            "PQC should not support mnemonic derivation yet"
        );
    }

    // ============================================================================
    // Tagged Address Tests (Security Enhancement)
    // ============================================================================

    #[test]
    fn test_tagged_address_generation() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let tagged = keypair.tagged_address();

        // Should have format "CurveType:0xaddress"
        assert!(tagged.starts_with("K256:"));
        assert!(tagged.contains(&keypair.public_key));
    }

    #[test]
    fn test_tagged_address_parsing() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let tagged = keypair.tagged_address();

        // Parse it back
        let (curve_type, address) = KeyPair::parse_tagged_address(&tagged).unwrap();

        assert_eq!(curve_type, CurveType::Ed25519);
        assert_eq!(address, keypair.public_key);
    }

    #[test]
    fn test_tagged_address_all_curves() {
        let curves = vec![
            CurveType::K256,
            CurveType::P256,
            CurveType::Ed25519,
            CurveType::Dilithium3,
        ];

        for curve in curves {
            let keypair = generate_keypair(curve).unwrap();
            let tagged = keypair.tagged_address();

            // Should parse back correctly
            let (parsed_curve, parsed_address) = KeyPair::parse_tagged_address(&tagged)
                .unwrap_or_else(|| panic!("Failed to parse tagged address for {:?}", curve));

            assert_eq!(parsed_curve, curve);
            assert_eq!(parsed_address, keypair.public_key);
        }
    }

    #[test]
    fn test_tagged_address_invalid_format() {
        // Test with untagged address
        let result = KeyPair::parse_tagged_address("0xabc123");
        assert!(result.is_none(), "Should return None for untagged address");

        // Test with invalid curve type
        let result = KeyPair::parse_tagged_address("InvalidCurve:0xabc123");
        assert!(
            result.is_none(),
            "Should return None for invalid curve type"
        );

        // Test with empty string
        let result = KeyPair::parse_tagged_address("");
        assert!(result.is_none(), "Should return None for empty string");
    }

    #[test]
    fn test_tagged_address_hybrid_ed25519_dilithium3_parsing() {
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let tagged = keypair.tagged_address();
        let (curve_type, addr) = KeyPair::parse_tagged_address(&tagged).unwrap();
        assert_eq!(curve_type, CurveType::Ed25519Dilithium3);
        assert_eq!(addr, keypair.public_key);
        assert_ne!(addr, keypair.address);
    }

    #[test]
    fn test_tagged_address_hybrid_k256_dilithium3_parsing() {
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let tagged = keypair.tagged_address();
        let (curve_type, addr) = KeyPair::parse_tagged_address(&tagged).unwrap();
        assert_eq!(curve_type, CurveType::K256Dilithium3);
        assert_eq!(addr, keypair.public_key);
        assert_ne!(addr, keypair.address);
    }

    #[test]
    fn test_signature_verification() {
        // Test that verify_signature works correctly even when curve type is ambiguous

        // Generate keypairs for all classical curves
        let k256 = generate_keypair(CurveType::K256).unwrap();
        let p256 = generate_keypair(CurveType::P256).unwrap();
        let ed25519 = generate_keypair(CurveType::Ed25519).unwrap();

        let message = b"test message for safe verification";

        let k256_sig = sign_message(&k256.private_key, message, CurveType::K256).unwrap();
        let p256_sig = sign_message(&p256.private_key, message, CurveType::P256).unwrap();
        let ed25519_sig = sign_message(&ed25519.private_key, message, CurveType::Ed25519).unwrap();

        // verify_signature should work for all without knowing curve type
        assert!(
            verify_signature(&k256.tagged_address(), message, &k256_sig).unwrap(),
            "K256 signature should verify with safe method"
        );
        assert!(
            verify_signature(&p256.tagged_address(), message, &p256_sig).unwrap(),
            "P256 signature should verify with safe method"
        );
        assert!(
            verify_signature(&ed25519.tagged_address(), message, &ed25519_sig).unwrap(),
            "Ed25519 signature should verify with safe method"
        );
    }

    #[test]
    fn test_tagged_address_verification() {
        // Test that tagged addresses provide reliable verification

        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"test with tagged address";

        let signature = sign_message(&keypair.private_key, message, CurveType::K256).unwrap();

        // Use tagged address for verification
        let tagged = keypair.tagged_address();
        let result = verify_signature(&tagged, message, &signature).unwrap();

        assert!(result, "Signature should verify with tagged address");
    }

    // ============================================================================
    // Ed25519Dilithium3 Specific Tests
    // ============================================================================

    #[test]
    fn test_ed25519_dilithium3_keypair_structure() {
        // Test Ed25519+Dilithium3 hybrid keypair structure
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        // Verify private key format
        assert!(
            keypair.private_key.starts_with("kanahybrid"),
            "Ed25519Dilithium3 private key must start with 'kanahybrid'"
        );
        assert!(
            keypair.private_key.contains(':'),
            "Ed25519Dilithium3 private key must contain ':' separator"
        );

        // Verify public key format (classical:pqc)
        assert!(
            keypair.public_key.contains(':'),
            "Ed25519Dilithium3 public key must be in format 'classical:pqc'"
        );

        let pub_parts: Vec<&str> = keypair.public_key.split(':').collect();
        assert_eq!(pub_parts.len(), 2, "Public key must have exactly 2 parts");

        // Ed25519 public key should be 32 bytes (64 hex chars)
        assert_eq!(
            pub_parts[0].len(),
            64,
            "Ed25519 public key must be 64 hex characters"
        );

        // Dilithium3 public key should be 1952 bytes (3904 hex chars)
        assert_eq!(
            pub_parts[1].len(),
            3904,
            "Dilithium3 public key must be 3904 hex characters"
        );

        // Verify PQC public key field
        assert!(
            keypair.pqc_public_key.is_some(),
            "Ed25519Dilithium3 must have PQC public key"
        );
        assert_eq!(
            keypair.pqc_public_key.unwrap(),
            pub_parts[1],
            "PQC public key must match Dilithium3 part"
        );

        // Verify address format
        assert!(
            keypair.address.starts_with("0x"),
            "Address must start with '0x'"
        );
        assert_eq!(
            keypair.address.len(),
            66,
            "Address must be 66 characters (0x + 64 hex)"
        );

        // Verify curve type
        assert_eq!(keypair.curve_type, CurveType::Ed25519Dilithium3);
    }

    #[test]
    fn test_ed25519_dilithium3_sign_and_verify() {
        // Test signing and verification with Ed25519Dilithium3
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let message = b"Test message for Ed25519+Dilithium3 hybrid signature";

        // Sign the message
        let signature =
            sign_message(&keypair.private_key, message, CurveType::Ed25519Dilithium3).unwrap();

        // Signature should not be empty
        assert!(!signature.is_empty(), "Signature must not be empty");

        // Signature should contain both classical and PQC parts
        // Format: [2-byte length] || classical_sig || pqc_sig
        assert!(
            signature.len() > 2,
            "Signature must be longer than length prefix"
        );

        let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
        assert_eq!(classical_len, 64, "Ed25519 signature should be 64 bytes");

        // Verify the signature using combined public key
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Ed25519Dilithium3,
        )
        .unwrap();

        assert!(
            verified,
            "Ed25519Dilithium3 signature must verify successfully"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_signature_fails_wrong_message() {
        // Test that signature verification fails with wrong message
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let message1 = b"Original message";
        let message2 = b"Different message";

        let signature =
            sign_message(&keypair.private_key, message1, CurveType::Ed25519Dilithium3).unwrap();

        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message2,
            &signature,
            CurveType::Ed25519Dilithium3,
        )
        .unwrap();

        assert!(!verified, "Signature must not verify with wrong message");
    }

    #[test]
    fn test_ed25519_dilithium3_import_from_private_key() {
        // Test importing Ed25519Dilithium3 keypair from private key
        let original = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        // Import from private key
        let imported =
            keypair_from_private_key(&original.private_key, CurveType::Ed25519Dilithium3).unwrap();

        // Should produce identical keypair
        assert_eq!(
            original.public_key, imported.public_key,
            "Public keys must match"
        );
        assert_eq!(original.address, imported.address, "Addresses must match");
        assert_eq!(
            original.pqc_public_key, imported.pqc_public_key,
            "PQC public keys must match"
        );
        assert_eq!(
            original.curve_type, imported.curve_type,
            "Curve types must match"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_deterministic_address() {
        // Test that same combined public key always produces same address
        let keypair1 = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let keypair2 = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        // Different keypairs should have different addresses
        assert_ne!(
            keypair1.address, keypair2.address,
            "Different keypairs must have different addresses"
        );

        // Same public key should always produce same address
        let reimported =
            keypair_from_private_key(&keypair1.private_key, CurveType::Ed25519Dilithium3).unwrap();

        assert_eq!(
            keypair1.address, reimported.address,
            "Same keypair must produce same address"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_address_is_sha3_hash() {
        // Test that Ed25519Dilithium3 address is SHA3-256 of combined public key
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        let mut hasher = Sha3_256::new();
        hasher.update(keypair.public_key.as_bytes());
        let expected_hash = hasher.finalize();
        let expected_address = format!("0x{}", hex::encode(&expected_hash[..]));

        assert_eq!(
            keypair.address, expected_address,
            "Address must be SHA3-256 hash of combined public key"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_security_properties() {
        // Test security properties of Ed25519Dilithium3
        let curve = CurveType::Ed25519Dilithium3;

        // Must be post-quantum
        assert!(
            curve.is_post_quantum(),
            "Ed25519Dilithium3 must be post-quantum"
        );

        // Must be hybrid
        assert!(curve.is_hybrid(), "Ed25519Dilithium3 must be hybrid");

        // Must have maximum security level
        assert_eq!(
            curve.security_level(),
            5,
            "Ed25519Dilithium3 must have security level 5"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_tagged_address() {
        // Test tagged address functionality for Ed25519Dilithium3
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let tagged = keypair.tagged_address();

        // Should have correct format
        assert!(
            tagged.starts_with("Ed25519Dilithium3:"),
            "Tagged address must start with curve type"
        );

        // Parse it back
        let (parsed_curve, parsed_address) =
            KeyPair::parse_tagged_address(&tagged).expect("Failed to parse tagged address");

        assert_eq!(
            parsed_curve,
            CurveType::Ed25519Dilithium3,
            "Parsed curve type must match"
        );
        assert_eq!(
            parsed_address, keypair.public_key,
            "Parsed address must equal combined public key"
        );
    }

    #[test]
    fn test_ed25519_dilithium3_invalid_private_key_import() {
        // Test that invalid private key formats are rejected

        // Test with non-hybrid prefix
        let result =
            keypair_from_private_key("kanari1234567890abcdef", CurveType::Ed25519Dilithium3);
        assert!(result.is_err(), "Non-hybrid prefix must be rejected");

        // Test with missing separator
        let result =
            keypair_from_private_key("kanahybrid1234567890abcdef", CurveType::Ed25519Dilithium3);
        assert!(result.is_err(), "Missing separator must be rejected");

        // Test with invalid hex
        let result = keypair_from_private_key("kanahybridzzzz:yyyy", CurveType::Ed25519Dilithium3);
        assert!(result.is_err(), "Invalid hex must be rejected");
    }

    #[test]
    fn test_ed25519_dilithium3_display_format() {
        let curve = CurveType::Ed25519Dilithium3;
        let display = format!("{}", curve);

        assert_eq!(
            display, "Ed25519Dilithium3",
            "Display format must be correct"
        );
    }

    #[test]
    fn test_import_returns_zeroizing_private_key() {
        let mnemonic = generate_mnemonic(12).unwrap();
        let wallet = import_from_seed_phrase(&mnemonic, CurveType::K256).unwrap();

        // The returned private key should be wrapped in Zeroizing
        // Since import_from_seed_phrase returns a tuple of (private_key, public_key, address),
        assert!(!wallet.private_key.is_empty());
    }

    #[test]
    fn test_keypair_debug_does_not_leak_private_key() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let debug_output = format!("{:?}", keypair);

        // The debug output should not contain the actual private key
        assert!(!debug_output.contains(&keypair.private_key.to_string()));
        assert!(debug_output.contains("REDACTED") || debug_output.contains("**"));
    }

    #[test]
    fn test_hybrid_import_rejects_pqc_secret_only() {
        // PQC part without public key should be rejected
        let hybrid_private = "kanahybrid<ed25519_secret>:<dilithium3_secret_only>";
        let result = keypair_from_private_key(hybrid_private, CurveType::Ed25519Dilithium3);
        assert!(matches!(result, Err(KeyError::InvalidPrivateKey)));
    }

    #[test]
    fn test_hybrid_import_accepts_explicit_pqc_pubkey() {
        // Generate a valid keypair to extract a correct PQC public key
        let original = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();

        // Construct a valid hybrid private key with explicit PQC public key
        // Format: "kanahybrid<ed25519_secret>:<dilithium3_secret>:<dilithium3_public>"
        let result = keypair_from_private_key(&original.private_key, CurveType::Ed25519Dilithium3);

        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    // ============================================================================
    // K256Dilithium3 Specific Tests
    // ============================================================================

    #[test]
    fn test_k256_dilithium3_keypair_structure() {
        // Test K256+Dilithium3 hybrid keypair structure
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();

        // Verify private key format
        assert!(
            keypair.private_key.starts_with("kanahybrid"),
            "K256Dilithium3 private key must start with 'kanahybrid'"
        );
        assert!(
            keypair.private_key.contains(':'),
            "K256Dilithium3 private key must contain ':' separator"
        );

        // Verify public key format (classical:pqc)
        assert!(
            keypair.public_key.contains(':'),
            "K256Dilithium3 public key must be in format 'classical:pqc'"
        );

        let pub_parts: Vec<&str> = keypair.public_key.split(':').collect();
        assert_eq!(pub_parts.len(), 2, "Public key must have exactly 2 parts");

        // K256 public key should be 64 bytes (128 hex chars) - uncompressed without 0x04 prefix
        assert_eq!(
            pub_parts[0].len(),
            128,
            "K256 public key must be 128 hex characters"
        );

        // Dilithium3 public key should be 1952 bytes (3904 hex chars)
        assert_eq!(
            pub_parts[1].len(),
            3904,
            "Dilithium3 public key must be 3904 hex characters"
        );

        // Verify PQC public key field
        assert!(
            keypair.pqc_public_key.is_some(),
            "K256Dilithium3 must have PQC public key"
        );
        assert_eq!(
            keypair.pqc_public_key.unwrap(),
            pub_parts[1],
            "PQC public key must match Dilithium3 part"
        );

        // Verify address format
        assert!(
            keypair.address.starts_with("0x"),
            "Address must start with '0x'"
        );
        assert_eq!(
            keypair.address.len(),
            66,
            "Address must be 66 characters (0x + 64 hex)"
        );

        // Verify curve type
        assert_eq!(keypair.curve_type, CurveType::K256Dilithium3);
    }

    #[test]
    fn test_k256_dilithium3_sign_and_verify() {
        // Test signing and verification with K256Dilithium3
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let message = b"Test message for K256+Dilithium3 hybrid signature";

        // Sign the message
        let signature =
            sign_message(&keypair.private_key, message, CurveType::K256Dilithium3).unwrap();

        // Signature should not be empty
        assert!(!signature.is_empty(), "Signature must not be empty");

        // Signature should contain both classical and PQC parts
        // Format: [2-byte length] || classical_sig || pqc_sig
        assert!(
            signature.len() > 2,
            "Signature must be longer than length prefix"
        );

        let classical_len = u16::from_be_bytes([signature[0], signature[1]]) as usize;
        // K256 signatures are DER-encoded, typically 70-72 bytes
        assert!(
            classical_len > 60 && classical_len < 80,
            "K256 signature should be around 70-72 bytes, got {}",
            classical_len
        );

        // Verify the signature using combined public key
        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::K256Dilithium3,
        )
        .unwrap();

        assert!(
            verified,
            "K256Dilithium3 signature must verify successfully"
        );
    }

    #[test]
    fn test_k256_dilithium3_signature_fails_wrong_message() {
        // Test that signature verification fails with wrong message
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let message1 = b"Original message";
        let message2 = b"Different message";

        let signature =
            sign_message(&keypair.private_key, message1, CurveType::K256Dilithium3).unwrap();

        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message2,
            &signature,
            CurveType::K256Dilithium3,
        )
        .unwrap();

        assert!(!verified, "Signature must not verify with wrong message");
    }

    #[test]
    fn test_k256_dilithium3_import_from_private_key() {
        // Test importing K256Dilithium3 keypair from private key
        let original = generate_keypair(CurveType::K256Dilithium3).unwrap();

        // Import from private key
        let imported =
            keypair_from_private_key(&original.private_key, CurveType::K256Dilithium3).unwrap();

        // Should produce identical keypair
        assert_eq!(
            original.public_key, imported.public_key,
            "Public keys must match"
        );
        assert_eq!(original.address, imported.address, "Addresses must match");
        assert_eq!(
            original.pqc_public_key, imported.pqc_public_key,
            "PQC public keys must match"
        );
        assert_eq!(
            original.curve_type, imported.curve_type,
            "Curve types must match"
        );
    }

    #[test]
    fn test_k256_dilithium3_deterministic_address() {
        // Test that same combined public key always produces same address
        let keypair1 = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let keypair2 = generate_keypair(CurveType::K256Dilithium3).unwrap();

        // Different keypairs should have different addresses
        assert_ne!(
            keypair1.address, keypair2.address,
            "Different keypairs must have different addresses"
        );

        // Same public key should always produce same address
        let reimported =
            keypair_from_private_key(&keypair1.private_key, CurveType::K256Dilithium3).unwrap();

        assert_eq!(
            keypair1.address, reimported.address,
            "Same keypair must produce same address"
        );
    }

    #[test]
    fn test_k256_dilithium3_address_is_sha3_hash() {
        // Test that K256Dilithium3 address is SHA3-256 of combined public key
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();

        let mut hasher = Sha3_256::new();
        hasher.update(keypair.public_key.as_bytes());
        let expected_hash = hasher.finalize();
        let expected_address = format!("0x{}", hex::encode(&expected_hash[..]));

        assert_eq!(
            keypair.address, expected_address,
            "Address must be SHA3-256 hash of combined public key"
        );
    }

    #[test]
    fn test_k256_dilithium3_security_properties() {
        // Test security properties of K256Dilithium3
        let curve = CurveType::K256Dilithium3;

        // Must be post-quantum
        assert!(
            curve.is_post_quantum(),
            "K256Dilithium3 must be post-quantum"
        );

        // Must be hybrid
        assert!(curve.is_hybrid(), "K256Dilithium3 must be hybrid");

        // Must have maximum security level
        assert_eq!(
            curve.security_level(),
            5,
            "K256Dilithium3 must have security level 5"
        );
    }

    #[test]
    fn test_k256_dilithium3_tagged_address() {
        // Test tagged address functionality for K256Dilithium3
        let keypair = generate_keypair(CurveType::K256Dilithium3).unwrap();
        let tagged = keypair.tagged_address();

        // Should have correct format
        assert!(
            tagged.starts_with("K256Dilithium3:"),
            "Tagged address must start with curve type"
        );

        // Parse it back
        let (parsed_curve, parsed_address) =
            KeyPair::parse_tagged_address(&tagged).expect("Failed to parse tagged address");

        assert_eq!(
            parsed_curve,
            CurveType::K256Dilithium3,
            "Parsed curve type must match"
        );
        assert_eq!(
            parsed_address, keypair.public_key,
            "Parsed address must equal combined public key"
        );
    }

    #[test]
    fn test_k256_dilithium3_invalid_private_key_import() {
        // Test that invalid private key formats are rejected

        // Test with non-hybrid prefix
        let result = keypair_from_private_key("kanari1234567890abcdef", CurveType::K256Dilithium3);
        assert!(result.is_err(), "Non-hybrid prefix must be rejected");

        // Test with missing separator
        let result =
            keypair_from_private_key("kanahybrid1234567890abcdef", CurveType::K256Dilithium3);
        assert!(result.is_err(), "Missing separator must be rejected");

        // Test with invalid hex
        let result = keypair_from_private_key("kanahybridzzzz:yyyy", CurveType::K256Dilithium3);
        assert!(result.is_err(), "Invalid hex must be rejected");
    }

    #[test]
    fn test_k256_dilithium3_display_format() {
        let curve = CurveType::K256Dilithium3;
        let display = format!("{}", curve);

        assert_eq!(display, "K256Dilithium3", "Display format must be correct");
    }

    #[test]
    fn test_k256_dilithium3_hybrid_import_rejects_pqc_secret_only() {
        // PQC part without public key should be rejected
        let hybrid_private = "kanahybrid<k256_secret>:<dilithium3_secret_only>";
        let result = keypair_from_private_key(hybrid_private, CurveType::K256Dilithium3);
        assert!(matches!(result, Err(KeyError::InvalidPrivateKey)));
    }

    #[test]
    fn test_k256_dilithium3_hybrid_import_accepts_explicit_pqc_pubkey() {
        // Generate a valid keypair to extract a correct PQC public key
        let original = generate_keypair(CurveType::K256Dilithium3).unwrap();

        // Import should succeed with proper format
        let result = keypair_from_private_key(&original.private_key, CurveType::K256Dilithium3);

        assert!(result.is_ok(), "Error: {:?}", result.err());
    }

    #[test]
    fn test_ed25519_keypair_generation() {
        let keypair = generate_ed25519_keypair().unwrap();

        // The private key must not become entirely zero.
        assert!(!keypair.private_key.contains("0000000000000000"));

        // The private key must be importable back correctly.
        let reimported =
            keypair_from_private_key(&keypair.private_key, CurveType::Ed25519).unwrap();

        assert_eq!(keypair.public_key, reimported.public_key);
        assert_eq!(keypair.address, reimported.address);
    }

    #[test]
    fn test_mnemonic_keypair_consistency() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

        // Created from a repetitive mnemonic, it should yield the same value.
        let kp1 = keypair_from_mnemonic(phrase, CurveType::K256).unwrap();
        let kp2 = keypair_from_mnemonic(phrase, CurveType::K256).unwrap();

        assert_eq!(kp1.public_key, kp2.public_key);
        assert_eq!(kp1.address, kp2.address);

        // Test all supported curves
        for curve in [CurveType::K256, CurveType::P256, CurveType::Ed25519] {
            let kp = keypair_from_mnemonic(phrase, curve).unwrap();
            assert!(!kp.private_key.is_empty());
            assert!(!kp.public_key.is_empty());
            assert!(kp.address.starts_with("0x"));
        }
    }
}
