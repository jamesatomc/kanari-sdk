// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#[cfg(test)]
mod tests {
    use kanari_crypto::keys::{AlgorithmFamily, CurveType, UsageProfile};
    use std::str::FromStr;

    #[test]
    fn every_supported_curve_has_complete_metadata() {
        assert_eq!(CurveType::ALL.len(), 11);

        for curve in CurveType::ALL {
            let metadata = curve.metadata();
            assert_eq!(metadata.curve_type, *curve);
            assert_eq!(metadata.family, curve.family());
            assert_eq!(metadata.hybrid, curve.is_hybrid());
            assert_eq!(metadata.quantum_safe, curve.is_post_quantum());
            assert_eq!(
                metadata.hd_wallet_derivation,
                curve.supports_hd_wallet_derivation()
            );
            assert!(metadata.signature_size_hint.unwrap_or_default() > 0);
            assert!(metadata.public_key_size_hint.unwrap_or_default() > 0);
            assert_eq!(metadata.nist_level, curve.nist_level());
        }
    }

    #[test]
    fn display_and_parse_round_trip_for_every_curve() {
        for curve in CurveType::ALL {
            let encoded = curve.to_string();
            let parsed = CurveType::from_str(&encoded).expect("curve display must parse");
            assert_eq!(parsed, *curve);
        }
    }

    #[test]
    fn family_and_hd_policy_are_consistent() {
        for curve in CurveType::ALL {
            match curve.family() {
                AlgorithmFamily::Classical => {
                    assert!(curve.is_classical());
                    assert!(curve.supports_hd_wallet_derivation());
                }
                AlgorithmFamily::PostQuantum => {
                    assert!(curve.is_post_quantum());
                    assert!(!curve.is_hybrid());
                    assert!(!curve.supports_hd_wallet_derivation());
                }
                AlgorithmFamily::Hybrid => {
                    assert!(curve.is_post_quantum());
                    assert!(curve.is_hybrid());
                    assert!(!curve.supports_hd_wallet_derivation());
                }
            }
        }
    }

    #[test]
    fn production_defaults_are_deliberate() {
        assert_eq!(CurveType::DEFAULT_PRODUCTION, CurveType::Ed25519Dilithium3);
        assert_eq!(CurveType::DEFAULT_HOT_WALLET, CurveType::Ed25519);
        assert_eq!(
            CurveType::DEFAULT_COLD_STORAGE,
            CurveType::SphincsPlusSha256Robust
        );

        assert!(CurveType::DEFAULT_PRODUCTION.is_recommended_for_new_accounts());
        assert!(CurveType::DEFAULT_HOT_WALLET.is_recommended_for_new_accounts());
        assert!(CurveType::DEFAULT_COLD_STORAGE.is_recommended_for_new_accounts());
    }

    #[test]
    fn usage_profiles_match_operational_expectations() {
        assert_eq!(
            CurveType::K256.usage_profile(),
            UsageProfile::Interoperability
        );
        assert_eq!(
            CurveType::P256.usage_profile(),
            UsageProfile::Interoperability
        );
        assert_eq!(CurveType::Ed25519.usage_profile(), UsageProfile::HotWallet);
        assert_eq!(
            CurveType::Dilithium3.usage_profile(),
            UsageProfile::GeneralPurpose
        );
        assert_eq!(
            CurveType::SphincsPlusSha256Robust.usage_profile(),
            UsageProfile::ColdStorage
        );
        assert_eq!(
            CurveType::Dilithium2.usage_profile(),
            UsageProfile::Specialized
        );
    }

    #[test]
    fn hybrid_size_hints_include_classical_prefix_and_pqc_signature() {
        let ed_hybrid = CurveType::Ed25519Dilithium3;
        let k_hybrid = CurveType::K256Dilithium3;

        assert_eq!(
            ed_hybrid.signature_size_hint(),
            Some(2 + CurveType::Ed25519.signature_size_hint().unwrap() + 3_309)
        );
        assert_eq!(
            k_hybrid.signature_size_hint(),
            Some(2 + CurveType::K256.signature_size_hint().unwrap() + 3_309)
        );
        assert!(
            ed_hybrid.public_key_size_hint().unwrap()
                > CurveType::Ed25519.public_key_size_hint().unwrap()
        );
        assert!(
            k_hybrid.public_key_size_hint().unwrap()
                > CurveType::K256.public_key_size_hint().unwrap()
        );
    }
}
