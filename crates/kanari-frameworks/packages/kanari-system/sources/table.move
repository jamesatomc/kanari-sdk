// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::table {
    use kanari_system::object::{Self, UID};
    use kanari_system::tx_context::TxContext;
    use kanari_system::dynamic_field;

    #[test_only]
    use kanari_system::tx_context;

    /// Error codes
    const ETableNotEmpty: u64 = 1;

    /// A Table map that stores key-value pairs dynamically
    struct Table<phantom K: copy + drop + store, phantom V: store> has key, store {
        id: UID,
        size: u64,
    }

    /// Creates a new, empty table
    public fun new<K: copy + drop + store, V: store>(ctx: &mut TxContext): Table<K, V> {
        Table {
            id: object::new(ctx),
            size: 0,
        }
    }

    /// Adds a key-value pair to the table
    public fun add<K: copy + drop + store, V: store>(table: &mut Table<K, V>, k: K, v: V) {
        dynamic_field::add(&mut table.id, k, v);
        table.size = table.size + 1;
    }

    /// Mutably borrows the value associated with the key
    public fun borrow_mut<K: copy + drop + store, V: store>(table: &mut Table<K, V>, k: K): &mut V {
        dynamic_field::borrow_mut(&mut table.id, k)
    }

    /// Immutably borrows the value associated with the key
    public fun borrow<K: copy + drop + store, V: store>(table: &Table<K, V>, k: K): &V {
        dynamic_field::borrow(&table.id, k)
    }

    /// Removes the key-value pair and returns the value
    public fun remove<K: copy + drop + store, V: store>(table: &mut Table<K, V>, k: K): V {
        let v = dynamic_field::remove(&mut table.id, k);
        table.size = table.size - 1;
        v
    }

    /// Returns true if the table contains the key
    public fun contains<K: copy + drop + store, V: store>(table: &Table<K, V>, k: K): bool {
        dynamic_field::exists_(&table.id, k)
    }

    /// Returns the number of entries in the table
    public fun length<K: copy + drop + store, V: store>(table: &Table<K, V>): u64 {
        table.size
    }

    /// Destroys an empty table
    public fun destroy_empty<K: copy + drop + store, V: store>(table: Table<K, V>) {
        let Table { id, size } = table;
        assert!(size == 0, ETableNotEmpty);
        object::delete(id);
    }

    #[test]
    fun test_table_dynamic_field_borrow_roundtrip() {
        let ctx = tx_context::dummy();
        let table = new<u64, u64>(&mut ctx);

        add(&mut table, 7, 11);
        assert!(contains(&table, 7), 0);
        assert!(*borrow(&table, 7) == 11, 1);

        *borrow_mut(&mut table, 7) = 42;
        assert!(*borrow(&table, 7) == 42, 2);
        assert!(length(&table) == 1, 3);

        let value = remove(&mut table, 7);
        assert!(value == 42, 4);
        assert!(!contains(&table, 7), 5);

        destroy_empty(table);
    }
}
