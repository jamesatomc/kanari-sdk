use crate::config::HarnessConfig;
use anyhow::{Context, Result};
use kanari_core::BlockchainEngine;
use kanari_crypto::hash_data_blake3;
use kanari_crypto::keys::{CurveType, KeyPair, keypair_from_private_key};
use kanari_types::transaction::{SignedTransaction, Transaction};

pub fn prepare_engine() -> Result<BlockchainEngine> {
    let mut engine = BlockchainEngine::new_in_memory()?;

    let keypair = kanari_crypto::keys::generate_keypair(CurveType::Ed25519)?;
    let hex_part = keypair.private_key.as_str().trim_start_matches("kanari");
    let signing_key_bytes_vec =
        hex::decode(hex_part).context("Failed to decode private key hex")?;
    let signing_key_bytes: [u8; 32] = signing_key_bytes_vec
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid private key length: expected 32 bytes"))?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&signing_key_bytes);

    let mut authority_public_keys = std::collections::BTreeMap::new();
    authority_public_keys.insert(
        engine.authority_id().to_string(),
        signing_key.verifying_key().to_bytes().to_vec(),
    );
    engine.set_consensus_signing_key(signing_key, authority_public_keys)?;

    Ok(engine)
}

pub fn build_signed_workload(
    config: &HarnessConfig,
    sender_count: usize,
) -> Result<Vec<SignedTransaction>> {
    let senders: Vec<_> = (0..sender_count)
        .map(deterministic_sender_keypair)
        .collect::<Result<_>>()?;

    let signed_txs = (0..config.tx_count)
        .map(|tx_index| {
            let sender_index = tx_index % sender_count;
            let sender = &senders[sender_index];
            let tx = Transaction::new_burn_with_gas(
                sender.tagged_address(),
                0,
                100_000,
                0,
            );
            let mut signed_tx = SignedTransaction::new(tx);
            signed_tx
                .sign(&sender.private_key, sender.curve_type)
                .expect("transaction signing should succeed");
            signed_tx
        })
        .collect();

    Ok(signed_txs)
}

fn deterministic_sender_keypair(index: usize) -> Result<KeyPair> {
    let mut seed_material = Vec::with_capacity(24);
    seed_material.extend_from_slice(b"kanari-bench-sender");
    seed_material.extend_from_slice(&(index as u64).to_le_bytes());
    let seed = hash_data_blake3(&seed_material);
    let private_key = format!("kanari{}", hex::encode(seed));
    keypair_from_private_key(&private_key, CurveType::Ed25519)
        .map_err(|e| anyhow::anyhow!("failed to derive deterministic sender keypair: {}", e))
}
