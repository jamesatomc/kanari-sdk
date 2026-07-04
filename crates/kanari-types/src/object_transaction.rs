// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Object-centric transaction envelope.
//!
//! This module intentionally has no account sequence number. Replay and
//! conflict protection come from exact object references and transaction
//! digests. Direct payment and transfer commands also carry every consumed
//! object reference inside the signed payload.

use crate::object::{ObjectID, ObjectRef, ObjectVersion};
use anyhow::{Result, ensure};
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObjectArg {
    ImmOrOwnedObject(ObjectRef),
    SharedObject {
        id: ObjectID,
        initial_shared_version: ObjectVersion,
        mutable: bool,
    },
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
    /// Split `amount` from the explicitly supplied native coin objects and
    /// transfer the new coin to `recipient`.
    Pay {
        coins: Vec<ObjectRef>,
        recipient: AccountAddress,
        amount: u64,
    },
    /// Transfer ownership of every explicitly supplied object.
    TransferObjects {
        objects: Vec<ObjectRef>,
        recipient: AccountAddress,
    },
}

impl ObjectTransactionKind {
    pub fn object_arguments(&self) -> impl Iterator<Item = &ObjectArg> {
        let arguments = match self {
            Self::MoveCall(call) => Some(call.arguments.as_slice()),
            _ => None,
        };
        arguments
            .into_iter()
            .flatten()
            .filter_map(|argument| match argument {
                CallArg::Object(object) => Some(object),
                CallArg::Pure(_) => None,
            })
    }

    pub fn direct_refs(&self) -> &[ObjectRef] {
        match self {
            Self::Pay { coins, .. } => coins,
            Self::TransferObjects { objects, .. } => objects,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TransactionExpiration {
    #[default]
    None,
    Epoch(u64),
}

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
        ensure!(
            !self.payment.is_empty(),
            "At least one gas object is required"
        );
        let ids: BTreeSet<_> = self
            .payment
            .iter()
            .map(|reference| reference.object_id)
            .collect();
        ensure!(
            ids.len() == self.payment.len(),
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

    /// Every owned object reference consumed by the command, excluding gas.
    pub fn owned_input_refs(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        self.input_objects()
            .filter_map(ObjectArg::object_ref)
            .chain(self.kind.direct_refs().iter().copied())
    }

    pub fn mutable_input_ids(&self) -> BTreeSet<ObjectID> {
        self.input_objects()
            .filter(|object| object.is_mutable())
            .map(ObjectArg::object_id)
            .chain(
                self.kind
                    .direct_refs()
                    .iter()
                    .map(|reference| reference.object_id),
            )
            .chain(
                self.gas_data
                    .payment
                    .iter()
                    .map(|reference| reference.object_id),
            )
            .collect()
    }

    pub fn read_input_ids(&self) -> BTreeSet<ObjectID> {
        self.input_objects()
            .map(ObjectArg::object_id)
            .chain(
                self.kind
                    .direct_refs()
                    .iter()
                    .map(|reference| reference.object_id),
            )
            .collect()
    }

    pub fn requires_consensus(&self) -> bool {
        self.input_objects().any(ObjectArg::requires_consensus)
    }

    pub fn requires_sponsor_signature(&self) -> bool {
        self.gas_data.is_sponsored_for(self.sender)
    }

    pub fn validate(&self) -> Result<()> {
        self.gas_data.validate()?;

        let mut ids = BTreeSet::new();
        for object_id in self.read_input_ids() {
            ensure!(
                ids.insert(object_id),
                "Object {} appears more than once in transaction inputs",
                object_id
            );
        }
        for gas in &self.gas_data.payment {
            ensure!(
                ids.insert(gas.object_id),
                "Gas object {} is also used as a regular input",
                gas.object_id
            );
        }

        match &self.kind {
            ObjectTransactionKind::MoveCall(call) => {
                ensure!(!call.module.is_empty(), "Move module name cannot be empty");
                ensure!(
                    !call.function.is_empty(),
                    "Move function name cannot be empty"
                );
            }
            ObjectTransactionKind::Publish { modules, .. } => {
                ensure!(!modules.is_empty(), "Publish transaction has no modules");
                ensure!(
                    modules.iter().all(|module| !module.is_empty()),
                    "Published module bytes cannot be empty"
                );
            }
            ObjectTransactionKind::Pay { coins, amount, .. } => {
                ensure!(!coins.is_empty(), "Pay command requires at least one coin");
                ensure!(*amount > 0, "Pay amount must be greater than zero");
            }
            ObjectTransactionKind::TransferObjects { objects, .. } => {
                ensure!(
                    !objects.is_empty(),
                    "TransferObjects requires at least one object"
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

    fn gas(owner: AccountAddress) -> GasData {
        GasData {
            payment: vec![object_ref("0x20", 3)],
            owner,
            price: 1,
            budget: 1_000,
        }
    }

    #[test]
    fn direct_pay_coins_are_signed_inputs() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let coin = object_ref("0x10", 7);
        let transaction = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::Pay {
                coins: vec![coin],
                recipient: AccountAddress::from_hex_literal("0x2").unwrap(),
                amount: 10,
            },
            gas(sender),
            TransactionExpiration::None,
        )
        .unwrap();

        assert_eq!(
            transaction.owned_input_refs().collect::<Vec<_>>(),
            vec![coin]
        );
        assert!(transaction.mutable_input_ids().contains(&coin.object_id));
    }

    #[test]
    fn rejects_gas_object_reused_as_pay_coin() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let duplicate = object_ref("0x20", 3);
        let transaction = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::Pay {
                coins: vec![duplicate],
                recipient: AccountAddress::from_hex_literal("0x2").unwrap(),
                amount: 10,
            },
            GasData {
                payment: vec![duplicate],
                owner: sender,
                price: 1,
                budget: 100,
            },
            TransactionExpiration::None,
        );
        assert!(transaction.is_err());
    }
}
