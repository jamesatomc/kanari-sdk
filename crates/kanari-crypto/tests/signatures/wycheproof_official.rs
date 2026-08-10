// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Official Wycheproof corpus regression tests.
//!
//! These tests intentionally separate Kanari's account-signature semantics from
//! upstream test-vector semantics:
//!
//! - Ed25519 vectors are exercised through the public Kanari verification API
//!   because Kanari Ed25519 signs/verifies raw RFC-8032 messages.
//! - ECDSA vectors are exercised against the underlying k256/p256 SHA-256
//!   verifiers because Wycheproof's ECDSA SHA-256 files are not Kanari account
//!   K256/P256 signatures. Kanari account K256/P256 uses SHA3-256 prehashing.

use kanari_crypto::{CurveType, signatures::verify_signature_with_curve};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WycheproofFile {
    algorithm: String,
    number_of_tests: usize,
    test_groups: Vec<TestGroup>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestGroup {
    public_key: PublicKey,
    tests: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicKey {
    #[serde(default)]
    pk: String,
    #[serde(default)]
    uncompressed: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestCase {
    tc_id: u64,
    msg: String,
    sig: String,
    result: String,
}

#[test]
fn official_wycheproof_ed25519_corpus_matches_kanari_policy() {
    let corpus: WycheproofFile =
        serde_json::from_str(include_str!("../fixtures/wycheproof/ed25519_test.json")).unwrap();
    assert_eq!(corpus.algorithm, "EDDSA");
    assert_eq!(count_tests(&corpus), corpus.number_of_tests);

    let mut valid = 0usize;
    let mut invalid = 0usize;
    let mut acceptable = 0usize;

    for group in &corpus.test_groups {
        assert!(!group.public_key.pk.is_empty());
        for test in &group.tests {
            let msg = decode_hex(&test.msg, test.tc_id);
            let sig = decode_hex(&test.sig, test.tc_id);
            let verified =
                verify_signature_with_curve(&group.public_key.pk, &msg, &sig, CurveType::Ed25519)
                    .unwrap_or(false);

            match test.result.as_str() {
                "valid" => {
                    valid += 1;
                    assert!(
                        verified,
                        "Wycheproof Ed25519 tcId {} must verify",
                        test.tc_id
                    );
                }
                "invalid" => {
                    invalid += 1;
                    assert!(
                        !verified,
                        "Wycheproof Ed25519 tcId {} must not verify",
                        test.tc_id
                    );
                }
                "acceptable" => {
                    acceptable += 1;
                }
                other => panic!("unknown Wycheproof result {other} for tcId {}", test.tc_id),
            }
        }
    }

    assert!(
        valid > 0,
        "official Ed25519 corpus should include valid cases"
    );
    assert!(
        invalid > 0,
        "official Ed25519 corpus should include invalid cases"
    );
    assert_eq!(valid + invalid + acceptable, corpus.number_of_tests);
}

#[test]
fn official_wycheproof_p256_sha256_corpus_matches_underlying_verifier_policy() {
    let corpus: WycheproofFile = serde_json::from_str(include_str!(
        "../fixtures/wycheproof/ecdsa_secp256r1_sha256_test.json"
    ))
    .unwrap();
    assert_eq!(corpus.algorithm, "ECDSA");
    assert_eq!(count_tests(&corpus), corpus.number_of_tests);

    run_ecdsa_corpus(&corpus, EcdsaCurve::P256);
}

#[test]
fn official_wycheproof_k256_sha256_corpus_matches_underlying_verifier_policy() {
    let corpus: WycheproofFile = serde_json::from_str(include_str!(
        "../fixtures/wycheproof/ecdsa_secp256k1_sha256_test.json"
    ))
    .unwrap();
    assert_eq!(corpus.algorithm, "ECDSA");
    assert_eq!(count_tests(&corpus), corpus.number_of_tests);

    run_ecdsa_corpus(&corpus, EcdsaCurve::K256);
}

#[derive(Clone, Copy)]
enum EcdsaCurve {
    K256,
    P256,
}

fn run_ecdsa_corpus(corpus: &WycheproofFile, curve: EcdsaCurve) {
    let mut valid = 0usize;
    let mut invalid = 0usize;
    let mut acceptable = 0usize;

    for group in &corpus.test_groups {
        assert!(!group.public_key.uncompressed.is_empty());
        let public_key = decode_hex(&group.public_key.uncompressed, 0);
        for test in &group.tests {
            let msg = decode_hex(&test.msg, test.tc_id);
            let sig = decode_hex(&test.sig, test.tc_id);
            let verified = verify_ecdsa_sha256(curve, &public_key, &msg, &sig);

            match test.result.as_str() {
                "valid" => {
                    valid += 1;
                    assert!(verified, "Wycheproof ECDSA tcId {} must verify", test.tc_id);
                }
                "invalid" => {
                    invalid += 1;
                    assert!(
                        !verified,
                        "Wycheproof ECDSA tcId {} must not verify",
                        test.tc_id
                    );
                }
                "acceptable" => {
                    acceptable += 1;
                }
                other => panic!("unknown Wycheproof result {other} for tcId {}", test.tc_id),
            }
        }
    }

    assert!(
        valid > 0,
        "official ECDSA corpus should include valid cases"
    );
    assert!(
        invalid > 0,
        "official ECDSA corpus should include invalid cases"
    );
    assert_eq!(valid + invalid + acceptable, corpus.number_of_tests);
}

fn verify_ecdsa_sha256(curve: EcdsaCurve, public_key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    let digest = Sha256::digest(msg);
    match curve {
        EcdsaCurve::K256 => {
            use secp256k1::{Message, PublicKey, Secp256k1, ecdsa::Signature};

            let Ok(key) = PublicKey::from_slice(public_key) else {
                return false;
            };
            let Ok(mut signature) = Signature::from_der(sig) else {
                return false;
            };
            signature.normalize_s();
            let message = Message::from_digest(digest.into());
            Secp256k1::verification_only()
                .verify_ecdsa(message, &signature, &key)
                .is_ok()
        }
        EcdsaCurve::P256 => {
            use p256::ecdsa::{Signature, VerifyingKey, signature::hazmat::PrehashVerifier};

            let Ok(key) = VerifyingKey::from_sec1_bytes(public_key) else {
                return false;
            };
            let Ok(signature) = Signature::from_der(sig) else {
                return false;
            };
            key.verify_prehash(&digest, &signature).is_ok()
        }
    }
}

fn decode_hex(hex_value: &str, tc_id: u64) -> Vec<u8> {
    hex::decode(hex_value).unwrap_or_else(|error| {
        panic!("invalid hex in Wycheproof tcId {tc_id}: {error}");
    })
}

fn count_tests(corpus: &WycheproofFile) -> usize {
    corpus
        .test_groups
        .iter()
        .map(|group| group.tests.len())
        .sum()
}
