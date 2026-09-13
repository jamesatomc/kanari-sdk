// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::{CurveType, hd_wallet};

#[test]
fn test_derive_supports_post_quantum_curves() {
    // Known BIP-39 test mnemonic (do not use in production)
    let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let password = "";
    let path = "m/44'/60'/0'/0/0";

    // PQC curves derive deterministically from mnemonic + path (BIP32 itself
    // is secp256k1-specific, so path binding uses SHAKE256 instead).
    for curve in [
        CurveType::Dilithium2,
        CurveType::Dilithium3,
        CurveType::Dilithium5,
        CurveType::SphincsPlusSha256Robust,
        CurveType::Falcon512,
        CurveType::Falcon1024,
        CurveType::Ed25519Dilithium3,
        CurveType::K256Dilithium3,
    ] {
        let kp = hd_wallet::derive_keypair_from_path(mnemonic, password, path, curve)
            .unwrap_or_else(|e| panic!("{curve} HD derivation should succeed: {e:?}"));
        assert_eq!(kp.curve_type, curve);
        assert!(kp.address.starts_with("0x"));

        // Deterministic: same inputs reproduce the same keypair
        let kp2 = hd_wallet::derive_keypair_from_path(mnemonic, password, path, curve).unwrap();
        assert_eq!(
            kp.address, kp2.address,
            "{curve} derivation not deterministic"
        );

        // Path-sensitive: a different path yields a different key
        let other =
            hd_wallet::derive_keypair_from_path(mnemonic, password, "m/44'/60'/0'/0/1", curve)
                .unwrap();
        assert_ne!(
            kp.address, other.address,
            "{curve} derivation is not path-sensitive"
        );
    }
}
