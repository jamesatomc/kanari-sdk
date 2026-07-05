// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Production-Ready Transfer Module
/// Uses proper address types with validation
module kanari_system::transfer {
    use std::vector;
    use kanari_system::object::UID;

    /// Error codes
    const ERR_INVALID_AMOUNT: u64 = 1;
    const ERR_SAME_ADDRESS: u64 = 2;

    /// Transfer record
    struct Transfer has copy, drop {
        from: address,
        to: address,
        amount: u64,
    }

    // ObjectStore: Global registry for transferred objects
    // Stores objects by owner address for later retrieval
    #[allow(unused_field)]
    struct ObjectStore<T: key + store> has key {
        id: UID,
        inner: T,
        owner: address,
    }

    /// Create a transfer record with full validation
    /// Validates: amount > 0 AND from != to
    public fun create_transfer(from: address, to: address, amount: u64): Transfer {
        assert!(amount > 0, ERR_INVALID_AMOUNT);
        assert!(from != to, ERR_SAME_ADDRESS);
        Transfer { from, to, amount }
    }

    /// Get transfer details
    public fun get_amount(transfer: &Transfer): u64 {
        transfer.amount
    }

    public fun get_from(transfer: &Transfer): address {
        transfer.from
    }

    public fun get_to(transfer: &Transfer): address {
        transfer.to
    }

    /// Calculate total from multiple transfers
    public fun total_amount(transfers: &vector<Transfer>): u64 {
        let total = 0u64;
        let len = vector::length(transfers);
        let i = 0u64;
        
        while (i < len) {
            let transfer = vector::borrow(transfers, i);
            total = total + transfer.amount;
            i = i + 1;
        };
        
        total
    }

    /// Check if amount is valid (non-zero)
    public fun is_valid_amount(amount: u64): bool {
        amount > 0
    }

    #[test]
    fun test_create_transfer() {
        let addr1 = @0x100;
        let addr2 = @0x200;
        let t = create_transfer(addr1, addr2, 500);
        assert!(get_from(&t) == addr1, 0);
        assert!(get_to(&t) == addr2, 1);
        assert!(get_amount(&t) == 500, 2);
    }

    /// Freeze an object (make it immutable).
    /// The object must have `key` and `store`.
    public native fun freeze_object<T: key + store>(obj: T);

    /// Helper to 'freeze' a metadata object returned by currency creation.
    /// Redirects to `freeze_object` native.
    public fun public_freeze_object<T: key + store>(obj: T) {
        freeze_object(obj)
    }

    /// Transfer an owned object to another address.
    public fun public_transfer<T: key + store>(obj: T, recipient: address) {
        transfer_with_uid(obj, recipient);
    }

    /// Internal transfer that extracts UID for tracking
    native fun transfer_with_uid<T: key + store>(obj: T, recipient: address);

    /// Share an object by returning it instead of transferring
    /// The caller should handle storage. This is a workaround for object tracking.
    public fun share_object<T: store>(obj: T): T {
        obj
    }

    #[test]
    fun test_total_amount() {
        let transfers = vector::empty<Transfer>();
        vector::push_back(&mut transfers, create_transfer(@0x1, @0x2, 100));
        vector::push_back(&mut transfers, create_transfer(@0x2, @0x3, 200));
        vector::push_back(&mut transfers, create_transfer(@0x3, @0x4, 300));
        
        assert!(total_amount(&transfers) == 600, 0);
    }

    #[test]
    #[expected_failure(abort_code = ERR_INVALID_AMOUNT)]
    fun test_create_transfer_zero_amount() {
        create_transfer(@0x1, @0x2, 0);
    }

    #[test]
    #[expected_failure(abort_code = ERR_SAME_ADDRESS)]
    fun test_create_transfer_same_address() {
        create_transfer(@0x1, @0x1, 100);
    }
}