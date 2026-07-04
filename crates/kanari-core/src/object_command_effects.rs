// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::engine::BlockchainEngine;
use anyhow::{Result, ensure};
use kanari_crypto::hash_data_blake3;
use kanari_types::object::{ObjectID, Owner};
use kanari_types::object_effects::{
    GasCostSummary, ObjectDelete, ObjectDeleteKind, ObjectTransactionEffectsV1, ObjectWrite,
    ObjectWriteKind, next_lamport_version,
};
use kanari_types::object_transaction::ObjectTransactionKind;
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use move_core_types::account_address::AccountAddress;
use std::collections::BTreeSet;

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
    fn build_shared_pay_gas_effects(
        &self,
        transaction: &SignedObjectTransaction,
        gas_used: u64,
    ) -> Result<ObjectTransactionEffectsV1> {
        let ObjectTransactionKind::Pay {
            coins,
            recipient,
            amount,
        } = &transaction.data.kind
        else {
            anyhow::bail!("Shared payment/gas effects require a Pay command");
        };
        ensure!(
            transaction.data.gas_data.owner == transaction.data.sender,
            "Shared payment/gas coins must be owned by the sender"
        );
        ensure!(
            gas_used <= transaction.data.gas_data.budget,
            "Gas usage exceeds transaction budget"
        );

        let pay_ids: BTreeSet<_> = coins.iter().map(|reference| reference.object_id).collect();
        let gas_ids: BTreeSet<_> = transaction
            .data
            .gas_data
            .payment
            .iter()
            .map(|reference| reference.object_id)
            .collect();
        ensure!(
            pay_ids == gas_ids,
            "Shared PaySui mode requires payment and gas to reference the same coin set"
        );
        for gas in &transaction.data.gas_data.payment {
            ensure!(
                coins.iter().any(|coin| coin == gas),
                "Pay and gas references disagree for object {}",
                gas.object_id
            );
        }

        let charged_amount = gas_used
            .checked_mul(transaction.data.gas_data.price)
            .ok_or_else(|| anyhow::anyhow!("Gas charge overflow"))?;
        let digest: [u8; 32] = transaction
            .digest()?
            .try_into()
            .map_err(|_| anyhow::anyhow!("Object transaction digest must contain 32 bytes"))?;
        let version = next_lamport_version(coins.iter().map(|reference| reference.version))?;
        let state = self.state_read();
        let mut loaded = Vec::with_capacity(coins.len());
        let mut total = 0u64;
        let mut coin_type = None;
        for reference in coins {
            let object =
                state.validate_address_owned_object_ref(reference, transaction.data.sender)?;
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

        let required = amount
            .checked_add(charged_amount)
            .ok_or_else(|| anyhow::anyhow!("Payment plus gas overflow"))?;
        ensure!(
            total > required,
            "Selected coin objects must retain a gas object after payment"
        );

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

        let (primary_ref, primary) = loaded.remove(0);
        let mut change_data = primary.data;
        set_balance(&mut change_data, total - required)?;
        let change_write = ObjectWrite::new(
            primary_ref.object_id,
            version,
            Some(primary_ref),
            Owner::AddressOwner(transaction.data.sender),
            primary.type_,
            change_data,
            digest,
            ObjectWriteKind::Mutated,
        )?;
        let mut effects = ObjectTransactionEffectsV1::new(
            digest,
            version,
            GasCostSummary {
                computation_cost: charged_amount,
                storage_cost: 0,
                storage_rebate: 0,
                non_refundable_storage_fee: 0,
            },
        );
        effects.gas_object = Some(change_write.object_ref);
        effects.mutated.push(change_write);
        effects.created.push(ObjectWrite::new(
            output,
            version,
            None,
            Owner::AddressOwner(*recipient),
            template.type_,
            output_data,
            digest,
            ObjectWriteKind::Created,
        )?);
        effects
            .deleted
            .extend(loaded.into_iter().map(|(object_ref, _)| ObjectDelete {
                object_ref,
                kind: ObjectDeleteKind::Deleted,
            }));
        effects.validate()?;
        Ok(effects)
    }

    pub fn build_object_command_effects(
        &self,
        transaction: &SignedObjectTransaction,
        gas_used: u64,
    ) -> Result<ObjectTransactionEffectsV1> {
        if let ObjectTransactionKind::Pay { coins, .. } = &transaction.data.kind
            && coins.iter().any(|coin| {
                transaction
                    .data
                    .gas_data
                    .payment
                    .iter()
                    .any(|gas| gas == coin)
            })
        {
            return self.build_shared_pay_gas_effects(transaction, gas_used);
        }

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
