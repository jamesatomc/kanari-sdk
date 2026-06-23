#![allow(dead_code)]

use anyhow::Result;
use kanari_types::coin::TreasuryCap;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use move_core_types::account_address::AccountAddress;

use crate::state::StateManager;

pub fn test_addr(hex: &str) -> Result<AccountAddress> {
    AccountAddress::from_hex_literal(hex).map_err(Into::into)
}

pub fn test_addr_unwrap(hex: &str) -> AccountAddress {
    AccountAddress::from_hex_literal(hex).unwrap()
}

pub fn dao_address() -> Result<AccountAddress> {
    test_addr(kanari_types::address::Address::DAO_ADDRESS)
}

pub fn set_native_supply_for_test(state: &mut StateManager, total_supply: u64) -> Result<()> {
    state.total_supply = total_supply;
    state.store.save(b"total_supply", &total_supply)?;
    let mut supply_key = b"supply:".to_vec();
    supply_key.extend_from_slice(KANARI_TOKEN_TYPE.as_bytes());
    state
        .store
        .save(&supply_key, &TreasuryCap { total_supply })?;
    Ok(())
}
