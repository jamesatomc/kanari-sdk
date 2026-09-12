// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

module kanari_system::collection {
    use std::string::{String, utf8};
    use std::vector;
    use kanari_system::tx_context::{TxContext};
    use kanari_system::tx_context;
    use kanari_system::event;
    use kanari_system::object;
    use kanari_system::object::UID;
    use kanari_system::transfer;
    use kanari_system::url::Url;

    /// A reusable Collection resource for NFTs and similar objects.
    struct Collection has key, store {
        id: UID,
        name: String,
        description: String,
        banner_url: Url,   
        website_url: Url,   
        creator: address,
        max_supply: u64,
    }
    
    /// A capability resource that governs minting within a Collection.
    struct NftCap has key, store {
        id: UID,
        remaining: u64,
        issued_counter: u64,
        collection_id: address,
    }

    /// Event emitted when a collection is created (for off-chain indexing)
    struct CollectionCreated has copy, drop {
        collection_id: address,
        creator: address,
        max_supply: u64,
    }

    const E_NO_SUPPLY: u64 = 1;
    const E_NOT_COLLECTION_CREATOR: u64 = 2;
    const E_INVALID_SUPPLY: u64 = 3;
    const E_SUPPLY_EXCEEDED: u64 = 4;
    const E_NAME_EMPTY: u64 = 5;
    const MAX_COLLECTION_SUPPLY: u64 = 1000000;

    /// Create a collection and its corresponding `NftCap`.
    /// Returns `(Collection, NftCap)` so callers can persist one or both.
    public fun create_collection(
        ctx: &mut TxContext,
        name: vector<u8>,
        description: vector<u8>,
        banner_url: vector<u8>,
        website_url: vector<u8>,
        max_supply: u64,
    ): (Collection, NftCap) {
        assert!(vector::length(&name) > 0, E_NAME_EMPTY);
        assert!(vector::length(&name) <= 100, E_NAME_EMPTY);
        assert!(vector::length(&description) <= 500, E_NAME_EMPTY);
        assert!(max_supply > 0, E_INVALID_SUPPLY);
        assert!(max_supply <= MAX_COLLECTION_SUPPLY, E_INVALID_SUPPLY);
        let id = object::new(ctx);
        let sender = tx_context::sender(ctx);
        let collection_addr = object::uid_address(&id);
        let coll = Collection {
            id,
            name: utf8(name),
            description: utf8(description),
            banner_url: kanari_system::url::new_from_bytes(banner_url),
            website_url: kanari_system::url::new_from_bytes(website_url),
            creator: sender,
            max_supply,
        };
        let cap = NftCap {
            id: object::new(ctx),
            remaining: max_supply,
            issued_counter: 0,
            collection_id: collection_addr,
        };
        event::emit(CollectionCreated { 
            collection_id: collection_addr, 
            creator: sender, 
            max_supply 
        });
        (coll, cap) 
    }  

    /// Returns the address (UID) of a `Collection`.
    public fun collection_id(_c: &Collection): address {
        // Consumers can call `object::uid_address(&c.id)` directly; keep API minimal.
        object::uid_address(&_c.id)
    }

    /// Returns the collection id stored in an `NftCap`.
    public fun cap_collection_id(cap: &NftCap): address {
        cap.collection_id
    }

    public fun collection_creator(c: &Collection): address {
        c.creator
    }

    public fun max_supply(c: &Collection): u64 {
        c.max_supply
    }

    /// Updates collection display metadata. Only the original collection creator
    /// may perform this mutation; supply and ownership are intentionally unchanged.
    public fun update_metadata(
        c: &mut Collection,
        banner_url: vector<u8>,
        website_url: vector<u8>,
        ctx: &TxContext,
    ) {
        assert!(tx_context::sender(ctx) == c.creator, E_NOT_COLLECTION_CREATOR);
        c.banner_url = kanari_system::url::new_from_bytes(banner_url);
        c.website_url = kanari_system::url::new_from_bytes(website_url);
        object::save_object(c);
    }

    public fun remaining(cap: &NftCap): u64 {
        cap.remaining
    }

    public fun issued(cap: &NftCap): u64 {
        cap.issued_counter
    }

    /// Consume one supply unit from the cap for minting.
    public fun consume_for_mint(cap: &mut NftCap) {
        assert!(cap.remaining > 0, E_NO_SUPPLY);
        cap.issued_counter = cap.issued_counter + 1;
        cap.remaining = cap.remaining - 1;
        object::save_object(cap);
    }

    /// Return one supply unit to cap (used on burn).
    public fun return_from_burn(cap: &mut NftCap) {
        // Prevent exceeding max supply via repeated burn returns (DoS)
        // `remaining` must never exceed `max` implied by issued_counter + remaining
        // We cap at original max via check against overflow: remaining + 1 must not overflow and must be <= MAX_COLLECTION_SUPPLY
        assert!(cap.remaining < MAX_COLLECTION_SUPPLY, E_SUPPLY_EXCEEDED);
        cap.remaining = cap.remaining + 1;
        object::save_object(cap);
    }

    /// Get the creator of a collection.
    /// Transfer helpers using `transfer::public_transfer`.
    public fun transfer_collection(c: Collection, recipient: address, _ctx: &mut TxContext) {
        transfer::public_transfer(c, recipient)
    }

    public fun transfer_cap(cap: NftCap, recipient: address, _ctx: &mut TxContext) {
        transfer::public_transfer(cap, recipient)
    }

    #[test]
    fun test_collection_lifecycle() {
        let ctx = tx_context::new_from_hint(@0xC0FFEE, 1, 0, 0, 0);

        // create collection with small supply
        let (coll, cap) = create_collection(
            &mut ctx, 
            b"Test Name",      // name
            b"Test Desc",      // description
            b"https://banner", // banner_url
            b"https://web",    // website_url
            2                  // max_supply
        );

        // initial checks
        assert!(remaining(&cap) == 2, 0);
        assert!(collection_creator(&coll) == tx_context::sender(&ctx), 1);

        // mint one
        consume_for_mint(&mut cap);
        assert!(remaining(&cap) == 1, 2);
        assert!(issued(&cap) == 1, 3);

        // burn and return supply
        return_from_burn(&mut cap);
        assert!(remaining(&cap) == 2, 4);

        // transfer cap and collection to sender (moves objects)
        transfer_cap(cap, tx_context::sender(&ctx), &mut ctx);
        transfer_collection(coll, tx_context::sender(&ctx), &mut ctx);
    }

    // Test functions that require constructing a `TxContext` are executed
    // in the framework's higher-level test suites. Keep this package focused
    // on the Collection API surface.
}
