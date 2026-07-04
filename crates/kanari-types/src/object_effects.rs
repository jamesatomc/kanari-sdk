// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Canonical effects emitted by object-centric execution.

use crate::object::{
    ObjectID, ObjectRef, ObjectVersion, Owner, compute_object_digest,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectWriteKind {
    Created,
    Mutated,
    Unwrapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectDeleteKind {
    Deleted,
    Wrapped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectWrite {
    pub object_ref: ObjectRef,
    pub owner: Owner,
    pub type_name: String,
    pub contents: Vec<u8>,
    pub previous_transaction: [u8; 32],
    pub kind: ObjectWriteKind,
}

impl ObjectWrite {
    pub fn new(
        id: ObjectID,
        version: ObjectVersion,
        owner: Owner,
        type_name: String,
        contents: Vec<u8>,
        previous_transaction: [u8; 32],
        kind: ObjectWriteKind,
    ) -> Result<Self> {
        let digest = compute_object_digest(
            id,
            version,
            &owner,
            &type_name,
            &contents,
            Some(previous_transaction),
        )?;
        Ok(Self {
            object_ref: ObjectRef::new(id, version, digest),
            owner,
            type_name,
            contents,
            previous_transaction,
            kind,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectDelete {
    pub object_ref: ObjectRef,
    pub kind: ObjectDeleteKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GasCostSummary {
    pub computation_cost: u64,
    pub storage_cost: u64,
    pub storage_rebate: u64,
    pub non_refundable_storage_fee: u64,
}

impl GasCostSummary {
    pub fn net_gas_usage(&self) -> Result<u64> {
        self.computation_cost
            .checked_add(self.storage_cost)
            .and_then(|cost| cost.checked_sub(self.storage_rebate))
            .ok_or_else(|| anyhow::anyhow!("Gas cost summary overflow or rebate exceeds cost"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    Success,
    Failure { error: String, command: Option<u64> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectTransactionEffectsV1 {
    pub transaction_digest: [u8; 32],
    pub lamport_version: ObjectVersion,
    pub status: ExecutionStatus,
    pub gas_cost_summary: GasCostSummary,
    pub gas_object: Option<ObjectRef>,
    pub created: Vec<ObjectWrite>,
    pub mutated: Vec<ObjectWrite>,
    pub deleted: Vec<ObjectDelete>,
}

impl ObjectTransactionEffectsV1 {
    pub fn new(
        transaction_digest: [u8; 32],
        lamport_version: ObjectVersion,
        gas_cost_summary: GasCostSummary,
    ) -> Self {
        Self {
            transaction_digest,
            lamport_version,
            status: ExecutionStatus::Success,
            gas_cost_summary,
            gas_object: None,
            created: Vec::new(),
            mutated: Vec::new(),
            deleted: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<()> {
        let mut ids = std::collections::BTreeSet::new();
        for write in self.created.iter().chain(self.mutated.iter()) {
            ensure!(
                write.object_ref.version == self.lamport_version,
                "Written object {} does not use transaction Lamport version",
                write.object_ref.object_id
            );
            ensure!(
                ids.insert(write.object_ref.object_id),
                "Object {} appears more than once in effects",
                write.object_ref.object_id
            );
        }
        for deleted in &self.deleted {
            ensure!(
                ids.insert(deleted.object_ref.object_id),
                "Deleted object {} also appears as a write",
                deleted.object_ref.object_id
            );
        }
        if let Some(gas_object) = self.gas_object {
            ensure!(
                self.mutated
                    .iter()
                    .any(|write| write.object_ref == gas_object),
                "Gas object reference is not present in mutated effects"
            );
        }
        Ok(())
    }
}

pub fn next_lamport_version(
    inputs: impl IntoIterator<Item = ObjectVersion>,
) -> Result<ObjectVersion> {
    inputs
        .into_iter()
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("Object Lamport version overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamport_version_uses_highest_mutable_input() {
        assert_eq!(next_lamport_version([2, 9, 4]).unwrap(), 10);
        assert_eq!(next_lamport_version([]).unwrap(), 1);
    }

    #[test]
    fn write_digest_commits_previous_transaction() {
        let id = ObjectID::from_hex_literal("0x42").unwrap();
        let owner = Owner::Immutable;
        let first = ObjectWrite::new(
            id,
            2,
            owner.clone(),
            "0x2::package::Package".to_string(),
            vec![1, 2],
            [1; 32],
            ObjectWriteKind::Mutated,
        )
        .unwrap();
        let second = ObjectWrite::new(
            id,
            2,
            owner,
            "0x2::package::Package".to_string(),
            vec![1, 2],
            [2; 32],
            ObjectWriteKind::Mutated,
        )
        .unwrap();
        assert_ne!(first.object_ref.digest, second.object_ref.digest);
    }
}
