// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Explicit gas-object planning for object-centric transactions.
//!
//! Every gas coin consumed here appears in `GasData.payment` and therefore in
//! the signed transaction. The planner never searches an owner's other coins.

use crate::engine::BlockchainEngine;
use anyhow::{Context, Result, ensure};
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use kanari_types::object::Owner;
use kanari_types::object_effects::{
    GasCostSummary, ObjectDelete, ObjectDeleteKind, ObjectTransactionEffectsV1, ObjectWrite,
    ObjectWriteKind, next_lamport_version,
};
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use move_core_types::language_storage::{StructTag, TypeTag};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

const UID_SIZE: usize = 32;
const U64_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectGasPlan {
    pub charged_amount: u64,
    pub remaining_balance: u64,
    pub effects: ObjectTransactionEffectsV1,
}

fn parse_native_coin(type_name: &str, data: &[u8]) -> Result<u64> {
    let coin = StructTag::from_str(type_name)
        .with_context(|| format!("Invalid gas coin type: {type_name}"))?;
    ensure!(
        coin.module.as_str() == "coin" && coin.name.as_str() == "Coin",
        "Gas payment must be a Coin<KANARI> object"
    );
    let expected = StructTag::from_str(KANARI_TOKEN_TYPE)?;
    let Some(TypeTag::Struct(token)) = coin.type_params.first() else {
        anyhow::bail!("Gas coin is missing its token type");
    };
    ensure!(
        token.as_ref() == &expected,
        "Gas payment must contain native KANARI"
    );
    ensure!(
        data.len() >= UID_SIZE + U64_SIZE,
        "Gas coin object has malformed contents"
    );
    Ok(u64::from_le_bytes(
        data[UID_SIZE..UID_SIZE + U64_SIZE].try_into()?,
    ))
}

fn write_native_coin_balance(data: &mut [u8], amount: u64) -> Result<()> {
    ensure!(
        data.len() >= UID_SIZE + U64_SIZE,
        "Gas coin object has malformed contents"
    );
    data[UID_SIZE..UID_SIZE + U64_SIZE].copy_from_slice(&amount.to_le_bytes());
    Ok(())
}

fn digest_array(transaction: &SignedObjectTransaction) -> Result<[u8; 32]> {
    transaction
        .digest()?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Object transaction digest must contain 32 bytes"))
}

impl BlockchainEngine {
    /// Build deterministic gas effects without mutating state.
    ///
    /// Multiple explicitly supplied gas coins are smashed into the first coin.
    /// The remaining gas coins are deleted. All writes use one Lamport version
    /// derived from the highest mutable input version in the transaction.
    pub fn plan_object_gas_charge(
        &self,
        transaction: &SignedObjectTransaction,
        gas_used: u64,
    ) -> Result<ObjectGasPlan> {
        transaction.verify()?;
        ensure!(
            gas_used <= transaction.data.gas_data.budget,
            "Gas usage {gas_used} exceeds budget {}",
            transaction.data.gas_data.budget
        );

        let charged_amount = gas_used
            .checked_mul(transaction.data.gas_data.price)
            .ok_or_else(|| anyhow::anyhow!("Gas charge overflow"))?;
        let transaction_digest = digest_array(transaction)?;
        let gas_owner = transaction.data.gas_data.owner;
        let lamport_version = next_lamport_version(
            transaction
                .data
                .owned_input_refs()
                .map(|reference| reference.version)
                .chain(
                    transaction
                        .data
                        .gas_data
                        .payment
                        .iter()
                        .map(|reference| reference.version),
                ),
        )?;

        let state = self.state_read();
        let mut gas_objects = Vec::with_capacity(transaction.data.gas_data.payment.len());
        let mut total_balance = 0u64;
        for reference in &transaction.data.gas_data.payment {
            let object = state.validate_address_owned_object_ref(reference, gas_owner)?;
            total_balance = total_balance
                .checked_add(parse_native_coin(&object.type_, &object.data)?)
                .ok_or_else(|| anyhow::anyhow!("Combined gas balance overflow"))?;
            gas_objects.push((*reference, object));
        }
        ensure!(
            total_balance >= charged_amount,
            "Insufficient gas balance: need {charged_amount}, found {total_balance}"
        );

        let remaining_balance = total_balance - charged_amount;
        let (primary_ref, primary_object) = gas_objects
            .first()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("At least one gas object is required"))?;
        let mut primary_contents = primary_object.data;
        write_native_coin_balance(&mut primary_contents, remaining_balance)?;

        let primary_write = ObjectWrite::new(
            primary_ref.object_id,
            lamport_version,
            Some(primary_ref),
            Owner::AddressOwner(gas_owner),
            primary_object.type_,
            primary_contents,
            transaction_digest,
            ObjectWriteKind::Mutated,
        )?;

        let mut effects = ObjectTransactionEffectsV1::new(
            transaction_digest,
            lamport_version,
            GasCostSummary {
                computation_cost: charged_amount,
                storage_cost: 0,
                storage_rebate: 0,
                non_refundable_storage_fee: 0,
            },
        );
        effects.gas_object = Some(primary_write.object_ref);
        effects.mutated.push(primary_write);
        effects
            .deleted
            .extend(
                gas_objects
                    .into_iter()
                    .skip(1)
                    .map(|(object_ref, _)| ObjectDelete {
                        object_ref,
                        kind: ObjectDeleteKind::Deleted,
                    }),
            );
        effects.validate()?;

        Ok(ObjectGasPlan {
            charged_amount,
            remaining_balance,
            effects,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_native_coin_balance_in_place() {
        let mut data = vec![0u8; UID_SIZE + U64_SIZE];
        write_native_coin_balance(&mut data, 123).unwrap();
        assert_eq!(
            parse_native_coin("0x2::coin::Coin<0x2::kanari::KANARI>", &data).unwrap(),
            123
        );
    }
}
