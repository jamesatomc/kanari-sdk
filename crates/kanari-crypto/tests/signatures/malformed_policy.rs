// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_crypto::{
    CurveType, SignatureError, generate_keypair,
    signatures::{
        BatchVerificationItem, sign_message, verify_batch_with_curve, verify_signature,
        verify_signature_with_curve,
    },
};
use proptest::prelude::*;

const SIGNATURE_POLICY_CASES: u32 = 512;

fn signature_policy_config() -> proptest::test_runner::Config {
    proptest::test_runner::Config {
        cases: std::env::var("KANARI_CRYPTO_PROPTEST_CASES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(SIGNATURE_POLICY_CASES),
        ..proptest::test_runner::Config::default()
    }
}

#[test]
fn untagged_verification_fails_closed_for_all_signature_curves() {
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
        let signature = sign_message(&keypair.private_key, b"message", curve).unwrap();
        assert!(matches!(
            verify_signature(&keypair.address, b"message", &signature),
            Err(SignatureError::InvalidFormat(_))
        ));
    }
}

#[test]
fn ed25519_true_batch_rejects_mixed_valid_and_invalid_signature() {
    let keypairs = [
        generate_keypair(CurveType::Ed25519).unwrap(),
        generate_keypair(CurveType::Ed25519).unwrap(),
        generate_keypair(CurveType::Ed25519).unwrap(),
    ];
    let messages: [&[u8]; 3] = [b"ed-batch-0", b"ed-batch-1", b"ed-batch-2"];
    let mut signatures: Vec<Vec<u8>> = keypairs
        .iter()
        .zip(messages)
        .map(|(keypair, message)| {
            sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap()
        })
        .collect();

    signatures[1][7] ^= 0x40;

    let items: Vec<BatchVerificationItem<'_>> = keypairs
        .iter()
        .zip(messages)
        .zip(signatures.iter())
        .map(|((keypair, message), signature)| {
            BatchVerificationItem::new(&keypair.public_key, message, signature)
        })
        .collect();
    assert!(!verify_batch_with_curve(&items, CurveType::Ed25519).unwrap());
}

#[test]
fn ecdsa_vector_style_malformed_signatures_fail_closed() {
    for curve in [CurveType::K256, CurveType::P256] {
        let keypair = generate_keypair(curve).unwrap();
        let message = b"ecdsa malformed vector";
        let signature = sign_message(&keypair.private_key, message, curve).unwrap();

        let truncated = &signature[..signature.len().saturating_sub(1)];
        assert!(matches!(
            verify_signature_with_curve(&keypair.public_key, message, truncated, curve),
            Err(SignatureError::InvalidFormat(_)) | Ok(false)
        ));

        let mut mutated = signature.clone();
        mutated[0] ^= 0x80;
        assert!(matches!(
            verify_signature_with_curve(&keypair.public_key, message, &mutated, curve),
            Err(SignatureError::InvalidFormat(_)) | Ok(false)
        ));

        assert!(
            !verify_signature_with_curve(&keypair.public_key, b"wrong message", &signature, curve)
                .unwrap()
        );
    }
}

proptest! {
    #![proptest_config(signature_policy_config())]

    #[test]
    fn malformed_tagged_addresses_do_not_verify(tag in ".{0,32}", addr in ".{0,256}", sig in prop::collection::vec(any::<u8>(), 0..512)) {
        let tagged = format!("{}:{}", tag, addr);
        let result = verify_signature(&tagged, b"message", &sig);

        if result.is_ok() {
            prop_assert!(!result.unwrap());
        }
    }

    #[test]
    fn hybrid_signature_mutations_do_not_verify(flip_index in 0usize..512usize) {
        let keypair = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
        let message = b"hybrid mutation policy";
        let mut signature = sign_message(&keypair.private_key, message, CurveType::Ed25519Dilithium3).unwrap();

        if !signature.is_empty() {
            let index = flip_index % signature.len();
            signature[index] = signature[index].wrapping_add(1);
        }

        let verified = verify_signature_with_curve(
            &keypair.public_key,
            message,
            &signature,
            CurveType::Ed25519Dilithium3,
        );

        if let Ok(ok) = verified {
            prop_assert!(!ok);
        }
    }
}
