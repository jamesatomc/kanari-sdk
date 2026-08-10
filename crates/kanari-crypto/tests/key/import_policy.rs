// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_crypto::keys::{
    CurveType, KANAHYBRID_PREFIX, KANAMLDSA_PREFIX, KANAPQC_PREFIX, KeyError, extract_raw_key,
    generate_keypair, keypair_from_private_key,
};
use proptest::prelude::*;

const IMPORT_POLICY_CASES: u32 = 512;

fn import_policy_config() -> proptest::test_runner::Config {
    proptest::test_runner::Config {
        cases: std::env::var("KANARI_CRYPTO_PROPTEST_CASES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(IMPORT_POLICY_CASES),
        ..proptest::test_runner::Config::default()
    }
}

#[test]
fn import_rejects_pqc_secret_without_public_key_for_all_pqc_curves() {
    for curve in [
        CurveType::Dilithium2,
        CurveType::Dilithium3,
        CurveType::Dilithium5,
        CurveType::SphincsPlusSha256Robust,
    ] {
        let secret_only = format!("{}deadbeef", KANAPQC_PREFIX);
        assert!(matches!(
            keypair_from_private_key(&secret_only, curve),
            Err(KeyError::InvalidPrivateKey)
        ));
    }
}

#[test]
fn import_preserves_generated_pqc_and_hybrid_keypairs() {
    let curves = vec![
        CurveType::Dilithium2,
        CurveType::Dilithium3,
        CurveType::Dilithium5,
        CurveType::Ed25519Dilithium3,
        CurveType::K256Dilithium3,
        #[cfg(feature = "slh-dsa")]
        CurveType::SphincsPlusSha256Robust,
    ];

    for curve in curves {
        let original = generate_keypair(curve).unwrap();
        let imported = keypair_from_private_key(&original.private_key, curve).unwrap();

        assert_eq!(imported.public_key, original.public_key);
        assert_eq!(imported.address, original.address);
        assert_eq!(imported.pqc_public_key, original.pqc_public_key);
    }
}

#[test]
fn import_rejects_mldsa_keys_with_invalid_seed_or_public_lengths() {
    let valid = generate_keypair(CurveType::Dilithium3).unwrap();
    let raw = valid.private_key.trim_start_matches(KANAMLDSA_PREFIX);
    let (_seed, public_key) = raw.split_once(':').unwrap();

    let short_seed = format!("{}{}:{}", KANAMLDSA_PREFIX, "aa", public_key);
    assert!(matches!(
        keypair_from_private_key(&short_seed, CurveType::Dilithium3),
        Err(KeyError::InvalidPrivateKey)
    ));

    let wrong_public = format!(
        "{}{}:{}",
        KANAMLDSA_PREFIX,
        "aa".repeat(32),
        "bb".repeat(32)
    );
    assert!(matches!(
        keypair_from_private_key(&wrong_public, CurveType::Dilithium3),
        Err(KeyError::InvalidPrivateKey)
    ));

    let expanded_secret = format!("{}{}:{}", KANAMLDSA_PREFIX, "aa".repeat(4032), public_key);
    assert!(matches!(
        keypair_from_private_key(&expanded_secret, CurveType::Dilithium3),
        Err(KeyError::InvalidPrivateKey)
    ));
}

#[test]
fn import_rejects_hybrid_keys_with_invalid_mldsa_component_lengths() {
    let valid = generate_keypair(CurveType::Ed25519Dilithium3).unwrap();
    let raw = valid.private_key.trim_start_matches(KANAHYBRID_PREFIX);
    let (classical, pqc_part) = raw.split_once(':').unwrap();
    let (_pqc_seed, pqc_public) = pqc_part.split_once(':').unwrap();

    let malformed = format!("{}{}:{}:{}", KANAHYBRID_PREFIX, classical, "aa", pqc_public);
    assert!(matches!(
        keypair_from_private_key(&malformed, CurveType::Ed25519Dilithium3),
        Err(KeyError::InvalidPrivateKey)
    ));

    let expanded_pqc = format!(
        "{}{}:{}:{}",
        KANAHYBRID_PREFIX,
        classical,
        "aa".repeat(4032),
        pqc_public
    );
    assert!(matches!(
        keypair_from_private_key(&expanded_pqc, CurveType::Ed25519Dilithium3),
        Err(KeyError::InvalidPrivateKey)
    ));
}

proptest! {
    #![proptest_config(import_policy_config())]

    #[test]
    fn malformed_classical_private_keys_never_panic(input in ".{0,256}") {
        for curve in [CurveType::K256, CurveType::P256, CurveType::Ed25519] {
            let result = keypair_from_private_key(&input, curve);
            if result.is_ok() {
                prop_assert!(input.starts_with("kanari") || hex::decode(extract_raw_key(&input)).is_ok());
            }
        }
    }

    #[test]
    fn malformed_pqc_imports_fail_closed(secret in "[0-9a-f]{0,128}", public in ".{0,128}") {
        let key = format!("{}{}:{}", KANAPQC_PREFIX, secret, public);
        let result = keypair_from_private_key(&key, CurveType::Dilithium3);

        if result.is_err() {
            prop_assert!(matches!(result, Err(KeyError::InvalidPrivateKey)));
        }
    }

    #[test]
    fn malformed_hybrid_imports_fail_closed(classical in ".{0,128}", pqc in ".{0,128}") {
        let key = format!("{}{}:{}", KANAHYBRID_PREFIX, classical, pqc);
        let result = keypair_from_private_key(&key, CurveType::Ed25519Dilithium3);

        if result.is_err() {
            prop_assert!(matches!(result, Err(KeyError::InvalidPrivateKey)));
        }
    }

    #[test]
    fn prefixed_imports_with_random_separator_patterns_fail_closed(
        prefix in prop::sample::select(vec![KANAPQC_PREFIX, KANAHYBRID_PREFIX]),
        parts in prop::collection::vec(".{0,96}", 0..5),
    ) {
        let key = format!("{}{}", prefix, parts.join(":"));
        for curve in [
            CurveType::Dilithium2,
            CurveType::Dilithium3,
            CurveType::Dilithium5,
            CurveType::SphincsPlusSha256Robust,
            CurveType::Ed25519Dilithium3,
            CurveType::K256Dilithium3,
        ] {
            let result = keypair_from_private_key(&key, curve);
            if result.is_err() {
                prop_assert!(matches!(result, Err(KeyError::InvalidPrivateKey)));
            }
        }
    }
}
