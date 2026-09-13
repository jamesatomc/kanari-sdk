// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::pay {
    use kanari_system::tx_context::{Self, TxContext};
    use kanari_system::coin::{Self, Coin};
    use kanari_system::transfer;
    use std::vector;

    /// For when empty vector is supplied into join function.
    const ENoCoins: u64 = 0;

    // #[allow(lint(self_transfer))]
    /// Transfer `c` to the sender of the current transaction
    public fun keep<T>(c: Coin<T>, ctx: &TxContext) {
        transfer::public_transfer(c, tx_context::sender(ctx))
    }

    /// Split coin `self` to two coins, one with balance `split_amount`,
    /// and the remaining balance is left is `self`.
    /// SECURITY: `>=` (not `>`) so the full balance can be split out.
    public entry fun split<T>(
        self: &mut Coin<T>, split_amount: u64, ctx: &mut TxContext
    ) {
        assert!(split_amount > 0, 1);
        assert!(coin::value(self) >= split_amount, 2);
        keep(coin::split(self, split_amount, ctx), ctx)
    }

    /// Split coin `self` into multiple coins, each with balance specified
    /// in `split_amounts`. Remaining balance is left in `self`.
    public entry fun split_vec<T>(
        self: &mut Coin<T>, split_amounts: vector<u64>, ctx: &mut TxContext
    ) {
        let (i, len) = (0, vector::length(&split_amounts));
        while (i < len) {
            split(self, *vector::borrow(&split_amounts, i), ctx);
            i = i + 1;
        };
    }

    /// Send `amount` units of `c` to `recipient`
    public entry fun split_and_transfer<T>(
        c: &mut Coin<T>, amount: u64, recipient: address, ctx: &mut TxContext
    ) {
        assert!(amount > 0, 1);
        assert!(recipient != @0x0, 2);
        assert!(coin::value(c) >= amount, 3);
        transfer::public_transfer(coin::split(c, amount, ctx), recipient)
    }


    // #[allow(lint(self_transfer))]
    /// Divide coin `self` into `n - 1` coins with equal balances. If the balance is
    /// not evenly divisible by `n`, the remainder is left in `self`.
    public entry fun divide_and_keep<T>(
        self: &mut Coin<T>, n: u64, ctx: &mut TxContext
    ) {
        let vec: vector<Coin<T>> = coin::divide_into_n(self, n, ctx);
        let (i, len) = (0, vector::length(&vec));
        while (i < len) {
            transfer::public_transfer(vector::pop_back(&mut vec), tx_context::sender(ctx));
            i = i + 1;
        };
        vector::destroy_empty(vec);
    }

    /// Join `coin` into `self`. Re-exports `coin::join` function.
    public entry fun join<T>(self: &mut Coin<T>, coin: Coin<T>) {
        coin::join(self, coin)
    }

    /// Join everything in `coins` with `self`
    public entry fun join_vec<T>(self: &mut Coin<T>, coins: vector<Coin<T>>) {
        let (i, len) = (0, vector::length(&coins));
        while (i < len) {
            let coin = vector::pop_back(&mut coins);
            coin::join(self, coin);
            i = i + 1
        };
        // safe because we've drained the vector
        vector::destroy_empty(coins)
    }

    /// Join a vector of `Coin` into a single object and transfer it to `receiver`.
    public entry fun join_vec_and_transfer<T>(coins: vector<Coin<T>>, receiver: address) {
        assert!(vector::length(&coins) > 0, ENoCoins);
        assert!(receiver != @0x0, 1);
        let self = vector::pop_back(&mut coins);
        join_vec(&mut self, coins);
        transfer::public_transfer(self, receiver)
    }
}