// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::coin {
    use std::option;
    use std::string;
    use std::ascii;
    use std::vector;
    use kanari_system::url;
    use kanari_system::object;
    use kanari_system::balance::Balance;
    use kanari_system::tx_context::TxContext;
    use kanari_system::transfer;
    
    // --- Error Codes ---
    /// Invalid arguments are passed to a function.
    const EInvalidArg: u64 = 1;
    /// Trying to split a coin more times than its balance allows.
    const ENotEnough: u64 = 2;
    /// Amount must be greater than zero.
    const EZERO_AMOUNT: u64 = 3;
    /// Arithmetic overflow.
    const EOVERFLOW: u64 = 4;
    /// Arithmetic underflow.
    const EUNDERFLOW: u64 = 5;
    /// Invalid decimal count.
    const EINVALID_DECIMALS: u64 = 6;

    // --- Data Structures ---

    /// Coin resource wrapper with balance
    struct Coin<phantom T> has key, store, drop {
        id: object::UID,
        balance: Balance<T>,
    }

    /// Capability allowing the bearer to mint and burn coins
    struct TreasuryCap<phantom T> has key, store, drop {
        id: object::UID,
        total_supply: u64, // Tracking total supply directly in the cap
    }

    /// Metadata resource for a currency (stored as an object with UID)
    struct CoinMetadata<phantom T> has key, store, drop {
        id: object::UID,
        decimals: u8,
        name: string::String,
        symbol: ascii::String,
        description: string::String,
        icon_url: option::Option<url::Url>,
    }



    // --- Public Functions ---

    /// Create a new currency with TreasuryCap for minting control and return the
    /// TreasuryCap and the Metadata object. Callers may transfer/freeze the
    /// returned objects as appropriate for their use-case.
    public fun create_currency<T: drop>(
        witness: T,
        decimals: u8,
        symbol_bytes: vector<u8>,
        name_bytes: vector<u8>,
        description_bytes: vector<u8>,
        icon_url: option::Option<url::Url>,
        ctx: &mut TxContext,
    ): (TreasuryCap<T>, CoinMetadata<T>) {
        // 1. Consume the witness type
        let _ = witness;
        
        // Basic safety checks for decimals
        assert!(decimals <= 27, EINVALID_DECIMALS); 

        // Convert byte literals into string types
        let symbol = ascii::string(symbol_bytes);
        let name = string::utf8(name_bytes);
        let description = string::utf8(description_bytes);

        // 2. Create the Capability and Metadata, explicitly specifying the generic type T
        let treasury_cap = TreasuryCap<T> { id: object::new(ctx), total_supply: 0 };
        let metadata = CoinMetadata<T> { 
            id: object::new(ctx), 
            decimals, 
            name, 
            symbol, 
            description, 
            icon_url 
        };

        // Return the newly-created capability and metadata.
        (treasury_cap, metadata)
    }

    /// Create a regulated currency (compatibility with kanari): returns a treasury capability,
    /// a deny-capability for administration of a deny-list, and the metadata object.
    public fun create_regulated_currency<T: drop>(
        witness: T,
        decimals: u8,
        symbol_bytes: vector<u8>,
        name_bytes: vector<u8>,
        description_bytes: vector<u8>,
        icon_url: option::Option<url::Url>,
        ctx: &mut TxContext,
    ): (TreasuryCap<T>, kanari_system::deny_list::DenyCap<T>, CoinMetadata<T>) {
        let _ = witness;
        assert!(decimals <= 27, EINVALID_DECIMALS);

        let symbol = ascii::string(symbol_bytes);
        let name = string::utf8(name_bytes);
        let description = string::utf8(description_bytes);

        let treasury_cap = TreasuryCap<T> { id: object::new(ctx), total_supply: 0 };
        let denycap = kanari_system::deny_list::new_denycap<T>(ctx);
        let metadata = CoinMetadata<T> { 
            id: object::new(ctx), 
            decimals, 
            name, 
            symbol, 
            description, 
            icon_url 
        };
        (treasury_cap, denycap, metadata)
    }

    /// Mint new coins using TreasuryCap
    /// Returns the newly minted Coin<T>.
    public fun mint<T>(
        cap: &mut TreasuryCap<T>,
        amount: u64,
        ctx: &mut TxContext,
    ): Coin<T> {
        assert!(amount > 0, EZERO_AMOUNT);
        let new_total = cap.total_supply + amount;
        assert!(new_total >= cap.total_supply, EOVERFLOW);
        cap.total_supply = new_total;
        object::save_object(cap);
        Coin {
            id: object::new(ctx),
            balance: kanari_system::balance::create<T>(amount),
        }
    }

    /// Mint and transfer to recipient
    public fun mint_and_transfer<T>(
        cap: &mut TreasuryCap<T>,
        amount: u64,
        recipient: address,
        ctx: &mut TxContext,
    ) {
        let coin = mint(cap, amount, ctx);
        transfer::public_transfer(coin, recipient);
    }

    /// Burn coins, decreasing total supply
    public fun burn<T>(cap: &mut TreasuryCap<T>, coin: Coin<T>): u64 {
        let Coin { id: _, balance } = coin;
        let value = kanari_system::balance::destroy<T>(balance);
        assert!(cap.total_supply >= value, EUNDERFLOW);
        cap.total_supply = cap.total_supply - value;
        object::save_object(cap);
        value
    }

    /// Convert a `Coin<T>` into its inner `Balance<T>`.
    public fun into_balance<T>(coin: Coin<T>): Balance<T> {
        let Coin { id: _, balance } = coin;
        balance
    }

    /// Construct a `Coin<T>` from a `Balance<T>`.
    public fun from_balance<T>(balance: Balance<T>, ctx: &mut TxContext): Coin<T> {
        Coin { 
            id: object::new(ctx),
            balance 
        }
    }

    /// Get total supply from TreasuryCap
    public fun total_supply<T>(cap: &TreasuryCap<T>): u64 {
        cap.total_supply
    }

    /// Get coin value
    public fun value<T>(coin: &Coin<T>): u64 {
        kanari_system::balance::value(&coin.balance)
    }

    /// Create a zero-value Coin<T> (useful for testing and as a starting point)
    public fun zero<T>(ctx: &mut TxContext): Coin<T> {
        Coin {
            id: object::new(ctx),
            balance: kanari_system::balance::create<T>(0),
        }
    }

    /// Split a coin into two. Returns the new coin with the specified amount.
    public fun split<T>(coin: &mut Coin<T>, amount: u64, ctx: &mut TxContext): Coin<T> {
        let new_balance = kanari_system::balance::split(&mut coin.balance, amount);
        object::save_object(coin);
        Coin {
            id: object::new(ctx),
            balance: new_balance,
        }
    }

    /// Join two coins together (adds the balance of 'other' into 'coin').
    public fun join<T>(coin: &mut Coin<T>, other: Coin<T>) {
        let Coin { id: _, balance } = other;
        kanari_system::balance::merge(&mut coin.balance, balance);
        object::save_object(coin);
    }

    /// Entry wrapper for joining two coin objects owned by the sender.
    public entry fun join_entry<T>(coin: &mut Coin<T>, other: Coin<T>) {
        join(coin, other);
    }

    /// Destroy a zero-balance coin. This function can only be called on coins with 0 balance.
    /// Useful for cleaning up empty coin objects to save storage.
    public fun destroy_zero<T>(coin: Coin<T>) {
        let Coin { id: _, balance } = coin;
        assert!(kanari_system::balance::value(&balance) == 0, EZERO_AMOUNT);
        kanari_system::balance::destroy<T>(balance);
    }


    /// Split coin `self` into `n - 1` coins with equal balances. The remainder is left in
    /// `self`. Return newly created coins.
    public fun divide_into_n<T>(
        self: &mut Coin<T>, n: u64, ctx: &mut TxContext
    ): vector<Coin<T>> {
        assert!(n > 0, EInvalidArg);
        assert!(n <= value(self), ENotEnough);

        let vec = vector::empty<Coin<T>>();
        let i = 0;
        let split_amount = value(self) / n;
        while (i < n - 1) {
            vector::push_back(&mut vec, split(self, split_amount, ctx));
            i = i + 1;
        };
        vec
    }


    // ==========================================
    // 🟢 Functions to update CoinMetadata
    // ==========================================

    /// Update the icon URL for the given coin type. 
    /// Only the holder of the TreasuryCap can perform this action.
    public fun update_icon_url<T>(
        _treasury: &TreasuryCap<T>,
        metadata: &mut CoinMetadata<T>,
        url: option::Option<url::Url>
    ) {
        metadata.icon_url = url;
    }

    /// Update the name for the given coin type.
    public fun update_name<T>(
        _treasury: &TreasuryCap<T>,
        metadata: &mut CoinMetadata<T>,
        name: string::String
    ) {
        metadata.name = name;
    }

    /// Update the symbol for the given coin type.
    public fun update_symbol<T>(
        _treasury: &TreasuryCap<T>,
        metadata: &mut CoinMetadata<T>,
        symbol: ascii::String
    ) {
        metadata.symbol = symbol;
    }

    /// Update the description for the given coin type.
    public fun update_description<T>(
        _treasury: &TreasuryCap<T>,
        metadata: &mut CoinMetadata<T>,
        description: string::String
    ) {
        metadata.description = description;
    }
    
}
