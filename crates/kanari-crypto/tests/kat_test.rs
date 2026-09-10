// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Known-answer tests (KAT) for deterministic key derivation.
//!
//! These vectors FREEZE the derivation behavior: the same BIP39 test mnemonic
//! must always reproduce byte-identical keys for every curve type. Any change
//! to the KDF, domain separators, seed handling, or provider key expansion
//! will fail these tests loudly.
//!
//! The mnemonics below are well-known public BIP39 test vectors
//! ("abandon ..." family). They MUST NOT be used with real funds; they exist
//! here solely so that derived keys are reproducible reference values.
//!
//! To regenerate the fixture after a DELIBERATE, reviewed derivation change:
//! `KANARI_KAT_REGENERATE=1 cargo test -p kanari-crypto --test kat_test`
//! and commit the resulting fixture alongside the code change.

use bip39::{Language, Mnemonic};
use kanari_crypto::keys::{
    CurveType, keypair_from_mnemonic, keypair_from_private_key, keypair_from_seed,
};
use std::str::FromStr;

const FIXTURE_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/pqc_mnemonic_kat.json"
);

const KAT_MNEMONICS: &[&str] = &[
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
];

const KAT_CURVES: &[CurveType] = &[
    CurveType::K256,
    CurveType::P256,
    CurveType::Ed25519,
    CurveType::Dilithium2,
    CurveType::Dilithium3,
    CurveType::Dilithium5,
    CurveType::SphincsPlusSha256Robust,
    CurveType::Falcon512,
    CurveType::Falcon1024,
    CurveType::Ed25519Dilithium3,
    CurveType::K256Dilithium3,
];

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct KatVector {
    mnemonic: String,
    seed_hex: String,
    curve: String,
    address: String,
    public_key: String,
    private_key: String,
    pqc_public_key: Option<String>,
}

fn seed_hex_for(mnemonic: &str) -> String {
    let m = Mnemonic::parse_in(Language::English, mnemonic).expect("valid test mnemonic");
    hex::encode(m.to_seed(""))
}

fn vector_for(mnemonic: &str, curve: CurveType) -> KatVector {
    let kp = keypair_from_mnemonic(mnemonic, curve)
        .unwrap_or_else(|e| panic!("KAT derivation failed for {curve}: {e}"));
    KatVector {
        mnemonic: mnemonic.to_string(),
        seed_hex: seed_hex_for(mnemonic),
        curve: curve.to_string(),
        address: kp.address.clone(),
        public_key: kp.public_key.clone(),
        private_key: kp.export_private_key_secure().to_string(),
        pqc_public_key: kp.pqc_public_key.clone(),
    }
}

fn check_vector(v: &KatVector) {
    let curve = CurveType::from_str(&v.curve).expect("fixture holds valid curve names");
    let seed = hex::decode(&v.seed_hex).expect("fixture holds valid seed hex");
    assert_eq!(seed.len(), 64, "BIP39 seed must be 64 bytes");

    // Entry point 1: mnemonic derivation reproduces the frozen vector.
    let kp = keypair_from_mnemonic(&v.mnemonic, curve).expect("mnemonic derivation");
    assert_eq!(kp.address, v.address, "address mismatch for {}", v.curve);
    assert_eq!(
        kp.public_key, v.public_key,
        "public key mismatch for {}",
        v.curve
    );
    assert_eq!(
        kp.export_private_key_secure().to_string(),
        v.private_key,
        "private key mismatch for {}",
        v.curve
    );
    assert_eq!(
        kp.pqc_public_key, v.pqc_public_key,
        "pqc key mismatch for {}",
        v.curve
    );

    // Entry point 2: raw-seed derivation (independent seed via bip39 crate
    // in this test target) agrees with mnemonic derivation.
    let kp_seed = keypair_from_seed(&seed, curve).expect("seed derivation");
    assert_eq!(
        kp_seed.address, v.address,
        "seed address mismatch for {}",
        v.curve
    );
    assert_eq!(
        kp_seed.public_key, v.public_key,
        "seed public key mismatch for {}",
        v.curve
    );

    // Entry point 3: private-key import reproduces the frozen vector.
    let kp_imported = keypair_from_private_key(&v.private_key, curve).expect("private-key import");
    assert_eq!(
        kp_imported.address, v.address,
        "import address mismatch for {}",
        v.curve
    );
    assert_eq!(
        kp_imported.public_key, v.public_key,
        "import public key mismatch for {}",
        v.curve
    );
}

#[test]
fn kat_mnemonic_vectors_frozen() {
    if std::env::var("KANARI_KAT_REGENERATE").is_ok() {
        let mut vectors = Vec::new();
        for mnemonic in KAT_MNEMONICS {
            for curve in KAT_CURVES {
                vectors.push(vector_for(mnemonic, *curve));
            }
        }
        let json = serde_json::to_string_pretty(&vectors).expect("serialize KAT");
        std::fs::write(FIXTURE_PATH, format!("{json}\n")).expect("write KAT fixture");
        return;
    }

    let raw = std::fs::read_to_string(FIXTURE_PATH).expect("read KAT fixture");
    let vectors: Vec<KatVector> = serde_json::from_str(&raw).expect("parse KAT fixture");
    assert_eq!(
        vectors.len(),
        KAT_MNEMONICS.len() * KAT_CURVES.len(),
        "fixture vector count changed"
    );
    for v in &vectors {
        check_vector(v);
    }
}
