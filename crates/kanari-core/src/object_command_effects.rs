// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::engine::BlockchainEngine;
use anyhow::{Result, ensure};
use kanari_crypto::hash_data_blake3;
use kanari_types::object::{ObjectID, Owner};
use kanari_types::object_effects::{
    ObjectDelete, ObjectDeleteKind, ObjectTransactionEffectsV1, ObjectWrite, ObjectWriteKind,
};
use kanari_types::object_transaction::ObjectTransactionKind;
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use move_core_types::account_address::AccountAddress;

const UID_SIZE: usize = 32;
const BALANCE_SIZE: usize = 8;

fn balance(data: &[u8]) -> Result<u64> {
    ensure!(
        data.len() >= UID_SIZE + BALANCE_SIZE,
        "Malformed coin object"
    );
    Ok(u64::from_le_bytes(
        data[UID_SIZE..UID_SIZE + BALANCE_SIZE].try_into()?,
    ))
}

fn set_balance(data: &mut [u8], value: u64) -> Result<()> {
    ensure!(
        data.len() >= UID_SIZE + BALANCE_SIZE,
        "Malformed coin object"
    );
    data[UID_SIZE..UID_SIZE + BALANCE_SIZE].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn output_id(transaction_digest: [u8; 32], index: u64) -> Result<ObjectID> {
    let mut seed = b"KANARI::PAY_OUTPUT::V1".to_vec();
    seed.extend_from_slice(&transaction_digest);
    seed.extend_from_slice(&index.to_le_bytes());
    Ok(ObjectID::new(AccountAddress::from_bytes(
        hash_data_blake3(&seed),
    )?))
}

impl BlockchainEngine {
    pub fn build_object_command_effects(
        &self,
        transaction: &SignedObjectTransaction,
        gas_used: u64,
    ) -> Result<ObjectTransactionEffectsV1> {
        let gas = self.plan_object_gas_charge(transaction, gas_used)?;
        let mut effects = gas.effects;
        let version = effects.lamport_version;
        let digest = effects.transaction_digest;

        match &transaction.data.kind {
            ObjectTransactionKind::Pay {
                coins,
                recipient,
                amount,
            } => {
                let state = self.state_read();
                let mut loaded = Vec::with_capacity(coins.len());
                let mut total = 0u64;
                let mut coin_type = None;
                for reference in coins {
                    let object = state
                        .validate_address_owned_object_ref(reference, transaction.data.sender)?;
                    ensure!(
                        object.type_.contains("::coin::Coin<"),
                        "Pay input is not a coin"
                    );
                    if let Some(expected) = &coin_type {
                        ensure!(
                            expected == &object.type_,
                            "Pay inputs use different coin types"
                        );
                    } else {
                        coin_type = Some(object.type_.clone());
                    }
                    total = total
                        .checked_add(balance(&object.data)?)
                        .ok_or_else(|| anyhow::anyhow!("Coin balance overflow"))?;
                    loaded.push((*reference, object));
                }
                ensure!(total >= *amount, "Insufficient selected coin balance");

                let output = output_id(digest, 0)?;
                ensure!(
                    !state.object_id_has_history(output)?,
                    "Pay output object ID has already been used"
                );
                let (_, template) = loaded
                    .first()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Pay requires a coin input"))?;
                let mut output_data = template.data.clone();
                output_data[..UID_SIZE].copy_from_slice(output.as_bytes());
                set_balance(&mut output_data, *amount)?;
                effects.created.push(ObjectWrite::new(
                    output,
                    version,
                    None,
                    Owner::AddressOwner(*recipient),
                    template.type_.clone(),
                    output_data,
                    digest,
                    ObjectWriteKind::Created,
                )?);

                let change = total - *amount;
                let (first_ref, first) = loaded.remove(0);
                if change == 0 {
                    effects.deleted.push(ObjectDelete {
                        object_ref: first_ref,
                        kind: ObjectDeleteKind::Deleted,
                    });
                } else {
                    let mut data = first.data;
                    set_balance(&mut data, change)?;
                    effects.mutated.push(ObjectWrite::new(
                        first_ref.object_id,
                        version,
                        Some(first_ref),
                        Owner::AddressOwner(transaction.data.sender),
                        first.type_,
                        data,
                        digest,
                        ObjectWriteKind::Mutated,
                    )?);
                }
                effects
                    .deleted
                    .extend(loaded.into_iter().map(|(reference, _)| ObjectDelete {
                        object_ref: reference,
                        kind: ObjectDeleteKind::Deleted,
                    }));
            }
            ObjectTransactionKind::TransferObjects { objects, recipient } => {
                let state = self.state_read();
                for reference in objects {
                    let object = state
                        .validate_address_owned_object_ref(reference, transaction.data.sender)?;
                    effects.mutated.push(ObjectWrite::new(
                        reference.object_id,
                        version,
                        Some(*reference),
                        Owner::AddressOwner(*recipient),
                        object.type_,
                        object.data,
                        digest,
                        ObjectWriteKind::Mutated,
                    )?);
                }
            }
            _ => anyhow::bail!("Transaction kind is not a direct object command"),
        }
        effects.validate()?;
        Ok(effects)
    }
}
