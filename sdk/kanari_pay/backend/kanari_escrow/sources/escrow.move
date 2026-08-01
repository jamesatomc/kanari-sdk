module kanari_escrow::escrow {
    use std::string::{Self, String};
    use std::vector;
    use std::option::{Self, Option};
    use kanari_system::coin::{Self, Coin};
    use kanari_system::event;
    use kanari_system::tx_context::{Self, TxContext};
    use kanari_system::transfer;
    use kanari_system::object;

    // ─── Error codes ────────────────────────────────────────────────
    const E_NOT_BUYER:         u64 = 1;
    const E_NOT_SELLER:        u64 = 2;
    const E_WRONG_STATE:       u64 = 3;
    const E_NOT_AUTHORIZED:    u64 = 6;
    const E_NOT_ENOUGH_BALANCE: u64 = 7;
    // The proof object supplied to an action must belong to that exact deal.
    // Without this check an otherwise authorized party could append an entry to
    // a proof object for a different escrow deal.
    const E_PROOF_DEAL_MISMATCH: u64 = 8;

    // ─── Deal states ─────────────────────────────────────────────────
    const STATE_LOCKED:     u8 = 1; // crypto locked, waiting delivery
    const STATE_DELIVERED:  u8 = 2; // seller confirmed delivery
    const STATE_COMPLETED:  u8 = 3; // buyer confirmed, funds released
    const STATE_DISPUTED:   u8 = 4; // dispute raised

    // ─── Structs ─────────────────────────────────────────────────────

    // Core escrow deal — stored under buyer's address
    struct EscrowDeal<phantom CoinType> has key, store {
        id:             object::UID,
        deal_id:        String,
        buyer:          address,
        seller:         address,
        amount:         u64,
        description:    String,
        state:          u8,
        locked_at:      u64,
        delivered_at:   u64,
        completed_at:   u64,
        funds:          Option<Coin<CoinType>>,
    }

    // On-chain proof record — Kanari metadata layer
    struct EscrowProof has key, store {
        id:         object::UID,
        deal_id:    String,
        entries:    vector<ProofEntry>,
    }

    struct ProofEntry has copy, drop, store {
        state:      u8,
        actor:      address,
        timestamp:  u64,
        note:       String,
    }

    // ─── Events ──────────────────────────────────────────────────────
    struct DealCreated has copy, drop, store {
        deal_id:    String,
        buyer:      address,
        seller:     address,
        amount:     u64,
    }

    struct DealStateChanged has copy, drop, store {
        deal_id:    String,
        old_state:  u8,
        new_state:  u8,
        actor:      address,
        timestamp:  u64,
    }

    // ─── Internal Helper ───────────────────────────────────────────

    /// Internal function to create deal from Coin reference
    /// Used by both entry function and tests
    fun create_deal_internal<CoinType>(
        deal_id:        String,
        seller:         address,
        amount:         u64,
        description:    String,
        buyer_coin:     &mut Coin<CoinType>,
        ctx:            &mut TxContext,
    ) {
        let buyer_addr = tx_context::sender(ctx);
        let now = tx_context::epoch_timestamp_ms(ctx);
        let deal_id_copy = copy deal_id;
        
        // Verify the coin has sufficient balance
        assert!(coin::value(buyer_coin) >= amount, E_NOT_ENOUGH_BALANCE);
        
        // Split the required amount from the buyer's coin
        let funds = coin::split(buyer_coin, amount, ctx);

        // Store deal under a fresh object address (not buyer's address directly)
        let deal = EscrowDeal<CoinType> {
            id:             object::new(ctx),
            deal_id,
            buyer:          buyer_addr,
            seller,
            amount,
            description,
            state:          STATE_LOCKED,
            locked_at:      now,
            delivered_at:   0,
            completed_at:   0,
            funds: option::some(funds),
        };

        // Transfer ownership to buyer
        transfer::public_transfer(deal, buyer_addr);

        // Create proof record
        let entry = ProofEntry {
            state:      STATE_LOCKED,
            actor:      buyer_addr,
            timestamp:  now,
            note:       string::utf8(b"Deal created — funds locked in escrow"),
        };
        let entries = vector::empty<ProofEntry>();
        vector::push_back(&mut entries, entry);

        let proof = EscrowProof {
            id:         object::new(ctx),
            deal_id:    deal_id_copy,
            entries,
        };

        // Transfer proof to buyer
        transfer::public_transfer(proof, buyer_addr);

        // Emit event
        event::emit(DealCreated {
            deal_id: deal_id_copy,
            buyer: buyer_addr,
            seller,
            amount,
        });
    }

    // ─── Entry Functions ────────────────────────────────────────────

    // Step 1: Buyer creates deal and locks crypto
    // For CLI usage: Uses borrow_global_mut to load Coin object from storage by Object ID
    public entry fun create_deal<CoinType>(
        deal_id:        String,
        seller:         address,
        amount:         u64,
        description:    String,
        buyer_coin_id:  address,  // Object ID ของ Coin ที่ต้องการใช้
        ctx:            &mut TxContext,
    ) {
        // Load Coin object from storage using borrow_global_mut
        let buyer_coin: &mut Coin<CoinType> = object::borrow_global_mut<Coin<CoinType>>(buyer_coin_id);
        
        // Delegate to internal function
        create_deal_internal(
            deal_id,
            seller,
            amount,
            description,
            buyer_coin,
            ctx,
        );
    }

    // API-first entry: buyer coin is an authenticated mutable object input.
    public entry fun create_deal_ref<CoinType>(
        deal_id:        String,
        seller:         address,
        amount:         u64,
        description:    String,
        buyer_coin:     &mut Coin<CoinType>,
        ctx:            &mut TxContext,
    ) {
        create_deal_internal(
            deal_id,
            seller,
            amount,
            description,
            buyer_coin,
            ctx,
        );
    }

    // Test-only: Create deal from Coin reference directly
    // For testing purposes where we have Coin object in memory
    #[test_only]
    public fun create_deal_from_coin<CoinType>(
        deal_id:        String,
        seller:         address,
        amount:         u64,
        description:    String,
        buyer_coin:     &mut Coin<CoinType>,
        ctx:            &mut TxContext,
    ) {
        create_deal_internal(
            deal_id,
            seller,
            amount,
            description,
            buyer_coin,
            ctx,
        );
    }

    // Helper function to get deal_id from deal reference
    fun get_deal_id<CoinType>(deal: &EscrowDeal<CoinType>): String {
        let EscrowDeal { 
            id: _, 
            deal_id, 
            buyer: _, 
            seller: _, 
            amount: _, 
            description: _, 
            state: _, 
            locked_at: _, 
            delivered_at: _, 
            completed_at: _, 
            funds: _ 
        } = deal;
        *deal_id
    }

    fun assert_proof_matches_deal<CoinType>(
        deal: &EscrowDeal<CoinType>,
        proof: &EscrowProof,
    ) {
        assert!(deal.deal_id == proof.deal_id, E_PROOF_DEAL_MISMATCH);
    }

    // Step 2: Seller confirms delivery
    // For CLI usage: Uses borrow_global_mut to load objects from storage by Object ID
    public entry fun confirm_delivery<CoinType>(
        deal_id:        address,  // Object ID ของ EscrowDeal
        proof_id:       address,  // Object ID ของ EscrowProof
        ctx:            &mut TxContext,
    ) {
        // Load objects from storage using borrow_global_mut
        let deal: &mut EscrowDeal<CoinType> = object::borrow_global_mut<EscrowDeal<CoinType>>(deal_id);
        let proof: &mut EscrowProof = object::borrow_global_mut<EscrowProof>(proof_id);
        
        let seller_addr = tx_context::sender(ctx);
        
        assert!(deal.seller == seller_addr, E_NOT_SELLER);
        assert!(deal.state == STATE_LOCKED, E_WRONG_STATE);
        assert_proof_matches_deal(deal, proof);

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_DELIVERED;
        deal.delivered_at = now;
        let deal_id_str = get_deal_id(deal);

        // Append proof entry
        let entry = ProofEntry {
            state:      STATE_DELIVERED,
            actor:      seller_addr,
            timestamp:  now,
            note:       string::utf8(b"Seller confirmed delivery"),
        };
        vector::push_back(&mut proof.entries, entry);

        // Save updated objects
        object::save_object(deal);
        object::save_object(proof);

        // Emit event
        event::emit(DealStateChanged {
            deal_id: deal_id_str,
            old_state,
            new_state: STATE_DELIVERED,
            actor: seller_addr,
            timestamp: now,
        });
    }

    // API-first entry: deal and proof are authenticated mutable object inputs.
    public entry fun confirm_delivery_ref<CoinType>(
        deal:           &mut EscrowDeal<CoinType>,
        proof:          &mut EscrowProof,
        ctx:            &mut TxContext,
    ) {
        let seller_addr = tx_context::sender(ctx);

        assert!(deal.seller == seller_addr, E_NOT_SELLER);
        assert!(deal.state == STATE_LOCKED, E_WRONG_STATE);
        assert_proof_matches_deal(deal, proof);

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_DELIVERED;
        deal.delivered_at = now;
        let deal_id_str = get_deal_id(deal);

        let entry = ProofEntry {
            state:      STATE_DELIVERED,
            actor:      seller_addr,
            timestamp:  now,
            note:       string::utf8(b"Seller confirmed delivery"),
        };
        vector::push_back(&mut proof.entries, entry);

        object::save_object(deal);
        object::save_object(proof);

        event::emit(DealStateChanged {
            deal_id: deal_id_str,
            old_state,
            new_state: STATE_DELIVERED,
            actor: seller_addr,
            timestamp: now,
        });
    }

    // API-first entry: deal and proof are authenticated mutable object inputs.
    public entry fun release_funds_ref<CoinType>(
        deal:           &mut EscrowDeal<CoinType>,
        proof:          &mut EscrowProof,
        ctx:            &mut TxContext,
    ) {
        let buyer_addr = tx_context::sender(ctx);

        assert!(deal.buyer == buyer_addr, E_NOT_BUYER);
        assert!(deal.state == STATE_DELIVERED, E_WRONG_STATE);
        assert_proof_matches_deal(deal, proof);

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_COMPLETED;
        deal.completed_at = now;
        let deal_id_str = get_deal_id(deal);

        let funds = option::extract(&mut deal.funds);
        transfer::public_transfer(funds, deal.seller);

        let entry = ProofEntry {
            state:      STATE_COMPLETED,
            actor:      buyer_addr,
            timestamp:  now,
            note:       string::utf8(b"Buyer released funds to seller"),
        };
        vector::push_back(&mut proof.entries, entry);

        object::save_object(deal);
        object::save_object(proof);

        event::emit(DealStateChanged {
            deal_id: deal_id_str,
            old_state,
            new_state: STATE_COMPLETED,
            actor: buyer_addr,
            timestamp: now,
        });
    }

    // API-first entry: deal and proof are authenticated mutable object inputs.
    public entry fun raise_dispute_ref<CoinType>(
        deal:           &mut EscrowDeal<CoinType>,
        proof:          &mut EscrowProof,
        reason:         String,
        ctx:            &mut TxContext,
    ) {
        let caller = tx_context::sender(ctx);

        assert!(caller == deal.buyer || caller == deal.seller, E_NOT_AUTHORIZED);
        assert!(deal.state == STATE_LOCKED || deal.state == STATE_DELIVERED, E_WRONG_STATE);
        assert_proof_matches_deal(deal, proof);

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_DISPUTED;
        let deal_id_str = get_deal_id(deal);

        let funds = option::extract(&mut deal.funds);
        transfer::public_transfer(funds, deal.buyer);

        let entry = ProofEntry {
            state:      STATE_DISPUTED,
            actor:      caller,
            timestamp:  now,
            note:       reason,
        };
        vector::push_back(&mut proof.entries, entry);

        object::save_object(deal);
        object::save_object(proof);

        event::emit(DealStateChanged {
            deal_id: deal_id_str,
            old_state,
            new_state: STATE_DISPUTED,
            actor: caller,
            timestamp: now,
        });
    }

    // Internal helper for testing with object references
    #[test_only]
    public fun confirm_delivery_internal<CoinType>(
        deal:           &mut EscrowDeal<CoinType>,
        proof:          &mut EscrowProof,
        ctx:            &mut TxContext,
    ) {
        let seller_addr = tx_context::sender(ctx);
        
        assert!(deal.seller == seller_addr, E_NOT_SELLER);
        assert!(deal.state == STATE_LOCKED, E_WRONG_STATE);
        assert_proof_matches_deal(deal, proof);

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_DELIVERED;
        deal.delivered_at = now;
        let deal_id = get_deal_id(deal);

        // Append proof entry
        let entry = ProofEntry {
            state:      STATE_DELIVERED,
            actor:      seller_addr,
            timestamp:  now,
            note:       string::utf8(b"Seller confirmed delivery"),
        };
        vector::push_back(&mut proof.entries, entry);

        // Save updated objects
        object::save_object(deal);
        object::save_object(proof);

        // Emit event
        event::emit(DealStateChanged {
            deal_id,
            old_state,
            new_state: STATE_DELIVERED,
            actor: seller_addr,
            timestamp: now,
        });
    }

    // Step 3: Buyer releases funds to seller
    // For CLI usage: Uses borrow_global_mut to load objects from storage by Object ID
    public entry fun release_funds<CoinType>(
        deal_id:        address,  // Object ID ของ EscrowDeal
        proof_id:       address,  // Object ID ของ EscrowProof
        ctx:            &mut TxContext,
    ) {
        // Load objects from storage using borrow_global_mut
        let deal: &mut EscrowDeal<CoinType> = object::borrow_global_mut<EscrowDeal<CoinType>>(deal_id);
        let proof: &mut EscrowProof = object::borrow_global_mut<EscrowProof>(proof_id);
        
        let buyer_addr = tx_context::sender(ctx);
        
        assert!(deal.buyer == buyer_addr, E_NOT_BUYER);
        assert!(deal.state == STATE_DELIVERED, E_WRONG_STATE);
        assert_proof_matches_deal(deal, proof);

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_COMPLETED;
        deal.completed_at = now;
        let deal_id_str = get_deal_id(deal);

        // Transfer funds to seller
        let funds = option::extract(&mut deal.funds);
        transfer::public_transfer(funds, deal.seller);

        // Append proof entry
        let entry = ProofEntry {
            state:      STATE_COMPLETED,
            actor:      buyer_addr,
            timestamp:  now,
            note:       string::utf8(b"Buyer released funds to seller"),
        };
        vector::push_back(&mut proof.entries, entry);

        // Save updated objects
        object::save_object(deal);
        object::save_object(proof);

        // Emit event
        event::emit(DealStateChanged {
            deal_id: deal_id_str,
            old_state,
            new_state: STATE_COMPLETED,
            actor: buyer_addr,
            timestamp: now,
        });
    }

    // Internal helper for testing with object references
    #[test_only]
    public fun release_funds_internal<CoinType>(
        deal:           &mut EscrowDeal<CoinType>,
        proof:          &mut EscrowProof,
        ctx:            &mut TxContext,
    ) {
        let buyer_addr = tx_context::sender(ctx);
        
        assert!(deal.buyer == buyer_addr, E_NOT_BUYER);
        assert!(deal.state == STATE_DELIVERED, E_WRONG_STATE);

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_COMPLETED;
        deal.completed_at = now;
        let deal_id = get_deal_id(deal);

        // Transfer funds to seller
        let funds = option::extract(&mut deal.funds);
        transfer::public_transfer(funds, deal.seller);

        // Append proof entry
        let entry = ProofEntry {
            state:      STATE_COMPLETED,
            actor:      buyer_addr,
            timestamp:  now,
            note:       string::utf8(b"Buyer released funds to seller"),
        };
        vector::push_back(&mut proof.entries, entry);

        // Save updated objects
        object::save_object(deal);
        object::save_object(proof);

        // Emit event
        event::emit(DealStateChanged {
            deal_id,
            old_state,
            new_state: STATE_COMPLETED,
            actor: buyer_addr,
            timestamp: now,
        });
    }

    // Raise dispute - refunds buyer immediately
    // For CLI usage: Uses borrow_global_mut to load objects from storage by Object ID
    public entry fun raise_dispute<CoinType>(
        deal_id:        address,  // Object ID ของ EscrowDeal
        proof_id:       address,  // Object ID ของ EscrowProof
        reason:         String,
        ctx:            &mut TxContext,
    ) {
        // Load objects from storage using borrow_global_mut
        let deal: &mut EscrowDeal<CoinType> = object::borrow_global_mut<EscrowDeal<CoinType>>(deal_id);
        let proof: &mut EscrowProof = object::borrow_global_mut<EscrowProof>(proof_id);
        
        let caller = tx_context::sender(ctx);
        
        assert!(caller == deal.buyer || caller == deal.seller, E_NOT_AUTHORIZED);
        
        // ✅ Only allow dispute in LOCKED (1) or DELIVERED (2) states
        assert!(deal.state == STATE_LOCKED || deal.state == STATE_DELIVERED, E_WRONG_STATE);
        assert_proof_matches_deal(deal, proof);
        
        //  Cannot dispute if deal is already COMPLETED (3) or DISPUTED (4)
        // This prevents refunding after deal completion

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_DISPUTED;
        let deal_id_str = get_deal_id(deal);

        // 🔥 Refund funds to buyer immediately
        let funds = option::extract(&mut deal.funds);
        transfer::public_transfer(funds, deal.buyer);

        // Append proof entry with dispute reason
        let entry = ProofEntry {
            state:      STATE_DISPUTED,
            actor:      caller,
            timestamp:  now,
            note:       reason,
        };
        vector::push_back(&mut proof.entries, entry);

        // Save updated objects
        object::save_object(deal);
        object::save_object(proof);

        // Emit event
        event::emit(DealStateChanged {
            deal_id: deal_id_str,
            old_state,
            new_state: STATE_DISPUTED,
            actor: caller,
            timestamp: now,
        });
    }

    // Internal helper for testing with object references
    #[test_only]
    public fun raise_dispute_internal<CoinType>(
        deal:           &mut EscrowDeal<CoinType>,
        proof:          &mut EscrowProof,
        reason:         String,
        ctx:            &mut TxContext,
    ) {
        let caller = tx_context::sender(ctx);
        
        assert!(caller == deal.buyer || caller == deal.seller, E_NOT_AUTHORIZED);
        
        // ✅ Only allow dispute in LOCKED (1) or DELIVERED (2) states
        assert!(deal.state == STATE_LOCKED || deal.state == STATE_DELIVERED, E_WRONG_STATE);
        
        // ❌ Cannot dispute if deal is already COMPLETED (3) or DISPUTED (4)
        // This prevents refunding after deal completion

        let now = tx_context::epoch_timestamp_ms(ctx);
        let old_state = deal.state;
        deal.state = STATE_DISPUTED;
        let deal_id = get_deal_id(deal);

        // 🔥 Refund funds to buyer immediately
        let funds = option::extract(&mut deal.funds);
        transfer::public_transfer(funds, deal.buyer);

        // Append proof entry with dispute reason
        let entry = ProofEntry {
            state:      STATE_DISPUTED,
            actor:      caller,
            timestamp:  now,
            note:       reason,
        };
        vector::push_back(&mut proof.entries, entry);

        // Save updated objects
        object::save_object(deal);
        object::save_object(proof);

        // Emit event
        event::emit(DealStateChanged {
            deal_id,
            old_state,
            new_state: STATE_DISPUTED,
            actor: caller,
            timestamp: now,
        });
    }

    // ─── View Functions (for RPC/Dart queries) ──────────────────────

    /// Get current deal state by Object ID
    /// Returns the state of the deal (1=LOCKED, 2=DELIVERED, 3=COMPLETED, 4=DISPUTED)
    public fun get_state<CoinType>(
        deal_id: address,
    ): u8 {
        let deal: &EscrowDeal<CoinType> = object::borrow_global<EscrowDeal<CoinType>>(deal_id);
        deal.state
    }

    /// Get current deal state from an authenticated object input.
    public fun get_state_ref<CoinType>(
        deal: &EscrowDeal<CoinType>,
    ): u8 {
        deal.state
    }

    /// Get proof entry count by Object ID  
    /// Returns the number of proof entries in the EscrowProof
    public fun get_proof_count(
        proof_id: address,
    ): u64 {
        let proof: &EscrowProof = object::borrow_global<EscrowProof>(proof_id);
        (vector::length(&proof.entries) as u64)
    }

    /// Get full deal details by Object ID
    /// อ่านข้อมูลทั้งหมดของ deal โดยไม่แก้ไข
    public fun get_deal_details<CoinType>(
        deal_id: address,
    ): (address, address, address, u64) {
        let deal: &EscrowDeal<CoinType> = object::borrow_global<EscrowDeal<CoinType>>(deal_id);
        
        (
            deal_id,  // ใช้ deal_id ที่รับเข้ามาโดยตรง (เป็น address อยู่แล้ว)
            deal.buyer,
            deal.seller,
            deal.amount,
        )
    }

    /// Get full deal details from an authenticated object input.
    public fun get_deal_details_ref<CoinType>(
        deal: &EscrowDeal<CoinType>,
    ): (address, address, address, u64) {
        (
            object::uid_address(&deal.id),
            deal.buyer,
            deal.seller,
            deal.amount,
        )
    }

    /// Check if deal is in specific state
    /// ตรวจสอบว่า deal อยู่ในสถานะที่กำหนดหรือไม่
    public fun check_deal_state<CoinType>(
        deal_id: address,
        expected_state: u8,
    ): bool {
        let deal: &EscrowDeal<CoinType> = object::borrow_global<EscrowDeal<CoinType>>(deal_id);
        deal.state == expected_state
    }
}
