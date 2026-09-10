#![allow(clippy::print_stdout)]
// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Example: intentionally trigger and show common errors from kanari-crypto
use kanari_crypto::keys::{CurveType, keypair_from_mnemonic};
use kanari_crypto::signatures::{sign_message, verify_signature, verify_signature_with_curve};

fn main() {
    // 1) Invalid mnemonic (too short)
    let bad_mnemonic = "abandon abandon abandon";
    match keypair_from_mnemonic(bad_mnemonic, CurveType::K256) {
        Ok(kp) => println!("Unexpected success (mnemonic): {}", kp.get_address()),
        Err(e) => eprintln!("Expected mnemonic error: {}", e),
    }

    // 2) Invalid private key for signing (bad hex/short)
    let invalid_priv = "kanarideadbeef"; // intentionally malformed
    let msg = b"hello kanari";
    match sign_message(invalid_priv, msg, CurveType::K256) {
        Ok(sig) => println!("Unexpected signature produced: {} bytes", sig.len()),
        Err(e) => eprintln!("Expected sign error: {}", e),
    }

    // 3) Verification failure (bad address / bad signature)
    let bad_address = "0x0123"; // invalid/too short address
    let fake_sig = vec![0u8; 64];
    match verify_signature(bad_address, msg, &fake_sig) {
        Ok(true) => println!("Unexpectedly verified"),
        Ok(false) => eprintln!("Expected verification to fail (returns false)"),
        Err(e) => eprintln!("Verification error: {}", e),
    }

    // 4) PQC mnemonic derivation (supported: deterministic Dilithium3 from mnemonic)
    let long_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    match keypair_from_mnemonic(long_mnemonic, CurveType::Dilithium3) {
        Ok(kp) => println!("PQC mnemonic derivation succeeded: {}", kp.get_address()),
        Err(e) => eprintln!("Unexpected PQC mnemonic error: {}", e),
    }

    // 5) Cross-algorithm check: sign with K256, attempt to verify with Dilithium3
    let valid_mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    match keypair_from_mnemonic(valid_mnemonic, CurveType::K256) {
        Ok(kp) => {
            let sec = kp.export_private_key_secure();
            let sig = match sign_message(&sec, msg, CurveType::K256) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to sign with K256: {}", e);
                    return;
                }
            };

            // Try verifying with the wrong algorithm (Dilithium3). This should NOT succeed.
            match verify_signature_with_curve(kp.get_address(), msg, &sig, CurveType::Dilithium3) {
                Ok(true) => {
                    eprintln!("SYSTEM PROBLEM: cross-algo verification unexpectedly succeeded!")
                }
                Ok(false) => println!("Cross-algo verification failed as expected"),
                Err(e) => println!("Cross-algo verification returned error (expected): {}", e),
            }
        }
        Err(e) => eprintln!("Failed to derive K256 keypair for cross-algo test: {}", e),
    }
}
