// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Object-centric transaction envelope.
//!
//! This module intentionally does not contain an account nonce. Replay and
//! conflict protection come from exact object references and the transaction
//! digest. The gas owner is independent from the sender so applications can
//! sponsor user transactions without transferring custody of user objects.

use crate::object::{ObjectID, ObjectRef, ObjectVersion};
use anyhow::{Result, ensure};
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectArg {
    /// Address-owned or immutable input with an exact version and digest.
    ImmOrOwnedObject(ObjectRef),
    /// Consensus-ordered object. The initial version identifies the shared
    /// object's creation state and remains stable across later mutations.
    SharedObject {
        id: ObjectID,
        initial_shared_version: ObjectVersion,
        mutable: bool,
    },
    /// Object that may be received through a parent object in this transaction.
    Receiving(ObjectRef),
}

impl ObjectArg {
    pub fn object_id(&self) -> ObjectID {
        match self {
            Self::ImmOrOwnedObject(reference) | Self::Receiving(reference) => reference.object_id,
            Self::SharedObject { id, .. } => *id,
        }
    }

    pub fn object_ref(&self) -> Option<ObjectRef> {
        match self {
            Self::ImmOrOwnedObject(reference) | Self::Receiving(reference) => Some(*reference),
            Self::SharedObject { .. } => None,
        }
    }

    pub fn is_mutable(&self) -> bool {
        match self {
            Self::ImmOrOwnedObject(_) | Self::Receiving(_) => true,
            Self::SharedObject { mutable, .. } => *mutable,
        }
    }

    pub fn requires_consensus(&self) -> bool {
        matches!(self, Self::SharedObject { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CallArg {
    Pure(Vec<u8>),
    Object(ObjectArg),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MoveCall {
    pub package: ObjectID,
    pub module: String,
    pub function: String,
    pub type_args: Vec<String>,
    pub arguments: Vec<CallArg>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectTransactionKind {
    MoveCall(MoveCall),
    Publish {
        modules: Vec<Vec<u8>>,
        dependencies: Vec<ObjectID>,
    },
}

impl ObjectTransactionKind {
    pub fn object_arguments(&self) -> impl Iterator<Item = &ObjectArg> {
        let args = match self {
            Self::MoveCall(call) => Some(call.arguments.as_slice()),
            Self::Publish { .. } => None,
        };
        args.into_iter()
            .flatten()
            .filter_map(|arg| match arg {
                CallArg::Object(object) => Some(object),
                CallArg::Pure(_) => None,
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TransactionExpiration {
    #[default]
    None,
    Epoch(u64),
}

/// Gas payment data. `owner` may differ from the transaction sender.
///
/// This separation is the basis for Kanari's sponsored/gasless user flow: the
/// application or service owns and signs for gas objects while the user signs
/// only the requested object operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GasData {
    pub payment: Vec<ObjectRef>,
    pub owner: AccountAddress,
    pub price: u64,
    pub budget: u64,
}

impl GasData {
    pub fn is_sponsored_for(&self, sender: AccountAddress) -> bool {
        self.owner != sender
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.budget > 0, "Gas budget must be greater than zero");
        ensure!(!self.payment.is_empty(), "At least one gas object is required");

        let unique: BTreeSet<_> = self
            .payment
            .iter()
            .map(|reference| reference.object_id)
            .collect();
        ensure!(
            unique.len() == self.payment.len(),
            "A gas object may only appear once"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectTransactionData {
    pub sender: AccountAddress,
    pub kind: ObjectTransactionKind,
    pub gas_data: GasData,
    #[serde(default)]
    pub expiration: TransactionExpiration,
}

impl ObjectTransactionData {
    pub fn new(
        sender: AccountAddress,
        kind: ObjectTransactionKind,
        gas_data: GasData,
        expiration: TransactionExpiration,
    ) -> Result<Self> {
        let transaction = Self {
            sender,
            kind,
            gas_data,
            expiration,
        };
        transaction.validate()?;
        Ok(transaction)
    }

    pub fn input_objects(&self) -> impl Iterator<Item = &ObjectArg> {
        self.kind.object_arguments()
    }

    pub fn owned_input_refs(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        self.input_objects().filter_map(ObjectArg::object_ref)
    }

    pub fn mutable_input_ids(&self) -> BTreeSet<ObjectID> {
        self.input_objects()
            .filter(|input| input.is_mutable())
            .map(ObjectArg::object_id)
            .chain(
                self.gas_data
                    .payment
                    .iter()
                    .map(|reference| reference.object_id),
            )
            .collect()
    }

    pub fn read_input_ids(&self) -> BTreeSet<ObjectID> {
        self.input_objects().map(ObjectArg::object_id).collect()
    }

    pub fn requires_consensus(&self) -> bool {
        self.input_objects().any(ObjectArg::requires_consensus)
    }

    pub fn requires_sponsor_signature(&self) -> bool {
        self.gas_data.is_sponsored_for(self.sender)
    }

    pub fn validate(&self) -> Result<()> {
        self.gas_data.validate()?;

        let mut all_ids = BTreeSet::new();
        for input in self.input_objects() {
            ensure!(
                all_ids.insert(input.object_id()),
                "Object {} appears more than once in transaction inputs",
                input.object_id()
            );
        }

        for gas in &self.gas_data.payment {
            ensure!(
                all_ids.insert(gas.object_id),
                "Gas object {} is also used as a regular input",
                gas.object_id
            );
        }

        match &self.kind {
            ObjectTransactionKind::MoveCall(call) => {
                ensure!(!call.module.is_empty(), "Move module name cannot be empty");
                ensure!(!call.function.is_empty(), "Move function name cannot be empty");
            }
            ObjectTransactionKind::Publish { modules, .. } => {
                ensure!(!modules.is_empty(), "Publish transaction has no modules");
                ensure!(
                    modules.iter().all(|module| !module.is_empty()),
                    "Published module bytes cannot be empty"
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{ObjectDigest, ObjectID};

    fn object_ref(id: &str, version: u64) -> ObjectRef {
        ObjectRef::new(
            ObjectID::from_hex_literal(id).unwrap(),
            version,
            ObjectDigest([version as u8; 32]),
        )
    }

    #[test]
    fn transaction_has_no_account_nonce() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let sponsor = AccountAddress::from_hex_literal("0x2").unwrap();
        let transaction = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::MoveCall(MoveCall {
                package: ObjectID::from_hex_literal("0x2").unwrap(),
                module: "pay".to_string(),
                function: "transfer".to_string(),
                type_args: vec![],
                arguments: vec![CallArg::Object(ObjectArg::ImmOrOwnedObject(object_ref(
                    "0x10", 7,
                )))],
            }),
            GasData {
                payment: vec![object_ref("0x20", 3)],
                owner: sponsor,
                price: 1,
                budget: 1_000_000,
            },
            TransactionExpiration::None,
        )
        .unwrap();

        assert!(transaction.requires_sponsor_signature());
        assert!(!transaction.requires_consensus());
        assert_eq!(transaction.mutable_input_ids().len(), 2);
    }

    #[test]
    fn rejects_duplicate_object_dependency() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let duplicate = object_ref("0x10", 1);
        let result = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::MoveCall(MoveCall {
                package: ObjectID::from_hex_literal("0x2").unwrap(),
                module: "example".to_string(),
                function: "call".to_string(),
                type_args: vec![],
                arguments: vec![CallArg::Object(ObjectArg::ImmOrOwnedObject(duplicate))],
            }),
            GasData {
                payment: vec![duplicate],
                owner: sender,
                price: 1,
                budget: 100,
            },
            TransactionExpiration::None,
        );
        assert!(result.is_err());
    }
}
