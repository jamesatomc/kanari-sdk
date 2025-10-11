
module kanari_library::kanari {
    use std::option;
    use std::string;
    use moveos_std::signer;
    use moveos_std::object::{Self, Object};
    use rooch_framework::coin;
    use rooch_framework::account_coin_store;
    use moveos_std::event;

    const ADMIN_ADDRESS: address = @kanari_library;

    // Error codes
    const ERROR_NOT_ADMIN: u64 = 1;
    const ERROR_ZERO_AMOUNT: u64 = 2;

    // Token details
    const DECIMALS: u8 = 8u8;
    const INITIAL_SUPPLY: u256 = 10_000_000_000_000_000u256; // 100 million tokens
    const KARI_ICON_URL: vector<u8> = b"";

    // Define the KARI token
    struct KARI has key, store {}

    // Define the token admin
    struct TokenAdmin has key, store {
        coin_info: Object<coin::CoinInfo<KARI>>,
    }

    // Initialize the token
    fun init() {
        let coin_info_obj = coin::register_extend<KARI>(
            string::utf8(b"KARI Token"),
            string::utf8(b"KARI"),
            option::some(string::utf8(KARI_ICON_URL)), // Add icon URL
            DECIMALS,
        );

        // Mint initial supply
        let initial_coins = coin::mint_extend<KARI>(&mut coin_info_obj, INITIAL_SUPPLY);

        let token_admin = TokenAdmin {
            coin_info: coin_info_obj,
        };

        let admin_obj = object::new_named_object(token_admin);
        object::transfer(admin_obj, ADMIN_ADDRESS);

        // Transfer initial supply to admin
        account_coin_store::deposit_extend(ADMIN_ADDRESS, initial_coins);
    }

    // Mint new tokens
    public entry fun mint(admin: &signer, to: address, amount: u256) {
        assert!(signer::address_of(admin) == ADMIN_ADDRESS, ERROR_NOT_ADMIN);
        assert!(amount > 0, ERROR_ZERO_AMOUNT);

        let admin_obj_id = object::named_object_id<TokenAdmin>();
        let admin_obj = object::borrow_mut_object<TokenAdmin>(admin, admin_obj_id);
        let token_admin = object::borrow_mut(admin_obj);
        let coin = coin::mint_extend<KARI>(&mut token_admin.coin_info, amount);
        account_coin_store::deposit_extend(to, coin);
    }

    // Burn tokens
    public entry fun burn(account: &signer, amount: u256) {
        assert!(amount > 0, ERROR_ZERO_AMOUNT);

        let admin_obj_id = object::named_object_id<TokenAdmin>();
        let admin_obj = object::borrow_mut_object_extend<TokenAdmin>(admin_obj_id);
        let token_admin = object::borrow_mut(admin_obj);
        let coin = account_coin_store::withdraw_extend<KARI>(signer::address_of(account), amount);
        coin::burn_extend<KARI>(&mut token_admin.coin_info, coin);
    }

    // Transfer tokens
    public entry fun transfer(from: &signer, to: address, amount: u256) {
        assert!(amount > 0, ERROR_ZERO_AMOUNT);
        account_coin_store::transfer_extend<KARI>(signer::address_of(from), to, amount);
    }

}
