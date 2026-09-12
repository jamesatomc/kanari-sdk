// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

#[test_only]
module kanari_system::coin_pay_tests {
    use kanari_system::coin;
    use kanari_system::pay;
    use kanari_system::transfer;
    use kanari_system::tx_context;
    use std::option;
    use std::vector;

    struct TestCoin has drop {}

    fun new_currency(
        ctx: &mut tx_context::TxContext,
    ): (coin::TreasuryCap<TestCoin>, coin::CoinMetadata<TestCoin>) {
        coin::create_currency<TestCoin>(
            TestCoin {},
            9,
            b"TST",
            b"Test Coin",
            b"Coin used by unit tests",
            option::none(),
            ctx,
        )
    }

    #[test]
    fun mint_split_join_and_burn_preserves_supply() {
        let ctx = &mut tx_context::dummy();
        let (cap, metadata) = new_currency(ctx);

        let coin = coin::mint(&mut cap, 1000, ctx);
        assert!(coin::total_supply(&cap) == 1000, 0);
        assert!(coin::value(&coin) == 1000, 1);

        let split = coin::split(&mut coin, 250, ctx);
        assert!(coin::value(&coin) == 750, 2);
        assert!(coin::value(&split) == 250, 3);

        coin::join(&mut coin, split);
        assert!(coin::value(&coin) == 1000, 4);

        let burned = coin::burn(&mut cap, coin);
        assert!(burned == 1000, 5);
        assert!(coin::total_supply(&cap) == 0, 6);

        transfer::public_transfer(cap, @0x1);
        transfer::public_transfer(metadata, @0x1);
    }

    #[test]
    #[expected_failure(location = kanari_system::coin, abort_code = 3)]
    fun mint_zero_aborts() {
        let ctx = &mut tx_context::dummy();
        let (cap, metadata) = new_currency(ctx);
        coin::mint(&mut cap, 0, ctx);
        transfer::public_transfer(cap, @0x1);
        transfer::public_transfer(metadata, @0x1);
    }

    #[test]
    fun divide_into_n_keeps_total_value() {
        let ctx = &mut tx_context::dummy();
        let (cap, metadata) = new_currency(ctx);
        let original = coin::mint(&mut cap, 100, ctx);

        let pieces = coin::divide_into_n(&mut original, 4, ctx);
        assert!(vector::length(&pieces) == 3, 10);
        assert!(coin::value(&original) == 25, 11);

        let total = coin::zero<TestCoin>(ctx);
        while (!vector::is_empty(&pieces)) {
            coin::join(&mut total, vector::pop_back(&mut pieces));
        };
        vector::destroy_empty(pieces);
        assert!(coin::value(&total) == 75, 12);

        coin::join(&mut total, original);
        assert!(coin::value(&total) == 100, 13);
        let burned = coin::burn(&mut cap, total);
        assert!(burned == 100, 14);

        transfer::public_transfer(cap, @0x1);
        transfer::public_transfer(metadata, @0x1);
    }

    #[test]
    #[expected_failure(location = kanari_system::pay, abort_code = 0)]
    fun join_vec_and_transfer_rejects_empty_vector() {
        let coins = vector::empty<coin::Coin<TestCoin>>();
        pay::join_vec_and_transfer(coins, @0x2);
    }
}