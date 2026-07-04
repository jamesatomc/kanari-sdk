// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Signatures for object-centric transactions.
//!
//! Sponsored transactions carry two independent authorizations:
//! - the sender authorizes use of the regular input objects;
//! - the gas owner authorizes use of the gas payment objects.

use crate::object_transaction::ObjectTransactionData;
use anyhow::{Result, ensure};
use kanari_crypto::keys::CurveType;
use kanari_crypto::{hash_data_blake3, signatures::sign_message, verify_signature};
use serde::{Deserialize, Serialize};

const OBJECT_TX_INTENT: &[u8] = b"KANARI::OBJECT_TRANSACTION::V1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedObjectTransaction {
    pub data: ObjectTransactionData,
    pub sender_signature: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sponsor_signature: Option<Vec<u8>>,
}

impl SignedObjectTransaction {
    pub fn new(data: ObjectTransactionData) -> Result<Self> {
        data.validate()?;
        Ok(Self {
            data,
            sender_signature: Vec::new(),
            sponsor_signature: None,
        })
    }

    /// Digest signed by both sender and sponsor. Domain separation prevents an
    /// object transaction signature from being reused for a legacy transaction.
    pub fn digest(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::from(OBJECT_TX_INTENT);
        bytes.extend_from_slice(&bcs::to_bytes(&self.data)?);
        Ok(hash_data_blake3(&bytes))
    }

    pub fn sign_sender(&mut self, private_key: &str, curve_type: CurveType) -> Result<()> {
        self.sender_signature = sign_message(private_key, &self.digest()?, curve_type)
            .map_err(|error| anyhow::anyhow!("Failed to sign object transaction: {error}"))?;
        Ok(())
    }

    pub fn sign_sponsor(&mut self, private_key: &str, curve_type: CurveType) -> Result<()> {
        ensure!(
            self.data.requires_sponsor_signature(),
            "Sponsor signature is unnecessary when sender owns the gas objects"
        );
        self.sponsor_signature = Some(
            sign_message(private_key, &self.digest()?, curve_type)
                .map_err(|error| anyhow::anyhow!("Failed to sign sponsored gas payment: {error}"))?,
        );
        Ok(())
    }

    pub fn verify(&self) -> Result<()> {
        self.data.validate()?;
        ensure!(
            !self.sender_signature.is_empty(),
            "Missing object transaction sender signature"
        );

        let digest = self.digest()?;
        let sender = self.data.sender.to_hex_literal();
        ensure!(
            verify_signature(&sender, &digest, &self.sender_signature)
                .map_err(|error| anyhow::anyhow!("Sender signature verification failed: {error}"))?,
            "Invalid object transaction sender signature"
        );

        if self.data.requires_sponsor_signature() {
            let signature = self
                .sponsor_signature
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing gas sponsor signature"))?;
            let sponsor = self.data.gas_data.owner.to_hex_literal();
            ensure!(
                verify_signature(&sponsor, &digest, signature).map_err(|error| {
                    anyhow::anyhow!("Gas sponsor signature verification failed: {error}")
                })?,
                "Invalid gas sponsor signature"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{ObjectDigest, ObjectID, ObjectRef};
    use crate::object_transaction::{
        CallArg, GasData, MoveCall, ObjectArg, ObjectTransactionKind, TransactionExpiration,
    };
    use move_core_types::account_address::AccountAddress;

    fn object_ref(id: &str) -> ObjectRef {
        ObjectRef::new(
            ObjectID::from_hex_literal(id).unwrap(),
            1,
            ObjectDigest([1; 32]),
        )
    }

    #[test]
    fn sponsored_transaction_requires_both_signatures() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let sponsor = AccountAddress::from_hex_literal("0x2").unwrap();
        let data = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::MoveCall(MoveCall {
                package: ObjectID::from_hex_literal("0x2").unwrap(),
                module: "pay".to_string(),
                function: "transfer".to_string(),
                type_args: vec![],
                arguments: vec![CallArg::Object(ObjectArg::ImmOrOwnedObject(object_ref(
                    "0x10",
                )))],
            }),
            GasData {
                payment: vec![object_ref("0x20")],
                owner: sponsor,
                price: 1,
                budget: 1_000,
            },
            TransactionExpiration::None,
        )
        .unwrap();

        let transaction = SignedObjectTransaction::new(data).unwrap();
        assert!(transaction.verify().is_err());
        assert!(transaction.data.requires_sponsor_signature());
    }
}
