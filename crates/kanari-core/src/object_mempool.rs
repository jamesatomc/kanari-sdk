// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Deterministic mempool for object-centric transactions.
//!
//! Unlike the legacy sender/sequence queue, this pool admits transactions by
//! exact object dependencies and reserves mutable object IDs. Two unrelated
//! senders can therefore execute concurrently, while conflicting writes are
//! rejected before consensus/execution.

use anyhow::{Result, ensure};
use kanari_types::object::ObjectID;
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default)]
pub struct ObjectMempool {
    pending: BTreeMap<Vec<u8>, SignedObjectTransaction>,
    mutable_locks: BTreeMap<ObjectID, Vec<u8>>,
}

impl ObjectMempool {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn contains(&self, digest: &[u8]) -> bool {
        self.pending.contains_key(digest)
    }

    /// Verify signatures and reserve every mutable input object.
    pub fn admit(&mut self, transaction: SignedObjectTransaction) -> Result<Vec<u8>> {
        transaction.verify()?;
        self.admit_verified(transaction)
    }

    /// Admit a transaction after an authority has already verified signatures
    /// and exact object references against its state snapshot.
    pub fn admit_verified(
        &mut self,
        transaction: SignedObjectTransaction,
    ) -> Result<Vec<u8>> {
        transaction.data.validate()?;
        let digest = transaction.digest()?;
        ensure!(
            !self.pending.contains_key(&digest),
            "Object transaction is already pending"
        );

        let mutable_ids = transaction.data.mutable_input_ids();
        for object_id in &mutable_ids {
            if let Some(owner_digest) = self.mutable_locks.get(object_id) {
                anyhow::bail!(
                    "Mutable object {} is already reserved by transaction {}",
                    object_id,
                    hex::encode(owner_digest)
                );
            }
        }

        for object_id in mutable_ids {
            self.mutable_locks.insert(object_id, digest.clone());
        }
        self.pending.insert(digest.clone(), transaction);
        Ok(digest)
    }

    pub fn get(&self, digest: &[u8]) -> Option<&SignedObjectTransaction> {
        self.pending.get(digest)
    }

    pub fn remove(&mut self, digest: &[u8]) -> Option<SignedObjectTransaction> {
        let transaction = self.pending.remove(digest)?;
        for object_id in transaction.data.mutable_input_ids() {
            if self
                .mutable_locks
                .get(&object_id)
                .is_some_and(|owner| owner.as_slice() == digest)
            {
                self.mutable_locks.remove(&object_id);
            }
        }
        Some(transaction)
    }

    pub fn locked_objects(&self) -> BTreeSet<ObjectID> {
        self.mutable_locks.keys().copied().collect()
    }

    /// Return a deterministic snapshot ordered by transaction digest.
    pub fn snapshot(&self) -> Vec<SignedObjectTransaction> {
        self.pending.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanari_types::object::{ObjectDigest, ObjectID, ObjectRef};
    use kanari_types::object_transaction::{
        CallArg, GasData, MoveCall, ObjectArg, ObjectTransactionData, ObjectTransactionKind,
        TransactionExpiration,
    };
    use move_core_types::account_address::AccountAddress;

    fn object_ref(id: &str, version: u64) -> ObjectRef {
        ObjectRef::new(
            ObjectID::from_hex_literal(id).unwrap(),
            version,
            ObjectDigest([version as u8; 32]),
        )
    }

    fn transaction(input: &str, gas: &str) -> SignedObjectTransaction {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let data = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::MoveCall(MoveCall {
                package: ObjectID::from_hex_literal("0x2").unwrap(),
                module: "pay".to_string(),
                function: "transfer".to_string(),
                type_args: vec![],
                arguments: vec![CallArg::Object(ObjectArg::ImmOrOwnedObject(object_ref(
                    input, 1,
                )))],
            }),
            GasData {
                payment: vec![object_ref(gas, 1)],
                owner: sender,
                price: 1,
                budget: 1_000,
            },
            TransactionExpiration::None,
        )
        .unwrap();
        SignedObjectTransaction::new(data).unwrap()
    }

    #[test]
    fn unrelated_objects_can_be_pending_together() {
        let mut pool = ObjectMempool::new();
        pool.admit_verified(transaction("0x10", "0x20")).unwrap();
        pool.admit_verified(transaction("0x11", "0x21")).unwrap();
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn conflicting_mutable_object_is_rejected() {
        let mut pool = ObjectMempool::new();
        let first = pool
            .admit_verified(transaction("0x10", "0x20"))
            .unwrap();
        let conflict = pool.admit_verified(transaction("0x10", "0x21"));
        assert!(conflict.is_err());

        pool.remove(&first).unwrap();
        pool.admit_verified(transaction("0x10", "0x21")).unwrap();
    }
}
