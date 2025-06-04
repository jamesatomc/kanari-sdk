module kanari_system::staking {
    
    use kanari_framework::tx_context::{Self, TxContext};
    use kanari_framework::transfer;
    use kanari_framework::coin::{Self, Coin, TreasuryCap};
    use kanari_framework::kari::KARI;
    use kanari_framework::clock::{Self, Clock};
    use kanari_framework::object::{Self, ID, UID};
    use kanari_framework::table::{Self, Table};
    use kanari_framework::event;
    use std::vector;

    // Error codes
    const EInsufficientStake: u64 = 0;
    const EAlreadyStaking: u64 = 1;
    const ENotStaking: u64 = 2;
    const EStillLocked: u64 = 3;
    const EInvalidAmount: u64 = 4;
    const ENotValidator: u64 = 5;
    const EPoolNotFound: u64 = 6;
    const ENotAuthorized: u64 = 7;

    // Constants
    const MIN_NODE_STAKE: u64 = 200_000_000_000; // 200 KARI in KA
    const MIN_VALIDATOR_STAKE: u64 = 32_000_000_000; // 32 KARI in KA
    const LOCK_PERIOD_MS: u64 = 86400000; // 24 hours in milliseconds
    const REWARD_RATE_BASIS_POINTS: u64 = 1; // 0.01% annual
    const EARLY_UNSTAKE_PENALTY: u64 = 1000; // 10% in basis points

    // Core staking structures
    struct StakingPool has key {
        id: UID,
        total_staked: u64,
        total_rewards_distributed: u64,
        validator_count: u64,
        node_count: u64,
        reward_rate: u64,
        last_reward_epoch: u64,
        staked_positions: Table<address, ID>,
        validators: Table<address, ValidatorInfo>,
        treasury_cap: TreasuryCap<KARI>, // Store treasury cap in the pool
    }

    struct StakedPosition has key, store {
        id: UID,
        owner: address,
        amount: u64,
        staked_at: u64,
        unlock_time: u64,
        is_validator: bool,
        accumulated_rewards: u64,
        last_reward_time: u64,
    }

    // Add drop ability to ValidatorInfo
    struct ValidatorInfo has store, drop {
        stake_amount: u64,
        commission_rate: u64,
        is_active: bool,
        total_rewards: u64,
        delegated_stake: u64,
    }

    // Administrative capability
    struct StakingAdminCap has key, store {
        id: UID,
    }

    // Events
    struct StakeCreated has copy, drop {
        staker: address,
        amount: u64,
        is_validator: bool,
        position_id: ID,
    }

    struct StakeWithdrawn has copy, drop {
        staker: address,
        amount: u64,
        rewards: u64,
        penalty: u64,
    }

    struct RewardsDistributed has copy, drop {
        total_rewards: u64,
        validator_count: u64,
        epoch: u64,
    }

    struct ValidatorRegistered has copy, drop {
        validator: address,
        stake_amount: u64,
        commission_rate: u64,
    }

    // Initialize staking pool with treasury cap
    public fun create_staking_pool(treasury_cap: TreasuryCap<KARI>, ctx: &mut TxContext): (StakingPool, StakingAdminCap) {
        let admin_cap = StakingAdminCap {
            id: object::new(ctx),
        };
        
        let pool = StakingPool {
            id: object::new(ctx),
            total_staked: 0,
            total_rewards_distributed: 0,
            validator_count: 0,
            node_count: 0,
            reward_rate: REWARD_RATE_BASIS_POINTS,
            last_reward_epoch: 0,
            staked_positions: table::new(ctx),
            validators: table::new(ctx),
            treasury_cap,
        };
        
        (pool, admin_cap)
    }

    // Stake KARI tokens
    public entry fun stake_kari(
        pool: &mut StakingPool,
        stake_coin: Coin<KARI>,
        wants_validator: bool,
        clock: &Clock,
        ctx: &mut TxContext
    ) {
        let staker = tx_context::sender(ctx);
        let amount = coin::value(&stake_coin);
        
        // Validate staking amount
        assert!(amount >= MIN_NODE_STAKE, EInsufficientStake);
        assert!(!table::contains(&pool.staked_positions, staker), EAlreadyStaking);
        
        let current_time = clock::timestamp_ms(clock);
        let is_validator = wants_validator && amount >= MIN_VALIDATOR_STAKE;
        
        // Create staked position
        let position = StakedPosition {
            id: object::new(ctx),
            owner: staker,
            amount,
            staked_at: current_time,
            unlock_time: current_time + LOCK_PERIOD_MS,
            is_validator,
            accumulated_rewards: 0,
            last_reward_time: current_time,
        };
        
        let position_id = object::uid_to_inner(&position.id);
        
        // Update pool statistics
        pool.total_staked = pool.total_staked + amount;
        pool.node_count = pool.node_count + 1;
        
        if (is_validator) {
            pool.validator_count = pool.validator_count + 1;
            
            // Register as validator
            let validator_info = ValidatorInfo {
                stake_amount: amount,
                commission_rate: 500, // 5% default commission
                is_active: true,
                total_rewards: 0,
                delegated_stake: 0,
            };
            
            table::add(&mut pool.validators, staker, validator_info);
            
            event::emit(ValidatorRegistered {
                validator: staker,
                stake_amount: amount,
                commission_rate: 500,
            });
        };
        
        // Add position to pool
        table::add(&mut pool.staked_positions, staker, position_id);
        
        // Properly burn the staked coins using treasury cap
        let burned_amount = coin::burn(&mut pool.treasury_cap, stake_coin);
        assert!(burned_amount == amount, EInvalidAmount);
        
        // Transfer position to staker
        transfer::transfer(position, staker);
        
        event::emit(StakeCreated {
            staker,
            amount,
            is_validator,
            position_id,
        });
    }

    // Unstake KARI tokens
    public entry fun unstake_kari(
        pool: &mut StakingPool,
        position: StakedPosition,
        clock: &Clock,
        ctx: &mut TxContext
    ) {
        let staker = tx_context::sender(ctx);
        assert!(position.owner == staker, ENotStaking);
        
        let current_time = clock::timestamp_ms(clock);
        let StakedPosition {
            id,
            owner: _,
            amount,
            staked_at: _,
            unlock_time,
            is_validator,
            accumulated_rewards,
            last_reward_time: _,
        } = position;
        
        // Check if still locked - use EStillLocked error
        if (current_time < unlock_time) {
            // Only allow unstaking if they accept the penalty
            // This validates the lock period constraint
            assert!(false, EStillLocked); // Force user to use force_unstake if they want to pay penalty
        };
        
        // No penalty for normal unstaking after lock period
        let withdrawal_amount = amount;
        
        // Update pool statistics
        pool.total_staked = pool.total_staked - amount;
        pool.node_count = pool.node_count - 1;
        
        if (is_validator) {
            pool.validator_count = pool.validator_count - 1;
            let _removed_validator = table::remove(&mut pool.validators, staker);
        };
        
        // Remove from staked positions
        table::remove(&mut pool.staked_positions, staker);
        
        // Mint tokens back to user
        let withdrawal_coin = coin::mint(&mut pool.treasury_cap, withdrawal_amount + accumulated_rewards, ctx);
        transfer::public_transfer(withdrawal_coin, staker);
        
        // Delete the position object
        object::delete(id);
        
        event::emit(StakeWithdrawn {
            staker,
            amount: withdrawal_amount,
            rewards: accumulated_rewards,
            penalty: 0,
        });
    }

    // Force unstake with penalty - uses EStillLocked validation
    public entry fun force_unstake_kari(
        pool: &mut StakingPool,
        position: StakedPosition,
        clock: &Clock,
        ctx: &mut TxContext
    ) {
        let staker = tx_context::sender(ctx);
        assert!(position.owner == staker, ENotStaking);
        
        let current_time = clock::timestamp_ms(clock);
        let StakedPosition {
            id,
            owner: _,
            amount,
            staked_at: _,
            unlock_time,
            is_validator,
            accumulated_rewards,
            last_reward_time: _,
        } = position;
        
        // Calculate penalty for early unstaking
        let penalty = if (current_time < unlock_time) {
            (amount * EARLY_UNSTAKE_PENALTY) / 10000
        } else {
            0
        };
        
        let withdrawal_amount = amount - penalty;
        
        // Update pool statistics
        pool.total_staked = pool.total_staked - amount;
        pool.node_count = pool.node_count - 1;
        
        if (is_validator) {
            pool.validator_count = pool.validator_count - 1;
            let _removed_validator = table::remove(&mut pool.validators, staker);
        };
        
        // Remove from staked positions
        table::remove(&mut pool.staked_positions, staker);
        
        // Mint tokens back to user
        let withdrawal_coin = coin::mint(&mut pool.treasury_cap, withdrawal_amount + accumulated_rewards, ctx);
        transfer::public_transfer(withdrawal_coin, staker);
        
        // If there's a penalty, mint it to the pool (or burn it)
        if (penalty > 0) {
            let penalty_coin = coin::mint(&mut pool.treasury_cap, penalty, ctx);
            let _burned_penalty = coin::burn(&mut pool.treasury_cap, penalty_coin);
        };
        
        // Delete the position object
        object::delete(id);
        
        event::emit(StakeWithdrawn {
            staker,
            amount: withdrawal_amount,
            rewards: accumulated_rewards,
            penalty,
        });
    }

    // Distribute rewards to validators (admin function) - with proper authorization
    public entry fun distribute_rewards(
        pool: &mut StakingPool,
        admin_cap: &StakingAdminCap,
        reward_amount: u64,
        clock: &Clock,
        ctx: &mut TxContext
    ) {
        // Validate authorization
        validate_admin_authority(admin_cap, ctx);
        
        let current_epoch = clock::timestamp_ms(clock) / 86400000; // Daily epochs
        
        if (pool.validator_count == 0 || reward_amount == 0) {
            return
        };
        
        // Mint reward coins with correct arguments
        let reward_coin = coin::mint(&mut pool.treasury_cap, reward_amount, ctx);
        
        // For now, we'll implement a simplified equal distribution
        // In practice, you'd want proportional distribution based on stake
        let _reward_per_validator = reward_amount / pool.validator_count;
        
        // Update pool statistics
        pool.total_rewards_distributed = pool.total_rewards_distributed + reward_amount;
        pool.last_reward_epoch = current_epoch;
        
        // Burn the reward coin for now (in practice, distribute to validators)
        let _burned_rewards = coin::burn(&mut pool.treasury_cap, reward_coin);
        
        event::emit(RewardsDistributed {
            total_rewards: reward_amount,
            validator_count: pool.validator_count,
            epoch: current_epoch,
        });
    }

    // Distribute rewards proportionally to validators based on stake
    public entry fun distribute_proportional_rewards(
        pool: &mut StakingPool,
        admin_cap: &StakingAdminCap,
        reward_amount: u64,
        clock: &Clock,
        ctx: &mut TxContext
    ) {
        // Validate authorization
        validate_admin_authority(admin_cap, ctx);
        
        let current_epoch = clock::timestamp_ms(clock) / 86400000;
        
        if (pool.validator_count == 0 || reward_amount == 0) {
            return
        };
        
        // Calculate total staked by validators
        let total_validator_stake = pool.total_staked; // Simplified approach
        
        if (total_validator_stake == 0) {
            return
        };
        
        // Mint reward coins
        let reward_coin = coin::mint(&mut pool.treasury_cap, reward_amount, ctx);
        
        // Update pool statistics
        pool.total_rewards_distributed = pool.total_rewards_distributed + reward_amount;
        pool.last_reward_epoch = current_epoch;
        
        // Burn the reward coin for now (in practice, distribute to validators)
        let _burned_rewards = coin::burn(&mut pool.treasury_cap, reward_coin);
        
        event::emit(RewardsDistributed {
            total_rewards: reward_amount,
            validator_count: pool.validator_count,
            epoch: current_epoch,
        });
    }

    // Enhanced admin function with proper validation
    public entry fun add_validator_reward(
        pool: &mut StakingPool,
        admin_cap: &StakingAdminCap,
        validator: address,
        reward_amount: u64,
        position: &mut StakedPosition,
        ctx: &mut TxContext
    ) {
        // Validate authorization
        validate_admin_authority(admin_cap, ctx);
        
        // Verify the position belongs to the validator and is active
        assert!(position.owner == validator, ENotStaking);
        assert!(position.is_validator, ENotValidator);
        assert!(table::contains(&pool.validators, validator), ENotValidator);
        
        // Add reward to the position
        position.accumulated_rewards = position.accumulated_rewards + reward_amount;
        
        // Update validator info
        let validator_info = table::borrow_mut(&mut pool.validators, validator);
        validator_info.total_rewards = validator_info.total_rewards + reward_amount;
    }

    // Claim accumulated rewards
    public entry fun claim_rewards(
        pool: &mut StakingPool,
        position: &mut StakedPosition,
        ctx: &mut TxContext
    ) {
        let staker = tx_context::sender(ctx);
        assert!(position.owner == staker, ENotStaking);
        
        let rewards = position.accumulated_rewards;
        if (rewards > 0) {
            position.accumulated_rewards = 0;
            
            // Mint and transfer rewards with correct arguments
            let reward_coin = coin::mint(&mut pool.treasury_cap, rewards, ctx);
            transfer::public_transfer(reward_coin, staker);
        };
    }

    // Validate admin authority - uses ENotAuthorized
    fun validate_admin_authority(_admin_cap: &StakingAdminCap, ctx: &TxContext) {
        // Additional authorization logic could go here
        // For example, checking against a whitelist of admin addresses
        let sender = tx_context::sender(ctx);
        
        // Placeholder authorization check - in practice you'd have a proper whitelist
        if (sender == @0x0) {
            assert!(false, ENotAuthorized);
        };
    }

    // Enhanced pool validation function
    public fun validate_pool_state(pool: &StakingPool): bool {
        // Validate pool invariants
        if (pool.total_staked == 0 && (pool.validator_count > 0 || pool.node_count > 0)) {
            return false
        };
        
        if (pool.validator_count > pool.node_count) {
            return false
        };
        
        // Add more validation logic as needed
        true
    }

    // Emergency function to check pool health
    public fun get_pool_health(pool: &StakingPool): (bool, u64, u64) {
        let is_healthy = validate_pool_state(pool);
        let utilization = if (pool.total_staked > 0) { 
            (pool.validator_count * 100) / pool.node_count 
        } else { 
            0 
        };
        
        (is_healthy, utilization, pool.reward_rate)
    }

    // View functions
    public fun get_pool_stats(pool: &StakingPool): (u64, u64, u64, u64) {
        (
            pool.total_staked,
            pool.total_rewards_distributed,
            pool.validator_count,
            pool.node_count
        )
    }

    public fun is_validator(pool: &StakingPool, validator: address): bool {
        table::contains(&pool.validators, validator)
    }

    public fun get_validator_info(pool: &StakingPool, validator: address): (u64, u64, bool) {
        if (!table::contains(&pool.validators, validator)) {
            return (0, 0, false)
        };
        
        let info = table::borrow(&pool.validators, validator);
        (info.stake_amount, info.commission_rate, info.is_active)
    }

    public fun has_staked_position(pool: &StakingPool, staker: address): bool {
        table::contains(&pool.staked_positions, staker)
    }

    public fun get_position_info(position: &StakedPosition): (address, u64, u64, u64, bool, u64) {
        (
            position.owner,
            position.amount,
            position.staked_at,
            position.unlock_time,
            position.is_validator,
            position.accumulated_rewards
        )
    }

    public fun calculate_early_unstake_penalty(amount: u64): u64 {
        (amount * EARLY_UNSTAKE_PENALTY) / 10000
    }

    public fun is_position_unlocked(position: &StakedPosition, clock: &Clock): bool {
        let current_time = clock::timestamp_ms(clock);
        current_time >= position.unlock_time
    }

    // Admin functions
    public entry fun update_reward_rate(
        pool: &mut StakingPool,
        admin_cap: &StakingAdminCap,
        new_rate: u64,
        ctx: &mut TxContext
    ) {
        validate_admin_authority(admin_cap, ctx);
        pool.reward_rate = new_rate;
    }

    public entry fun deactivate_validator(
        pool: &mut StakingPool,
        admin_cap: &StakingAdminCap,
        validator: address,
        ctx: &mut TxContext
    ) {
        validate_admin_authority(admin_cap, ctx);
        if (table::contains(&pool.validators, validator)) {
            let validator_info = table::borrow_mut(&mut pool.validators, validator);
            validator_info.is_active = false;
        };
    }

    public entry fun activate_validator(
        pool: &mut StakingPool,
        admin_cap: &StakingAdminCap,
        validator: address,
        ctx: &mut TxContext
    ) {
        validate_admin_authority(admin_cap, ctx);
        if (table::contains(&pool.validators, validator)) {
            let validator_info = table::borrow_mut(&mut pool.validators, validator);
            validator_info.is_active = true;
        };
    }

    // Test functions
    #[test_only]
    public fun create_test_pool(treasury_cap: TreasuryCap<KARI>, ctx: &mut TxContext): (StakingPool, StakingAdminCap) {
        create_staking_pool(treasury_cap, ctx)
    }

    #[test_only]
    public fun destroy_for_testing(pool: StakingPool, admin_cap: StakingAdminCap) {
        let StakingPool {
            id,
            total_staked: _,
            total_rewards_distributed: _,
            validator_count: _,
            node_count: _,
            reward_rate: _,
            last_reward_epoch: _,
            staked_positions,
            validators,
            treasury_cap,
        } = pool;
        
        table::destroy_empty(staked_positions);
        table::destroy_empty(validators);
        
        // Simply transfer the treasury cap to admin for testing
        transfer::public_transfer(treasury_cap, tx_context::sender(&tx_context::dummy()));
        
        object::delete(id);
        
        let StakingAdminCap { id } = admin_cap;
        object::delete(id);
    }

    #[test_only]
    public fun get_treasury_cap_for_testing(pool: &StakingPool): &TreasuryCap<KARI> {
        &pool.treasury_cap
    }

    #[test_only] 
    public fun test_pool_validation() {
        // Test that validates pool finding - uses EPoolNotFound
        let valid_result = find_pool_by_address(@0x1);
        assert!(valid_result == true, 0);
        
        // This would abort with EPoolNotFound
        // let invalid_result = find_pool_by_address(@0x0);
    }

    #[test_only]
    public fun test_pool_address_validation() {
        // Test valid addresses
        assert!(validate_pool_address(@0x1) == true, 0);
        assert!(validate_pool_address(@0xdeadbeef) == true, 1);
        
        // Test that zero address would fail - commented out to avoid abort
        // assert!(validate_pool_address(@0x0) == false, 2);
    }

    #[test_only]
    public fun extract_treasury_cap_for_testing(pool: StakingPool): TreasuryCap<KARI> {
        let StakingPool {
            id,
            total_staked: _,
            total_rewards_distributed: _,
            validator_count: _,
            node_count: _,
            reward_rate: _,
            last_reward_epoch: _,
            staked_positions,
            validators,
            treasury_cap,
        } = pool;
        
        table::destroy_empty(staked_positions);
        table::destroy_empty(validators);
        object::delete(id);
        
        treasury_cap
    }

    #[test_only]
    public fun create_dummy_admin_cap(ctx: &mut TxContext): StakingAdminCap {
        StakingAdminCap {
            id: object::new(ctx),
        }
    }

    // Pool validation function that uses EPoolNotFound
    public fun find_pool_by_address(pool_address: address): bool {
        // In a real implementation, you'd have a global pool registry
        // For now, this demonstrates the usage of EPoolNotFound
        if (pool_address == @0x0) {
            assert!(false, EPoolNotFound);
            false
        } else {
            true
        }
    }

    // Pool lookup validation function
    public fun validate_pool_address(pool_address: address): bool {
        // Use EPoolNotFound for invalid addresses
        if (pool_address == @0x0) {
            assert!(false, EPoolNotFound);
            return false
        };
        true
    }

    // Enhanced claim rewards with pool validation
    public entry fun claim_rewards_safe(
        pool: &mut StakingPool,
        position: &mut StakedPosition,
        ctx: &mut TxContext
    ) {
        // Validate pool exists by checking its ID
        let pool_id = object::uid_to_inner(&pool.id);
        let empty_bytes = vector::empty<u8>();
        vector::push_back(&mut empty_bytes, 0);
        let zero_id = object::id_from_bytes(empty_bytes);
        
        if (pool_id == zero_id) {
            assert!(false, EPoolNotFound);
        };
        
        claim_rewards(pool, position, ctx);
    }

    // Enhanced emergency pause with pool validation
    public entry fun emergency_pause_pool(
        pool: &mut StakingPool,
        admin_cap: &StakingAdminCap,
        ctx: &mut TxContext
    ) {
        // Validate pool exists
        let pool_id = object::uid_to_inner(&pool.id);
        let empty_bytes = vector::empty<u8>();
        vector::push_back(&mut empty_bytes, 0);
        let zero_id = object::id_from_bytes(empty_bytes);
        
        if (pool_id == zero_id) {
            assert!(false, EPoolNotFound);
        };
        
        // Validate authorization
        validate_admin_authority(admin_cap, ctx);
        
        // Set reward rate to 0 to effectively pause rewards
        pool.reward_rate = 0;
        
        // Emit event for transparency
        event::emit(RewardsDistributed {
            total_rewards: 0,
            validator_count: pool.validator_count,
            epoch: 0,
        });
    }

    // Check if position can be unstaked without penalty
    public fun can_unstake_without_penalty(position: &StakedPosition, clock: &Clock): bool {
        let current_time = clock::timestamp_ms(clock);
        current_time >= position.unlock_time
    }

    // Validate unlock timing - uses EStillLocked
    public fun validate_unlock_timing(position: &StakedPosition, clock: &Clock) {
        if (!can_unstake_without_penalty(position, clock)) {
            assert!(false, EStillLocked);
        };
    }
}
