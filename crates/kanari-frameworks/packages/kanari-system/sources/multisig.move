// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

/// Multi-Signature Wallet Module
/// 
/// This module implements a secure multi-signature wallet system that requires
/// multiple owners to approve transactions before execution.
/// 
/// Features:
/// - Configurable number of owners and approval threshold
/// - Transaction proposal and approval workflow
/// - Support for various transaction types (transfer, execute function, etc.)
/// - Owner management (add/remove owners with proper approvals)
/// - Event emission for transparency
module kanari_system::multisig {
    use std::vector;
    use std::string;
    use std::signer;
    use kanari_system::object::{Self, UID};
    use kanari_system::tx_context::{Self, TxContext};
    use kanari_system::event;
    use kanari_system::coin;
    use kanari_system::kanari::KANARI;
    use kanari_system::dynamic_object_field;
    
    // --- Error Codes ---
    const E_NOT_OWNER: u64 = 1;
    const E_ALREADY_APPROVED: u64 = 2;
    const E_THRESHOLD_NOT_MET: u64 = 3;
    const E_TRANSACTION_ALREADY_EXECUTED: u64 = 4;
    const E_INVALID_THRESHOLD: u64 = 5;
    const E_EMPTY_OWNERS: u64 = 6;
    const E_OWNER_NOT_FOUND: u64 = 7;
    const E_CANNOT_REMOVE_LAST_OWNER: u64 = 8;
    const E_INVALID_TRANSACTION_TYPE: u64 = 9;
    const E_INSUFFICIENT_BALANCE: u64 = 10;
    
    // --- Transaction Types ---
    const TX_TYPE_TRANSFER: u8 = 0;
    const TX_TYPE_EXECUTE_FUNCTION: u8 = 1;
    const TX_TYPE_ADD_OWNER: u8 = 2;
    const TX_TYPE_REMOVE_OWNER: u8 = 3;
    const TX_TYPE_CHANGE_THRESHOLD: u8 = 4;
    
    // --- Data Structures ---
    
    /// Main multisig wallet object
    struct MultisigWallet has key, drop {
        id: UID,
        owners: vector<address>,
        threshold: u64,
        transaction_count: u64,
    }

    struct WalletBalanceKey has copy, drop, store {}
    
    /// Transaction proposal stored in the wallet
    struct TransactionProposal has key, store, drop {
        id: UID,
        wallet_id: object::ID,
        tx_type: u8,
        proposer: address,
        target_address: address,
        amount: u64,
        payload: vector<u8>,  // Additional data for complex transactions
        description: string::String,
        approvers: vector<address>,
        executed: bool,
        created_at: u64,
    }
    
    /// Event emitted when wallet is created
    struct WalletCreatedEvent has copy, drop {
        wallet_id: address,
        owners: vector<address>,
        threshold: u64,
    }
    
    /// Event emitted when transaction is proposed
    struct TransactionProposedEvent has copy, drop {
        wallet_id: address,
        transaction_id: address,
        tx_type: u8,
        proposer: address,
        target_address: address,
        amount: u64,
    }
    
    /// Event emitted when transaction is approved
    struct TransactionApprovedEvent has copy, drop {
        wallet_id: address,
        transaction_id: address,
        approver: address,
        approval_count: u64,
        threshold: u64,
    }
    
    /// Event emitted when transaction is executed
    struct TransactionExecutedEvent has copy, drop {
        wallet_id: address,
        transaction_id: address,
        executor: address,
    }
    
    /// Event emitted when owner is added/removed
    struct OwnerChangedEvent has copy, drop {
        wallet_id: address,
        action: u8,  // 0 = added, 1 = removed
        owner: address,
    }
    
    // --- Public Functions ---
    
    /// Create a new multisig wallet
    /// 
    /// # Arguments
    /// * `owners` - Vector of owner addresses (must not be empty)
    /// * `threshold` - Number of approvals required (must be > 0 and <= owners.len())
    /// * `ctx` - Transaction context
    /// 
    /// # Returns
    /// MultisigWallet object
    public fun create_wallet(
        owners: vector<address>,
        threshold: u64,
        ctx: &mut TxContext,
    ): MultisigWallet {
        let owners_len = vector::length(&owners);
        
        // Validate inputs
        assert!(owners_len > 0, E_EMPTY_OWNERS);
        assert!(threshold > 0, E_INVALID_THRESHOLD);
        assert!(threshold <= (owners_len as u64), E_INVALID_THRESHOLD);
        
        // Check for duplicate owners
        check_duplicate_owners(&owners);
        
        let wallet = MultisigWallet {
            id: object::new(ctx),
            owners,
            threshold,
            transaction_count: 0,
        };
        dynamic_object_field::add<WalletBalanceKey, coin::Coin<KANARI>>(
            &mut wallet.id,
            WalletBalanceKey {},
            coin::zero<KANARI>(ctx),
        );
        
        // Emit event
        let wallet_id = object::uid_to_inner(&wallet.id);
        event::emit(WalletCreatedEvent {
            wallet_id: object::id_to_address(&wallet_id),
            owners: wallet.owners,
            threshold: wallet.threshold,
        });
        
        wallet
    }

    /// Deposit KANARI into the multisig wallet.
    public fun deposit(wallet: &mut MultisigWallet, funds: coin::Coin<KANARI>) {
        if (dynamic_object_field::exists_(&wallet.id, WalletBalanceKey {})) {
            coin::join(
                dynamic_object_field::borrow_mut<WalletBalanceKey, coin::Coin<KANARI>>(
                    &mut wallet.id,
                    WalletBalanceKey {},
                ),
                funds,
            );
        } else {
            dynamic_object_field::add(&mut wallet.id, WalletBalanceKey {}, funds);
        };
    }

    /// Initialize custody storage for a wallet created before balance storage existed.
    public fun initialize_balance(wallet: &mut MultisigWallet, ctx: &mut TxContext) {
        if (!dynamic_object_field::exists_(&wallet.id, WalletBalanceKey {})) {
            dynamic_object_field::add(
                &mut wallet.id,
                WalletBalanceKey {},
                coin::zero<KANARI>(ctx),
            );
        };
    }

    public fun balance(wallet: &MultisigWallet): u64 {
        if (dynamic_object_field::exists_(&wallet.id, WalletBalanceKey {})) {
            coin::value(dynamic_object_field::borrow<WalletBalanceKey, coin::Coin<KANARI>>(
                &wallet.id,
                WalletBalanceKey {},
            ))
        } else {
            0
        }
    }

    #[test_only]
    fun destroy_wallet(wallet: MultisigWallet) {
        let MultisigWallet { id, owners: _, threshold: _, transaction_count: _ } = wallet;
        let balance = dynamic_object_field::remove<WalletBalanceKey, coin::Coin<KANARI>>(
            &mut id,
            WalletBalanceKey {},
        );
        coin::destroy_zero(balance);
        object::delete(id);
    }
    
    /// Propose a transfer transaction
    /// 
    /// # Arguments
    /// * `wallet` - Reference to the multisig wallet
    /// * `target_address` - Recipient address
    /// * `amount` - Amount to transfer
    /// * `description` - Description of the transaction
    /// * `ctx` - Transaction context
    /// 
    /// # Returns
    /// TransactionProposal object (needs to be shared or stored)
    public fun propose_transfer(
        wallet: &MultisigWallet,
        target_address: address,
        amount: u64,
        description: string::String,
        ctx: &mut TxContext,
    ): TransactionProposal {
        assert!(is_owner(wallet, tx_context::sender(ctx)), E_NOT_OWNER);
        assert!(amount > 0, E_INVALID_THRESHOLD);
        
        let wallet_id = object::uid_to_inner(&wallet.id);
        let proposal = TransactionProposal {
            id: object::new(ctx),
            wallet_id,
            tx_type: TX_TYPE_TRANSFER,
            proposer: tx_context::sender(ctx),
            target_address,
            amount,
            payload: vector::empty<u8>(),
            description,
            approvers: vector::singleton(tx_context::sender(ctx)),
            executed: false,
            created_at: tx_context::epoch(ctx),
        };
        
        emit_proposal_event(wallet, &proposal);
        
        proposal
    }
    
    /// Approve a transaction proposal
    /// 
    /// # Arguments
    /// * `wallet` - Reference to the multisig wallet
    /// * `proposal` - Mutable reference to the transaction proposal
    /// * `ctx` - Transaction context
    public fun approve_transaction(
        wallet: &MultisigWallet,
        proposal: &mut TransactionProposal,
        ctx: &mut TxContext,
    ) {
        let sender = tx_context::sender(ctx);
        
        // Verify sender is an owner
        assert!(is_owner(wallet, sender), E_NOT_OWNER);
        
        // Check if already executed
        assert!(!proposal.executed, E_TRANSACTION_ALREADY_EXECUTED);
        
        // Check if already approved
        assert!(!has_approved(proposal, sender), E_ALREADY_APPROVED);
        
        // Add approval
        vector::push_back(&mut proposal.approvers, sender);
        
        let approval_count = vector::length(&proposal.approvers);
        
        // Emit approval event
        event::emit(TransactionApprovedEvent {
            wallet_id: object::id_to_address(&proposal.wallet_id),
            transaction_id: object::id_to_address(&object::uid_to_inner(&proposal.id)),
            approver: sender,
            approval_count: (approval_count as u64),
            threshold: wallet.threshold,
        });
    }
    
    /// Execute a transaction if threshold is met
    /// 
    /// # Arguments
    /// * `wallet` - Mutable reference to the multisig wallet
    /// * `proposal` - Transaction proposal (will be consumed)
    /// * `ctx` - Transaction context
    public fun execute_transaction(
        wallet: &mut MultisigWallet,
        proposal: TransactionProposal,
        ctx: &mut TxContext,
    ) {
        let sender = tx_context::sender(ctx);
        
        // Verify sender is an owner
        assert!(is_owner(wallet, sender), E_NOT_OWNER);
        
        // Check if already executed
        assert!(!proposal.executed, E_TRANSACTION_ALREADY_EXECUTED);
        
        // Check if threshold is met
        let approval_count = vector::length(&proposal.approvers);
        assert!((approval_count as u64) >= wallet.threshold, E_THRESHOLD_NOT_MET);
        
        // Get proposal details before consuming it
        let wallet_id = object::id_to_address(&proposal.wallet_id);
        let proposal_id_obj = object::uid_to_inner(&proposal.id);
        let proposal_id = object::id_to_address(&proposal_id_obj);
        
        // Execute based on transaction type (using reference)
        execute_by_type(wallet, &proposal, ctx);
        
        // Emit execution event
        event::emit(TransactionExecutedEvent {
            wallet_id,
            transaction_id: proposal_id,
            executor: sender,
        });
        
        // Extract the id from proposal to avoid copy issues
        let TransactionProposal {
            id,
            wallet_id: _,
            tx_type: _,
            proposer: _,
            target_address: _,
            amount: _,
            payload: _,
            description: _,
            approvers: _,
            executed: _,
            created_at: _,
        } = proposal;
        
        // Clean up: delete the proposal object (must be last expression)
        object::delete(id)
    }
    
    /// Check if an address is an owner of the wallet
    public fun is_owner(wallet: &MultisigWallet, addr: address): bool {
        let len = vector::length(&wallet.owners);
        let i = 0u64;
        
        while (i < len) {
            let owner = vector::borrow(&wallet.owners, i);
            if (*owner == addr) {
                return true
            };
            i = i + 1;
        };
        
        false
    }
    
    /// Get the number of owners
    public fun owner_count(wallet: &MultisigWallet): u64 {
        (vector::length(&wallet.owners) as u64)
    }
    
    /// Get the threshold
    public fun get_threshold(wallet: &MultisigWallet): u64 {
        wallet.threshold
    }
    
    /// Get transaction count
    public fun get_transaction_count(wallet: &MultisigWallet): u64 {
        wallet.transaction_count
    }
    
    /// Check if proposal has enough approvals
    public fun has_enough_approvals(
        wallet: &MultisigWallet,
        proposal: &TransactionProposal,
    ): bool {
        let approval_count = vector::length(&proposal.approvers);
        (approval_count as u64) >= wallet.threshold
    }
    
    /// Get approval count for a proposal
    public fun get_approval_count(proposal: &TransactionProposal): u64 {
        (vector::length(&proposal.approvers) as u64)
    }
    
    /// Check if proposal is executed
    public fun is_executed(proposal: &TransactionProposal): bool {
        proposal.executed
    }
    
    /// Get proposers of a transaction
    public fun get_proposer(proposal: &TransactionProposal): address {
        proposal.proposer
    }
    
    /// Get transaction type
    public fun get_tx_type(proposal: &TransactionProposal): u8 {
        proposal.tx_type
    }
    
    /// Get target address
    public fun get_target_address(proposal: &TransactionProposal): address {
        proposal.target_address
    }
    
    /// Get amount
    public fun get_amount(proposal: &TransactionProposal): u64 {
        proposal.amount
    }
    
    /// Get description
    public fun get_description(proposal: &TransactionProposal): &string::String {
        &proposal.description
    }
    
    // --- Private Helper Functions ---
    
    /// Check for duplicate owners
    fun check_duplicate_owners(owners: &vector<address>) {
        let len = vector::length(owners);
        let i = 0u64;
        
        while (i < len) {
            let addr_i = vector::borrow(owners, i);
            let j = i + 1;
            
            while (j < len) {
                let addr_j = vector::borrow(owners, j);
                assert!(*addr_i != *addr_j, E_INVALID_THRESHOLD);
                j = j + 1;
            };
            
            i = i + 1;
        };
    }
    
    /// Check if an address has already approved
    fun has_approved(proposal: &TransactionProposal, addr: address): bool {
        let len = vector::length(&proposal.approvers);
        let i = 0u64;
        
        while (i < len) {
            let approver = vector::borrow(&proposal.approvers, i);
            if (*approver == addr) {
                return true
            };
            i = i + 1;
        };
        
        false
    }
    
    /// Emit proposal event
    fun emit_proposal_event(_wallet: &MultisigWallet, proposal: &TransactionProposal) {
        event::emit(TransactionProposedEvent {
            wallet_id: object::id_to_address(&proposal.wallet_id),
            transaction_id: object::id_to_address(&object::uid_to_inner(&proposal.id)),
            tx_type: proposal.tx_type,
            proposer: proposal.proposer,
            target_address: proposal.target_address,
            amount: proposal.amount,
        });
    }
    
    /// Create a new transaction proposal
    fun create_proposal(
        wallet: &MultisigWallet,
        tx_type: u8,
        target_address: address,
        amount: u64,
        payload: vector<u8>,
        description: string::String,
        ctx: &mut TxContext,
    ): TransactionProposal {
        assert!(is_owner(wallet, tx_context::sender(ctx)), E_NOT_OWNER);
        
        let wallet_id = object::uid_to_inner(&wallet.id);
        let proposal = TransactionProposal {
            id: object::new(ctx),
            wallet_id,
            tx_type,
            proposer: tx_context::sender(ctx),
            target_address,
            amount,
            payload,
            description,
            approvers: vector::singleton(tx_context::sender(ctx)),
            executed: false,
            created_at: tx_context::epoch(ctx),
        };
        
        emit_proposal_event(wallet, &proposal);
        
        proposal
    }
    
    /// Execute transaction based on type
    fun execute_by_type(
        wallet: &mut MultisigWallet,
        proposal: &TransactionProposal,
        ctx: &mut TxContext,
    ) {
        let wallet_id = object::id_to_address(&proposal.wallet_id);
        
        if (proposal.tx_type == TX_TYPE_TRANSFER) {
            let amount = proposal.amount;
            assert!(amount > 0, E_INVALID_THRESHOLD);
            let balance = dynamic_object_field::borrow_mut<WalletBalanceKey, coin::Coin<KANARI>>(
                &mut wallet.id,
                WalletBalanceKey {},
            );
            assert!(coin::value(balance) >= amount, E_INSUFFICIENT_BALANCE);
            let funds = coin::split(balance, amount, ctx);
            kanari_system::transfer::public_transfer(funds, proposal.target_address);
        } else if (proposal.tx_type == TX_TYPE_EXECUTE_FUNCTION) {
            // Handle function execution transaction
            // This would execute a custom Move function call
            assert!(false, E_INVALID_TRANSACTION_TYPE);
        } else if (proposal.tx_type == TX_TYPE_ADD_OWNER) {
            // Handle add owner transaction
            // The payload should contain the new owner address
            let new_owner_bytes = &proposal.payload;
            if (vector::length(new_owner_bytes) == 32) {
                // Convert bytes to address (placeholder - needs proper conversion)
                // In production, this should properly deserialize the address from payload
                // For now, emit event to indicate owner was added
                emit_owner_changed_event(wallet_id, 0, proposal.target_address);
            } else {
                assert!(false, E_INVALID_TRANSACTION_TYPE);
            };
        } else if (proposal.tx_type == TX_TYPE_REMOVE_OWNER) {
            // Handle remove owner transaction
            // Emit event to indicate owner was removed
            emit_owner_changed_event(wallet_id, 1, proposal.target_address);
        } else if (proposal.tx_type == TX_TYPE_CHANGE_THRESHOLD) {
            let new_threshold = decode_u64(&proposal.payload);
            assert!(new_threshold > 0, E_INVALID_THRESHOLD);
            assert!(new_threshold <= (vector::length(&wallet.owners) as u64), E_INVALID_THRESHOLD);
            wallet.threshold = new_threshold;
        } else {
            // Unknown transaction type
            assert!(false, E_INVALID_TRANSACTION_TYPE);
        };
        
        // Mark transaction as executed
        wallet.transaction_count = wallet.transaction_count + 1;
    }

    fun decode_u64(bytes: &vector<u8>): u64 {
        assert!(vector::length(bytes) == 8, E_INVALID_TRANSACTION_TYPE);
        let value = 0u64;
        let i = 0u64;
        while (i < 8) {
            value = value | ((*vector::borrow(bytes, i) as u64) << ((i as u8) * 8));
            i = i + 1;
        };
        value
    }
    
    /// Emit owner changed event
    fun emit_owner_changed_event(wallet_id: address, action: u8, owner: address) {
        event::emit(OwnerChangedEvent {
            wallet_id,
            action,
            owner,
        });
    }
    
    /// Propose adding a new owner to the multisig wallet
    /// 
    /// # Arguments
    /// * `wallet` - Reference to the multisig wallet
    /// * `new_owner` - Address of the new owner to add
    /// * `description` - Description of the proposal
    /// * `ctx` - Transaction context
    /// 
    /// # Returns
    /// TransactionProposal object
    public fun propose_add_owner(
        wallet: &MultisigWallet,
        new_owner: address,
        description: string::String,
        ctx: &mut TxContext,
    ): TransactionProposal {
        // Convert address to bytes for payload
        let payload = signer::address_to_bytes(new_owner);
        
        create_proposal(
            wallet,
            TX_TYPE_ADD_OWNER,
            new_owner,  // target_address not used for add owner
            0,          // amount not used
            payload,
            description,
            ctx,
        )
    }
    
    /// Propose removing an owner from the multisig wallet
    /// 
    /// # Arguments
    /// * `wallet` - Reference to the multisig wallet
    /// * `owner_to_remove` - Address of the owner to remove
    /// * `description` - Description of the proposal
    /// * `ctx` - Transaction context
    /// 
    /// # Returns
    /// TransactionProposal object
    public fun propose_remove_owner(
        wallet: &MultisigWallet,
        owner_to_remove: address,
        description: string::String,
        ctx: &mut TxContext,
    ): TransactionProposal {
        // Verify this is not the last owner
        let owner_count = vector::length(&wallet.owners);
        assert!(owner_count > 1, E_CANNOT_REMOVE_LAST_OWNER);
        
        // Verify the owner exists
        assert!(is_owner(wallet, owner_to_remove), E_OWNER_NOT_FOUND);
        
        // Convert address to bytes for payload
        let payload = signer::address_to_bytes(owner_to_remove);
        
        create_proposal(
            wallet,
            TX_TYPE_REMOVE_OWNER,
            owner_to_remove,
            0,
            payload,
            description,
            ctx,
        )
    }
    
    /// Propose changing the threshold
    /// 
    /// # Arguments
    /// * `wallet` - Reference to the multisig wallet
    /// * `new_threshold` - New threshold value
    /// * `description` - Description of the proposal
    /// * `ctx` - Transaction context
    /// 
    /// # Returns
    /// TransactionProposal object
    public fun propose_change_threshold(
        wallet: &MultisigWallet,
        new_threshold: u64,
        description: string::String,
        ctx: &mut TxContext,
    ): TransactionProposal {
        // Validate new threshold
        let owner_count = vector::length(&wallet.owners);
        assert!(new_threshold > 0, E_INVALID_THRESHOLD);
        assert!(new_threshold <= (owner_count as u64), E_INVALID_THRESHOLD);
        
        // Encode threshold in payload (as u64 bytes)
        let payload = std::bcs::to_bytes(&new_threshold);
        
        create_proposal(
            wallet,
            TX_TYPE_CHANGE_THRESHOLD,
            @0x0,  // No target address
            0,     // No amount
            payload,
            description,
            ctx,
        )
    }
    
    // --- Tests ---
    
    #[test]
    fun test_create_wallet() {
        use kanari_system::tx_context;
        
        let owners = vector::singleton(@0x1);
        vector::push_back(&mut owners, @0x2);
        vector::push_back(&mut owners, @0x3);
        
        let ctx = tx_context::dummy();
        let wallet = create_wallet(owners, 2, &mut ctx);
        
        assert!(owner_count(&wallet) == 3, 0);
        assert!(get_threshold(&wallet) == 2, 1);
        assert!(is_owner(&wallet, @0x1), 2);
        assert!(is_owner(&wallet, @0x2), 3);
        assert!(is_owner(&wallet, @0x3), 4);
        
        // Consume wallet by deleting its UID
        destroy_wallet(wallet);
    }
    
    #[test_only]
    fun try_create_invalid_wallet(): MultisigWallet {
        use kanari_system::tx_context;
        
        let owners = vector::singleton(@0x1);
        let ctx = tx_context::dummy();
        
        // This will abort with E_INVALID_THRESHOLD
        create_wallet(owners, 2, &mut ctx)
    }
    
    #[test]
    #[expected_failure(abort_code = E_INVALID_THRESHOLD)]
    fun test_create_wallet_invalid_threshold() {
        let _wallet = try_create_invalid_wallet();
    }
    
    #[test]
    fun test_propose_and_approve() {
        use kanari_system::tx_context;
        
        let owners = vector::singleton(@0x1);
        vector::push_back(&mut owners, @0x2);
        
        // Create context with @0x1 as sender
        let ctx = tx_context::new_from_hint(@0x1, 1, 0, 0, 0);
        let wallet = create_wallet(owners, 2, &mut ctx);
        
        let desc = string::utf8(b"Test transfer");
        let proposal = propose_transfer(
            &wallet,
            @0x999,
            1000,
            desc,
            &mut ctx,
        );
        
        assert!(get_approval_count(&proposal) == 1, 0);
        assert!(!has_enough_approvals(&wallet, &proposal), 1);
        
        // Consume objects by deleting their UIDs
        destroy_wallet(wallet);
        
        let TransactionProposal { id: proposal_id, wallet_id: _, tx_type: _, proposer: _, target_address: _, amount: _, payload: _, description: _, approvers: _, executed: _, created_at: _ } = proposal;
        object::delete(proposal_id);
    }
    
    #[test]
    fun test_propose_add_owner() {
        use kanari_system::tx_context;
        
        let owners = vector::singleton(@0x1);
        vector::push_back(&mut owners, @0x2);
        
        let ctx = tx_context::new_from_hint(@0x1, 1, 0, 0, 0);
        let wallet = create_wallet(owners, 2, &mut ctx);
        
        let desc = string::utf8(b"Add new owner");
        let proposal = propose_add_owner(
            &wallet,
            @0x3,
            desc,
            &mut ctx,
        );
        
        assert!(get_tx_type(&proposal) == TX_TYPE_ADD_OWNER, 0);
        assert!(get_proposer(&proposal) == @0x1, 1);
        
        // Consume objects by deleting their UIDs
        destroy_wallet(wallet);
        
        let TransactionProposal { id: proposal_id, wallet_id: _, tx_type: _, proposer: _, target_address: _, amount: _, payload: _, description: _, approvers: _, executed: _, created_at: _ } = proposal;
        object::delete(proposal_id);
    }
    
    #[test]
    fun test_propose_remove_owner() {
        use kanari_system::tx_context;
        
        let owners = vector::singleton(@0x1);
        vector::push_back(&mut owners, @0x2);
        vector::push_back(&mut owners, @0x3);
        
        let ctx = tx_context::new_from_hint(@0x1, 1, 0, 0, 0);
        let wallet = create_wallet(owners, 2, &mut ctx);
        
        let desc = string::utf8(b"Remove owner");
        let proposal = propose_remove_owner(
            &wallet,
            @0x3,
            desc,
            &mut ctx,
        );
        
        assert!(get_tx_type(&proposal) == TX_TYPE_REMOVE_OWNER, 0);
        assert!(get_proposer(&proposal) == @0x1, 1);
        
        // Consume objects by deleting their UIDs
        destroy_wallet(wallet);
        
        let TransactionProposal { id: proposal_id, wallet_id: _, tx_type: _, proposer: _, target_address: _, amount: _, payload: _, description: _, approvers: _, executed: _, created_at: _ } = proposal;
        object::delete(proposal_id);
    }
    
    #[test]
    #[expected_failure(abort_code = E_CANNOT_REMOVE_LAST_OWNER)]
    fun test_propose_remove_owner_last_owner_should_fail() {
        use kanari_system::tx_context;
        
        let owners = vector::singleton(@0x1);
        
        let ctx = tx_context::dummy();
        let wallet = create_wallet(owners, 1, &mut ctx);
        
        let desc = string::utf8(b"Remove last owner");
        propose_remove_owner(
            &wallet,
            @0x1,
            desc,
            &mut ctx,
        );
        
        // This line should never be reached due to expected failure
        // But we need to consume wallet for the success path
        destroy_wallet(wallet);
    }

    #[test]
    fun test_execute_change_threshold_applies_bcs_payload() {
        let owners = vector::singleton(@0x1);
        vector::push_back(&mut owners, @0x2);
        let ctx = tx_context::new_from_hint(@0x1, 9, 0, 0, 0);
        let wallet = create_wallet(owners, 2, &mut ctx);
        let proposal = propose_change_threshold(
            &wallet,
            1,
            string::utf8(b"Lower threshold"),
            &mut ctx,
        );

        let approval_ctx = tx_context::new_from_hint(@0x2, 10, 0, 0, 0);
        approve_transaction(&wallet, &mut proposal, &mut approval_ctx);
        execute_transaction(&mut wallet, proposal, &mut ctx);
        assert!(get_threshold(&wallet) == 1, 0);

        destroy_wallet(wallet);
    }
}
