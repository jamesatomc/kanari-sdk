// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::deny_list {
    // A DenyCap is an authorization capability, not a constructor token.
    // Only coin::create_regulated_currency may mint one in production.
    friend kanari_system::coin;

    use std::vector;
    use kanari_system::object;
    use kanari_system::tx_context::TxContext;

    /// Deny list resource storing addresses
    struct DenyList has key, store, drop {
        addresses: vector<address>,
    }

    /// Capability to mutate a DenyList for a specific coin type
    struct DenyCap<phantom T> has key, store, drop {
        id: object::UID,
    }

    /// Create a new empty DenyList
    public fun new_denylist(): DenyList {
        DenyList { addresses: vector::empty<address>() }
    }

    /// Create a DenyCap while constructing a regulated currency.  Keeping this
    /// friend-only prevents an arbitrary transaction from forging a capability
    /// for another asset type and changing its deny list.
    public(friend) fun new_denycap<T>(ctx: &mut TxContext): DenyCap<T> {
        DenyCap<T> { id: object::new(ctx) }
    }

    #[test_only]
    /// Test-only constructor for unit tests.  It is not present in published
    /// bytecode and therefore cannot become a production authorization path.
    public fun new_denycap_for_testing<T>(ctx: &mut TxContext): DenyCap<T> {
        DenyCap<T> { id: object::new(ctx) }
    }

    /// Add an address to the deny list
    public fun deny_list_add<T>(d: &mut DenyList, _cap: &DenyCap<T>, addr: address, _ctx: &mut TxContext) {
        // Check if address already exists in the deny list
        let len = vector::length(&d.addresses);
        let  i = 0;
        while (i < len) {
            let existing_addr = *vector::borrow(&d.addresses, i);
            if (existing_addr == addr) {
                // Address already exists, no need to add again
                return
            };
            i = i + 1;
        };
        
        // Address not found, add it to the deny list
        vector::push_back(&mut d.addresses, addr);
    }

    /// Remove an address from the deny list
    public fun deny_list_remove<T>(d: &mut DenyList, _cap: &DenyCap<T>, addr: address, _ctx: &mut TxContext) {
        let len = vector::length(&d.addresses);
        let i = 0;
        while (i < len) {
            let existing_addr = *vector::borrow(&d.addresses, i);
            if (existing_addr == addr) {
                // Found the address, remove it from the vector
                vector::remove(&mut d.addresses, i);
                return
            };
            i = i + 1;
        };
        // If address not found, do nothing (no-op)
    }

    // Get the length of the deny list
    #[test_only]
    public fun length(d: &DenyList): u64 {
        vector::length(&d.addresses)
    }

    // Check if an address is in the deny list
    #[test_only]
    public fun contains(d: &DenyList, addr: address): bool {
        let len = vector::length(&d.addresses);
        let  i = 0;
        while (i < len) {
            let existing_addr = *vector::borrow(&d.addresses, i);
            if (existing_addr == addr) {
                return true
            };
            i = i + 1;
        };
        false
    }

    // Get address at index (for testing purposes)
    #[test_only]
    public fun get_address_at(d: &DenyList, index: u64): address {
        *vector::borrow(&d.addresses, index)
    }
}
