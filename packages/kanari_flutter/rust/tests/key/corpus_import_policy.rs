use rust::keys::{
    CurveType, KANAHYBRID_PREFIX, KANAPQC_PREFIX, KeyError, keypair_from_private_key,
};

#[test]
fn seeded_malformed_private_key_corpus_fails_closed() {
    let corpus = [
        "",
        "kanari",
        "kanari00",
        "kanari::",
        "kanapqc",
        "kanapqc:",
        "kanapqcdeadbeef:",
        "kanapqczz:00",
        "kanapqc00:zz",
        "kanapqc00:11:22",
        "kanahybrid",
        "kanahybrid:",
        "kanahybrid00:",
        "kanahybrid:00",
        "kanahybrid00:11",
        "kanahybridzz:11:22",
        "kanahybrid00:11:22:33",
        "K256:0x0",
        "SphincsPlusSha256Robust:0x0",
        "0x",
        "0xzz",
    ];

    for input in corpus {
        for curve in [
            CurveType::K256,
            CurveType::P256,
            CurveType::Ed25519,
            CurveType::Dilithium2,
            CurveType::Dilithium3,
            CurveType::Dilithium5,
            CurveType::SphincsPlusSha256Robust,
            CurveType::Ed25519Dilithium3,
            CurveType::K256Dilithium3,
        ] {
            if let Err(err) = keypair_from_private_key(input, curve) {
                assert!(matches!(err, KeyError::InvalidPrivateKey));
            }
        }
    }
}

#[test]
fn seeded_oversized_pqc_and_hybrid_imports_fail_closed() {
    let oversized_hex = "aa".repeat(80_000);
    let cases = [
        format!("{}{}", KANAPQC_PREFIX, oversized_hex),
        format!("{}{}:{}", KANAPQC_PREFIX, oversized_hex, "bb".repeat(64)),
        format!("{}{}:{}", KANAHYBRID_PREFIX, "cc".repeat(32), oversized_hex),
    ];

    for input in cases {
        for curve in [
            CurveType::Dilithium3,
            CurveType::SphincsPlusSha256Robust,
            CurveType::Ed25519Dilithium3,
            CurveType::K256Dilithium3,
        ] {
            assert!(matches!(
                keypair_from_private_key(&input, curve),
                Err(KeyError::InvalidPrivateKey)
            ));
        }
    }
}
