// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#[test_only]
module kanari_system::deny_list_tests {
    use kanari_system::deny_list;
    use kanari_system::tx_context;

    // =================================================================
    // Tests: Basic Operations - Add and Remove
    // =================================================================
    
    // Test: Creating a new deny list should be empty
    #[test]
    fun test_new_denylist_is_empty() {
        let denylist = deny_list::new_denylist();
        
        // Verify the addresses vector is empty
        assert!(deny_list::length(&denylist) == 0, 0);
    }

    // Test: Adding an address to an empty deny list
    #[test]
    fun test_add_address_to_empty_list() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        
        // Create a dummy capability (using u64 as phantom type)
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        // Add an address
        let addr1 = @0x1;
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        
        // Verify the address was added
        assert!(deny_list::length(denylist_ref) == 1, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 1);
    }

    // Test: Adding multiple addresses
    #[test]
    fun test_add_multiple_addresses() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        let addr2 = @0x2;
        let addr3 = @0x3;
        
        // Add three different addresses
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr2, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr3, ctx);
        
        // Verify all addresses are present
        assert!(deny_list::length(denylist_ref) == 3, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 1);
        assert!(deny_list::get_address_at(denylist_ref, 1) == addr2, 2);
        assert!(deny_list::get_address_at(denylist_ref, 2) == addr3, 3);
    }

    // Test: Removing an existing address
    #[test]
    fun test_remove_existing_address() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        let addr2 = @0x2;
        let addr3 = @0x3;
        
        // Add three addresses
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr2, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr3, ctx);
        
        // Remove the middle address
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr2, ctx);
        
        // Verify addr2 was removed and others remain
        assert!(deny_list::length(denylist_ref) == 2, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 1);
        assert!(deny_list::get_address_at(denylist_ref, 1) == addr3, 2);
    }

    // Test: Removing the first address
    #[test]
    fun test_remove_first_address() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        let addr2 = @0x2;
        
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr2, ctx);
        
        // Remove the first address
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr1, ctx);
        
        assert!(deny_list::length(denylist_ref) == 1, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr2, 1);
    }

    // Test: Removing the last address
    #[test]
    fun test_remove_last_address() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        let addr2 = @0x2;
        
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr2, ctx);
        
        // Remove the last address
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr2, ctx);
        
        assert!(deny_list::length(denylist_ref) == 1, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 1);
    }

    // =================================================================
    // Tests: Duplicate Prevention
    // =================================================================
    
    // Test: Adding duplicate address should not increase list size
    #[test]
    fun test_add_duplicate_address_prevented() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        
        // Add the same address twice
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        
        // Should still have only one entry
        assert!(deny_list::length(denylist_ref) == 1, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 1);
    }

    // Test: Multiple duplicate additions with other addresses
    #[test]
    fun test_mixed_duplicates_and_unique() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        let addr2 = @0x2;
        let addr3 = @0x3;
        
        // Add addr1, addr2, addr1 again, addr3, addr2 again
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr2, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx); // duplicate
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr3, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr2, ctx); // duplicate
        
        // Should have exactly 3 unique addresses
        assert!(deny_list::length(denylist_ref) == 3, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 1);
        assert!(deny_list::get_address_at(denylist_ref, 1) == addr2, 2);
        assert!(deny_list::get_address_at(denylist_ref, 2) == addr3, 3);
    }

    // =================================================================
    // Tests: Edge Cases
    // =================================================================
    
    // Test: Removing from empty list should be no-op
    #[test]
    fun test_remove_from_empty_list() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        
        // Try to remove from empty list
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr1, ctx);
        
        // Should still be empty
        assert!(deny_list::length(denylist_ref) == 0, 0);
    }

    // Test: Removing non-existent address should be no-op
    #[test]
    fun test_remove_non_existent_address() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        let addr2 = @0x2;
        
        // Add only addr1
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        
        // Try to remove addr2 (doesn't exist)
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr2, ctx);
        
        // Should still have only addr1
        assert!(deny_list::length(denylist_ref) == 1, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 1);
    }

    // Test: Add and remove same address multiple times
    #[test]
    fun test_add_remove_cycle() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        
        // Add, remove, add again
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        assert!(deny_list::length(denylist_ref) == 1, 0);
        
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr1, ctx);
        assert!(deny_list::length(denylist_ref) == 0, 1);
        
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        assert!(deny_list::length(denylist_ref) == 1, 2);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 3);
    }

    // Test: Remove all addresses one by one
    #[test]
    fun test_remove_all_addresses_sequentially() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        let addr2 = @0x2;
        let addr3 = @0x3;
        
        // Add three addresses
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr2, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr3, ctx);
        
        // Remove them in reverse order
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr3, ctx);
        assert!(deny_list::length(denylist_ref) == 2, 0);
        
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr2, ctx);
        assert!(deny_list::length(denylist_ref) == 1, 1);
        
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr1, ctx);
        assert!(deny_list::length(denylist_ref) == 0, 2);
    }

    // =================================================================
    // Tests: Contains Function
    // =================================================================
    
    // Test: contains returns true for existing address
    #[test]
    fun test_contains_existing_address() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        let addr2 = @0x2;
        
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        
        assert!(deny_list::contains(denylist_ref, addr1), 0);
        assert!(!deny_list::contains(denylist_ref, addr2), 1);
    }

    // Test: contains returns false after removal
    #[test]
    fun test_contains_after_removal() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        let addr1 = @0x1;
        
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        assert!(deny_list::contains(denylist_ref, addr1), 0);
        
        deny_list::deny_list_remove<u64>(denylist_ref, &cap, addr1, ctx);
        assert!(!deny_list::contains(denylist_ref, addr1), 1);
    }

    // =================================================================
    // Tests: Large Address Values
    // =================================================================
    
    // Test: Using full-length addresses
    #[test]
    fun test_with_full_length_addresses() {
        let denylist_ref = &mut deny_list::new_denylist();
        let ctx = &mut tx_context::dummy();
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);
        
        // Use realistic full addresses
        let addr1 = @0x0000000000000000000000000000000000000000000000000000000000000001;
        let addr2 = @0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF;
        
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, addr2, ctx);
        
        assert!(deny_list::length(denylist_ref) == 2, 0);
        assert!(deny_list::get_address_at(denylist_ref, 0) == addr1, 1);
        assert!(deny_list::get_address_at(denylist_ref, 1) == addr2, 2);
    }
}
