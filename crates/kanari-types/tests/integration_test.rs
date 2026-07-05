// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for kanari-types
//!
//! Test integration between Rust types and Move modules

use kanari_types::address::Address as KanariAddress;
use kanari_types::stdlib::*;
use kanari_types::*;
use move_core_types::account_address::AccountAddress;

#[test]
fn test_all_modules_have_valid_module_ids() {
    // Test Kanari System Modules
    assert!(balance::BalanceModule::get_module_id().is_ok());
    assert!(coin::CoinModule::get_module_id().is_ok());
    assert!(kanari::KanariModule::get_module_id().is_ok());
    assert!(transfer::TransferModule::get_module_id().is_ok());
    assert!(tx_context::TxContextModule::get_module_id().is_ok());

    // Test Move Stdlib Modules
    assert!(ascii::AsciiModule::get_module_id().is_ok());
    // assert!(error::ErrorModule::get_module_id().is_ok());
    assert!(option::OptionModule::get_module_id().is_ok());
    assert!(signer::SignerModule::get_module_id().is_ok());
    assert!(string::StringModule::get_module_id().is_ok());
    assert!(vector::VectorModule::get_module_id().is_ok());
}

#[test]
fn test_module_addresses_are_correct() {
    // Kanari system modules should be at 0x2
    let balance_module = balance::BalanceModule::get_module_id().unwrap();
    let expected_addr =
        AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS).unwrap();
    assert_eq!(balance_module.address(), &expected_addr);

    let coin_module = coin::CoinModule::get_module_id().unwrap();
    assert_eq!(coin_module.address(), &expected_addr);

    // Stdlib modules should be at 0x1
    let ascii_module = ascii::AsciiModule::get_module_id().unwrap();
    let std_addr = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();
    assert_eq!(ascii_module.address(), &std_addr);

    let string_module = string::StringModule::get_module_id().unwrap();
    assert_eq!(string_module.address(), &std_addr);
}

#[test]
fn test_module_names_match_move_modules() {
    // Test module names
    let balance_module = balance::BalanceModule::get_module_id().unwrap();
    assert_eq!(balance_module.name().as_str(), "balance");

    let coin_module = coin::CoinModule::get_module_id().unwrap();
    assert_eq!(coin_module.name().as_str(), "coin");

    let kanari_module = kanari::KanariModule::get_module_id().unwrap();
    assert_eq!(kanari_module.name().as_str(), "kanari");

    let ascii_module = ascii::AsciiModule::get_module_id().unwrap();
    assert_eq!(ascii_module.name().as_str(), "ascii");

    let option_module = option::OptionModule::get_module_id().unwrap();
    assert_eq!(option_module.name().as_str(), "option");
}

#[test]
fn test_function_names_are_accessible() {
    // Test that all function name structs are accessible
    let balance_fns = balance::BalanceModule::function_names();
    assert_eq!(balance_fns.zero, "zero");
    assert_eq!(balance_fns.create, "create");
    assert_eq!(balance_fns.increase, "increase");

    let coin_fns = coin::CoinModule::function_names();
    assert_eq!(coin_fns.create_currency, "create_currency");
    assert_eq!(coin_fns.treasury_into_supply, "treasury_into_supply");

    let transfer_fns = transfer::TransferModule::function_names();
    assert_eq!(transfer_fns.create_transfer, "create_transfer");
    assert_eq!(transfer_fns.get_amount, "get_amount");
}

#[test]
fn test_kanari_constants_match_move() {
    // Test that Kanari constants are consistent
    assert_eq!(kanari::KanariModule::MIST_PER_KANARI, 1_000_000_000);
    assert_eq!(kanari::KanariModule::TOTAL_SUPPLY_KANARI, 11_000_000);
    assert_eq!(
        kanari::KanariModule::TOTAL_SUPPLY_MIST,
        kanari::KanariModule::TOTAL_SUPPLY_KANARI * kanari::KanariModule::MIST_PER_KANARI
    );
}

#[test]
fn test_error_codes_match_move() {
    // Test that error codes match Move definitions
    // assert_eq!(error::ErrorModule::INVALID_ARGUMENT, 0x1);
    // assert_eq!(error::ErrorModule::OUT_OF_RANGE, 0x2);
    // assert_eq!(error::ErrorModule::NOT_FOUND, 0x6);

    assert_eq!(ascii::AsciiModule::EINVALID_ASCII_CHARACTER, 0x10000);
    assert_eq!(vector::VectorModule::EINDEX_OUT_OF_BOUNDS, 0x20000);
    assert_eq!(option::OptionModule::EOPTION_IS_SET, 0x40000);
}

#[test]
fn test_data_structures_serializable() {
    use serde_json;

    // Test Balance serialization
    let balance = balance::BalanceRecord::new(1000);
    let json = serde_json::to_string(&balance).unwrap();
    let deserialized: balance::BalanceRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(balance.value, deserialized.value);

    // Test Coin serialization
    let uid = object::UIDRecord::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS).unwrap();
    let balance = balance::BalanceRecord::new(500);
    let coin = coin::CoinRecord::new(uid, balance);
    let json = serde_json::to_string(&coin).unwrap();
    let deserialized: coin::CoinRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(coin.value(), deserialized.value());

    // Test ASCII string serialization
    let ascii_str = ascii::AsciiString::from_str("Hello").unwrap();
    let json = serde_json::to_string(&ascii_str).unwrap();
    let deserialized: ascii::AsciiString = serde_json::from_str(&json).unwrap();
    assert_eq!(ascii_str, deserialized);
}

#[test]
fn test_move_address_conversion() {
    // Test conversion between our Address and Move's AccountAddress
    use kanari_types::address::Address;

    // Create a Move AccountAddress
    let move_addr = AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS).unwrap();

    // Convert to our Address
    let our_addr: Address = move_addr.into();

    // Verify it works
    assert!(!our_addr.to_hex().is_empty());
}

#[test]
fn test_complete_workflow() {
    // Simulate a complete workflow using multiple modules

    // 1. Create a balance
    let mut balance = balance::BalanceRecord::new(1000);
    assert_eq!(balance.value, 1000);

    // 2. Increase balance
    balance.increase(500).unwrap();
    assert_eq!(balance.value, 1500);

    // 3. Create a transfer record
    let transfer = transfer::TransferRecord::from_hex_literals(
        KanariAddress::STD_ADDRESS,
        KanariAddress::KANARI_SYSTEM_ADDRESS,
        500,
    )
    .unwrap();

    // 4. Decrease balance by transfer amount
    balance.decrease(transfer.amount).unwrap();
    assert_eq!(balance.value, 1000);

    // 5. Verify all modules are accessible
    assert!(balance::BalanceModule::get_module_id().is_ok());
    assert!(transfer::TransferModule::get_module_id().is_ok());
}

#[test]
fn test_stdlib_types_work_together() {
    // Test that stdlib types can work together

    // Create an Option with a String
    let string = string::Utf8String::from_str("Hello, World!");
    let opt = option::OptionValue::some(string.clone());

    assert!(opt.is_some());
    assert_eq!(opt.as_ref().unwrap().bytes, string.bytes);

    // Create ASCII string
    let ascii = ascii::AsciiString::from_str("ASCII").unwrap();
    assert!(ascii.all_characters_printable());

    // Test signer
    let signer = signer::SignerRef::new(KanariAddress::STD_ADDRESS.to_string());
    assert_eq!(signer.address(), KanariAddress::STD_ADDRESS);
}

#[test]
fn test_module_id_uniqueness() {
    // Verify each module has a unique ModuleId
    use std::collections::HashSet;

    let mut module_ids = HashSet::new();

    // Kanari system modules
    module_ids.insert(balance::BalanceModule::get_module_id().unwrap());
    module_ids.insert(coin::CoinModule::get_module_id().unwrap());
    module_ids.insert(kanari::KanariModule::get_module_id().unwrap());
    module_ids.insert(transfer::TransferModule::get_module_id().unwrap());
    module_ids.insert(tx_context::TxContextModule::get_module_id().unwrap());

    // Stdlib modules
    module_ids.insert(ascii::AsciiModule::get_module_id().unwrap());
    // module_ids.insert(error::ErrorModule::get_module_id().unwrap());
    module_ids.insert(option::OptionModule::get_module_id().unwrap());
    module_ids.insert(signer::SignerModule::get_module_id().unwrap());
    module_ids.insert(string::StringModule::get_module_id().unwrap());
    module_ids.insert(vector::VectorModule::get_module_id().unwrap());

    // Should have 11 unique module IDs
    assert_eq!(module_ids.len(), 11);
}

#[test]
fn test_kanari_amount_conversions() {
    // Test Kanari amount conversions work correctly
    let one_kanari = kanari::KanariModule::kanari_to_mist(1);
    assert_eq!(one_kanari, 1_000_000_000);

    let back_to_kanari = kanari::KanariModule::mist_to_kanari(one_kanari);
    assert_eq!(back_to_kanari, 1);

    // Test formatting
    let formatted = kanari::KanariModule::format_kanari(1_500_000_000);
    assert!(formatted.contains("1.5"));
    assert!(formatted.contains("KANARI"));
}
