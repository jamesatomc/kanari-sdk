// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::deny_list {
    // A DenyCap is an authorization capability, not a constructor token.
    // Only coin::create_regulated_currency may mint one in production.
    friend kanari_system::coin;

    use std::vector;
    use kanari_system::object;
    use kanari_system::tx_context::TxContext;

    /// Deny list resource storing addresses — per-coin-type (phantom T) to prevent cross-type confusion
    struct DenyList<phantom T> has key, store, drop {
        id: object::UID,
        addresses: vector<address>,
    }

    /// Capability to mutate a DenyList for a specific coin type
    struct DenyCap<phantom T> has key, store, drop {
        id: object::UID,
    }

    /// Return DenyList ID address
    public fun denylist_id<T>(d: &DenyList<T>): address {
        object::uid_address(&d.id)
    }

    /// Create a new empty DenyList — per-type, caller must hold matching DenyCap<T>
    public fun new_denylist<T>(ctx: &mut TxContext): DenyList<T> {
        DenyList<T> { id: object::new(ctx), addresses: vector::empty<address>() }
    }

    #[test_only]
    /// Test helper when type inference needs explicit T
    public fun new_denylist_for_testing<T>(ctx: &mut TxContext): DenyList<T> {
        new_denylist<T>(ctx)
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

    const EZeroAddress: u64 = 0;
    const EDenyListFull: u64 = 1;
    const MAX_DENY_LIST_SIZE: u64 = 1000;

    /// Add an address to the deny list — requires valid DenyCap<T> for the coin type.
    public fun deny_list_add<T>(d: &mut DenyList<T>, _cap: &DenyCap<T>, addr: address, _ctx: &mut TxContext) {
        assert!(addr != @0x0, EZeroAddress);
        assert!(vector::length(&d.addresses) < MAX_DENY_LIST_SIZE, EDenyListFull);
        let len = vector::length(&d.addresses);
        let i = 0;
        while (i < len) {
            let existing_addr = *vector::borrow(&d.addresses, i);
            if (existing_addr == addr) {
                return
            };
            i = i + 1;
        };
        vector::push_back(&mut d.addresses, addr);
        object::save_object(d);
    }

    /// Remove an address from the deny list
    public fun deny_list_remove<T>(d: &mut DenyList<T>, _cap: &DenyCap<T>, addr: address, _ctx: &mut TxContext) {
        assert!(addr != @0x0, EZeroAddress);
        let len = vector::length(&d.addresses);
        let i = 0;
        while (i < len) {
            let existing_addr = *vector::borrow(&d.addresses, i);
            if (existing_addr == addr) {
                vector::remove(&mut d.addresses, i);
                object::save_object(d);
                return
            };
            i = i + 1;
        };
    }

    #[test_only]
    public fun destroy_for_testing<T>(d: DenyList<T>) {
        let DenyList<T> { id, addresses: _ } = d;
        object::delete(id);
    }

    // Get the length of the deny list
    #[test_only]
    public fun length<T>(d: &DenyList<T>): u64 {
        vector::length(&d.addresses)
    }

    // Check if an address is in the deny list
    #[test_only]
    public fun contains<T>(d: &DenyList<T>, addr: address): bool {
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
    public fun get_address_at<T>(d: &DenyList<T>, index: u64): address {
        *vector::borrow(&d.addresses, index)
    }
}
