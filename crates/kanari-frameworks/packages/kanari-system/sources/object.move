// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::object {
    use kanari_system::tx_context;
    use kanari_system::tx_context::TxContext;
    use std::signer;

    /// Simple UID wrapper used for resource IDs in this package.
    /// The UID contains an object-style address generated from the
    /// transaction context, ensuring it is unique per creation.
    struct UID has store, drop {
        addr: address,
    }

    /// ID is a copyable, storable identifier for an object.
    /// It is used to reference objects without requiring ownership of the UID.
    struct ID has copy, drop, store {
        bytes: address,
    }

    // --- Public Creator ---

    /// Create a new UID by deriving a fresh object address from the
    /// transaction context. This ensures the address is unique and based on the
    /// current transaction input (e.g., transaction hash, counter).
    /// Used by resources that need a guaranteed unique ID when they are created.
    public fun new(ctx: &mut TxContext): UID {
        UID { addr: tx_context::fresh_object_address(ctx) }
    }

    // --- ID Getters & Converters ---

    /// Extract an `ID` from a `UID`.
    public fun uid_to_inner(uid: &UID): ID {
        ID { bytes: uid.addr }
    }

    /// Create an `ID` directly from an address.
    public fun id_from_address(bytes: address): ID {
        ID { bytes }
    }

    /// Get the underlying address of an `ID`.
    public fun id_to_address(id: &ID): address {
        id.bytes
    }

    /// Get the address of an `ID` as a byte vector.
    public fun id_to_bytes(id: &ID): vector<u8> {
        signer::address_to_bytes(id.bytes)
    }

    // --- UID Getters ---

    /// Return the underlying address for a UID.
    /// This is the canonical representation of the object's ID.
    public fun uid_address(u: &UID): address {
        u.addr
    }

    /// Return the object's address as a `u64` value.
    public fun uid_to_u64(u: &UID): u64 {
        signer::address_to_u64(u.addr)
    }

    /// Return the object's address as a `vector<u8>`.
    public fun uid_to_bytes(u: &UID): vector<u8> {
        signer::address_to_bytes(u.addr)
    }

    /// Return the object's address as a `vector<u8>`.
    /// This is useful for serialization, hashing, and interoperability across modules.
    public fun id_bytes(u: &UID): vector<u8> {
        signer::address_to_bytes(u.addr)
    }

    // --- Native Persistence ---
    // Explicitly request the runtime to persist changes to an object reference.
    // The runtime must only supply mutable references after ownership/shared-object
    // authorization has completed.
    public native fun save_object<T: key>(obj: &T);

    /// Internal-only legacy loader retained for runtime compatibility.
    ///
    /// This function is intentionally not public. Arbitrary published modules must
    /// receive mutable object references as transaction inputs so the trusted runtime
    /// can authenticate ownership before Move execution begins.
    public native fun borrow_global_mut<T: key>(addr: address): &mut T;

    /// Load an object from storage by its address and return an immutable reference.
    /// Read access does not grant mutation or persistence authority.
    public native fun borrow_global<T: key>(addr: address): &T;

    /// Delete an object by consuming its UID.
    /// This removes the object from storage and potentially triggers a storage rebate.
    public fun delete(id: UID) {
        delete_impl(id);
    }

    native fun delete_impl(id: UID);

    // --- Tests ---
    #[test]
    fun test_uid_id_getters() {
        let test_addr = @0x1234;
        let test_u64 = signer::address_to_u64(test_addr);

        let uid = UID { addr: test_addr };
        
        // 1. Check UID address
        assert!(uid_address(&uid) == test_addr, 0);

        // 2. Check u64 conversion
        assert!(uid_to_u64(&uid) == test_u64, 1);

        // 3. Test ID Extraction
        let id = uid_to_inner(&uid);
        assert!(id_to_address(&id) == test_addr, 2);

        // 4. Test ID to Address mapping
        let created_id = id_from_address(test_addr);
        assert!(id_to_address(&created_id) == test_addr, 3);
    }
}
