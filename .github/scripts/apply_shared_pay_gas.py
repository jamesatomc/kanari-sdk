from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old in text:
        return text.replace(old, new, 1)
    if new in text:
        return text
    raise RuntimeError(f"missing shared gas marker: {label}")


def update_transaction_validation() -> None:
    path = Path("crates/kanari-types/src/object_transaction.rs")
    text = path.read_text()
    old = '''        for gas in &self.gas_data.payment {
            ensure!(
                ids.insert(gas.object_id),
                "Gas object {} is also used as a regular input",
                gas.object_id
            );
        }
'''
    new = '''        for gas in &self.gas_data.payment {
            if ids.insert(gas.object_id) {
                continue;
            }
            let shared_pay_coin = matches!(
                &self.kind,
                ObjectTransactionKind::Pay { coins, .. }
                    if coins.iter().any(|coin| coin == gas)
            );
            ensure!(
                shared_pay_coin,
                "Gas object {} is also used as a regular input",
                gas.object_id
            );
            ensure!(
                self.gas_data.owner == self.sender,
                "A sponsored gas object cannot also be a payment coin"
            );
        }
'''
    path.write_text(replace_once(text, old, new, "pay coin as gas validation"))


def update_admission_balance_validation() -> None:
    path = Path("crates/kanari-core/src/object_transaction_engine_v2.rs")
    text = path.read_text()
    old = '''    if let ObjectTransactionKind::Pay { coins, amount, .. } = &tx.data.kind {
        let mut available = 0u64;
        for reference in coins {
            let object = state.validate_address_owned_object_ref(reference, tx.data.sender)?;
            available = available
                .checked_add(native_coin_balance(&object.type_, &object.data)?)
                .ok_or_else(|| anyhow::anyhow!("Pay coin balance overflow"))?;
        }
        ensure!(
            available >= *amount,
            "Insufficient payment coin balance: need {}, found {}",
            amount,
            available
        );
    }

    let required = tx
        .data
        .gas_data
        .budget
        .checked_mul(tx.data.gas_data.price)
        .ok_or_else(|| anyhow::anyhow!("Gas fee overflow"))?;
    let mut available = 0u64;
    for payment in &tx.data.gas_data.payment {
        let object = state.validate_address_owned_object_ref(payment, tx.data.gas_data.owner)?;
        available = available
            .checked_add(native_coin_balance(&object.type_, &object.data)?)
            .ok_or_else(|| anyhow::anyhow!("Gas balance overflow"))?;
    }
    ensure!(
        available >= required,
        "Insufficient gas coin balance: need {required}, found {available}"
    );
'''
    new = '''    let mut pay_available = 0u64;
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
            ensure!(pay_ref == payment, "Pay and gas references disagree for the same object");
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
'''
    path.write_text(replace_once(text, old, new, "combined payment and gas admission"))


def replace_effect_builder() -> None:
    Path("crates/kanari-core/src/object_command_effects.rs").write_text(r'''// Copyright (c) KanariNetwork, Inc.
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
    ensure!(data.len() >= UID_SIZE + BALANCE_SIZE, "Malformed coin object");
    Ok(u64::from_le_bytes(
        data[UID_SIZE..UID_SIZE + BALANCE_SIZE].try_into()?,
    ))
}

fn set_balance(data: &mut [u8], value: u64) -> Result<()> {
    ensure!(data.len() >= UID_SIZE + BALANCE_SIZE, "Malformed coin object");
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
            ensure!(object.type_.contains("::coin::Coin<"), "Pay input is not a coin");
            if let Some(expected) = &coin_type {
                ensure!(expected == &object.type_, "Pay inputs use different coin types");
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
                    ensure!(object.type_.contains("::coin::Coin<"), "Pay input is not a coin");
                    if let Some(expected) = &coin_type {
                        ensure!(expected == &object.type_, "Pay inputs use different coin types");
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
''')


update_transaction_validation()
update_admission_balance_validation()
replace_effect_builder()
