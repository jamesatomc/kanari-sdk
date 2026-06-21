// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::object {
    use kanari_system::tx_context;
    use kanari_system::tx_context::TxContext;
    use std::signer;

    const E_MUTABLE_OBJECT_BORROW_DISABLED: u64 = 9005;

    struct UID has store, drop {
        addr: address,
    }

    struct ID has copy, drop, store {
        bytes: address,
    }

    public fun new(ctx: &mut TxContext): UID {
        UID { addr: tx_context::fresh_object_address(ctx) }
    }

    public fun uid_to_inner(uid: &UID): ID {
        ID { bytes: uid.addr }
    }

    public fun id_from_address(bytes: address): ID {
        ID { bytes }
    }

    public fun id_to_address(id: &ID): address {
        id.bytes
    }

    public fun id_to_bytes(id: &ID): vector<u8> {
        signer::address_to_bytes(id.bytes)
    }

    public fun uid_address(u: &UID): address {
        u.addr
    }

    public fun uid_to_u64(u: &UID): u64 {
        signer::address_to_u64(u.addr)
    }

    public fun uid_to_bytes(u: &UID): vector<u8> {
        signer::address_to_bytes(u.addr)
    }

    public fun id_bytes(u: &UID): vector<u8> {
        signer::address_to_bytes(u.addr)
    }

    public native fun save_object<T: key>(obj: &T);

    public fun borrow_global_mut<T: key>(_addr: address): &mut T {
        abort E_MUTABLE_OBJECT_BORROW_DISABLED
    }

    public native fun borrow_global<T: key>(addr: address): &T;

    public fun delete(id: UID) {
        delete_impl(id);
    }

    native fun delete_impl(id: UID);

    #[test]
    fun test_uid_id_getters() {
        let test_addr = @0x1234;
        let test_u64 = signer::address_to_u64(test_addr);
        let uid = UID { addr: test_addr };
        assert!(uid_address(&uid) == test_addr, 0);
        assert!(uid_to_u64(&uid) == test_u64, 1);
        let id = uid_to_inner(&uid);
        assert!(id_to_address(&id) == test_addr, 2);
        let created_id = id_from_address(test_addr);
        assert!(id_to_address(&created_id) == test_addr, 3);
    }
}
