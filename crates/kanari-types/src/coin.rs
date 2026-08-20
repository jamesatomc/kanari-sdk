// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::address::Address;
use crate::balance::BalanceRecord;
use crate::object::UIDRecord;
use anyhow::{Context, Result};
use move_core_types::{
    account_address::AccountAddress,
    identifier::Identifier,
    language_storage::{ModuleId, TypeTag},
};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Coin wrapper (mirrors `Coin<T>` in Move)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoinRecord {
    pub id: UIDRecord,
    pub balance: BalanceRecord,
}

impl CoinRecord {
    /// Create a new coin from a balance and UID
    pub fn new(id: UIDRecord, balance: BalanceRecord) -> Self {
        Self { id, balance }
    }

    /// Get coin value
    pub fn value(&self) -> u64 {
        self.balance.value()
    }

    /// Burn coin and return value (consumes coin)
    pub fn burn(self) -> u64 {
        self.balance.destroy()
    }

    /// Convert coin into its inner BalanceRecord
    pub fn into_balance(self) -> BalanceRecord {
        self.balance
    }

    /// Construct a coin from a BalanceRecord and UID
    pub fn from_balance(id: UIDRecord, balance: BalanceRecord) -> Self {
        Self { id, balance }
    }

    /// Split off `amount` from this coin, returning a new coin with that amount.
    /// Caller must provide the `new_id` for the created coin.
    pub fn split(&mut self, amount: u64, new_id: UIDRecord) -> Self {
        let new_balance = self.balance.split(amount);
        Self::new(new_id, new_balance)
    }

    /// Join another coin into this one (adds value)
    pub fn join(&mut self, other: CoinRecord) {
        self.balance.merge(other.balance);
    }
}

/// Currency metadata structure
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CurrencyMetadata {
    pub symbol: Vec<u8>,
    pub name: Vec<u8>,
    pub description: Vec<u8>,
    pub decimals: u8,
    pub icon_url: Option<Vec<u8>>,
}

impl CurrencyMetadata {
    /// Create new metadata
    pub fn new(
        symbol: Vec<u8>,
        name: Vec<u8>,
        description: Vec<u8>,
        decimals: u8,
        icon_url: Option<Vec<u8>>,
    ) -> Self {
        Self {
            symbol,
            name,
            description,
            decimals,
            icon_url,
        }
    }

    /// Get symbol as string
    pub fn symbol_str(&self) -> Result<String> {
        String::from_utf8(self.symbol.clone()).context("Invalid UTF-8 in symbol")
    }

    /// Get name as string
    pub fn name_str(&self) -> Result<String> {
        String::from_utf8(self.name.clone()).context("Invalid UTF-8 in name")
    }
}

/// Treasury capability (tracks total supply and acts as mint/burn authority)
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TreasuryCap {
    pub total_supply: u64,
}

impl TreasuryCap {
    /// Create a new treasury cap with zero supply
    pub fn new() -> Self {
        Self { total_supply: 0 }
    }

    /// Get total supply tracked by this cap
    pub fn total_supply(&self) -> u64 {
        self.total_supply
    }

    /// Mint new coins, increasing the tracked total supply and returning a CoinRecord
    pub fn mint(&mut self, amount: u64, new_id: UIDRecord) -> CoinRecord {
        assert!(amount > 0, "zero amount");
        let new_total = self.total_supply.checked_add(amount).expect("overflow");
        self.total_supply = new_total;
        CoinRecord::new(new_id, BalanceRecord::new(amount))
    }

    /// Burn coins and decrease tracked total supply. Returns burned value.
    pub fn burn(&mut self, coin: CoinRecord) -> u64 {
        let value = coin.burn();
        assert!(self.total_supply >= value, "underflow");
        self.total_supply -= value;
        value
    }
}

impl Default for TreasuryCap {
    fn default() -> Self {
        Self::new()
    }
}

/// Coin module constants and utilities
pub struct CoinModule;

impl CoinModule {
    pub const COIN_MODULE: &'static str = "coin";
    /// Name of the Coin struct in Move
    pub const COIN_STRUCT: &'static str = "Coin";
    /// Name of the TreasuryCap struct in Move
    pub const TREASURY_CAP_STRUCT: &'static str = "TreasuryCap";

    /// Get the module ID for kanari_system::coin
    pub fn get_module_id() -> Result<ModuleId> {
        let address = AccountAddress::from_hex_literal(Address::KANARI_SYSTEM_ADDRESS)
            .context("Invalid system address")?;

        let module_name = Identifier::new(Self::COIN_MODULE).context("Invalid coin module name")?;

        Ok(ModuleId::new(address, module_name))
    }

    /// Fully qualified Move module path.
    pub fn module_path() -> String {
        format!("{}::{}", Address::KANARI_SYSTEM_ADDRESS, Self::COIN_MODULE)
    }

    /// Fully qualified `Coin<T>` object type for a token type.
    pub fn coin_type(token_type: &str) -> String {
        format!(
            "{}::{}<{}>",
            Self::module_path(),
            Self::COIN_STRUCT,
            token_type
        )
    }

    /// Canonicalize a Move token type while preserving unknown legacy spellings.
    pub fn normalize_token_type(token_type: &str) -> String {
        TypeTag::from_str(token_type)
            .map(|tag| tag.to_string())
            .unwrap_or_else(|_| token_type.to_string())
    }

    /// Read the `u64` balance from the canonical `Coin<T>` BCS layout.
    pub fn read_balance(data: &[u8]) -> Option<u64> {
        let amount = data.get(32..40)?;
        Some(u64::from_le_bytes(amount.try_into().ok()?))
    }

    /// Legacy nested coin path accepted for older indexed objects.
    pub fn legacy_nested_coin_type(token_type: &str) -> String {
        format!(
            "{}::{}::{}::{}<{}>",
            Address::KANARI_SYSTEM_ADDRESS,
            Self::COIN_MODULE,
            Self::COIN_MODULE,
            Self::COIN_STRUCT,
            token_type
        )
    }

    /// Match canonical and legacy object type spellings for `Coin<T>`.
    pub fn is_coin_type_for(object_type: &str, token_type: &str) -> bool {
        object_type == Self::coin_type(token_type)
            || object_type == Self::legacy_nested_coin_type(token_type)
    }

    /// Get function names used in coin module
    pub fn function_names() -> CoinFunctions {
        CoinFunctions {
            create_currency: "create_currency",
            create_regulated_currency: "create_regulated_currency",
            mint: "mint",
            mint_and_transfer: "mint_and_transfer",
            from_balance: "from_balance",
            burn: "burn",
            total_supply: "total_supply",
            value: "value",
            split: "split",
            join: "join",
            join_entry: "join_entry",
            into_balance: "into_balance",
            update_icon_url: "update_icon_url",
            update_name: "update_name",
            update_description: "update_description",
            update_symbol: "update_symbol",
        }
    }
}

/// Coin module function names
pub struct CoinFunctions {
    pub create_currency: &'static str,
    pub create_regulated_currency: &'static str,
    pub mint: &'static str,
    pub mint_and_transfer: &'static str,
    pub from_balance: &'static str,
    pub burn: &'static str,
    pub total_supply: &'static str,
    pub value: &'static str,
    pub split: &'static str,
    pub join: &'static str,
    pub join_entry: &'static str,
    pub into_balance: &'static str,
    pub update_icon_url: &'static str,
    pub update_name: &'static str,
    pub update_description: &'static str,
    pub update_symbol: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address as KanariAddress;

    #[test]
    fn test_coin_creation() {
        let uid = UIDRecord::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();
        let balance = BalanceRecord::new(1000);
        let coin = CoinRecord::new(uid, balance);
        assert_eq!(coin.value(), 1000);
    }

    #[test]
    fn test_coin_burn() {
        let uid = UIDRecord::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS).unwrap();
        let balance = BalanceRecord::new(500);
        let coin = CoinRecord::new(uid, balance);
        let value = coin.burn();
        assert_eq!(value, 500);
    }

    #[test]
    fn test_metadata() {
        let metadata = CurrencyMetadata::new(
            b"KANARI".to_vec(),
            b"Kanari Coin".to_vec(),
            b"Native coin of Kanari".to_vec(),
            9u8,
            None,
        );
        assert_eq!(metadata.symbol_str().unwrap(), "KANARI");
        assert_eq!(metadata.name_str().unwrap(), "Kanari Coin");
        assert_eq!(metadata.decimals, 9u8);
    }

    #[test]
    fn coin_layout_balance_reader_is_bounded() {
        assert_eq!(CoinModule::read_balance(&[0; 39]), None);

        let mut bytes = vec![0u8; 40];
        bytes[32..40].copy_from_slice(&42u64.to_le_bytes());
        assert_eq!(CoinModule::read_balance(&bytes), Some(42));
    }

    #[test]
    fn token_type_normalization_is_canonical_and_legacy_safe() {
        assert_eq!(
            CoinModule::normalize_token_type("0x2::kanari::KANARI"),
            "0x2::kanari::KANARI"
        );
        assert_eq!(CoinModule::normalize_token_type("not-a-type"), "not-a-type");
    }
}
