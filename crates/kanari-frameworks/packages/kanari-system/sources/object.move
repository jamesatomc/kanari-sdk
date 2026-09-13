// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::object {
    use kanari_system::tx_context;
    use kanari_system::tx_context::TxContext;
    use std::signer;

    /// Globally unique ID of an object. Sui-compatible layout: `UID { id: ID }`.
    /// NOTE: keeps `drop` as a Kanari extension (Sui's UID has `store` only).
    /// `drop` is permissive — Sui code calling `object::delete(uid)` still
    /// compiles and behaves identically. BCS layout is identical (32 bytes).
    struct UID has store, drop {
        id: ID,
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
        UID { id: ID { bytes: tx_context::fresh_object_address(ctx) } }
    }

    // --- ID Getters & Converters ---

    /// Get a reference to the inner `ID` of a `UID` (Sui API).
    public fun uid_as_inner(uid: &UID): &ID {
        &uid.id
    }

    /// Extract an `ID` from a `UID`.
    public fun uid_to_inner(uid: &UID): ID {
        uid.id
    }

    /// Create an `ID` directly from an address.
    public fun id_from_address(bytes: address): ID {
        ID { bytes }
    }

    /// Make an `ID` from raw bytes (Sui API). Aborts unless 32 bytes.
    public fun id_from_bytes(bytes: vector<u8>): ID {
        ID { bytes: kanari_system::address::from_bytes(bytes) }
    }

    /// Get the underlying address of an `ID`.
    public fun id_to_address(id: &ID): address {
        id.bytes
    }

    /// Get the address of an `ID` as a byte vector.
    public fun id_to_bytes(id: &ID): vector<u8> {
        signer::address_to_bytes(id.bytes)
    }

    /// Get the raw bytes for the underlying `ID` of `obj` (Sui API).
    public fun id_bytes<T: key>(obj: &T): vector<u8> {
        id_to_bytes(&id(obj))
    }

    /// Get the inner address for the underlying `ID` of `obj` (Sui API).
    public fun id_address<T: key>(obj: &T): address {
        id_to_address(&id(obj))
    }

    // --- UID Getters ---

    /// Return the underlying address for a UID.
    /// This is the canonical representation of the object's ID.
    public fun uid_address(u: &UID): address {
        u.id.bytes
    }

    /// Sui-compatible alias for `uid_address`.
    public fun uid_to_address(uid: &UID): address {
        uid.id.bytes
    }

    /// Return the object's address as a `u64` value.
    public fun uid_to_u64(u: &UID): u64 {
        signer::address_to_u64(u.id.bytes)
    }

    /// Return the object's address as a `vector<u8>`.
    public fun uid_to_bytes(u: &UID): vector<u8> {
        signer::address_to_bytes(u.id.bytes)
    }

    /// Return the object's address as a `vector<u8>`.
    /// This is useful for serialization, hashing, and interoperability across modules.
    /// NOTE: Sui names the `&UID -> vector<u8>` getter `uid_to_bytes`;
    /// this `UID`-taking overload is kept for Kanari callers.
    public fun id_bytes_from_uid(u: &UID): vector<u8> {
        signer::address_to_bytes(u.id.bytes)
    }

    // --- Native Persistence ---
    // Explicitly request the runtime to persist changes to an object reference.
    // The runtime must only supply mutable references after ownership/shared-object
    // authorization has completed.
    public native fun save_object<T: key>(obj: &T);

    /// Returns the ID of an object (first field UID). Used by hot-potato patterns like `borrow`.
    public native fun id<T: key>(obj: &T): ID;

    // Internal-only legacy loader retained for runtime compatibility.
    // SECURITY: `#[test_only]` — arbitrary published modules must receive
    // mutable object references as transaction inputs so the trusted runtime
    // can authenticate ownership before Move execution begins. A public
    // `borrow_global_mut` would let any module mutate any object by address,
    // bypassing ownership checks. Only tests may use it.
    #[test_only]
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

        let uid = UID { id: ID { bytes: test_addr } };

        // 1. Check UID address
        assert!(uid_address(&uid) == test_addr, 0);
        assert!(uid_to_address(&uid) == test_addr, 0);

        // 2. Check u64 conversion
        assert!(uid_to_u64(&uid) == test_u64, 1);

        // 3. Test ID Extraction
        let id = uid_to_inner(&uid);
        assert!(id_to_address(&id) == test_addr, 2);
        assert!(id_to_address(uid_as_inner(&uid)) == test_addr, 2);

        // 4. Test ID to Address mapping
        let created_id = id_from_address(test_addr);
        assert!(id_to_address(&created_id) == test_addr, 3);
        assert!(id_to_address(&id_from_bytes(signer::address_to_bytes(test_addr))) == test_addr, 3);
    }
}
