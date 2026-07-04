// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::engine::BlockchainEngine;
use anyhow::{Context, Result, ensure};
use kanari_move_runtime_v1::state::StateManager;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use kanari_types::object::{ObjectID, ObjectRef};
use kanari_types::object_transaction::{ObjectArg, ObjectTransactionKind, TransactionExpiration};
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use move_core_types::language_storage::{StructTag, TypeTag};
use std::collections::BTreeMap;
use std::str::FromStr;

const INDEX_KEY: &[u8] = b"mempool:object:index";
const LOCKS_KEY: &[u8] = b"mempool:object:locks";
const MAX_PENDING: usize = 1_000_000;
const UID_SIZE: usize = 32;
const U64_SIZE: usize = 8;

type LockMap = BTreeMap<ObjectID, Vec<u8>>;

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
        .with_context(|| format!("Invalid native coin type: {type_name}"))?;
    ensure!(
        coin.module.as_str() == "coin" && coin.name.as_str() == "Coin",
        "Object must be Coin<KANARI>"
    );
    let expected = StructTag::from_str(KANARI_TOKEN_TYPE)?;
    let Some(TypeTag::Struct(token)) = coin.type_params.first() else {
        anyhow::bail!("Coin is missing its token type");
    };
    ensure!(
        token.as_ref() == &expected,
        "Coin must contain native KANARI"
    );
    ensure!(
        data.len() >= UID_SIZE + U64_SIZE,
        "Malformed native coin contents"
    );
    Ok(u64::from_le_bytes(
        data[UID_SIZE..UID_SIZE + U64_SIZE].try_into()?,
    ))
}

fn validate_inputs(state: &StateManager, tx: &SignedObjectTransaction) -> Result<()> {
    tx.data.validate()?;
    ensure!(
        matches!(tx.data.expiration, TransactionExpiration::None),
        "Epoch expiration is not enabled yet"
    );
    ensure!(
        !matches!(&tx.data.kind, ObjectTransactionKind::Publish { .. }),
        "Object package publishing is not enabled yet"
    );

    for input in tx.data.input_objects() {
        if matches!(input, ObjectArg::SharedObject { .. }) {
            anyhow::bail!("Shared objects are not enabled yet");
        }
    }

    // This includes Move-call object arguments plus direct Pay/TransferObjects
    // references. No object may enter execution without exact ref and ownership
    // validation against the same state snapshot used for admission locks.
    for reference in tx.data.owned_input_refs() {
        state.validate_address_owned_object_ref(&reference, tx.data.sender)?;
    }

    let mut pay_available = 0u64;
    let mut pay_refs = BTreeMap::new();
    let pay_amount = if let ObjectTransactionKind::Pay { coins, amount, .. } = &tx.data.kind {
        for reference in coins {
            let object = state.validate_address_owned_object_ref(reference, tx.data.sender)?;
            let coin_balance = native_coin_balance(&object.type_, &object.data)?;
            pay_available = pay_available
                .checked_add(coin_balance)
                .ok_or_else(|| anyhow::anyhow!("Pay coin balance overflow"))?;
            pay_refs.insert(reference.object_id, (*reference, coin_balance));
        }
        ensure!(
            pay_available >= *amount,
            "Insufficient payment coin balance: need {}, found {}",
            amount,
            pay_available
        );
        Some(*amount)
    } else {
        None
    };

    let required = tx
        .data
        .gas_data
        .budget
        .checked_mul(tx.data.gas_data.price)
        .ok_or_else(|| anyhow::anyhow!("Gas fee overflow"))?;
    let mut gas_available = 0u64;
    let mut shared_balance = 0u64;
    for payment in &tx.data.gas_data.payment {
        let object = state.validate_address_owned_object_ref(payment, tx.data.gas_data.owner)?;
        let coin_balance = native_coin_balance(&object.type_, &object.data)?;
        gas_available = gas_available
            .checked_add(coin_balance)
            .ok_or_else(|| anyhow::anyhow!("Gas balance overflow"))?;
        if let Some((pay_ref, balance)) = pay_refs.get(&payment.object_id) {
            ensure!(
                pay_ref == payment,
                "Pay and gas references disagree for the same object"
            );
            shared_balance = shared_balance
                .checked_add(*balance)
                .ok_or_else(|| anyhow::anyhow!("Shared coin balance overflow"))?;
        }
    }
    ensure!(
        gas_available >= required,
        "Insufficient gas coin balance: need {required}, found {gas_available}"
    );
    if let Some(amount) = pay_amount
        && shared_balance > 0
    {
        ensure!(
            tx.data.gas_data.owner == tx.data.sender,
            "Sponsored gas cannot overlap payment coins"
        );
        let union_available = pay_available
            .checked_add(gas_available)
            .and_then(|value| value.checked_sub(shared_balance))
            .ok_or_else(|| anyhow::anyhow!("Combined payment and gas balance overflow"))?;
        let required_total = amount
            .checked_add(required)
            .ok_or_else(|| anyhow::anyhow!("Combined payment and gas requirement overflow"))?;
        ensure!(
            union_available >= required_total,
            "Insufficient shared payment/gas balance: need {required_total}, found {union_available}"
        );
    }
    Ok(())
}

fn release_locks(locks: &mut LockMap, tx: &SignedObjectTransaction, digest: &[u8]) {
    for object_id in tx.data.mutable_input_ids() {
        if locks
            .get(&object_id)
            .is_some_and(|holder| holder.as_slice() == digest)
        {
            locks.remove(&object_id);
        }
    }
}

impl BlockchainEngine {
    pub fn submit_object_transaction(&self, tx: SignedObjectTransaction) -> Result<Vec<u8>> {
        tx.verify()?;
        let digest = tx.digest()?;
        let state = self.state_write();
        validate_inputs(&state, &tx)?;
        let store = state.store.clone();

        ensure!(
            store.load::<bool>(&executed_key(&digest))?.is_none(),
            "Object transaction already executed"
        );
        ensure!(
            store
                .load::<SignedObjectTransaction>(&pending_key(&digest))?
                .is_none(),
            "Object transaction already pending"
        );

        let mut index: Vec<Vec<u8>> = store.load(INDEX_KEY)?.unwrap_or_default();
        ensure!(
            index.len() < MAX_PENDING,
            "Object transaction mempool is full"
        );
        let mut locks: LockMap = store.load(LOCKS_KEY)?.unwrap_or_default();
        let mutable_ids = tx.data.mutable_input_ids();
        for object_id in &mutable_ids {
            if let Some(holder) = locks.get(object_id) {
                anyhow::bail!(
                    "Mutable object {object_id} is reserved by {}",
                    hex::encode(holder)
                );
            }
        }
        for object_id in mutable_ids {
            locks.insert(object_id, digest.clone());
        }
        index.push(digest.clone());
        index.sort();
        index.dedup();

        store.apply_raw_changes(
            &[
                (pending_key(&digest), bcs::to_bytes(&tx)?),
                (INDEX_KEY.to_vec(), bcs::to_bytes(&index)?),
                (LOCKS_KEY.to_vec(), bcs::to_bytes(&locks)?),
            ],
            &[],
        )?;
        Ok(digest)
    }

    pub fn pending_object_transaction_len(&self) -> Result<usize> {
        Ok(self
            .state_read()
            .store
            .load::<Vec<Vec<u8>>>(INDEX_KEY)?
            .unwrap_or_default()
            .len())
    }

    pub fn pending_object_transactions(&self) -> Result<Vec<SignedObjectTransaction>> {
        let state = self.state_read();
        let index = state
            .store
            .load::<Vec<Vec<u8>>>(INDEX_KEY)?
            .unwrap_or_default();
        let mut transactions = Vec::with_capacity(index.len());
        for digest in index {
            if let Some(transaction) = state.store.load(&pending_key(&digest))? {
                transactions.push(transaction);
            }
        }
        Ok(transactions)
    }

    pub fn is_object_transaction_executed(&self, digest: &[u8]) -> Result<bool> {
        Ok(self
            .state_read()
            .store
            .load::<bool>(&executed_key(digest))?
            .unwrap_or(false))
    }

    pub fn release_object_transaction(
        &self,
        digest: &[u8],
    ) -> Result<Option<SignedObjectTransaction>> {
        self.finish_object_transaction(digest, false)
    }

    pub fn finalize_object_transaction(
        &self,
        digest: &[u8],
    ) -> Result<Option<SignedObjectTransaction>> {
        self.finish_object_transaction(digest, true)
    }

    fn finish_object_transaction(
        &self,
        digest: &[u8],
        executed: bool,
    ) -> Result<Option<SignedObjectTransaction>> {
        let state = self.state_write();
        let store = state.store.clone();
        let key = pending_key(digest);
        let Some(transaction) = store.load::<SignedObjectTransaction>(&key)? else {
            return Ok(None);
        };
        let mut index = store.load::<Vec<Vec<u8>>>(INDEX_KEY)?.unwrap_or_default();
        index.retain(|entry| entry.as_slice() != digest);
        let mut locks: LockMap = store.load(LOCKS_KEY)?.unwrap_or_default();
        release_locks(&mut locks, &transaction, digest);
        let mut updates = vec![
            (INDEX_KEY.to_vec(), bcs::to_bytes(&index)?),
            (LOCKS_KEY.to_vec(), bcs::to_bytes(&locks)?),
        ];
        if executed {
            updates.push((executed_key(digest), bcs::to_bytes(&true)?));
        }
        store.apply_raw_changes(&updates, &[key])?;
        Ok(Some(transaction))
    }

    pub fn validate_object_reference(&self, reference: &ObjectRef) -> Result<()> {
        self.state_read().validate_object_ref_exact(reference)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_native_coin_balance() {
        let mut data = vec![0u8; UID_SIZE + U64_SIZE];
        data[UID_SIZE..].copy_from_slice(&55u64.to_le_bytes());
        assert_eq!(
            native_coin_balance("0x2::coin::Coin<0x2::kanari::KANARI>", &data).unwrap(),
            55
        );
    }
}
