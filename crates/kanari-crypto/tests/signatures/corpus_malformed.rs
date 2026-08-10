// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_crypto::{
    CurveType, SignatureError, generate_keypair,
    signatures::{sign_message, verify_signature, verify_signature_with_curve},
};

#[test]
fn seeded_malformed_tagged_address_corpus_fails_closed() {
    let signatures = [vec![], vec![0], vec![1; 64], vec![2; 1024]];
    let corpus = [
        "",
        ":",
        "K256",
        "K256:",
        "K256:0x",
        "K256:0xzz",
        "Unknown:0x0",
        "SphincsPlusSha256Robust:",
        "SphincsPlusSha256Robust:0xzz",
        "Ed25519Dilithium3:0x0:extra",
    ];

    for address in corpus {
        for signature in &signatures {
            let result = verify_signature(address, b"message", signature);
            if let Ok(ok) = result {
                assert!(!ok);
            }
        }
    }
}

#[test]
fn oversized_signatures_are_rejected_before_curve_specific_parsing() {
    let oversized_signature = vec![0u8; 64 * 1024 + 1];

    let curves = vec![
        CurveType::K256,
        CurveType::P256,
        CurveType::Ed25519,
        CurveType::Dilithium2,
        CurveType::Dilithium3,
        CurveType::Dilithium5,
        CurveType::Ed25519Dilithium3,
        CurveType::K256Dilithium3,
        #[cfg(feature = "slh-dsa")]
        CurveType::SphincsPlusSha256Robust,
    ];

    for curve in curves {
        let keypair = generate_keypair(curve).unwrap();
        assert!(matches!(
            verify_signature(&keypair.tagged_address(), b"message", &oversized_signature),
            Err(SignatureError::InvalidFormat(_))
        ));
    }
}

#[test]
fn seeded_hybrid_signature_truncations_do_not_verify() {
    for curve in [CurveType::Ed25519Dilithium3, CurveType::K256Dilithium3] {
        let keypair = generate_keypair(curve).unwrap();
        let message = b"hybrid truncation corpus";
        let signature = sign_message(&keypair.private_key, message, curve).unwrap();

        for len in [
            0,
            1,
            2,
            3,
            4,
            8,
            16,
            32,
            64,
            signature.len().saturating_sub(1),
        ] {
            let truncated = &signature[..len.min(signature.len())];
            let result =
                verify_signature_with_curve(&keypair.public_key, message, truncated, curve);
            if let Ok(ok) = result {
                assert!(!ok);
            }
        }
    }
}
