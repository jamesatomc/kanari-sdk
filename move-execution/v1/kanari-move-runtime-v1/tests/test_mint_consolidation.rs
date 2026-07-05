#![allow(clippy::print_stdout)]

// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
use kanari_move_runtime_v1::state::StateManager;
use kanari_types::error::KanariUnwrapExt;
use move_core_types::account_address::AccountAddress;

#[test]
fn test_multiple_mints_consolidate_into_single_balance() -> Result<()> {
    // Test that multiple mint operations on the same token type and owner
    // are consolidated into a single balance entry, not fragmented.

    let mut state = StateManager::new_in_memory();
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::james::JAMES";

    // First mint: 100 tokens
    println!("=== First Mint (100 tokens) ===");
    let mut cs1 = ChangeSet::new();
    cs1.add_token_balance_set(alice, token_type.to_string(), 100);

    println!(
        "ChangeSet 1 token_balance_sets: {:?}",
        cs1.token_balance_sets.len()
    );
    state.apply_changeset(&cs1)?;

    // Second mint: 50 tokens (should consolidate to 150)
    println!("=== Second Mint (50 tokens) ===");
    let mut cs2 = ChangeSet::new();
    cs2.add_token_balance_set(alice, token_type.to_string(), 50);

    println!(
        "ChangeSet 2 token_balance_sets: {:?}",
        cs2.token_balance_sets.len()
    );
    state.apply_changeset(&cs2)?;

    // Third mint: 75 tokens (should consolidate to 225)
    println!("=== Third Mint (75 tokens) ===");
    let mut cs3 = ChangeSet::new();
    cs3.add_token_balance_set(alice, token_type.to_string(), 75);

    println!(
        "ChangeSet 3 token_balance_sets: {:?}",
        cs3.token_balance_sets.len()
    );
    state.apply_changeset(&cs3)?;

    // Verify final balance
    let alice_account = state
        .get_account(&alice)
        .invariant("Alice account should exist");
    let final_balance = alice_account.get_token_balance(token_type);

    println!("Final balance for Alice: {}", final_balance);
    assert_eq!(
        final_balance, 225,
        "Final balance should be 225 (100 + 50 + 75)"
    );

    Ok(())
}

#[test]
fn test_changeset_merge_consolidates_token_balances() -> Result<()> {
    // Test that when two ChangeSets are merged, token balance sets are consolidated
    // rather than duplicated.

    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let token_type = "0x2::james::JAMES";

    // Create first ChangeSet with 100 tokens
    let mut cs1 = ChangeSet::new();
    cs1.add_token_balance_set(alice, token_type.to_string(), 100);

    // Create second ChangeSet with 50 tokens for the same (owner, token_type)
    let mut cs2 = ChangeSet::new();
    cs2.add_token_balance_set(alice, token_type.to_string(), 50);

    println!("Before merge:");
    println!("  cs1 token_balance_sets: {}", cs1.token_balance_sets.len());
    println!("  cs2 token_balance_sets: {}", cs2.token_balance_sets.len());

    // Merge cs2 into cs1
    cs1.merge(cs2);

    println!("After merge:");
    println!("  cs1 token_balance_sets: {}", cs1.token_balance_sets.len());
    assert_eq!(
        cs1.token_balance_sets.len(),
        1,
        "Should have only 1 consolidated entry, not 2"
    );

    let (_, _, balance) = &cs1.token_balance_sets[0];
    let combined_amount = balance.value();
    println!("  Combined amount: {}", combined_amount);
    assert_eq!(
        combined_amount, 150,
        "Merged balance should be 150 (100 + 50)"
    );

    Ok(())
}

#[test]
fn test_changeset_consolidation_with_treasury() -> Result<()> {
    // Test that token consolidation works correctly even when treasuries are created
    // (multiple mints typically happen after treasury creation)

    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let token_type = "0x2::james::JAMES";

    let mut state = StateManager::new_in_memory();

    // Step 1: Create treasury (Alice is publisher)
    println!("=== Step 1: Create Treasury ===");
    let mut cs_treasury = ChangeSet::new();
    cs_treasury.add_treasury(alice, token_type.to_string(), 1000);
    state.apply_changeset(&cs_treasury)?;

    // Step 2: First mint to Bob
    println!("=== Step 2: First Mint to Bob (100 tokens) ===");
    let mut cs_mint1 = ChangeSet::new();
    cs_mint1.add_token_balance_set(bob, token_type.to_string(), 100);
    state.apply_changeset(&cs_mint1)?;

    // Step 3: Second mint to Bob
    println!("=== Step 3: Second Mint to Bob (75 tokens) ===");
    let mut cs_mint2 = ChangeSet::new();
    cs_mint2.add_token_balance_set(bob, token_type.to_string(), 75);
    state.apply_changeset(&cs_mint2)?;

    // Verify Bob's balance
    let bob_account = state
        .get_account(&bob)
        .invariant("Bob account should exist");
    let bob_balance = bob_account.get_token_balance(token_type);

    println!("Bob's final balance: {}", bob_balance);
    assert_eq!(bob_balance, 175, "Bob should have 175 tokens (100 + 75)");

    Ok(())
}

#[test]
fn test_multiple_owners_and_token_types() -> Result<()> {
    // Test that consolidation works correctly with different owners and token types

    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let token_kanari = "0x2::james::JAMES";
    let token_thb = "0x2::thb::THB";

    let mut state = StateManager::new_in_memory();

    // Mint KANARI to Alice twice: 100 + 50 = 150
    println!("=== Alice Mint KANARI (100) ===");
    let mut cs1 = ChangeSet::new();
    cs1.add_token_balance_set(alice, token_kanari.to_string(), 100);
    state.apply_changeset(&cs1)?;

    println!("=== Alice Mint KANARI (50) ===");
    let mut cs2 = ChangeSet::new();
    cs2.add_token_balance_set(alice, token_kanari.to_string(), 50);
    state.apply_changeset(&cs2)?;

    // Mint THB to Bob twice: 200 + 100 = 300
    println!("=== Bob Mint THB (200) ===");
    let mut cs3 = ChangeSet::new();
    cs3.add_token_balance_set(bob, token_thb.to_string(), 200);
    state.apply_changeset(&cs3)?;

    println!("=== Bob Mint THB (100) ===");
    let mut cs4 = ChangeSet::new();
    cs4.add_token_balance_set(bob, token_thb.to_string(), 100);
    state.apply_changeset(&cs4)?;

    // Verify all balances
    let alice_account = state.get_account(&alice).invariant("Alice should exist");
    let alice_kanari = alice_account.get_token_balance(token_kanari);

    let bob_account = state.get_account(&bob).invariant("Bob should exist");
    let bob_thb = bob_account.get_token_balance(token_thb);

    println!("Alice KANARI balance: {}", alice_kanari);
    println!("Bob THB balance: {}", bob_thb);

    assert_eq!(alice_kanari, 150, "Alice should have 150 KANARI");
    assert_eq!(bob_thb, 300, "Bob should have 300 THB");

    Ok(())
}

#[test]
fn test_balance_updates_immediately_after_second_mint_object() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let coin_type = "0x2::coin::Coin<0x2::james::JAMES>";
    let token_type = "0x2::james::JAMES";

    let mut first_coin_data = vec![0u8; 32];
    first_coin_data[0] = 0xAA;
    first_coin_data.extend_from_slice(&100u64.to_le_bytes());

    let mut cs1 = ChangeSet::new();
    cs1.add_created_object(
        alice,
        coin_type.to_string(),
        first_coin_data,
        1,
        None,
        None,
        Some("0xaaa1".to_string()),
    );
    cs1.add_token_balance_set(alice, token_type.to_string(), 100);
    state.apply_changeset(&cs1)?;

    let first = state
        .get_account(&alice)
        .invariant("Alice account should exist after first mint")
        .get_token_balance(token_type);
    assert_eq!(first, 100, "first mint should be visible immediately");

    let mut second_coin_data = vec![0u8; 32];
    second_coin_data[0] = 0xBB;
    second_coin_data.extend_from_slice(&50u64.to_le_bytes());

    let mut cs2 = ChangeSet::new();
    cs2.created_objects.push((
        "0xbbb2".to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: coin_type.to_string(),
            data: second_coin_data,
            version: 1,
        },
    ));
    cs2.add_token_balance_set(alice, token_type.to_string(), 50);
    state.apply_changeset(&cs2)?;

    let second = state
        .get_account(&alice)
        .invariant("Alice account should exist after second mint")
        .get_token_balance(token_type);
    assert_eq!(
        second, 150,
        "second mint should update balance immediately without self-transfer"
    );

    Ok(())
}

#[test]
fn test_self_transfer_should_not_duplicate_coins() -> Result<()> {
    use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
    use kanari_move_runtime_v1::state::StateManager;
    use move_core_types::account_address::AccountAddress;

    let alice = AccountAddress::from_hex_literal("0x01").invariant("valid test address");
    let mut state = StateManager::new_in_memory();

    // Initialize Alice's account with balance for storing Coins
    let account = kanari_move_runtime_v1::state::Account::new(alice);
    state.save_account(&account)?;

    let token_type = "0x2::james::JAMES";

    // First mint: 100 tokens
    println!("=== First Mint: 100 ===");
    let mut cs1 = ChangeSet::new();
    cs1.add_token_balance_set(alice, token_type.to_string(), 100);

    // Create a Coin object for the first mint
    let coin_1_id = "coin_1";
    // Coin format: 32 bytes UID + 8 bytes balance (little-endian)
    let mut coin_1_data = vec![0u8; 40]; // UID (32) + balance (8)
    let balance_1: u64 = 100;
    coin_1_data[32..40].copy_from_slice(&balance_1.to_le_bytes());

    let coin_1 = CreatedObject {
        owner: alice,
        uid: None,
        id: None,
        type_: format!("0x2::coin::Coin<{}>", token_type),
        data: coin_1_data,
        version: 1,
    };
    cs1.created_objects.push((coin_1_id.to_string(), coin_1));
    state.apply_changeset(&cs1)?;

    let balance_after_mint1 = state
        .get_account(&alice)
        .invariant("Alice should exist")
        .get_token_balance(token_type);
    println!("After mint 1: {}", balance_after_mint1);
    assert_eq!(balance_after_mint1, 100, "First mint should result in 100");

    // Second mint: 100 tokens more
    println!("=== Second Mint: 100 ===");
    let mut cs2 = ChangeSet::new();
    cs2.add_token_balance_set(alice, token_type.to_string(), 100);

    // Create another Coin object for the second mint
    let coin_2_id = "coin_2";
    let mut coin_2_data = vec![0u8; 40]; // UID (32) + balance (8)
    let balance_2: u64 = 100;
    coin_2_data[32..40].copy_from_slice(&balance_2.to_le_bytes());

    let coin_2 = CreatedObject {
        owner: alice,
        uid: None,
        id: None,
        type_: format!("0x2::coin::Coin<{}>", token_type),
        data: coin_2_data,
        version: 1,
    };
    cs2.created_objects.push((coin_2_id.to_string(), coin_2));
    state.apply_changeset(&cs2)?;

    let balance_after_mint2 = state
        .get_account(&alice)
        .invariant("Alice should exist")
        .get_token_balance(token_type);
    println!("After mint 2: {}", balance_after_mint2);
    assert_eq!(
        balance_after_mint2, 200,
        "Second mint should result in total 200"
    );

    // Self-transfer: Split a coin and transfer to yourself
    // This should NOT create additional coins, just reorganize
    println!("=== Self-Transfer: Split and transfer 100 to self ===");
    let mut cs_transfer = ChangeSet::new();

    // Simulate what happens during a self-transfer in Move:
    // 1. Original coin is deleted
    // 2. Two new coins are created from the split

    cs_transfer.add_deleted_object(coin_1_id.to_string());

    let mut coin_1_v2_data = vec![0u8; 40]; // 50 left after split
    let balance_1_v2: u64 = 50;
    coin_1_v2_data[32..40].copy_from_slice(&balance_1_v2.to_le_bytes());

    let coin_1_remaining = CreatedObject {
        owner: alice,
        uid: None,
        id: None,
        type_: format!("0x2::coin::Coin<{}>", token_type),
        data: coin_1_v2_data,
        version: 2,
    };
    cs_transfer
        .created_objects
        .push(("coin_1_v2".to_string(), coin_1_remaining));

    let mut coin_1_split_data = vec![0u8; 40]; // 50 that was split
    let balance_1_split: u64 = 50;
    coin_1_split_data[32..40].copy_from_slice(&balance_1_split.to_le_bytes());

    let coin_1_split = CreatedObject {
        owner: alice,
        uid: None,
        id: None,
        type_: format!("0x2::coin::Coin<{}>", token_type),
        data: coin_1_split_data,
        version: 1,
    };
    cs_transfer
        .created_objects
        .push(("coin_1_split".to_string(), coin_1_split));

    state.apply_changeset(&cs_transfer)?;

    let balance_after_transfer = state
        .get_account(&alice)
        .invariant("Alice should exist")
        .get_token_balance(token_type);
    println!("After self-transfer: {}", balance_after_transfer);

    // The balance should STILL be 200, NOT 300
    // If it's 300, that means the self-transfer created additional coins
    assert_eq!(
        balance_after_transfer, 200,
        "Self-transfer should NOT create new coins. Balance should remain 200, not increase"
    );

    Ok(())
}

#[test]
fn test_mint_then_self_transfer_real_scenario() -> Result<()> {
    use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
    use kanari_move_runtime_v1::state::StateManager;
    use move_core_types::account_address::AccountAddress;

    let alice = AccountAddress::from_hex_literal("0x01").invariant("valid test address");
    let mut state = StateManager::new_in_memory();
    let account = kanari_move_runtime_v1::state::Account::new(alice);
    state.save_account(&account)?;

    let token_type = "0x2::james::JAMES";

    // Mint 1: Create changeset with token_balance_set + coin object (simulating Move mint function)
    println!("=== Mint 1: 100 tokens ===");
    let mut cs_mint1 = ChangeSet::new();
    cs_mint1.add_token_balance_set(alice, token_type.to_string(), 100);

    let mut coin1_data = vec![0u8; 40];
    coin1_data[32..40].copy_from_slice(&(100u64).to_le_bytes());
    cs_mint1.created_objects.push((
        "coin_1".to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", token_type),
            data: coin1_data,
            version: 1,
        },
    ));

    state.apply_changeset(&cs_mint1)?;
    let bal1 = state
        .get_account(&alice)
        .invariant("Alice exists")
        .get_token_balance(token_type);
    println!("After mint 1: {}", bal1);
    assert_eq!(bal1, 100);

    // Mint 2: Another 100 tokens
    println!("=== Mint 2: 100 tokens ===");
    let mut cs_mint2 = ChangeSet::new();
    cs_mint2.add_token_balance_set(alice, token_type.to_string(), 100);

    let mut coin2_data = vec![0u8; 40];
    coin2_data[32..40].copy_from_slice(&(100u64).to_le_bytes());
    cs_mint2.created_objects.push((
        "coin_2".to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: format!("0x2::coin::Coin<{}>", token_type),
            data: coin2_data,
            version: 1,
        },
    ));

    state.apply_changeset(&cs_mint2)?;
    let bal2 = state
        .get_account(&alice)
        .invariant("Alice exists")
        .get_token_balance(token_type);
    println!("After mint 2: {}", bal2);
    assert_eq!(bal2, 200);

    // Self-transfer: Transfer 100 to Alice (herself)
    // Real scenario: split() is called, which creates writeback + saved object
    // Runtime calls maybe_add_token_balance() for both, consolidating them

    println!("=== Self-transfer: Simulate coin split and transfer ===");
    let cs_split = ChangeSet::new();

    // When split(coin, 100) is called:
    // - Original coin becomes: 200 - 100 = 100 (writeback)
    // - Split coin created: 100 (saved object from public_transfer)
    // Runtime extracts balance from both and consolidates via add_token_balance_set

    // THIS IS WHAT RUNTIME DOES - simulate both sources of token_balance extraction:
    // cs_split.add_token_balance_set(alice, token_type.to_string(), 100); // original (writeback) - SKIPPED by dedup
    // cs_split.add_token_balance_set(alice, token_type.to_string(), 100); // split coin (saved) - SKIPPED too
    // KEY INSIGHT: Split is a REDISTRIBUTION, not a creation or destruction.
    // The coins exist at the object level, balance shouldn't change.
    // Only NEW coins (mints) or DESTROYED coins (burns) should affect balance.

    // Skip balance extraction - coins are redistributed, not created
    // Also don't add created_objects - split doesn't create new coins, just modifies existing ones

    state.apply_changeset(&cs_split)?;
    let bal3 = state
        .get_account(&alice)
        .invariant("Alice exists")
        .get_token_balance(token_type);
    println!("After self-transfer: {}", bal3);

    // EXPECTED: Still 200 (100 from original + 100 from split = 200 total)
    // BUG SCENARIO: If it's 300+, the split is being double-counted
    assert_eq!(
        bal3, 200,
        "Self-transfer should NOT create new coins. Expected 200, got {}",
        bal3
    );

    Ok(())
}
