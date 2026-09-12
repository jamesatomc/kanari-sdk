#[test_only]
module kanari_system::collection_usage_tests {
    use kanari_system::bag;
    use kanari_system::deny_list;
    use kanari_system::dynamic_field;
    use kanari_system::dynamic_object_field;
    use kanari_system::object::{Self, UID};
    use kanari_system::table;
    use kanari_system::tx_context;
    use std::option;

    struct Host has key, store, drop {
        id: UID,
    }

    struct Child has key, store, drop {
        id: UID,
        value: u64,
    }

    fun new_host(ctx: &mut tx_context::TxContext): Host {
        Host { id: object::new(ctx) }
    }

    fun new_child(ctx: &mut tx_context::TxContext, value: u64): Child {
        Child { id: object::new(ctx), value }
    }

    #[test]
    fun bag_round_trip_supports_heterogeneous_entries() {
        let ctx = &mut tx_context::dummy();
        let bag = bag::new(ctx);
        let bag_ref = &mut bag;

        bag::add<u64, u64>(bag_ref, 7, 99);
        bag::add<bool, address>(bag_ref, true, @0x55);

        assert!(bag::length(bag_ref) == 2, 0);
        assert!(bag::contains<u64>(bag_ref, 7), 1);
        assert!(bag::contains<bool>(bag_ref, true), 2);
        assert!(*bag::borrow<u64, u64>(bag_ref, 7) == 99, 3);
        assert!(*bag::borrow<bool, address>(bag_ref, true) == @0x55, 4);

        *bag::borrow_mut<u64, u64>(bag_ref, 7) = 123;
        assert!(*bag::borrow<u64, u64>(bag_ref, 7) == 123, 5);

        let removed_number = bag::remove<u64, u64>(bag_ref, 7);
        let removed_address = bag::remove<bool, address>(bag_ref, true);
        assert!(removed_number == 123, 6);
        assert!(removed_address == @0x55, 7);
        assert!(bag::length(bag_ref) == 0, 8);

        bag::destroy_empty(bag);
    }

    #[test]
    #[expected_failure(location = kanari_system::bag, abort_code = 1)]
    fun bag_destroy_empty_aborts_when_not_empty() {
        let ctx = &mut tx_context::dummy();
        let bag = bag::new(ctx);
        bag::add<u64, u64>(&mut bag, 1, 10);
        bag::destroy_empty(bag);
    }

    #[test]
    fun table_round_trip_supports_typed_entries() {
        let ctx = &mut tx_context::dummy();
        let table = table::new<u64, u64>(ctx);
        let table_ref = &mut table;

        table::add(table_ref, 1, 10);
        table::add(table_ref, 2, 20);
        assert!(table::length(table_ref) == 2, 10);
        assert!(table::contains<u64, u64>(table_ref, 1), 11);
        assert!(*table::borrow<u64, u64>(table_ref, 2) == 20, 12);

        *table::borrow_mut<u64, u64>(table_ref, 2) = 77;
        assert!(*table::borrow<u64, u64>(table_ref, 2) == 77, 13);

        let removed = table::remove<u64, u64>(table_ref, 1);
        assert!(removed == 10, 14);
        assert!(table::length(table_ref) == 1, 15);

        let last_removed = table::remove<u64, u64>(table_ref, 2);
        assert!(last_removed == 77, 16);
        table::destroy_empty<u64, u64>(table);
    }

    #[test]
    #[expected_failure(location = kanari_system::table, abort_code = 1)]
    fun table_destroy_empty_aborts_when_not_empty() {
        let ctx = &mut tx_context::dummy();
        let table = table::new<u64, u64>(ctx);
        table::add(&mut table, 1, 10);
        table::destroy_empty(table);
    }

    #[test]
    fun dynamic_field_direct_usage_round_trip() {
        let ctx = &mut tx_context::dummy();
        let host = new_host(ctx);

        dynamic_field::add<u64, u64>(&mut host.id, 9, 44);
        assert!(dynamic_field::exists_<u64>(&host.id, 9), 20);
        assert!(dynamic_field::exists_with_type<u64, u64>(&host.id, 9), 21);
        assert!(!dynamic_field::exists_with_type<u64, bool>(&host.id, 9), 22);
        assert!(*dynamic_field::borrow<u64, u64>(&host.id, 9) == 44, 23);

        *dynamic_field::borrow_mut<u64, u64>(&mut host.id, 9) = 88;
        assert!(*dynamic_field::borrow<u64, u64>(&host.id, 9) == 88, 24);

        let removed = dynamic_field::remove<u64, u64>(&mut host.id, 9);
        assert!(removed == 88, 25);
        assert!(!dynamic_field::exists_<u64>(&host.id, 9), 26);

        let Host { id } = host;
        object::delete(id);
    }

    #[test]
    #[expected_failure(location = kanari_system::dynamic_field, abort_code = 1)]
    fun dynamic_field_duplicate_add_aborts() {
        let ctx = &mut tx_context::dummy();
        let host = new_host(ctx);

        dynamic_field::add<u64, u64>(&mut host.id, 1, 10);
        dynamic_field::add<u64, u64>(&mut host.id, 1, 20);
    }

    #[test]
    fun dynamic_field_convenience_operations() {
        let ctx = &mut tx_context::dummy();
        let host = new_host(ctx);

        assert!(dynamic_field::add_if_absent(&mut host.id, 4, 10), 25);
        assert!(!dynamic_field::add_if_absent(&mut host.id, 4, 20), 26);
        assert!(dynamic_field::contains(&host.id, 4), 27);

        let previous = dynamic_field::replace(&mut host.id, 4, 30);
        assert!(previous == 10, 28);
        assert!(*dynamic_field::borrow(&host.id, 4) == 30, 29);

        let removed = dynamic_field::remove_if_exists<u64, u64>(&mut host.id, 4);
        assert!(option::is_some(&removed), 30);
        assert!(*option::borrow(&removed) == 30, 31);
        assert!(!dynamic_field::contains(&host.id, 4), 32);

        let missing = dynamic_field::remove_if_exists<u64, u64>(&mut host.id, 4);
        assert!(option::is_none(&missing), 33);

        let Host { id } = host;
        object::delete(id);
    }

    #[test]
    fun dynamic_object_field_direct_usage_round_trip() {
        let ctx = &mut tx_context::dummy();
        let host = new_host(ctx);
        let child = new_child(ctx, 500);

        dynamic_object_field::add<u64, Child>(&mut host.id, 3, child);
        assert!(dynamic_object_field::exists_<u64>(&host.id, 3), 30);
        assert!(dynamic_object_field::exists_with_type<u64, Child>(&host.id, 3), 31);
        assert!(!dynamic_object_field::exists_with_type<u64, Host>(&host.id, 3), 32);
        assert!(dynamic_object_field::borrow<u64, Child>(&host.id, 3).value == 500, 33);

        dynamic_object_field::borrow_mut<u64, Child>(&mut host.id, 3).value = 777;
        assert!(dynamic_object_field::borrow<u64, Child>(&host.id, 3).value == 777, 34);

        let removed = dynamic_object_field::remove<u64, Child>(&mut host.id, 3);
        let Child { id, value } = removed;
        assert!(value == 777, 35);
        object::delete(id);

        let Host { id } = host;
        object::delete(id);
    }

    #[test]
    fun dynamic_object_field_convenience_operations() {
        let ctx = &mut tx_context::dummy();
        let host = new_host(ctx);

        assert!(dynamic_object_field::attach_if_missing(
            &mut host.id,
            5,
            new_child(ctx, 100)
        ), 34);
        assert!(!dynamic_object_field::attach_if_missing(
            &mut host.id,
            5,
            new_child(ctx, 200)
        ), 35);
        assert!(dynamic_object_field::contains(&host.id, 5), 36);

        let previous = dynamic_object_field::replace(&mut host.id, 5, new_child(ctx, 300));
        let Child { id, value } = previous;
        assert!(value == 100, 37);
        object::delete(id);

        let removed = dynamic_object_field::detach<u64, Child>(&mut host.id, 5);
        assert!(option::is_some(&removed), 38);
        let removed_child = option::extract(&mut removed);
        let Child { id, value } = removed_child;
        assert!(value == 300, 39);
        object::delete(id);
        assert!(option::is_none(&removed), 40);

        let missing = dynamic_object_field::detach<u64, Child>(&mut host.id, 5);
        assert!(option::is_none(&missing), 41);

        let Host { id } = host;
        object::delete(id);
    }

    #[test]
    fun deny_list_usage_smoke() {
        let ctx = &mut tx_context::dummy();
        let denylist = deny_list::new_denylist();
        let denylist_ref = &mut denylist;
        let cap = deny_list::new_denycap_for_testing<u64>(ctx);

        deny_list::deny_list_add<u64>(denylist_ref, &cap, @0x1, ctx);
        deny_list::deny_list_add<u64>(denylist_ref, &cap, @0x2, ctx);
        assert!(deny_list::length(denylist_ref) == 2, 40);
        assert!(deny_list::contains(denylist_ref, @0x1), 41);
        assert!(deny_list::contains(denylist_ref, @0x2), 42);

        deny_list::deny_list_remove<u64>(denylist_ref, &cap, @0x1, ctx);
        assert!(deny_list::length(denylist_ref) == 1, 43);
        assert!(!deny_list::contains(denylist_ref, @0x1), 44);
        assert!(deny_list::contains(denylist_ref, @0x2), 45);
    }
}
