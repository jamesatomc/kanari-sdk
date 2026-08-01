// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, bail};
use kanari_crypto::wallet::Wallet;
use kanari_rpc_api::{BuildCallFunctionRequest, ObjectInfo};
use kanari_rpc_client::RpcClient;
use kanari_types::coin::CoinModule;
use kanari_types::gas_coin::GAS_COIN;
use kanari_types::transaction::{GasPayment, ObjectInput, ObjectOwnerKind, ObjectRef};

use crate::command::rpc_helpers::{map_nonce_error, sign_and_call_function};

fn read_coin_balance(data: &[u8]) -> Option<u64> {
    if data.len() < 40 {
        return None;
    }

    let mut amount_bytes = [0u8; 8];
    amount_bytes.copy_from_slice(&data[32..40]);
    Some(u64::from_le_bytes(amount_bytes))
}

#[derive(Debug, Clone)]
pub struct SelectedCoinObject {
    pub coin_object_id: String,
    pub coin_object_ref: ObjectRef,
    pub selected_balance: u64,
    pub total_balance: u64,
}

#[derive(Debug, Clone)]
pub struct SpendableCoinObject {
    pub coin_object_id: String,
    pub coin_object_ref: ObjectRef,
    pub balance: u64,
}

pub fn spendable_coin_objects(
    owned_objects: &[ObjectInfo],
    token_type: &str,
) -> Vec<SpendableCoinObject> {
    let coin_type = CoinModule::coin_type(token_type);
    let mut coins = Vec::new();

    for obj in owned_objects {
        if obj.type_ != coin_type {
            continue;
        }

        let Some(balance) = read_coin_balance(&obj.data) else {
            continue;
        };
        if balance == 0 {
            continue;
        }

        coins.push(SpendableCoinObject {
            coin_object_id: obj.id.clone(),
            coin_object_ref: ObjectRef::new(obj.id.clone(), Some(obj.version), obj.digest.clone()),
            balance,
        });
    }

    coins
}

pub fn select_coin_object(
    owned_objects: &[ObjectInfo],
    token_type: &str,
    required_amount: u64,
) -> Result<SelectedCoinObject> {
    let coins = spendable_coin_objects(owned_objects, token_type);
    let mut total_balance = 0u64;
    let mut smallest_sufficient: Option<(ObjectRef, u64)> = None;
    let mut largest_available: Option<(ObjectRef, u64)> = None;

    for coin in coins {
        let balance = coin.balance;
        total_balance = total_balance.saturating_add(balance);

        if balance >= required_amount {
            match &smallest_sufficient {
                Some((_, current)) if *current <= balance => {}
                _ => smallest_sufficient = Some((coin.coin_object_ref.clone(), balance)),
            }
        }

        match &largest_available {
            Some((_, current)) if *current >= balance => {}
            _ => largest_available = Some((coin.coin_object_ref.clone(), balance)),
        }
    }

    let (coin_object_ref, selected_balance) = smallest_sufficient
        .or(largest_available)
        .context("No spendable native coin object found")?;

    Ok(SelectedCoinObject {
        coin_object_id: coin_object_ref.object_id.clone(),
        coin_object_ref,
        selected_balance,
        total_balance,
    })
}

pub fn select_native_coin_object(
    owned_objects: &[ObjectInfo],
    required_amount: u64,
) -> Result<SelectedCoinObject> {
    select_coin_object(owned_objects, GAS_COIN, required_amount)
}

pub fn select_native_gas_coin_object(
    owned_objects: &[ObjectInfo],
    required_amount: u64,
    exclude_object_ids: &[&str],
) -> Result<SelectedCoinObject> {
    let excluded = exclude_object_ids
        .iter()
        .map(|id| id.to_ascii_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    let filtered = owned_objects
        .iter()
        .filter(|obj| !excluded.contains(&obj.id.to_ascii_lowercase()))
        .cloned()
        .collect::<Vec<_>>();

    select_native_coin_object(&filtered, required_amount)
}

pub fn build_native_gas_payment(
    owned_objects: &[ObjectInfo],
    sender: &str,
    gas_limit: u64,
    gas_price: u64,
    exclude_object_ids: &[&str],
) -> Result<(SelectedCoinObject, GasPayment)> {
    let gas_coin = select_native_gas_coin_object(
        owned_objects,
        gas_limit.saturating_mul(gas_price),
        exclude_object_ids,
    )
    .or_else(|_| {
        if exclude_object_ids.is_empty() {
            select_native_coin_object(owned_objects, gas_limit.saturating_mul(gas_price))
        } else {
            Err(anyhow::anyhow!("No spendable native gas coin object found"))
        }
    })?;

    let gas_payment = GasPayment {
        payment_objects: vec![gas_coin.coin_object_ref.clone()],
        owner: sender.to_string(),
        budget: gas_limit,
        price: gas_price,
    };

    Ok((gas_coin, gas_payment))
}

pub async fn consolidate_coin_objects(
    client: &RpcClient,
    wallet: &Wallet,
    sender_tagged: &str,
    token_type: &str,
    spendable_coins: &[SpendableCoinObject],
    required_balance: u64,
    _starting_sequence: u64,
    gas_limit: u64,
    gas_price: u64,
) -> Result<(SelectedCoinObject, u64)> {
    let mut coins = spendable_coins.to_vec();
    coins.sort_by_key(|coin| std::cmp::Reverse(coin.balance));

    let Some(primary) = coins.first().cloned() else {
        bail!("No spendable Coin<{}> object found", token_type);
    };

    let mut accumulated = primary.balance;
    let mut last_prepared_nonce = 0;

    for coin in coins.iter().skip(1) {
        if accumulated >= required_balance {
            break;
        }

        let join_req = client
            .build_call_function(BuildCallFunctionRequest {
                sender: sender_tagged.to_string(),
                package: "0x2".to_string(),
                module: CoinModule::COIN_MODULE.to_string(),
                function: CoinModule::function_names().join_entry.to_string(),
                type_args: vec![token_type.to_string()],
                args: vec![
                    move_core_types::account_address::AccountAddress::from_hex_literal(
                        &primary.coin_object_id,
                    )
                    .context("Invalid primary coin object ID")?
                    .to_vec(),
                    move_core_types::account_address::AccountAddress::from_hex_literal(
                        &coin.coin_object_id,
                    )
                    .context("Invalid merge coin object ID")?
                    .to_vec(),
                ],
                object_inputs: None,
                gas_limit,
                gas_price,
                nonce: None,
                execute_immediate: Some(true),
            })
            .await
            .context("Failed to build coin consolidation transaction")?;
        last_prepared_nonce = join_req.canonical_nonce().map_err(map_nonce_error)?;
        let status = sign_and_call_function(client, wallet, join_req).await?;
        eprintln!(
            "  Consolidated coin {} into {} (tx: {})",
            coin.coin_object_id, primary.coin_object_id, status.hash
        );
        accumulated = accumulated.saturating_add(coin.balance);
    }

    Ok((
        SelectedCoinObject {
            coin_object_id: primary.coin_object_id,
            coin_object_ref: primary.coin_object_ref,
            selected_balance: accumulated,
            total_balance: spendable_coins
                .iter()
                .fold(0u64, |sum, coin| sum.saturating_add(coin.balance)),
        },
        last_prepared_nonce,
    ))
}

pub fn object_call_context(
    sender: &str,
    primary_object_ref: ObjectRef,
    gas_limit: u64,
    gas_price: u64,
) -> (Vec<ObjectInput>, GasPayment) {
    (
        vec![ObjectInput {
            object_ref: primary_object_ref.clone(),
            owner: Some(ObjectOwnerKind::AddressOwner(sender.to_string())),
            mutable: true,
        }],
        GasPayment {
            payment_objects: vec![primary_object_ref],
            owner: sender.to_string(),
            budget: gas_limit,
            price: gas_price,
        },
    )
}
