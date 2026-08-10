// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#![no_main]

use kanari_crypto::{
    CurveType, generate_keypair,
    signatures::{sign_message, verify_signature, verify_signature_with_curve},
};
use libfuzzer_sys::fuzz_target;

fn flip_byte(bytes: &[u8], index_seed: u8, mask: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    if !out.is_empty() {
        let index = usize::from(index_seed) % out.len();
        out[index] ^= mask.max(1);
    }
    out
}

fn assert_does_not_verify(result: Result<bool, kanari_crypto::SignatureError>) {
    if let Ok(true) = result {
        panic!("hybrid attack mutation unexpectedly verified");
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }

    let curve = if data[0] & 1 == 0 {
        CurveType::K256Dilithium3
    } else {
        CurveType::Ed25519Dilithium3
    };
    let mutation = data[1] % 8;
    let flip_index = data[2];
    let flip_mask = data[3];
    let message = &data[4..];

    let Ok(keypair) = generate_keypair(curve) else {
        return;
    };
    let Ok(signature) = sign_message(&keypair.private_key, message, curve) else {
        return;
    };
    let tagged = keypair.tagged_address();

    assert!(
        verify_signature(&tagged, message, &signature).unwrap_or(false),
        "valid hybrid signature must verify before mutation"
    );

    match mutation {
        0 => {
            let tampered_sig = flip_byte(&signature, flip_index, flip_mask);
            assert_does_not_verify(verify_signature(&tagged, message, &tampered_sig));
        }
        1 => {
            let tampered_msg = flip_byte(message, flip_index, flip_mask);
            assert_does_not_verify(verify_signature(&tagged, &tampered_msg, &signature));
        }
        2 => {
            let mut truncated = signature.clone();
            truncated.truncate(usize::from(flip_index) % signature.len().max(1));
            assert_does_not_verify(verify_signature(&tagged, message, &truncated));
        }
        3 => {
            let mut oversized_len = signature.clone();
            if oversized_len.len() >= 2 {
                oversized_len[0] = 0xFF;
                oversized_len[1] = 0xFF;
            }
            assert_does_not_verify(verify_signature(&tagged, message, &oversized_len));
        }
        4 => {
            let attacker = generate_keypair(curve).expect("hybrid attacker keypair");
            assert_does_not_verify(verify_signature(
                &attacker.tagged_address(),
                message,
                &signature,
            ));
        }
        5 => {
            let untagged = tagged
                .split_once(':')
                .map(|(_, rest)| rest)
                .unwrap_or(tagged.as_str());
            assert!(
                verify_signature(untagged, message, &signature).is_err(),
                "hybrid verifier must reject untagged address"
            );
        }
        6 => {
            let wrong_curve = if curve == CurveType::K256Dilithium3 {
                CurveType::Ed25519Dilithium3
            } else {
                CurveType::K256Dilithium3
            };
            let untagged = tagged
                .split_once(':')
                .map(|(_, rest)| rest)
                .unwrap_or(tagged.as_str());
            assert_does_not_verify(verify_signature_with_curve(
                untagged,
                message,
                &signature,
                wrong_curve,
            ));
        }
        _ => {
            let malformed_tag = format!(
                "{}:{}",
                curve,
                flip_byte(tagged.as_bytes(), flip_index, flip_mask).escape_ascii()
            );
            assert_does_not_verify(verify_signature(&malformed_tag, message, &signature));
        }
    }
});
