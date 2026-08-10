// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_crypto::{
    BatchVerificationItem, CurveType, SignatureError, generate_keypair,
    signatures::{sign_message, verify_batch_with_curve, verify_signature_with_curve},
};

fn assert_rejects_or_returns_false(
    public_key: &str,
    message: &[u8],
    signature: &[u8],
    curve: CurveType,
) {
    assert!(matches!(
        verify_signature_with_curve(public_key, message, signature, curve),
        Err(SignatureError::InvalidFormat(_))
            | Err(SignatureError::InvalidSignatureLength)
            | Ok(false)
    ));
}

#[test]
fn wycheproof_style_ecdsa_der_edge_cases_fail_closed() {
    let edge_cases: &[&[u8]] = &[
        b"",
        &[0x30],
        &[0x30, 0x00],
        &[0x30, 0x06, 0x02, 0x01, 0x01, 0x02],
        &[0x30, 0x06, 0x02, 0x01, 0x00, 0x02, 0x01, 0x00],
        &[0x30, 0x08, 0x02, 0x02, 0x00, 0x00, 0x02, 0x02, 0x00, 0x01],
        &[0x30, 0x08, 0x02, 0x02, 0x80, 0x00, 0x02, 0x02, 0x00, 0x01],
        &[0x30, 0x81, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x01],
    ];

    for curve in [CurveType::K256, CurveType::P256] {
        let keypair = generate_keypair(curve).unwrap();
        let message = b"wycheproof style ecdsa der policy";

        for signature in edge_cases {
            assert_rejects_or_returns_false(&keypair.public_key, message, signature, curve);
        }
    }
}

#[test]
fn k256_and_p256_parallel_batch_verify_valid_and_reject_invalid_items() {
    for curve in [CurveType::K256, CurveType::P256] {
        let keypairs = [
            generate_keypair(curve).unwrap(),
            generate_keypair(curve).unwrap(),
            generate_keypair(curve).unwrap(),
            generate_keypair(curve).unwrap(),
        ];
        let messages: [&[u8]; 4] = [
            b"ecdsa-batch-0",
            b"ecdsa-batch-1",
            b"ecdsa-batch-2",
            b"ecdsa-batch-3",
        ];
        let signatures: Vec<Vec<u8>> = keypairs
            .iter()
            .zip(messages)
            .map(|(keypair, message)| sign_message(&keypair.private_key, message, curve).unwrap())
            .collect();

        let valid_items: Vec<BatchVerificationItem<'_>> = keypairs
            .iter()
            .zip(messages)
            .zip(signatures.iter())
            .map(|((keypair, message), signature)| {
                BatchVerificationItem::new(&keypair.public_key, message, signature)
            })
            .collect();

        assert!(verify_batch_with_curve(&valid_items, curve).unwrap());

        let wrong_message_items: Vec<BatchVerificationItem<'_>> = keypairs
            .iter()
            .zip(signatures.iter())
            .map(|(keypair, signature)| {
                BatchVerificationItem::new(&keypair.public_key, b"wrong message", signature)
            })
            .collect();

        assert!(!verify_batch_with_curve(&wrong_message_items, curve).unwrap());
    }
}

#[test]
fn batch_verification_rejects_resource_abuse_before_curve_parsing() {
    let keypair = generate_keypair(CurveType::Ed25519).unwrap();
    let message = b"small message";
    let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();

    let huge_public_key = "a".repeat(9 * 1024);
    let huge_message = vec![0u8; 16 * 1024 * 1024 + 1];
    let huge_signature = vec![0u8; 64 * 1024 + 1];

    for item in [
        BatchVerificationItem::new(&huge_public_key, message, &signature),
        BatchVerificationItem::new(&keypair.public_key, &huge_message, &signature),
        BatchVerificationItem::new(&keypair.public_key, message, &huge_signature),
    ] {
        assert!(verify_batch_with_curve(&[item], CurveType::Ed25519).is_err());
    }
}

#[test]
fn direct_verification_rejects_oversized_inputs_before_curve_parsing() {
    let keypair = generate_keypair(CurveType::Ed25519).unwrap();
    let message = b"small message";
    let signature = sign_message(&keypair.private_key, message, CurveType::Ed25519).unwrap();
    let huge_public_key = "a".repeat(9 * 1024);
    let huge_message = vec![0u8; 16 * 1024 * 1024 + 1];

    assert!(
        verify_signature_with_curve(&huge_public_key, message, &signature, CurveType::Ed25519)
            .is_err()
    );
    assert!(
        verify_signature_with_curve(
            &keypair.public_key,
            &huge_message,
            &signature,
            CurveType::Ed25519,
        )
        .is_err()
    );
}
