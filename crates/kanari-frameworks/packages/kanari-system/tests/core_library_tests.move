// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#[test_only]
module kanari_system::core_library_tests {
    use kanari_system::address;
    use kanari_system::base64;
    use kanari_system::clock;
    use kanari_system::tx_context;
    use kanari_system::url;
    use std::ascii;
    use std::string;
    use std::vector;

    #[test]
    fun address_round_trip_and_formatting() {
        let value = @0x1234;
        let bytes = address::to_bytes(value);

        assert!(vector::length(&bytes) == address::length(), 0);
        assert!(address::from_bytes(bytes) == value, 1);
        assert!(address::from_ascii_bytes(
            &b"0000000000000000000000000000000000000000000000000000000000001234"
        ) == value, 2);
        assert!(address::to_string(value) == string::utf8(
            b"0000000000000000000000000000000000000000000000000000000000001234"
        ), 3);
    }

    #[test]
    #[expected_failure(location = kanari_system::address, abort_code = 0)]
    fun address_rejects_invalid_ascii_length() {
        address::from_ascii_bytes(&b"1234");
    }

    #[test]
    fun base64_encode_decode_round_trip() {
        let input = b"Kanari";
        let encoded = base64::encode(&input);
        assert!(encoded == b"S2FuYXJp", 10);
        assert!(base64::decode(&encoded) == input, 11);
    }

    #[test]
    fun url_update_replaces_value() {
        let value = url::new_unsafe_from_bytes(b"https://kanari.network/old");
        let mut_value = &mut value;
        url::update(mut_value, ascii::string(b"https://kanari.network/new"));
        assert!(url::inner_url(mut_value) == ascii::string(
            b"https://kanari.network/new"
        ), 20);
    }

    #[test]
    fun clock_testing_helpers_track_monotonic_time() {
        let ctx = &mut tx_context::dummy();
        let clock_value = clock::create_for_testing(ctx);

        assert!(clock::timestamp_ms(&clock_value) == 0, 30);
        clock::increment_for_testing(&mut clock_value, 1000);
        assert!(clock::timestamp_ms(&clock_value) == 1000, 31);
        clock::set_for_testing(&mut clock_value, 2500);
        assert!(clock::timestamp_ms(&clock_value) == 2500, 32);

        clock::destroy_for_testing(clock_value);
    }

    #[test]
    #[expected_failure(location = kanari_system::clock, abort_code = 1)]
    fun clock_rejects_backwards_time() {
        let ctx = &mut tx_context::dummy();
        let clock_value = clock::create_for_testing(ctx);
        clock::set_for_testing(&mut clock_value, 1);
        clock::set_for_testing(&mut clock_value, 0);
        clock::destroy_for_testing(clock_value);
    }

    #[test]
    fun tx_context_exposes_metadata_and_unique_ids() {
        let sender = @0x42;
        let ctx = &mut tx_context::new_from_hint(sender, 7, 9, 1234, 0);

        assert!(tx_context::sender(ctx) == sender, 40);
        assert!(tx_context::epoch(ctx) == 9, 41);
        assert!(tx_context::epoch_timestamp_ms(ctx) == 1234, 42);
        assert!(tx_context::get_ids_created(ctx) == 0, 43);

        let first = tx_context::fresh_object_address(ctx);
        assert!(tx_context::get_ids_created(ctx) == 1, 44);
        assert!(tx_context::last_created_object_id(ctx) == first, 45);

        tx_context::increment_epoch_number(ctx);
        tx_context::increment_epoch_timestamp(ctx, 500);
        assert!(tx_context::epoch(ctx) == 10, 46);
        assert!(tx_context::epoch_timestamp_ms(ctx) == 1734, 47);
    }
}