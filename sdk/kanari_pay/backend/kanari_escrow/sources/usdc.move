module kanari_escrow::usdc {

    use kanari_system::coin;
    use kanari_system::coin::{Coin, TreasuryCap};
    use kanari_system::tx_context::{TxContext};

    use std::option;
    use kanari_system::transfer;
    use kanari_system::url;

    use std::string;
    use std::ascii;

    /// Only the publishing address may initialize the currency.  Without this
    /// guard, any sender could call `setup` again and obtain another
    /// `TreasuryCap<USDC>`, which would make the asset supply untrustworthy.
    const E_NOT_DEPLOYER: u64 = 1;
    
    /// Name of the coin
    struct USDC has drop {}

    /// Initialize and register the USDC currency.
    /// Returns the `TreasuryCap<USDC>` which can be used to mint tokens.
    /// This should be invoked once (e.g., during genesis or deployment).
    fun init(witness: USDC ,ctx: &mut TxContext): (TreasuryCap<USDC>, coin::CoinMetadata<USDC>) {
        let (treasury, metadata) = coin::create_currency<USDC>(
            witness,
            6,
            b"USDC",
            b"USDC Token",
            b"",
            option::some(url::new_unsafe_from_bytes(b"https://assets.coingecko.com/coins/images/6319/standard/USDC.png?1769615602")),
            ctx,
        );
        // Return both TreasuryCap and Metadata so callers can persist them.
        (treasury, metadata)
    }

    /// Public setup entry that creates the required `USDC` witness,
    /// invokes `init`, and transfers the created objects to the
    /// transaction sender so they are persisted in the caller's account.
    public entry fun setup(ctx: &mut TxContext) {
        assert!(kanari_system::tx_context::sender(ctx) == @kanari_escrow, E_NOT_DEPLOYER);
        let witness = USDC {};
        let (treasury, metadata) = init(witness, ctx);
        let sender = kanari_system::tx_context::sender(ctx);
        transfer::public_transfer(treasury, sender);
        transfer::public_transfer(metadata, sender);
    }


/// Mint new USDC tokens
    /// Only the holder of TreasuryCap can call this
    /// Usage: kanari move call --function mint --args <amount> <recipient>
    /// 
    /// This function mints tokens directly to the recipient's address
    /// The runtime will automatically create or update the recipient's Coin object
    public entry fun mint(
    treasury_cap: &mut TreasuryCap<USDC>,
        amount: u64,
        recipient: address,
        ctx: &mut TxContext
    ) {
        // Mint a new Coin with the specified amount
        let coin = coin::mint<USDC>(treasury_cap, amount, ctx);
        
        // Transfer the Coin to the recipient
        // The runtime will merge Coins of the same type automatically
        transfer::public_transfer(coin, recipient);
    }


    /// Transfer a specific `amount` of USDC from a mutable Coin held by the caller
    /// Usage: provide the caller's coin, the amount to send, and the recipient
    public entry fun transfer_amount(
        c: &mut coin::Coin<USDC>,
        amount: u64,
        recipient: address,
        ctx: &mut TxContext
    ) {
        // 1. Check if sender is the same as recipient, if so, do nothing
        let sender = kanari_system::tx_context::sender(ctx);

       // 2. sender is not the same as recipient, proceed with transfer
        if (sender == recipient) {
            return
        };

        // 3. Split the specified amount from the sender's coin
        let split_coin = coin::split(c, amount, ctx);
        transfer::public_transfer(split_coin, recipient);
    }


    /// Burn a specific `amount` of USDC from a mutable Coin held by the caller
    /// Usage: provide the TreasuryCap, a mutable coin owned by caller, amount to burn, and tx context
    public entry fun burn_amount(
    treasury_cap: &mut TreasuryCap<USDC>,
    c: &mut Coin<USDC>,
        amount: u64,
        ctx: &mut TxContext
    ) {
        let to_burn = coin::split(c, amount, ctx);
        let _burned = coin::burn(treasury_cap, to_burn);
    }

    // ==========================================
    // 🟢 Entry wrappers for CLI calling
    // ==========================================

    /// Usage: kanari move call --function update_icon --args <TreasuryCap_ID> <Metadata_ID> "https://..."
    public entry fun update_icon(
        treasury_cap: &TreasuryCap<USDC>,
        metadata: &mut coin::CoinMetadata<USDC>,
        new_url: vector<u8>,
    ) {
        let new_url_obj = url::new_unsafe_from_bytes(new_url);
        coin::update_icon_url<USDC>(treasury_cap, metadata, option::some(new_url_obj));
    }

    /// Usage: kanari move call --function update_name --args <TreasuryCap_ID> <Metadata_ID> "Thai Baht"
    public entry fun update_name(
        treasury_cap: &TreasuryCap<USDC>,
        metadata: &mut coin::CoinMetadata<USDC>,
        new_name: vector<u8>,
    ) {
        let name_str = string::utf8(new_name);
        coin::update_name<USDC>(treasury_cap, metadata, name_str);
    }

    /// Usage: kanari move call --function update_symbol --args <TreasuryCap_ID> <Metadata_ID> "USDC"
    public entry fun update_symbol(
        treasury_cap: &TreasuryCap<USDC>,
        metadata: &mut coin::CoinMetadata<USDC>,
        new_symbol: vector<u8>,
    ) {
        let symbol_str = ascii::string(new_symbol);
        coin::update_symbol<USDC>(treasury_cap, metadata, symbol_str);
    }

    /// Usage: kanari move call --function update_description --args <TreasuryCap_ID> <Metadata_ID> "My new USDC description"
    public entry fun update_description(
        treasury_cap: &TreasuryCap<USDC>,
        metadata: &mut coin::CoinMetadata<USDC>,
        new_description: vector<u8>,
    ) {
        let desc_str = string::utf8(new_description);
        coin::update_description<USDC>(treasury_cap, metadata, desc_str);
    }
}
