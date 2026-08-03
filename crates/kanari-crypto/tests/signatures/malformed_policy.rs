use kanari_crypto::{
    CurveType, SignatureError, generate_keypair,
    signatures::{sign_message, verify_signature, verify_signature_with_curve},
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
        #[cfg(feature = "experimental-slh-dsa")]
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
