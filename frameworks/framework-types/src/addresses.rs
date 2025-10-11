use move_core_types::account_address::AccountAddress;
use std::{collections::BTreeMap, str::FromStr};

pub use moveos_types::addresses::*;

pub const KANARI_LIBRARY_ADDRESS_NAME: &str = "kanari-library";
pub const KANARI_LIBRARY_ADDRESS_LITERAL: &str = "0x6";
pub const KANARI_LIBRARY_ADDRESS: AccountAddress = {
    let mut addr = [0u8; AccountAddress::LENGTH];
    addr[AccountAddress::LENGTH - 1] = 6u8;
    AccountAddress::new(addr)
};

pub static KANARI_LIBRARY_ADDRESS_MAPPING: [(&str, &str); 1] =
    [(KANARI_LIBRARY_ADDRESS_NAME, KANARI_LIBRARY_ADDRESS_LITERAL)];

pub fn kanari_framework_named_addresses() -> BTreeMap<String, AccountAddress> {
    let mut address_mapping = moveos_stdlib::moveos_stdlib_named_addresses();
    address_mapping.extend(
        KANARI_LIBRARY_ADDRESS_MAPPING
            .iter()
            .map(|(name, addr)| (name.to_string(), AccountAddress::from_str(addr).unwrap())),
    );
    address_mapping
}
