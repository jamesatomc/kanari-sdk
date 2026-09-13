// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::balance {
    friend kanari_system::coin;
    friend kanari_system::pay;

    /// Error codes
    const ERR_INSUFFICIENT_BALANCE: u64 = 1;
    const ERR_OVERFLOW: u64 = 2;
    const ERR_ZERO_AMOUNT: u64 = 3; // Cannot decrease, transfer, or mint an amount of zero.
    /// For when trying to destroy a non-zero balance (Sui `ENonZero` code).
    const ENonZero: u64 = 0;

    /// Balance resource - Stores the balance value (generic per token type)
    /// SECURITY: no `drop` — a droppable Balance could be silently burned,
    /// desyncing `TreasuryCap.total_supply`. Must go through `destroy`/`merge`/`split`
    /// (friend-only) or `coin::from_balance` round-trip.
    struct Balance<phantom T> has store {
        value: u64,
    }

    /// Supply: mutable minting handle consumed to create balances
    /// SECURITY: no `drop` — dropping a Supply with `total > 0` would lose
    /// supply accounting. Must call `destroy_supply` explicitly.
    struct Supply<phantom T> has store {
        total: u64,
    }

    /// Create a zero `Balance` for type `T` (Sui API).
    public fun zero<T>(): Balance<T> {
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

    /// Join two balances together (Sui API). Returns the new balance value.
    public fun join<T>(self: &mut Balance<T>, balance: Balance<T>): u64 {
        let Balance { value } = balance;
        let new_value = self.value + value;
        assert!(new_value >= self.value, ERR_OVERFLOW);
        self.value = new_value;
        self.value
    }

    /// Split a `Balance` and take a sub balance from it (Sui API).
    public fun split<T>(self: &mut Balance<T>, value: u64): Balance<T> {
        assert!(self.value >= value, ERR_INSUFFICIENT_BALANCE);
        self.value = self.value - value;
        Balance { value }
    }

    /// Withdraw all balance, leaving zero behind (Sui API).
    public fun withdraw_all<T>(self: &mut Balance<T>): Balance<T> {
        let value = self.value;
        split(self, value)
    }

    /// Destroy a zero `Balance` (Sui API, Sui abort code `ENonZero`).
    public fun destroy_zero<T>(balance: Balance<T>) {
        let Balance { value } = balance;
        assert!(value == 0, ENonZero);
    }

    /// Increase the balance value — package-internal
    public(friend) fun increase<T>(balance: &mut Balance<T>, amount: u64) {
        let new_value = balance.value + amount;
        assert!(new_value >= balance.value, ERR_OVERFLOW);
        balance.value = new_value;
    }

    /// Decrease the balance value — package-internal
    public(friend) fun decrease<T>(balance: &mut Balance<T>, amount: u64) {
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

    /// Create a new supply for type T (Sui API). Consumes a one-time witness
    /// like Sui; the witness is dropped after consumption.
    public fun create_supply<T: drop>(w: T): Supply<T> {
        let _ = w;
        new_supply<T>()
    }

    /// Increase supply: add `amount` to `s` and return a `Balance` for the newly minted amount.
    /// Sui-compatible: zero amounts are allowed (no-op mint).
    public(friend) fun increase_supply<T>(s: &mut Supply<T>, amount: u64): Balance<T> {
        let new_total = s.total + amount;
        assert!(new_total >= s.total, ERR_OVERFLOW);
        s.total = new_total;
        create<T>(amount)
    }

    /// Decrease/destroy a supply handle.
    /// SECURITY: aborts unless `total == 0` — prevents losing track of
    /// outstanding balances by deleting a non-empty supply.
    const ERR_SUPPLY_NOT_EMPTY: u64 = 4;
    public fun destroy_supply<T>(s: Supply<T>) {
        let Supply { total } = s;
        assert!(total == 0, ERR_SUPPLY_NOT_EMPTY);
    }

    /// Decrease supply by `amount`. Useful for burning coins.
    /// Sui-compatible: zero amounts are allowed (no-op burn).
    public(friend) fun decrease_supply<T>(s: &mut Supply<T>, amount: u64) {
        assert!(s.total >= amount, ERR_INSUFFICIENT_BALANCE);
        s.total = s.total - amount;
    }

    /// Read the current total supply value from a supply handle.
    public fun supply_total<T>(s: &Supply<T>): u64 {
        s.total
    }

    /// Sui-compatible alias for `supply_total`.
    public fun supply_value<T>(s: &Supply<T>): u64 {
        s.total
    }

    /// Merge two Balances together — package-internal (uses public `join`).
    public(friend) fun merge<T>(dst: &mut Balance<T>, src: Balance<T>) {
        join(dst, src);
    }

    #[test_only]
    /// Create a `Balance` of any coin for testing purposes (Sui API).
    public fun create_for_testing<T>(value: u64): Balance<T> {
        Balance { value }
    }

    #[test_only]
    /// Destroy a `Balance` of any coin for testing purposes (Sui API).
    public fun destroy_for_testing<T>(self: Balance<T>): u64 {
        let Balance { value } = self;
        value
    }

    #[test_only]
    /// Create a `Supply` of any coin for testing purposes (Sui API).
    public fun create_supply_for_testing<T>(): Supply<T> {
        Supply { total: 0 }
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
        // Burn down remaining supply before destroying the handle.
        decrease_supply<u8>(&mut s, 700);
        destroy_supply(s);
    }

    #[test]
    #[expected_failure(abort_code = ERR_INSUFFICIENT_BALANCE)]
    fun test_decrease_supply_insufficient() {
        let s = new_supply<u8>();
        let b = increase_supply<u8>(&mut s, 100);
        destroy<u8>(b);
        decrease_supply<u8>(&mut s, 200);
        destroy_supply(s);
    }
}