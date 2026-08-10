// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#![no_main]

use kanari_crypto::{
    CurveType, generate_keypair, keypair_from_mnemonic, keypair_from_private_key,
    keys::{extract_raw_key, format_private_key},
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 2 {
        return;
    }

    let curve_indicator = data[0];
    let key_data = &data[1..];

    // Test key generation for different curves
    let curve_type = match curve_indicator % 9 {
        0 => CurveType::K256,
        1 => CurveType::P256,
        2 => CurveType::Ed25519,
        3 => CurveType::Dilithium2,
        4 => CurveType::Dilithium3,
        5 => CurveType::Dilithium5,
        6 => CurveType::SphincsPlusSha256Robust,
        7 => CurveType::Ed25519Dilithium3,
        8 => CurveType::K256Dilithium3,
        _ => return,
    };

    // Generate keypair - should not panic
    if let Ok(keypair) = generate_keypair(curve_type) {
        // Verify address format
        assert!(
            keypair.address.starts_with("0x"),
            "Address should start with 0x"
        );

        // Verify public key is not empty
        assert!(
            !keypair.public_key.is_empty(),
            "Public key should not be empty"
        );

        // Verify tagged address format
        let tagged = keypair.tagged_address();
        assert!(
            tagged.contains(':'),
            "Tagged address should contain ':' separator"
        );

        // Parse tagged address back
        if let Some((parsed_curve, parsed_addr)) =
            kanari_crypto::keys::KeyPair::parse_tagged_address(&tagged)
        {
            assert_eq!(parsed_curve, curve_type, "Parsed curve should match");
            assert!(
                !parsed_addr.is_empty(),
                "Parsed address should not be empty"
            );
        } else {
            panic!("Tagged address should be parseable");
        }

        // Test private key formatting
        let raw_key = extract_raw_key(&keypair.private_key);
        let formatted = format_private_key(raw_key);

        // Should be able to extract again
        let re_extracted = extract_raw_key(&formatted);
        assert_eq!(re_extracted, raw_key, "Re-extracted key should match");
    }

    // Test mnemonic import (only for classical curves)
    if matches!(
        curve_type,
        CurveType::K256 | CurveType::P256 | CurveType::Ed25519
    ) {
        // Try to parse key_data as UTF-8 for mnemonic testing
        if let Ok(mnemonic_str) = std::str::from_utf8(key_data) {
            // Only test with valid-looking mnemonics (avoid panics from invalid formats)
            if mnemonic_str.split_whitespace().count() == 12
                || mnemonic_str.split_whitespace().count() == 24
            {
                let result = keypair_from_mnemonic(mnemonic_str, curve_type);

                // If mnemonic is valid, should succeed
                if result.is_ok() {
                    let kp = result.unwrap();
                    assert!(
                        kp.address.starts_with("0x"),
                        "Mnemonic-derived address should be valid"
                    );
                }
                // Invalid mnemonics may fail - that's acceptable
            }
        }
    }

    // Test private key import
    if let Ok(pk_str) = std::str::from_utf8(key_data) {
        // Skip if contains invalid characters for hex
        if pk_str.chars().all(|c| c.is_ascii_hexdigit() || c == '_') && !pk_str.is_empty() {
            let result = keypair_from_private_key(pk_str, curve_type);

            // May fail for invalid keys - that's acceptable
            if result.is_ok() {
                let kp = result.unwrap();
                assert!(
                    kp.address.starts_with("0x"),
                    "Imported key address should be valid"
                );
            }
        }
    }
});
