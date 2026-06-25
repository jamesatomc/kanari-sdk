#![allow(dead_code)]

use crate::engine::BlockchainEngine;
use kanari_crypto::keys::{CurveType, KeyPair, generate_keypair};
use kanari_move_runtime_v1::changeset::ChangeSet;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use kanari_types::transaction::{SignedTransaction, Transaction};
use move_core_types::account_address::AccountAddress;
use std::collections::BTreeMap;
use std::sync::Mutex;

pub static ENV_LOCK: Mutex<()> = Mutex::new(());

pub fn authority_key(seed: u8) -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
}

pub fn signed_transfer(sequence_number: u64) -> SignedTransaction {
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    signed_transfer_from(&sender, sequence_number)
}

/// Build a signed, non-legacy transaction for admission, replay, sequence and
/// failure-path tests. Execution is expected to fail cleanly because the test
/// module is not published; importantly this does not exercise the removed
/// account-ledger KANARI transfer shortcut.
pub fn signed_transfer_from(sender: &KeyPair, sequence_number: u64) -> SignedTransaction {
    let gas = kanari_types::gas::GasConfig::default();
    let tx = Transaction::ExecuteFunction {
        sender: sender.tagged_address(),
        module: "0x2::test_support".to_string(),
        function: "noop".to_string(),
        type_args: vec![],
        args: vec![],
        gas_limit: gas.default_transaction_gas_limit(),
        gas_price: gas.default_transaction_gas_price(),
        sequence_number,
    };
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    signed_tx
}

/// Test-only funding is equivalent to genesis/migration setup: establish the
/// account balance first without validating the intermediate supply snapshot,
/// then publish the matching treasury and validate the completed state.
pub fn fund_sender(engine: &BlockchainEngine, address: &str, balance: u64) {
    let addr = AccountAddress::from_hex_literal(address).unwrap();
    let dao =
        AccountAddress::from_hex_literal(kanari_types::address::Address::DAO_ADDRESS).unwrap();
    let mut state = engine.state.write().unwrap_or_else(|e| e.into_inner());

    let mut mint = ChangeSet::new();
    mint.mint(addr, balance);
    state
        .apply_changeset_without_supply_validation(&mint)
        .unwrap();

    let mut treasury = ChangeSet::new();
    treasury.add_treasury(dao, KANARI_TOKEN_TYPE.to_string(), state.total_supply);
    state.apply_changeset(&treasury).unwrap();
}

pub fn secure_consensus_keys(
    authorities: &[String],
    local_authority: &str,
) -> (ed25519_dalek::SigningKey, BTreeMap<String, Vec<u8>>) {
    let mut public_keys = BTreeMap::new();
    let mut local_signing_key = None;

    for (index, authority) in authorities.iter().enumerate() {
        let signing_key = authority_key(index as u8 + 11);
        if authority == local_authority {
            local_signing_key = Some(signing_key.clone());
        }
        public_keys.insert(
            authority.clone(),
            signing_key.verifying_key().to_bytes().to_vec(),
        );
    }

    (
        local_signing_key.expect("local authority must be in authority set"),
        public_keys,
    )
}
