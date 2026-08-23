// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Print Rust-generated PQC signature vectors for Move framework tests.
//!
//! This is intentionally an example instead of a unit test so maintainers can
//! refresh vectors manually without making normal test output noisy.

use kanari_crypto::{
    keys::{CurveType, generate_keypair},
    signatures::sign_message,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #![allow(clippy::print_stdout)] // example utility intentionally prints vectors to stdout
    let message = b"kanari move pqc positive vector";
    println!("message={}", hex::encode(message));

    for curve in [
        CurveType::Falcon512,
        CurveType::Falcon1024,
        CurveType::SphincsPlusSha256Robust,
    ] {
        let keypair = generate_keypair(curve)?;
        let signature = sign_message(&keypair.private_key, message, curve)?;
        println!("curve={curve}");
        println!("public_key={}", keypair.public_key);
        println!("signature={}", hex::encode(signature));
    }

    Ok(())
}
