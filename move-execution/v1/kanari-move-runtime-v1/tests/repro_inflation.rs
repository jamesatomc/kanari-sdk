use anyhow::Result;
use kanari_types::error::KanariUnwrapExt;

use kanari_move_runtime_v1::{ChangeSet, changeset::CreatedObject, state::StateManager};
use move_core_types::account_address::AccountAddress;

#[test]
fn test_ownership_transfer() -> Result<()> {
    // 1. Initialize StateManager (in-memory)
    let mut state = StateManager::new_in_memory();

    // 2. Define Alice and Bob
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;

    // 3. Create an object (Coin)
    let object_id = "0xAAAA";
    let object_data = vec![1, 2, 3]; // Dummy data
    let object_type = "0x2::coin::Coin<0x2::james::JAMES>";

    let created_obj = CreatedObject {
        owner: alice,
        uid: None,
        id: None,
        type_: object_type.to_string(),
        data: object_data.clone(),
        version: 1,
    };

    // 4. Create ChangeSet to add object for Alice
    let mut cs1 = ChangeSet::new();
    cs1.created_objects
        .push((object_id.to_string(), created_obj));

    // 5. Apply ChangeSet 1
    state.apply_changeset(&cs1)?;

    // Verify Alice owns it
    let alice_owned = state.get_owned_objects(&alice)?;
    assert!(
        alice_owned.contains(&object_id.to_string()),
        "Alice should own the object"
    );

    // Verify Bob does not own it
    let bob_owned = state.get_owned_objects(&bob)?;
    assert!(
        !bob_owned.contains(&object_id.to_string()),
        "Bob should not own the object"
    );

    // 6. Transfer object to Bob (Update object with new owner)
    let updated_obj = CreatedObject {
        owner: bob, // New owner
        uid: None,
        id: None,
        type_: object_type.to_string(),
        data: object_data.clone(),
        version: 2, // Version incremented
    };

    // 7. Create ChangeSet 2 to update object
    let mut cs2 = ChangeSet::new();
    cs2.created_objects
        .push((object_id.to_string(), updated_obj));

    // 8. Apply ChangeSet 2
    state.apply_changeset(&cs2)?;

    // Verify Bob owns it
    let bob_owned_after = state.get_owned_objects(&bob)?;
    assert!(
        bob_owned_after.contains(&object_id.to_string()),
        "Bob should own the object after transfer"
    );

    // Verify Alice NO LONGER owns it (The Fix)
    let alice_owned_after = state.get_owned_objects(&alice)?;
    assert!(
        !alice_owned_after.contains(&object_id.to_string()),
        "Alice should NOT own the object after transfer"
    );

    Ok(())
}

#[test]
fn test_object_transfer_removes_from_old_owner() -> Result<()> {
    // Test that transferring an object removes it from old owner's list
    let mut state = StateManager::new_in_memory();
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let charlie = AccountAddress::from_hex_literal("0x3333")?;

    // Create object owned by Alice
    let object_id = "0xAAAA";
    let object_data = vec![1, 2, 3];
    let object_type = "0x2::coin::coin::Coin<0x2::james::JAMES>";

    let created_obj = CreatedObject {
        owner: alice,
        uid: None,
        id: None,
        type_: object_type.to_string(),
        data: object_data.clone(),
        version: 1,
    };

    let mut cs1 = ChangeSet::new();
    cs1.created_objects
        .push((object_id.to_string(), created_obj));
    state.apply_changeset(&cs1)?;

    // Verify initial state
    let alice_owned_1 = state.get_owned_objects(&alice)?;
    assert!(alice_owned_1.contains(&object_id.to_string()));
    assert_eq!(alice_owned_1.len(), 1);

    let bob_owned_1 = state.get_owned_objects(&bob)?;
    assert_eq!(bob_owned_1.len(), 0, "Bob should own nothing initially");

    // Transfer to Bob (update with new owner)
    let transferred_obj = CreatedObject {
        owner: bob,
        uid: None,
        id: None,
        type_: object_type.to_string(),
        data: object_data.clone(),
        version: 2,
    };

    let mut cs2 = ChangeSet::new();
    cs2.created_objects
        .push((object_id.to_string(), transferred_obj.clone()));
    state.apply_changeset(&cs2)?;

    // Verify transfer: Alice should NOT own it anymore
    let alice_owned_2 = state.get_owned_objects(&alice)?;
    assert!(
        !alice_owned_2.contains(&object_id.to_string()),
        "Alice should not own the object after transfer"
    );
    assert_eq!(alice_owned_2.len(), 0, "Alice should own 0 objects");

    // Verify Bob owns it
    let bob_owned_2 = state.get_owned_objects(&bob)?;
    assert!(bob_owned_2.contains(&object_id.to_string()));
    assert_eq!(bob_owned_2.len(), 1);

    // Transfer again to Charlie
    let transferred_obj2 = CreatedObject {
        owner: charlie,
        uid: None,
        id: None,
        type_: object_type.to_string(),
        data: object_data.clone(),
        version: 3,
    };

    let mut cs3 = ChangeSet::new();
    cs3.created_objects
        .push((object_id.to_string(), transferred_obj2));
    state.apply_changeset(&cs3)?;

    // Verify second transfer: Bob should NOT own it anymore
    let bob_owned_3 = state.get_owned_objects(&bob)?;
    assert!(
        !bob_owned_3.contains(&object_id.to_string()),
        "Bob should not own the object after second transfer"
    );
    assert_eq!(bob_owned_3.len(), 0, "Bob should own 0 objects");

    // Verify Charlie owns it
    let charlie_owned = state.get_owned_objects(&charlie)?;
    assert!(charlie_owned.contains(&object_id.to_string()));
    assert_eq!(charlie_owned.len(), 1);

    Ok(())
}

#[test]
fn test_coin_split_inflation() -> Result<()> {
    // 1. Initialize StateManager
    let mut state = StateManager::new_in_memory();
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;

    // 2. Create Coin A (1000) owned by Alice
    let coin_a_id = "0xAAAA";
    // Actually, RPC parses last 8 bytes. Let's make data 32 bytes (UID) + 8 bytes (Balance).
    let mut coin_a_data = vec![0u8; 32]; // UID
    coin_a_data.extend_from_slice(&1000u64.to_le_bytes()); // Balance 1000

    let object_type = "0x2::coin::Coin<0x2::james::JAMES>";

    let created_a = CreatedObject {
        owner: alice,
        uid: None,
        id: None,
        type_: object_type.to_string(),
        data: coin_a_data.clone(),
        version: 1,
    };

    let mut cs_init = ChangeSet::new();
    cs_init
        .created_objects
        .push((coin_a_id.to_string(), created_a));
    state.apply_changeset(&cs_init)?;

    // Verify Initial State
    let alice_owned = state.get_owned_objects(&alice)?;
    assert!(alice_owned.contains(&coin_a_id.to_string()));
    // We can't easily check balance sum here without RPC logic, but we can check objects.

    // 3. Simulate Transfer Amount (Split 500)
    // Coin A: 1000 -> 500 (Updated, Owner=Alice)
    // Coin B: 500 (Created, Owner=Bob)

    let mut coin_a_data_500 = vec![0u8; 32];
    coin_a_data_500.extend_from_slice(&500u64.to_le_bytes());

    let updated_a = CreatedObject {
        owner: alice,
        uid: None,
        id: None,
        type_: object_type.to_string(),
        data: coin_a_data_500,
        version: 2,
    };

    let coin_b_id = "0xBBBB";
    let mut coin_b_data_500 = vec![0u8; 32];
    coin_b_data_500[0] = 0xBB; // Distinct UID
    coin_b_data_500.extend_from_slice(&500u64.to_le_bytes());

    let created_b = CreatedObject {
        owner: bob,
        uid: None,
        id: None,
        type_: object_type.to_string(),
        data: coin_b_data_500,
        version: 1,
    };

    let mut cs_split = ChangeSet::new();
    cs_split
        .created_objects
        .push((coin_a_id.to_string(), updated_a));
    cs_split
        .created_objects
        .push((coin_b_id.to_string(), created_b));

    state.apply_changeset(&cs_split)?;

    // 4. Verify Final State
    let alice_owned_final = state.get_owned_objects(&alice)?;
    assert!(alice_owned_final.contains(&coin_a_id.to_string()));
    assert!(!alice_owned_final.contains(&coin_b_id.to_string()));
    assert_eq!(alice_owned_final.len(), 1, "Alice should only own Coin A");

    let bob_owned_final = state.get_owned_objects(&bob)?;
    assert!(bob_owned_final.contains(&coin_b_id.to_string()));
    assert_eq!(bob_owned_final.len(), 1, "Bob should only own Coin B");

    // Verify Data of Coin A in DB
    let stored_a = state.get_object(coin_a_id)?.invariant("Coin A must exist");
    // Check last 8 bytes
    let stored_balance_bytes: [u8; 8] = stored_a.data[32..40]
        .try_into()
        .invariant("stored coin balance bytes should be 8 bytes");
    let stored_balance = u64::from_le_bytes(stored_balance_bytes);
    assert_eq!(stored_balance, 500, "Coin A balance should be 500");

    Ok(())
}

#[test]
fn test_transfer_with_same_version_is_not_treated_as_collision() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let object_id = "0xABCD";
    let coin_type = "0x2::coin::Coin<0x2::james::JAMES>";

    let mut coin_data = AccountAddress::from_hex_literal(object_id)?.to_vec();
    coin_data.extend_from_slice(&500u64.to_le_bytes());

    let mut init_cs = ChangeSet::new();
    init_cs.created_objects.push((
        object_id.to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: coin_type.to_string(),
            data: coin_data.clone(),
            version: 2,
        },
    ));
    state.apply_changeset(&init_cs)?;

    // Same object id/type/size moved to Bob but version is equal (not incremented).
    let mut transfer_cs = ChangeSet::new();
    transfer_cs.created_objects.push((
        object_id.to_string(),
        CreatedObject {
            owner: bob,
            uid: None,
            id: None,
            type_: coin_type.to_string(),
            data: coin_data,
            version: 2,
        },
    ));
    state.apply_changeset(&transfer_cs)?;

    let alice_owned = state.get_owned_objects(&alice)?;
    let bob_owned = state.get_owned_objects(&bob)?;

    assert!(
        !alice_owned.contains(&object_id.to_string()),
        "Alice should no longer own object after transfer"
    );
    assert!(
        bob_owned.contains(&object_id.to_string()),
        "Bob should own the transferred object id"
    );
    assert_eq!(
        bob_owned.len(),
        1,
        "Transfer should not create collision id"
    );

    Ok(())
}

#[test]
fn test_duplicate_created_object_id_keeps_latest_owner() -> Result<()> {
    let mut state = StateManager::new_in_memory();
    let alice = AccountAddress::from_hex_literal("0x1111")?;
    let bob = AccountAddress::from_hex_literal("0x2222")?;
    let object_id = "0xDEAD";
    let coin_type = "0x2::coin::Coin<0x2::james::JAMES>";

    let mut init_data = AccountAddress::from_hex_literal(object_id)?.to_vec();
    init_data.extend_from_slice(&1000u64.to_le_bytes());
    let mut cs_init = ChangeSet::new();
    cs_init.created_objects.push((
        object_id.to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: coin_type.to_string(),
            data: init_data.clone(),
            version: 1,
        },
    ));
    state.apply_changeset(&cs_init)?;

    // Simulate a malformed/duplicated changeset entry for same object id in one tx:
    // first an old-owner writeback, then final transferred owner.
    let mut new_data = AccountAddress::from_hex_literal(object_id)?.to_vec();
    new_data.extend_from_slice(&1000u64.to_le_bytes());
    let mut cs = ChangeSet::new();
    cs.created_objects.push((
        object_id.to_string(),
        CreatedObject {
            owner: alice,
            uid: None,
            id: None,
            type_: coin_type.to_string(),
            data: init_data,
            version: 2,
        },
    ));
    cs.created_objects.push((
        object_id.to_string(),
        CreatedObject {
            owner: bob,
            uid: None,
            id: None,
            type_: coin_type.to_string(),
            data: new_data,
            version: 2,
        },
    ));
    state.apply_changeset(&cs)?;

    let alice_owned = state.get_owned_objects(&alice)?;
    let bob_owned = state.get_owned_objects(&bob)?;
    assert!(
        !alice_owned.contains(&object_id.to_string()),
        "Alice should not keep duplicate id after final transfer"
    );
    assert!(
        bob_owned.contains(&object_id.to_string()),
        "Latest owner should keep the object id"
    );

    Ok(())
}
