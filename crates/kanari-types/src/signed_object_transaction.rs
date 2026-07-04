// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::address::Address;
use crate::object_transaction::ObjectTransactionData;
use anyhow::{Result, ensure};
use kanari_crypto::keys::CurveType;
use kanari_crypto::{hash_data_blake3, signatures::sign_message, verify_signature};
use serde::{Deserialize, Serialize};

const OBJECT_TX_INTENT: &[u8] = b"KANARI::OBJECT_TRANSACTION::V1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedObjectTransaction {
    pub data: ObjectTransactionData,
    /// Tagged signer string, for example `Ed25519:0x...`.
    pub sender_authenticator: String,
    pub sender_signature: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sponsor_authenticator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sponsor_signature: Option<Vec<u8>>,
}

impl SignedObjectTransaction {
    pub fn new(data: ObjectTransactionData, sender_authenticator: String) -> Result<Self> {
        data.validate()?;
        let signer = Address::parse_to_account_address(&sender_authenticator)?;
        ensure!(signer == data.sender, "Sender authenticator does not match sender address");
        Ok(Self {
            data,
            sender_authenticator,
            sender_signature: Vec::new(),
            sponsor_authenticator: None,
            sponsor_signature: None,
        })
    }

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

    pub fn sign_sponsor(
        &mut self,
        sponsor_authenticator: String,
        private_key: &str,
        curve_type: CurveType,
    ) -> Result<()> {
        ensure!(
            self.data.requires_sponsor_signature(),
            "Sponsor signature is unnecessary when sender owns the gas objects"
        );
        let sponsor = Address::parse_to_account_address(&sponsor_authenticator)?;
        ensure!(
            sponsor == self.data.gas_data.owner,
            "Sponsor authenticator does not match gas owner"
        );
        self.sponsor_signature = Some(
            sign_message(private_key, &self.digest()?, curve_type)
                .map_err(|error| anyhow::anyhow!("Failed to sign sponsored gas payment: {error}"))?,
        );
        self.sponsor_authenticator = Some(sponsor_authenticator);
        Ok(())
    }

    pub fn verify(&self) -> Result<()> {
        self.data.validate()?;
        ensure!(!self.sender_signature.is_empty(), "Missing sender signature");
        ensure!(
            Address::parse_to_account_address(&self.sender_authenticator)? == self.data.sender,
            "Sender authenticator does not match sender address"
        );

        let digest = self.digest()?;
        ensure!(
            verify_signature(&self.sender_authenticator, &digest, &self.sender_signature)
                .map_err(|error| anyhow::anyhow!("Sender signature verification failed: {error}"))?,
            "Invalid object transaction sender signature"
        );

        if self.data.requires_sponsor_signature() {
            let authenticator = self
                .sponsor_authenticator
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing gas sponsor authenticator"))?;
            ensure!(
                Address::parse_to_account_address(authenticator)? == self.data.gas_data.owner,
                "Sponsor authenticator does not match gas owner"
            );
            let signature = self
                .sponsor_signature
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing gas sponsor signature"))?;
            ensure!(
                verify_signature(authenticator, &digest, signature).map_err(|error| {
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
    fn rejects_authenticator_for_another_sender() {
        let sender = AccountAddress::from_hex_literal("0x1").unwrap();
        let data = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::MoveCall(MoveCall {
                package: ObjectID::from_hex_literal("0x2").unwrap(),
                module: "pay".to_string(),
                function: "transfer".to_string(),
                type_args: vec![],
                arguments: vec![CallArg::Object(ObjectArg::ImmOrOwnedObject(object_ref("0x10")))],
            }),
            GasData {
                payment: vec![object_ref("0x20")],
                owner: sender,
                price: 1,
                budget: 1_000,
            },
            TransactionExpiration::None,
        )
        .unwrap();

        assert!(SignedObjectTransaction::new(data, "Ed25519:0x2".to_string()).is_err());
    }
}
