// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::HarnessConfig;
use anyhow::{Context, Result};
use kanari_core::BlockchainEngine;
use kanari_core::kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
use kanari_crypto::hash_data_blake3;
use kanari_crypto::keys::{CurveType, KeyPair, keypair_from_private_key};
use kanari_types::coin::CoinModule;
use kanari_types::gas_coin::GAS_COIN;
use kanari_types::transaction::{
    GasPayment, ObjectOwnerKind, ObjectRef, SignedTransaction, Transaction,
};
use move_core_types::account_address::AccountAddress;
use tempfile::TempDir;

pub struct PreparedEngine {
    pub engine: BlockchainEngine,
    #[allow(dead_code)]
    temp_dir: Option<TempDir>,
}

pub fn prepare_engine(use_persistent_state: bool) -> Result<PreparedEngine> {
    let temp_dir = if use_persistent_state {
        Some(tempfile::tempdir().context("failed to create temporary benchmark state dir")?)
    } else {
        None
    };
    let mut engine = if let Some(dir) = &temp_dir {
        BlockchainEngine::new_dir(
            dir.path()
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("benchmark state dir is not valid UTF-8"))?,
        )?
    } else {
        BlockchainEngine::new_in_memory()?
    };

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

    Ok(PreparedEngine { engine, temp_dir })
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
            let nonce = deterministic_workload_nonce(tx_index, sender_index);
            let sender = &senders[sender_index];
            let tx = Transaction::new_burn_with_gas(sender.tagged_address(), 0, nonce, 100_000, 0);
            let mut signed_tx = SignedTransaction::new(tx);
            signed_tx
                .sign(&sender.private_key, sender.curve_type)
                .expect("transaction signing should succeed");
            signed_tx
        })
        .collect();

    Ok(signed_txs)
}

pub fn build_funded_owned_fastpath_workload(
    engine: &BlockchainEngine,
    config: &HarnessConfig,
    sender_count: usize,
) -> Result<Vec<SignedTransaction>> {
    let senders: Vec<_> = (0..sender_count)
        .map(deterministic_sender_keypair)
        .collect::<Result<_>>()?;
    let mut funding = ChangeSet::new();
    let coin_balance = 1_000_000u64;
    let mut signed_txs = Vec::with_capacity(config.tx_count);

    for tx_index in 0..config.tx_count {
        let sender_index = tx_index % sender_count;
        let nonce = deterministic_workload_nonce(tx_index, sender_index);
        let sender = &senders[sender_index];
        let owner = AccountAddress::from_hex_literal(&sender.address)
            .with_context(|| format!("invalid benchmark sender address {}", sender.address))?;
        let object_id = deterministic_object_id(tx_index);
        let mut coin_data = vec![0u8; 40];
        coin_data[32..40].copy_from_slice(&coin_balance.to_le_bytes());
        funding.created_objects.push((
            object_id.clone(),
            CreatedObject {
                owner,
                owner_kind: ObjectOwnerKind::AddressOwner(sender.address.clone()),
                uid: None,
                id: None,
                type_: CoinModule::coin_type(GAS_COIN),
                data: coin_data,
                version: 1,
            },
        ));
        funding.mint(owner, coin_balance);

        let mut tx = Transaction::new_burn_with_gas(sender.tagged_address(), 1, nonce, 100_000, 1);
        if let Transaction::ExecuteFunction { gas_payment, .. } = &mut tx {
            *gas_payment = Some(GasPayment {
                payment_objects: vec![native_coin_object_ref(&object_id, coin_balance)],
                owner: sender.address.clone(),
                budget: 100_000,
                price: 1,
            });
        }
        let mut signed_tx = SignedTransaction::new(tx);
        signed_tx
            .sign(&sender.private_key, sender.curve_type)
            .expect("transaction signing should succeed");
        signed_txs.push(signed_tx);
    }

    {
        let mut state = engine.state_write();
        state
            .apply_changeset_without_supply_validation(&funding)
            .context("failed to fund owned-fastpath benchmark senders")?;
        state
            .commit()
            .context("failed to commit owned-fastpath benchmark funding state")?;
    }

    Ok(signed_txs)
}

fn deterministic_workload_nonce(tx_index: usize, sender_index: usize) -> u64 {
    let mut material = Vec::with_capacity(32);
    material.extend_from_slice(b"kanari-bench-tx-nonce-v1");
    material.extend_from_slice(&(tx_index as u64).to_le_bytes());
    material.extend_from_slice(&(sender_index as u64).to_le_bytes());
    let digest = hash_data_blake3(&material);
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(bytes).max(1)
}

fn deterministic_object_id(tx_index: usize) -> String {
    let mut material = Vec::with_capacity(32);
    material.extend_from_slice(b"kanari-bench-owned-object");
    material.extend_from_slice(&(tx_index as u64).to_le_bytes());
    let digest = hash_data_blake3(&material);
    format!("0x{}", hex::encode(&digest[..32]))
}

fn native_coin_object_ref(object_id: &str, balance: u64) -> ObjectRef {
    let mut coin_data = vec![0u8; 40];
    coin_data[32..40].copy_from_slice(&balance.to_le_bytes());
    ObjectRef::new(
        object_id.to_string(),
        Some(1),
        Some(format!(
            "0x{}",
            hex::encode(kanari_crypto::hash_data_blake3(&coin_data))
        )),
    )
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
