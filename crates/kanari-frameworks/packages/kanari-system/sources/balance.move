// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::balance {
    friend kanari_system::coin;
    friend kanari_system::pay;

    /// Error codes
    const ERR_INSUFFICIENT_BALANCE: u64 = 1;
    const ERR_OVERFLOW: u64 = 2;
    const ERR_ZERO_AMOUNT: u64 = 3; // Cannot decrease, transfer, or mint an amount of zero.

    /// Balance resource - Stores the balance value (generic per token type)
    struct Balance<phantom T> has store, drop {
        value: u64,
    }

    /// Supply: mutable minting handle consumed to create balances
    struct Supply<phantom T> has store, drop {
        total: u64,
    }

    /// Create a new zero-value Balance — package-internal, use coin::zero instead.
    public(friend) fun zero<T>(): Balance<T> {
        Balance<T> { value: 0 }
    }

    /// Create a new Balance with an initial value — package-internal, use coin::mint instead.
    public(friend) fun create<T>(value: u64): Balance<T> {
        Balance<T> { value }
    }

    /// Get the current balance value
    public fun value<T>(balance: &Balance<T>): u64 {
        balance.value
    }

    /// Increase the balance value — package-internal
    public(friend) fun increase<T>(balance: &mut Balance<T>, amount: u64) {
        let new_value = balance.value + amount;
        assert!(new_value >= balance.value, ERR_OVERFLOW);
        balance.value = new_value;
    }

    /// Decrease the balance value — package-internal
    public(friend) fun decrease<T>(balance: &mut Balance<T>, amount: u64) {
        assert!(amount > 0, ERR_ZERO_AMOUNT);
        assert!(balance.value >= amount, ERR_INSUFFICIENT_BALANCE);
        balance.value = balance.value - amount;
    }

    /// Transfer value from one Balance to another — package-internal
    public(friend) fun transfer<T>(from: &mut Balance<T>, to: &mut Balance<T>, amount: u64) {
        assert!(amount > 0, ERR_ZERO_AMOUNT);
        decrease<T>(from, amount);
        increase<T>(to, amount);
    }

    /// Check if the balance is sufficient for a given amount
    public fun has_sufficient<T>(balance: &Balance<T>, amount: u64): bool {
        balance.value >= amount
    }

    /// Destroy the Balance and return its value — package-internal
    public(friend) fun destroy<T>(balance: Balance<T>): u64 {
        let Balance { value } = balance;
        value
    }

    /// Create a new (empty) supply handle
    public fun new_supply<T>(): Supply<T> {
        Supply<T> { total: 0 }
    }

    /// Increase supply: add `amount` to `s` and return a `Balance` for the newly minted amount.
    public(friend) fun increase_supply<T>(s: &mut Supply<T>, amount: u64): Balance<T> {
        assert!(amount > 0, ERR_ZERO_AMOUNT);
        let new_total = s.total + amount;
        assert!(new_total >= s.total, ERR_OVERFLOW);
        s.total = new_total;
        create<T>(amount)
    }

    /// Decrease/destroy a supply handle (legacy)
    public fun destroy_supply<T>(s: Supply<T>) {
        let Supply { total: _ } = s;
    }

    /// Decrease supply by `amount`. Useful for burning coins.
    public(friend) fun decrease_supply<T>(s: &mut Supply<T>, amount: u64) {
        assert!(amount > 0, ERR_ZERO_AMOUNT);
        assert!(s.total >= amount, ERR_INSUFFICIENT_BALANCE);
        s.total = s.total - amount;
    }

    /// Read the current total supply value from a supply handle.
    public fun supply_total<T>(s: &Supply<T>): u64 {
        s.total
    }

    /// Merge two Balances together — package-internal
    public(friend) fun merge<T>(dst: &mut Balance<T>, src: Balance<T>) {
        let value = destroy<T>(src);
        increase<T>(dst, value);
    }

    /// Split the Balance into two — package-internal
    public(friend) fun split<T>(balance: &mut Balance<T>, amount: u64): Balance<T> {
        decrease<T>(balance, amount);
        create<T>(amount)
    }

    #[test]
    fun test_balance_operations() {
        let balance = create<u8>(1000);
        assert!(value(&balance) == 1000, 0);

        increase<u8>(&mut balance, 500);
        assert!(value(&balance) == 1500, 1);

        decrease<u8>(&mut balance, 300);
        assert!(value(&balance) == 1200, 2);

        let final_value = destroy<u8>(balance);
        assert!(final_value == 1200, 3);
    }

    #[test]
    fun test_transfer() {
        let balance1 = create<u8>(1000);
        let balance2 = create<u8>(500);

        transfer<u8>(&mut (balance1), &mut (balance2), 300);

        assert!(value(&balance1) == 700, 0);
        assert!(value(&balance2) == 800, 1);

        destroy<u8>(balance1);
        destroy<u8>(balance2);
    }

    #[test]
    fun test_split_merge() {
        let balance1 = create<u8>(1000);
        let balance2 = split<u8>(&mut (balance1), 400);

        assert!(value(&balance1) == 600, 0);
        assert!(value(&balance2) == 400, 1);

        merge<u8>(&mut balance1, balance2);
        assert!(value(&balance1) == 1000, 2);

        destroy<u8>(balance1);
    }

    #[test]
    #[expected_failure(abort_code = ERR_INSUFFICIENT_BALANCE)]
    fun test_insufficient_balance() {
        let balance = create<u8>(100);
        decrease<u8>(&mut (balance), 200);
        destroy<u8>(balance);
    }

    #[test]
    fun test_supply_operations() {
        let s = new_supply<u8>();
        let b1 = increase_supply<u8>(&mut s, 1000);
        assert!(supply_total(&s) == 1000, 0);
        let b2 = increase_supply<u8>(&mut s, 500);
        assert!(value(&b2) == 500, 1);
        assert!(supply_total(&s) == 1500, 2);
        decrease_supply<u8>(&mut s, 800);
        assert!(supply_total(&s) == 700, 3);
        destroy<u8>(b1);
        destroy<u8>(b2);
    }

    #[test]
    #[expected_failure(abort_code = ERR_INSUFFICIENT_BALANCE)]
    fun test_decrease_supply_insufficient() {
        let s = new_supply<u8>();
        increase_supply<u8>(&mut s, 100);
        decrease_supply<u8>(&mut s, 200);
    }
}