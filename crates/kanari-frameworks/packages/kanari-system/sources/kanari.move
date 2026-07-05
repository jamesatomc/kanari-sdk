// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Coin<KANARI> is the token used to pay for gas in KANARI.
/// It has 9 decimals, and the smallest unit (10^-9) is called "MIST".
module kanari_system::kanari {
    use kanari_system::coin;
    use kanari_system::coin::{Coin, TreasuryCap};
    use kanari_system::tx_context::{Self, TxContext};
    use std::option;
    use kanari_system::transfer;
    use kanari_system::url;

    #[allow(unused_const)]
    /// The amount of Mist per Kanari token based on the fact that mist is
    /// 10^-9 of a Kanari token
    const MIST_PER_KANARI: u64 = 1_000_000_000;

    #[allow(unused_const)]
    /// The total supply of Kanari denominated in whole Kanari tokens (11 Million)
    const TOTAL_SUPPLY_KANARI: u64 = 11_000_000;

    /// The total supply of Kanari denominated in Mist (11 Million * 10^9)
    const TOTAL_SUPPLY_MIST: u64 = 11_000_000_000_000_000;

    // Token distribution percentages (in basis points, 10000 = 100%)
    const DEV_ALLOCATION_BPS: u64 = 1000;      // 10%
    const SUPPORTER_ALLOCATION_BPS: u64 = 2000; // 20%
    const LOCKED_ALLOCATION_BPS: u64 = 4000;    // 40%
    const ECO_ALLOCATION_BPS: u64 = 2000;       // 20%
    const ICO_ALLOCATION_BPS: u64 = 1000;       // 10%

    /// Name of the coin
    struct KANARI has drop {}

    #[allow(unused_function)]
    // Register the `KANARI` Coin to acquire its `Supply`.
    // This should be called only once during genesis creation.
    // Distributes tokens according to allocation plan.
    fun init(witness: KANARI, ctx: &mut TxContext) {
        let (treasury, metadata) = coin::create_currency(
            witness,
            9,
            b"KANARI",
            b"Kanari Network Coin",
            b"",
            option::some(url::new_unsafe_from_bytes(b"https://avatars.githubusercontent.com/u/127471673?s=200&v=4")),
            ctx
        );
        transfer::public_freeze_object(metadata);

        // make a mutable binding for minting (use a different name than the original)
        let treasury_cap = treasury;

        // Calculate allocations in Mist using basis points
        // Divide first to avoid u64 overflow: (supply / 10000) * bps
        let base_unit = TOTAL_SUPPLY_MIST / 10000;
        let dev_amount = base_unit * DEV_ALLOCATION_BPS;
        let supporter_amount = base_unit * SUPPORTER_ALLOCATION_BPS;
        let locked_amount = base_unit * LOCKED_ALLOCATION_BPS;
        let eco_amount = base_unit * ECO_ALLOCATION_BPS;
        let ico_amount = base_unit * ICO_ALLOCATION_BPS;

        // Verify total equals supply (should be exact with these percentages)
        let total_allocated = dev_amount + supporter_amount + locked_amount + eco_amount + ico_amount;
        assert!(total_allocated == TOTAL_SUPPLY_MIST, 0);

        // Dev's wallet: 10% (1,100,000 KANARI)
        let dev_address: address = @0x3ba63b92aac5f2bff87e580e820b61faf1c5fe9ae12f0bc8addd931a340b3146;
        let dev_coin: Coin<KANARI> = coin::mint(&mut treasury_cap, dev_amount, ctx);
        transfer::public_transfer(dev_coin, dev_address);

        // First-generation supporter's wallet: 20% (2,200,000 KANARI)
        let supporter_address: address = @0x2;
        let supporter_coin: Coin<KANARI> = coin::mint(&mut treasury_cap, supporter_amount, ctx);
        transfer::public_transfer(supporter_coin, supporter_address);

        // Locked out: 40% (4,400,000 KANARI)
        let locked_address: address = @0x3;
        let locked_coin: Coin<KANARI> = coin::mint(&mut treasury_cap, locked_amount, ctx);
        transfer::public_transfer(locked_coin, locked_address);

        // For eco-use: 20% (2,200,000 KANARI)
        let eco_address: address = @0x4;
        let eco_coin: Coin<KANARI> = coin::mint(&mut treasury_cap, eco_amount, ctx);
        transfer::public_transfer(eco_coin, eco_address);

        // ICO: 10% (1,100,000 KANARI)
        let ico_address: address = @0x5;
        let ico_coin: Coin<KANARI> = coin::mint(&mut treasury_cap, ico_amount, ctx);
        transfer::public_transfer(ico_coin, ico_address);

        // Transfer the treasury cap to the sender (deployer)
        transfer::public_transfer(treasury_cap, tx_context::sender(ctx));
    }


    /// Transfer a specific amount of KANARI from a mutable coin object.
    public entry fun transfer(
        c: &mut coin::Coin<KANARI>,
        amount: u64,
        recipient: address,
        ctx: &mut TxContext
    ) {
        let split_coin = coin::split(c, amount, ctx);
        transfer::public_transfer(split_coin, recipient);
    }


    /// Burns KANARI tokens, decreasing total supply
    public entry fun burn(treasury_cap: &mut TreasuryCap<KANARI>, coin: Coin<KANARI>) {
        coin::burn(treasury_cap, coin);
    }
}
