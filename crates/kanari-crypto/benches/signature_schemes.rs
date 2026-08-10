// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#![allow(clippy::print_stdout)]

use kanari_crypto::{
    BatchVerificationItem, CurveType, generate_keypair,
    signatures::{sign_message, verify_batch_with_curve, verify_signature_with_curve},
};
use std::time::{Duration, Instant};

const SAMPLE_COUNT: usize = 32;

fn main() {
    println!("Kanari crypto signature benchmark");
    println!("samples_per_curve={SAMPLE_COUNT}");

    for curve in [
        CurveType::Ed25519,
        CurveType::K256,
        CurveType::P256,
        CurveType::Dilithium3,
        CurveType::Falcon512,
        CurveType::Ed25519Dilithium3,
    ] {
        match bench_curve(curve) {
            Ok(result) => println!(
                "{curve}: sign={:.2} ops/s verify={:.2} ops/s batch_api={:.2} ops/s sig_bytes={}",
                result.sign_ops_per_sec,
                result.verify_ops_per_sec,
                result.batch_ops_per_sec,
                result.signature_bytes
            ),
            Err(error) => println!("{curve}: skipped/error={error}"),
        }
    }
}

struct BenchResult {
    sign_ops_per_sec: f64,
    verify_ops_per_sec: f64,
    batch_ops_per_sec: f64,
    signature_bytes: usize,
}

fn bench_curve(curve: CurveType) -> Result<BenchResult, Box<dyn std::error::Error>> {
    let keypair = generate_keypair(curve)?;
    let messages: Vec<Vec<u8>> = (0..SAMPLE_COUNT)
        .map(|index| format!("kanari-crypto-benchmark-{curve}-{index}").into_bytes())
        .collect();

    let sign_start = Instant::now();
    let signatures: Vec<Vec<u8>> = messages
        .iter()
        .map(|message| sign_message(&keypair.private_key, message, curve))
        .collect::<Result<_, _>>()?;
    let sign_elapsed = sign_start.elapsed();

    let public_key = keypair.public_key.as_str();
    let verify_start = Instant::now();
    for (message, signature) in messages.iter().zip(signatures.iter()) {
        if !verify_signature_with_curve(public_key, message, signature, curve)? {
            return Err("signature failed verification".into());
        }
    }
    let verify_elapsed = verify_start.elapsed();

    let batch_items: Vec<BatchVerificationItem<'_>> = messages
        .iter()
        .zip(signatures.iter())
        .map(|(message, signature)| BatchVerificationItem::new(public_key, message, signature))
        .collect();
    let batch_start = Instant::now();
    if !verify_batch_with_curve(&batch_items, curve)? {
        return Err("batch verification failed".into());
    }
    let batch_elapsed = batch_start.elapsed();

    Ok(BenchResult {
        sign_ops_per_sec: ops_per_sec(SAMPLE_COUNT, sign_elapsed),
        verify_ops_per_sec: ops_per_sec(SAMPLE_COUNT, verify_elapsed),
        batch_ops_per_sec: ops_per_sec(SAMPLE_COUNT, batch_elapsed),
        signature_bytes: signatures.first().map(Vec::len).unwrap_or_default(),
    })
}

fn ops_per_sec(count: usize, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();
    if seconds == 0.0 {
        return f64::INFINITY;
    }
    count as f64 / seconds
}
