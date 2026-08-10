// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod tests {
    use kanari_crypto::{
        BatchVerificationItem, CryptoSigner, CryptoVerifier, CurveType, PublicKeyVerifier,
        TaggedAddressVerifier, UsageProfile, all_algorithm_metadata, generate_keypair,
        recommended_curves_for_usage, signatures::verify_signature_with_keypair,
        verify_batch_tagged, verify_batch_with_curve,
    };

    #[test]
    fn keypair_implements_signer_and_verifier_traits() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"trait-api-roundtrip";

        let signature = keypair.sign(message).unwrap();
        assert!(keypair.verify(message, &signature).unwrap());
        assert!(!keypair.verify(b"wrong message", &signature).unwrap());

        assert_eq!(
            CryptoSigner::metadata(&keypair).curve_type,
            CurveType::Ed25519
        );
        assert_eq!(
            CryptoVerifier::metadata(&keypair).usage_profile,
            UsageProfile::HotWallet
        );
    }

    #[test]
    fn public_key_verifier_preserves_existing_explicit_curve_api() {
        let keypair = generate_keypair(CurveType::K256).unwrap();
        let message = b"explicit-public-key-verifier";
        let signature = keypair.sign(message).unwrap();

        let verifier = PublicKeyVerifier::new(&keypair.public_key, CurveType::K256);
        assert!(verifier.verify(message, &signature).unwrap());
        assert!(!verifier.verify(b"wrong message", &signature).unwrap());
    }

    #[test]
    fn tagged_address_verifier_preserves_fail_closed_policy() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let message = b"tagged-address-verifier";
        let signature = keypair.sign(message).unwrap();
        let tagged_address = keypair.tagged_address();

        let verifier = TaggedAddressVerifier::new(&tagged_address);
        assert_eq!(verifier.curve_type(), CurveType::Ed25519);
        assert!(verifier.verify(message, &signature).unwrap());

        let untagged = TaggedAddressVerifier::new(&keypair.public_key);
        assert!(untagged.verify(message, &signature).is_err());
    }

    #[test]
    fn algorithm_metadata_helpers_are_complete_and_stable() {
        let metadata = all_algorithm_metadata();
        assert_eq!(metadata.len(), CurveType::ALL.len());
        assert_eq!(metadata[0].curve_type, CurveType::K256);
        assert_eq!(
            metadata.last().unwrap().curve_type,
            CurveType::K256Dilithium3
        );
        assert!(
            metadata
                .iter()
                .all(|entry| entry.signature_size_hint.is_some())
        );
    }

    #[test]
    fn recommended_curves_are_policy_filtered() {
        let hot = recommended_curves_for_usage(UsageProfile::HotWallet);
        assert_eq!(hot, vec![CurveType::Ed25519]);

        let cold = recommended_curves_for_usage(UsageProfile::ColdStorage);
        assert_eq!(cold, vec![CurveType::SphincsPlusSha256Robust]);

        let interop = recommended_curves_for_usage(UsageProfile::Interoperability);
        assert!(
            interop.is_empty(),
            "interop curves require app-specific choice"
        );
    }

    #[test]
    fn batch_verification_is_fail_closed_and_matches_single_verify() {
        let keypairs = [
            generate_keypair(CurveType::Ed25519).unwrap(),
            generate_keypair(CurveType::Ed25519).unwrap(),
            generate_keypair(CurveType::Ed25519).unwrap(),
        ];
        let messages: [&[u8]; 3] = [b"batch-0", b"batch-1", b"batch-2"];
        let signatures: Vec<Vec<u8>> = keypairs
            .iter()
            .zip(messages)
            .map(|(keypair, message)| keypair.sign(message).unwrap())
            .collect();

        let items: Vec<BatchVerificationItem<'_>> = keypairs
            .iter()
            .zip(messages)
            .zip(signatures.iter())
            .map(|((keypair, message), signature)| {
                BatchVerificationItem::new(&keypair.public_key, message, signature)
            })
            .collect();
        assert!(verify_batch_with_curve(&items, CurveType::Ed25519).unwrap());
        assert!(verify_batch_with_curve(&[], CurveType::Ed25519).is_err());

        let tagged_addresses: Vec<String> = keypairs
            .iter()
            .map(|keypair| keypair.tagged_address())
            .collect();
        let tagged_items: Vec<BatchVerificationItem<'_>> = tagged_addresses
            .iter()
            .zip(messages)
            .zip(signatures.iter())
            .map(|((tagged_address, message), signature)| {
                BatchVerificationItem::new(tagged_address, message, signature)
            })
            .collect();
        assert!(verify_batch_tagged(&tagged_items).unwrap());

        let mut bad_signature = signatures[0].clone();
        bad_signature[0] ^= 0x80;
        let bad_items = [BatchVerificationItem::new(
            &keypairs[0].public_key,
            messages[0],
            &bad_signature,
        )];
        assert!(!verify_batch_with_curve(&bad_items, CurveType::Ed25519).unwrap());
    }

    #[test]
    fn keypair_verifier_rejects_resource_abuse_before_curve_parsing() {
        let keypair = generate_keypair(CurveType::Ed25519).unwrap();
        let signature = keypair.sign(b"small message").unwrap();
        let huge_message = vec![0u8; 16 * 1024 * 1024 + 1];
        let huge_signature = vec![0u8; 64 * 1024 + 1];

        assert!(verify_signature_with_keypair(&keypair, &huge_message, &signature).is_err());
        assert!(
            verify_signature_with_keypair(&keypair, b"small message", &huge_signature).is_err()
        );
    }
}
