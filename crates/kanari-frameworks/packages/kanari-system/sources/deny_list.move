// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::deny_list {
    // A DenyCap is an authorization capability, not a constructor token.
    // Only coin::create_regulated_currency may mint one in production.
    friend kanari_system::coin;

    use std::vector;
    use kanari_system::object;
    use kanari_system::tx_context::TxContext;

    /// SECURITY: `DenyList<phantom T>` is bound to its `DenyCap<T>` type.
    /// Previously `DenyList` was non-generic, so *any* `DenyCap<T>` could
    /// add/remove *any* list — cross-currency deny-list confusion.
    struct DenyList<phantom T> has key, store, drop {
        id: object::UID,
        addresses: vector<address>,
    }

    struct DenyCap<phantom T> has key, store, drop {
        id: object::UID,
    }

    public fun denylist_id<T>(d: &DenyList<T>): address {
        object::uid_address(&d.id)
    }

    public fun new_denylist<T>(ctx: &mut TxContext): DenyList<T> {
        DenyList { id: object::new(ctx), addresses: vector::empty<address>() }
    }

    #[test_only]
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

    #[test_only]
    public fun destroy_for_testing<T>(d: DenyList<T>) {
        let DenyList { id, addresses: _ } = d;
        object::delete(id);
    }

    #[test_only]
    public fun destroy_cap_for_testing<T>(c: DenyCap<T>) {
        let DenyCap<T> { id } = c;
        object::delete(id);
    }

    const EZeroAddress: u64 = 2;
    const EDenyListFull: u64 = 3;
    const MAX_DENY_LIST_SIZE: u64 = 1000;

    public fun deny_list_add<T>(d: &mut DenyList<T>, _cap: &DenyCap<T>, addr: address, _ctx: &mut TxContext) {
        assert!(addr != @0x0, EZeroAddress);
        assert!(vector::length(&d.addresses) < MAX_DENY_LIST_SIZE, EDenyListFull);
        let len = vector::length(&d.addresses);
        let i = 0;
        while (i < len) {
            if (*vector::borrow(&d.addresses, i) == addr) return;
            i = i + 1;
        };
        vector::push_back(&mut d.addresses, addr);
        object::save_object(d);
    }

    public fun deny_list_remove<T>(d: &mut DenyList<T>, _cap: &DenyCap<T>, addr: address, _ctx: &mut TxContext) {
        assert!(addr != @0x0, EZeroAddress);
        let len = vector::length(&d.addresses);
        let i = 0;
        while (i < len) {
            if (*vector::borrow(&d.addresses, i) == addr) {
                vector::remove(&mut d.addresses, i);
                object::save_object(d);
                return
            };
            i = i + 1;
        };
    }

    #[test_only]
    public fun length<T>(d: &DenyList<T>): u64 {
        vector::length(&d.addresses)
    }

    /// Returns true iff `addr` is on the list. Public (not just test-only)
    /// so `coin::deny_list_contains` (Sui API) can use it on-chain.
    public fun contains<T>(d: &DenyList<T>, addr: address): bool {
        let len = vector::length(&d.addresses);
        let i = 0;
        while (i < len) {
            if (*vector::borrow(&d.addresses, i) == addr) return true;
            i = i + 1;
        };
        false
    }

    #[test_only]
    public fun get_address_at<T>(d: &DenyList<T>, index: u64): address {
        *vector::borrow(&d.addresses, index)
    }
}
