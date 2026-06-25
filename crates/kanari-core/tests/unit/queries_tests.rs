#![allow(clippy::duplicate_mod)]

use super::*;
use crate::{CheckpointSyncData, consensus::Checkpoint};
use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
use move_core_types::account_address::AccountAddress;

#[path = "test_support.rs"]
mod test_support;

use test_support::{secure_consensus_keys, signed_transfer};

fn configure_single_authority(
    engine: &mut BlockchainEngine,
) -> (
    ed25519_dalek::SigningKey,
    std::collections::BTreeMap<String, Vec<u8>>,
) {
    let authorities = vec!["0x1".to_string()];
    engine.set_authorities("0x1".to_string(), authorities.clone());
    let (key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    engine
        .set_consensus_signing_key(key.clone(), public_keys.clone())
        .unwrap();
    (key, public_keys)
}

#[test]
fn account_info_reports_native_balance_after_gas_debit() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = AccountAddress::from_hex_literal("0x1111").unwrap();
    let dao =
        AccountAddress::from_hex_literal(kanari_types::address::Address::DAO_ADDRESS).unwrap();

    let mut coin_data = vec![0u8; 40];
    coin_data[32..40].copy_from_slice(&1_000u64.to_le_bytes());

    {
        let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());
        let mut create = ChangeSet::new();
        create.created_objects.push((
            "0xaaaa".to_string(),
            CreatedObject {
                owner: sender,
                uid: None,
                id: None,
                type_: format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
                data: coin_data.clone(),
                version: 1,
            },
        ));
        state
            .apply_changeset_without_supply_validation(&create)
            .unwrap();

        let mut gas_debit = ChangeSet::new();
        gas_debit.created_objects.push((
            "0xaaaa".to_string(),
            CreatedObject {
                owner: sender,
                uid: None,
                id: None,
                type_: format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
                data: coin_data,
                version: 2,
            },
        ));
        gas_debit.get_or_create_change(sender).debit(100);
        gas_debit.collect_gas(dao, 100);
        state
            .apply_changeset_without_supply_validation(&gas_debit)
            .unwrap();
    }

    let info = engine.get_account_info("0x1111").unwrap();
    assert_eq!(info.token_balances.get(KANARI_TOKEN_TYPE), Some(&900));
    let object_balance = info.owned_objects.unwrap()[0].data[32..40]
        .try_into()
        .map(u64::from_le_bytes)
        .unwrap();
    assert_eq!(object_balance, 900);
}

#[test]
fn sync_checkpoint_from_data_rejects_empty_checkpoint() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    let (key, public_keys) = configure_single_authority(&mut engine);
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };
    let state_root = engine
        .state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();
    let mut checkpoint = Checkpoint::new(1, vec![], vec![], state_root, 42, prev_hash);
    checkpoint
        .attach_single_authority_certificate(
            "0x1".to_string(),
            &key,
            &public_keys,
            0,
            1,
        )
        .unwrap();
    let sync_data = CheckpointSyncData { checkpoint };

    let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Refusing to sync empty checkpoint")
    );
    assert_eq!(engine.get_stats().height, 0);
}

#[test]
fn sync_checkpoint_from_data_rejects_root_mismatch() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    let (key, public_keys) = configure_single_authority(&mut engine);
    let prev_hash = {
        let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.latest_checkpoint().hash().unwrap()
    };
    let signed_tx = signed_transfer(0);
    let mut checkpoint = Checkpoint::new(
        1,
        vec![[7u8; 32]],
        vec![signed_tx],
        vec![9u8; 32],
        42,
        prev_hash,
    );
    checkpoint
        .attach_single_authority_certificate(
            "0x1".to_string(),
            &key,
            &public_keys,
            0,
            1,
        )
        .unwrap();
    let sync_data = CheckpointSyncData { checkpoint };

    let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
    assert!(error.to_string().contains("state root mismatch"));
    assert_eq!(engine.get_stats().height, 0);
}
