// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Balance queries derived from address-owned `Coin<T>` objects.

use crate::state::StateManager;
use anyhow::Result;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::{StructTag, TypeTag};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::str::FromStr;

const UID_SIZE: usize = 32;
const BALANCE_SIZE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectBalance {
    pub token_type: String,
    pub balance: u64,
    pub coin_count: u64,
}

fn coin_amount(type_name: &str, data: &[u8]) -> Option<(String, u64)> {
    let tag = StructTag::from_str(type_name).ok()?;
    if tag.module.as_str() != "coin" || tag.name.as_str() != "Coin" {
        return None;
    }
    let token_type = match tag.type_params.first()? {
        TypeTag::Struct(token) => token.to_string(),
        other => other.to_string(),
    };
    let bytes: [u8; BALANCE_SIZE] = data
        .get(UID_SIZE..UID_SIZE + BALANCE_SIZE)?
        .try_into()
        .ok()?;
    Some((token_type, u64::from_le_bytes(bytes)))
}

impl StateManager {
    pub fn object_balances(&self, owner: &AccountAddress) -> Result<Vec<ObjectBalance>> {
        let mut totals: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for object_id in self.get_owned_objects(owner)? {
            let Some(object) = self.get_object(&object_id)? else {
                continue;
            };
            let Some((token_type, amount)) = coin_amount(&object.type_, &object.data) else {
                continue;
            };
            let entry = totals.entry(token_type).or_default();
            entry.0 = entry
                .0
                .checked_add(amount)
                .ok_or_else(|| anyhow::anyhow!("Object balance overflow"))?;
            entry.1 = entry
                .1
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Coin count overflow"))?;
        }
        Ok(totals
            .into_iter()
            .map(|(token_type, (balance, coin_count))| ObjectBalance {
                token_type,
                balance,
                coin_count,
            })
            .collect())
    }

    pub fn object_balance(&self, owner: &AccountAddress, token_type: &str) -> Result<u64> {
        Ok(self
            .object_balances(owner)?
            .into_iter()
            .find(|balance| balance.token_type == token_type)
            .map(|balance| balance.balance)
            .unwrap_or_default())
    }
}
