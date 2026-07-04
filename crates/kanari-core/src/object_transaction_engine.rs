// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Object-centric transaction admission for `BlockchainEngine`.
//!
//! Pending object transactions are stored outside canonical chain state. A
//! state write guard serializes admission while the persistent store applies
//! the payload, index, and mutable-object locks atomically.

use crate::engine::BlockchainEngine;
use anyhow::{Context, Result, ensure};
use kanari_move_runtime_v1::state::StateManager;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use kanari_types::object::{ObjectID, ObjectRef};
use kanari_types::object_transaction::{
    ObjectArg, ObjectTransactionKind, TransactionExpiration,
};
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use move_core_types::language_storage::{StructTag, TypeTag};
use std::collections::BTreeMap;
use std::str::FromStr;

const OBJECT_MEMPOOL_INDEX_KEY: &[u8] = b"mempool:object:index";
const OBJECT_MEMPOOL_LOCKS_KEY: &[u8] = b"mempool:object:locks";
const MAX_OBJECT_MEMPOOL_SIZE: usize = 1_000_000;
const UID_SIZE: usize = 32;
const U64_SIZE: usize = 8;

type ObjectLockMap = BTreeMap<ObjectID, Vec<u8>>;

fn pending_key(digest: &[u8]) -> Vec<u8> {
    let mut key = b"mempool:object:tx:".to_vec();
    key.extend_from_slice(hex::encode(digest).as_bytes());
    key
}

fn executed_key(digest: &[u8]) -> Vec<u8> {
    let mut key = b"object_tx:executed:".to_vec();
    key.extend_from_slice(hex::encode(digest).as_bytes());
    key
}

fn native_coin_balance(type_name: &str, data: &[u8]) -> Result<u64> {
    let coin = StructTag::from_str(type_name)
        .with_context(|| format!("Gas object type is not a valid struct tag: {type_name}"))?;
    ensure!(
        coin.module.as_str() == "coin" && coin.name.as_str() == "Coin",
        "Gas payment object must be Coin<KANARI>, found {type_name}"
    );
    let expected = StructTag::from_str(KANARI_TOKEN_TYPE)?;
    let Some(TypeTag::Struct(token)) = coin.type_params.first() else {
        anyhow::bail!("Gas coin does not contain a token type");
    };
    ensure!(
        token.as_ref() == &expected,
        "Gas payment object must contain native KANARI"
    );
    ensure!(
        data.len() >= UID_SIZE + U64_SIZE,
        "Gas coin object has malformed contents"
    );
    let amount = u64::from_le_bytes(
        data[UID_SIZE..UID_SIZE + U64_SIZE]
            .try_into()
            .expect("slice length is checked"),
    );
    Ok(amount)
}

fn validate_state_inputs(
    state: &StateManager,
    transaction: &SignedObjectTransaction,
) -> Result<()> {
    transaction.data.validate()?;

    ensure!(
        matches!(transaction.data.expiration, TransactionExpiration::None),
        "Epoch expiration is not enabled until epoch state is canonical"
    );
    ensure!(
        !matches!(transaction.data.kind, ObjectTransactionKind::Publish { .. }),
        "Object package publishing is not enabled until packages are stored as objects"
    );

    let sender = transaction.data.sender;
    for input in transaction.data.input_objects() {
        match input {
            ObjectArg::ImmOrOwnedObject(reference) | ObjectArg::Receiving(reference) => {
                state.validate_address_owned_object_ref(reference, sender)?;
            }
            ObjectArg::SharedObject { .. } => {
                anyhow::bail!(
                    "Shared objects are disabled until persisted Owner::Shared metadata is active"
                );
            }
        }
    }

    let gas_owner = transaction.data.gas_data.owner;
    let max_fee = transaction
        .data
        .gas_data
        .budget
        .checked_mul(transaction.data.gas_data.price)
        .ok_or_else(|| anyhow::anyhow!("Gas budget multiplied by gas price overflows u64"))?;
    let mut available = 0u64;
    for payment in &transaction.data.gas_data.payment {
        let object = state.validate_address_owned_object_ref(payment, gas_owner)?;
        available = available
            .checked_add(native_coin_balance(&object.type_, &object.data)?)
            .ok_or_else(|| anyhow::anyhow!("Combined gas coin balance overflows u64"))?;
    }
    ensure!(
        available >= max_fee,
        "Insufficient gas coin balance: need at least {max_fee}, found {available}"
    );
    Ok(())
}

impl BlockchainEngine {
    /// Verify and atomically admit an object-centric transaction.
    pub fn submit_object_transaction(
        &self,
        transaction: SignedObjectTransaction,
    ) -> Result<Vec<u8>> {
        transaction.verify()?;
        let digest = transaction.digest()?;

        // This guard serializes object admission with state transitions so exact
        // references cannot be validated against one snapshot and queued after
        // another snapshot has already committed.
        let state = self.state_write();
        validate_state_inputs(&state, &transaction)?;
        let store = state.store.clone();

        ensure!(
            store.load::<bool>(&executed_key(&digest))?.is_none(),
            "Object transaction has already executed"
        );
        ensure!(
            store
                .load::<SignedObjectTransaction>(&pending_key(&digest))?
                .is_none(),
            "Object transaction is already pending"
        );

        let mut index: Vec<Vec<u8>> = store.load(OBJECT_MEMPOOL_INDEX_KEY)?.unwrap_or_default();
        ensure!(
            index.len() < MAX_OBJECT_MEMPOOL_SIZE,
            "Object transaction mempool is full"
        );
        let mut locks: ObjectLockMap =
            store.load(OBJECT_MEMPOOL_LOCKS_KEY)?.unwrap_or_default();

        for object_id in transaction.data.mutable_input_ids() {
            if let Some(holder) = locks.get(&object_id) {
                anyhow::bail!(
                    "Mutable object {} is already reserved by transaction {}",
                    object_id,
                    hex::encode(holder)
                );
            }
        }

        for object_id in transaction.data.mutable_input_ids() {
            locks.insert(object_id, digest.clone());
        }
        index.push(digest.clone());
        index.sort();
        index.dedup();

        let updates = vec![
            (pending_key(&digest), bcs::to_bytes(&transaction)?),
            (OBJECT_MEMPOOL_INDEX_KEY.to_vec(), bcs::to_bytes(&index)?),
            (OBJECT_MEMPOOL_LOCKS_KEY.to_vec(), bcs::to_bytes(&locks)?),
        ];
        store.apply_raw_changes(&updates, &[])?;
        Ok(digest)
    }

    pub fn pending_object_transaction_len(&self) -> Result<usize> {
        let state = self.state_read();
        let index: Vec<Vec<u8>> = state
            .store
            .load(OBJECT_MEMPOOL_INDEX_KEY)?
            .unwrap_or_default();
        Ok(index.len())
    }

    pub fn pending_object_transactions(&self) -> Result<Vec<SignedObjectTransaction>> {
        let state = self.state_read();
        let index: Vec<Vec<u8>> = state
            .store
            .load(OBJECT_MEMPOOL_INDEX_KEY)?
            .unwrap_or_default();
        let mut transactions = Vec::with_capacity(index.len());
        for digest in index {
            if let Some(transaction) = state
                .store
                .load::<SignedObjectTransaction>(&pending_key(&digest))?
            {
                transactions.push(transaction);
            }
        }
        Ok(transactions)
    }

    /// Remove a pending object transaction and release its mutable inputs.
    pub fn release_object_transaction(
        &self,
        digest: &[u8],
    ) -> Result<Option<SignedObjectTransaction>> {
        let state = self.state_write();
        let store = state.store.clone();
        let key = pending_key(digest);
        let Some(transaction) = store.load::<SignedObjectTransaction>(&key)? else {
            return Ok(None);
        };

        let mut index: Vec<Vec<u8>> = store.load(OBJECT_MEMPOOL_INDEX_KEY)?.unwrap_or_default();
        index.retain(|entry| entry.as_slice() != digest);
        let mut locks: ObjectLockMap =
            store.load(OBJECT_MEMPOOL_LOCKS_KEY)?.unwrap_or_default();
        for object_id in transaction.data.mutable_input_ids() {
            if locks
                .get(&object_id)
                .is_some_and(|holder| holder.as_slice() == digest)
            {
                locks.remove(&object_id);
            }
        }

        let updates = vec![
            (OBJECT_MEMPOOL_INDEX_KEY.to_vec(), bcs::to_bytes(&index)?),
            (OBJECT_MEMPOOL_LOCKS_KEY.to_vec(), bcs::to_bytes(&locks)?),
        ];
        store.apply_raw_changes(&updates, &[key])?;
        Ok(Some(transaction))
    }

    /// Finalize a transaction after execution and retain replay protection.
    pub fn finalize_object_transaction(
        &self,
        digest: &[u8],
    ) -> Result<Option<SignedObjectTransaction>> {
        let transaction = self.release_object_transaction(digest)?;
        if transaction.is_some() {
            let state = self.state_read();
            state.store.save(&executed_key(digest), &true)?;
        }
        Ok(transaction)
    }

    pub fn validate_object_reference(&self, reference: &ObjectRef) -> Result<()> {
        self.state_read().validate_object_ref_exact(reference)?;
        Ok(())
    }
}
