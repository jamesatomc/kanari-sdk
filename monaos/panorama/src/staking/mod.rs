// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use bincode;
use lazy_static::lazy_static;
use log::{debug, info, warn};
use mona_blockchain::blockchain::{BALANCES, BlockchainError, normalize_address};
use mona_crypto::hash_data_blake3;
use mona_storage::BlockchainStorage;
use mona_types::address::Address;
use mona_types::kari::{
    KA_PER_KARI, NODE_STAKING_MINIMUM_KA, POOL_ADDRESS, POOL_RESERVED_KA,
    STAKING_REWARD_PERCENTAGE, VALIDATOR_STAKING_MINIMUM_KA,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, RwLock, atomic::AtomicU64};
use std::time::{SystemTime, UNIX_EPOCH};

// Constants for staking
pub const STAKING_LOCK_PERIOD_SECONDS: u64 = 86400; // 24 hours lock period
pub const REWARDS_PER_BLOCK: u64 = 100_000; // Base rewards per block in KA

// Staking data structures
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StakedNode {
    pub address: Address,
    pub staked_amount: u64,
    pub is_validator: bool,
    pub staked_at: u64,
    pub unlock_time: u64,
    pub last_reward_time: u64,
    pub accumulated_rewards: u64,
    pub security_hash: String, // Add security hash
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StakingPool {
    pub total_staked: u64,
    pub nodes_count: usize,
    pub validators_count: usize,
    pub total_rewards_distributed: u64, // Track total rewards from pool
}

// Thread-safe staking globals
lazy_static! {
    pub static ref STAKING_NODES: RwLock<HashMap<String, StakedNode>> = RwLock::new(HashMap::new());
    pub static ref NODE_HASH_CACHE: RwLock<HashMap<String, String>> = RwLock::new(HashMap::new());
    pub static ref STAKING_POOL: Mutex<StakingPool> = Mutex::new(StakingPool {
        total_staked: 0,
        nodes_count: 0,
        validators_count: 0,
        total_rewards_distributed: 0,
    });
    pub static ref ACTIVE_VALIDATORS: RwLock<HashSet<String>> = RwLock::new(HashSet::new());
    pub static ref POOL_REMAINING_REWARDS: AtomicU64 = AtomicU64::new(POOL_RESERVED_KA);
}

// Main staking functions
pub fn stake_tokens(
    address: &Address,
    amount: u64,
    wants_to_validate: bool,
) -> Result<StakedNode, BlockchainError> {
    // Validate input parameters
    if amount == 0 {
        return Err(BlockchainError::Transaction(
            "Staking amount cannot be zero".to_string(),
        ));
    }

    if amount < NODE_STAKING_MINIMUM_KA {
        return Err(BlockchainError::Transaction(format!(
            "Staking amount {} is below minimum required ({})",
            amount, NODE_STAKING_MINIMUM_KA
        )));
    }

    let address_str = address.to_hex_literal();

    // Check if already staking
    {
        let staking_nodes = STAKING_NODES.read().unwrap();
        if staking_nodes.contains_key(&address_str) {
            return Err(BlockchainError::Transaction(format!(
                "Address {} is already staking",
                address_str
            )));
        }
    }

    // Verify user has enough balance
    let balance = mona_blockchain::blockchain::get_balance(&address_str)?;
    if balance < amount {
        return Err(BlockchainError::InsufficientFunds(format!(
            "Address {} has insufficient balance for staking",
            address_str
        )));
    }

    // Calculate lock period with overflow protection
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let unlock_time = current_time.saturating_add(STAKING_LOCK_PERIOD_SECONDS);

    // Determine if can be validator
    let is_validator = wants_to_validate && amount >= VALIDATOR_STAKING_MINIMUM_KA;

    // Calculate node identity hash for verification
    let node_identity = format!("{}:{}:{}", address_str, amount, current_time);
    let node_hash = hex::encode(hash_data_blake3(node_identity.as_bytes()));

    // Create staked node with security hash
    let staked_node = StakedNode {
        address: address.clone(),
        staked_amount: amount,
        is_validator,
        staked_at: current_time,
        unlock_time,
        last_reward_time: current_time,
        accumulated_rewards: 0,
        security_hash: node_hash.clone(),
    };

    // Update balances first to ensure atomic operation
    {
        let mut balances = BALANCES.lock().unwrap();
        if let Some(user_balance) = balances.get_mut(&address_str) {
            if *user_balance < amount {
                return Err(BlockchainError::InsufficientFunds(format!(
                    "Insufficient balance during staking lock"
                )));
            }
            *user_balance = user_balance.saturating_sub(amount);
        } else {
            return Err(BlockchainError::InsufficientFunds(format!(
                "User balance not found during staking process"
            )));
        }
    }

    // Update staking data with overflow protection
    {
        let mut staking_nodes = STAKING_NODES.write().unwrap();
        let mut staking_pool = STAKING_POOL.lock().unwrap();
        let mut active_validators = ACTIVE_VALIDATORS.write().unwrap();
        let mut hash_cache = NODE_HASH_CACHE.write().unwrap();

        // Update staking stats with overflow protection
        staking_pool.total_staked = staking_pool.total_staked.saturating_add(amount);
        staking_pool.nodes_count = staking_pool.nodes_count.saturating_add(1);

        if is_validator {
            staking_pool.validators_count = staking_pool.validators_count.saturating_add(1);
            active_validators.insert(address_str.clone());
        }

        // Add the staked node and security hash
        staking_nodes.insert(address_str.clone(), staked_node.clone());
        hash_cache.insert(address_str.clone(), node_hash);
    }

    info!(
        "Address {} staked {} KARI ({} KA). Validator status: {}",
        address_str,
        amount as f64 / KA_PER_KARI as f64,
        amount,
        is_validator
    );

    // Save staking state
    save_staking_state()?;

    Ok(staked_node)
}

// Unstake tokens (with penalty if within lock period)
pub fn unstake_tokens(address: &Address) -> Result<(u64, u64), BlockchainError> {
    let address_str = address.to_hex_literal();

    // Get staked node info
    let (staked_amount, is_validator, unlock_time, accumulated_rewards) = {
        let staking_nodes = STAKING_NODES.read().unwrap();

        match staking_nodes.get(&address_str) {
            Some(node) => (
                node.staked_amount,
                node.is_validator,
                node.unlock_time,
                node.accumulated_rewards,
            ),
            None => {
                return Err(BlockchainError::Transaction(format!(
                    "Address {} is not staking",
                    address_str
                )));
            }
        }
    };

    // Validate staked amount
    if staked_amount == 0 {
        return Err(BlockchainError::Transaction(format!(
            "Invalid staked amount for address {}",
            address_str
        )));
    }

    // Calculate current time and check if within lock period
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Calculate withdrawal amount (with penalty if unlocking early)
    let mut withdrawal_amount = staked_amount;
    let mut early_unlock_penalty = 0;

    if current_time < unlock_time {
        // Calculate penalty (10% of staked amount) with overflow protection
        early_unlock_penalty = staked_amount / 10;
        withdrawal_amount = withdrawal_amount.saturating_sub(early_unlock_penalty);

        warn!(
            "Early unstaking by {}. Penalty: {} KARI ({} KA)",
            address_str,
            early_unlock_penalty as f64 / KA_PER_KARI as f64,
            early_unlock_penalty
        );
    }

    // Update staking data with underflow protection
    {
        let mut staking_nodes = STAKING_NODES.write().unwrap();
        let mut staking_pool = STAKING_POOL.lock().unwrap();
        let mut active_validators = ACTIVE_VALIDATORS.write().unwrap();

        staking_pool.total_staked = staking_pool.total_staked.saturating_sub(staked_amount);
        staking_pool.nodes_count = staking_pool.nodes_count.saturating_sub(1);

        if is_validator {
            staking_pool.validators_count = staking_pool.validators_count.saturating_sub(1);
            active_validators.remove(&address_str);
        }

        // Remove the staked node
        staking_nodes.remove(&address_str);
    }

    // Return tokens to user's balance with overflow protection
    {
        let mut balances = BALANCES.lock().unwrap();
        let user_balance = balances.entry(address_str.clone()).or_insert(0);
        *user_balance = user_balance
            .saturating_add(withdrawal_amount)
            .saturating_add(accumulated_rewards);
    }

    info!(
        "Address {} unstaked {} KARI ({} KA) with {} KARI penalty. Rewards: {} KARI",
        address_str,
        staked_amount as f64 / KA_PER_KARI as f64,
        staked_amount,
        early_unlock_penalty as f64 / KA_PER_KARI as f64,
        accumulated_rewards as f64 / KA_PER_KARI as f64
    );

    // Save staking state
    save_staking_state()?;

    Ok((withdrawal_amount, accumulated_rewards))
}

// Verify a node's integrity using its security hash
pub fn verify_node_integrity(address: &Address) -> bool {
    let address_str = address.to_hex_literal();

    // Get the node and its security hash
    let (node, hash) = {
        let nodes = STAKING_NODES.read().unwrap();
        let hashes = NODE_HASH_CACHE.read().unwrap();

        match (nodes.get(&address_str), hashes.get(&address_str)) {
            (Some(node), Some(hash)) => (node.clone(), hash.clone()),
            _ => return false,
        }
    };

    // Recalculate the security hash
    let node_identity = format!("{}:{}:{}", address_str, node.staked_amount, node.staked_at);
    let calculated_hash = hex::encode(hash_data_blake3(node_identity.as_bytes()));

    // Verify hash matches
    calculated_hash == hash
}

// Calculate and distribute staking rewards
pub fn process_rewards(block_height: u32) -> Result<u64, BlockchainError> {
    // Validate block height
    if block_height == 0 {
        return Ok(0);
    }

    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Process rewards only on every 5th block
    if block_height % 5 != 0 {
        return Ok(0);
    }

    // Get current pool balance - rewards come from pool
    let pool_addr_str = match normalize_address(POOL_ADDRESS) {
        Ok(addr) => addr.to_hex_literal(),
        Err(_) => {
            return Err(BlockchainError::Transaction(
                "Invalid pool address".to_string(),
            ));
        }
    };

    let pool_balance = match mona_blockchain::blockchain::get_balance(&pool_addr_str) {
        Ok(balance) => balance,
        Err(_) => {
            return Err(BlockchainError::Transaction(
                "Failed to get pool balance".to_string(),
            ));
        }
    };

    // If pool has no tokens left, no rewards can be distributed
    if pool_balance == 0 {
        info!("No funds left in reward pool. Staking rewards have been exhausted.");
        return Ok(0);
    }

    // Get list of nodes to process and calculate rewards
    let mut nodes_to_update: Vec<(String, u64)> = Vec::new();
    let mut total_reward_calculation = 0u64;

    {
        let staking_nodes = STAKING_NODES.read().unwrap();

        for (address, node) in staking_nodes.iter() {
            // Skip if not a validator
            if !node.is_validator {
                continue;
            }

            // Validate node data
            if node.staked_amount == 0 || node.last_reward_time > current_time {
                warn!("Invalid node data for validator {}", address);
                continue;
            }

            // Calculate time since last reward with overflow protection
            let time_since_last_reward = current_time.saturating_sub(node.last_reward_time);

            // Skip if less than an hour has passed
            if time_since_last_reward < 3600 {
                continue;
            }

            // Prevent division by zero and validate percentage
            if STAKING_REWARD_PERCENTAGE <= 0.0 {
                warn!(
                    "Invalid staking reward percentage: {}",
                    STAKING_REWARD_PERCENTAGE
                );
                continue;
            }

            // Calculate reward based on staked amount and time passed
            let daily_reward_rate = STAKING_REWARD_PERCENTAGE / 365.0;

            // Convert time to days with bounds checking
            let days_passed = (time_since_last_reward as f64 / 86400.0).min(365.0); // Cap at 1 year

            // Calculate rewards with overflow protection
            let reward_f64 = node.staked_amount as f64 * daily_reward_rate * days_passed;
            let reward = if reward_f64.is_finite() && reward_f64 >= 0.0 {
                reward_f64.round() as u64
            } else {
                0
            };

            // Skip if reward is 0
            if reward == 0 {
                continue;
            }

            // Add to total calculated rewards with overflow protection
            total_reward_calculation = total_reward_calculation.saturating_add(reward);

            // Save address and reward for later processing
            nodes_to_update.push((address.clone(), reward));
        }
    }

    // If no rewards to calculate, return early
    if total_reward_calculation == 0 || nodes_to_update.is_empty() {
        return Ok(0);
    }

    let mut total_rewards = total_reward_calculation;

    // Check if pool has sufficient funds
    if total_reward_calculation > pool_balance {
        // Scale down rewards proportionally
        let scale_factor = pool_balance as f64 / total_reward_calculation as f64;
        let mut scaled_nodes_to_update: Vec<(String, u64)> = Vec::new();
        total_rewards = 0;

        for (address, reward) in nodes_to_update {
            let scaled_reward = (reward as f64 * scale_factor).round() as u64;
            scaled_nodes_to_update.push((address, scaled_reward));
            total_rewards = total_rewards.saturating_add(scaled_reward);
        }

        nodes_to_update = scaled_nodes_to_update;

        warn!(
            "Insufficient funds in reward pool. Rewards scaled down to {}% of calculated value.",
            (scale_factor * 100.0).round()
        );
    }

    // If there are no rewards to distribute, return
    if total_rewards == 0 {
        return Ok(0);
    }

    // Transfer rewards from pool to validators
    {
        let mut balances = BALANCES.lock().unwrap();

        // Decrease pool balance
        if let Some(pool_balance_ref) = balances.get_mut(&pool_addr_str) {
            if *pool_balance_ref < total_rewards {
                warn!(
                    "Pool has insufficient balance for rewards. Expected: {}, Actual: {}",
                    total_rewards, *pool_balance_ref
                );
                return Err(BlockchainError::InsufficientFunds(
                    "Insufficient funds in reward pool".to_string(),
                ));
            }
            *pool_balance_ref = pool_balance_ref.saturating_sub(total_rewards);
        } else {
            warn!("Pool address not found in balances");
            return Err(BlockchainError::Transaction(
                "Pool address not found in balances".to_string(),
            ));
        }

        // Update validator nodes with rewards
        let mut staking_nodes = STAKING_NODES.write().unwrap();
        let mut staking_pool = STAKING_POOL.lock().unwrap();

        // Update total rewards distributed statistic with overflow protection
        staking_pool.total_rewards_distributed = staking_pool
            .total_rewards_distributed
            .saturating_add(total_rewards);

        for (address, reward) in &nodes_to_update {
            // Update node accumulated rewards
            if let Some(node) = staking_nodes.get_mut(address) {
                node.accumulated_rewards = node.accumulated_rewards.saturating_add(*reward);
                node.last_reward_time = current_time;

                debug!(
                    "Validator {} earned {} KARI ({} KA) reward from pool",
                    address,
                    *reward as f64 / KA_PER_KARI as f64,
                    reward
                );
            }
        }
    }

    // Save staking state
    save_staking_state()?;

    Ok(total_rewards)
}

// Get a list of active validators for consensus
pub fn get_active_validators() -> Vec<Address> {
    match (ACTIVE_VALIDATORS.read(), STAKING_NODES.read()) {
        (Ok(validators), Ok(nodes)) => validators
            .iter()
            .filter_map(|addr| nodes.get(addr).map(|node| node.address.clone()))
            .collect(),
        _ => {
            warn!("Failed to acquire locks for getting active validators");
            Vec::new()
        }
    }
}

// Check if an address is a validator
pub fn is_validator(address: &Address) -> bool {
    let address_str = address.to_hex_literal();

    match ACTIVE_VALIDATORS.read() {
        Ok(validators) => validators.contains(&address_str),
        Err(_) => false,
    }
}

// Get staking info for an address
pub fn get_staking_info(address: &Address) -> Option<StakedNode> {
    let address_str = address.to_hex_literal();

    match STAKING_NODES.read() {
        Ok(nodes) => nodes.get(&address_str).cloned(),
        Err(_) => None,
    }
}

// Get current staking statistics
pub fn get_staking_stats() -> StakingPool {
    match STAKING_POOL.lock() {
        Ok(pool) => pool.clone(),
        Err(_) => StakingPool {
            total_staked: 0,
            nodes_count: 0,
            validators_count: 0,
            total_rewards_distributed: 0,
        },
    }
}

// Get remaining pool balance for rewards
pub fn get_pool_remaining_balance() -> Result<u64, BlockchainError> {
    let pool_addr_str = match normalize_address(POOL_ADDRESS) {
        Ok(addr) => addr.to_hex_literal(),
        Err(_) => {
            return Err(BlockchainError::Transaction(
                "Invalid pool address".to_string(),
            ));
        }
    };

    match mona_blockchain::blockchain::get_balance(&pool_addr_str) {
        Ok(balance) => Ok(balance),
        Err(e) => Err(e),
    }
}

// Save staking state to storage
fn save_staking_state() -> Result<(), BlockchainError> {
    // Get data with proper error handling
    let nodes = STAKING_NODES
        .read()
        .map_err(|_| BlockchainError::Storage("Failed to read staking nodes".to_string()))?
        .clone();

    let pool = STAKING_POOL
        .lock()
        .map_err(|_| BlockchainError::Storage("Failed to read staking pool".to_string()))?
        .clone();

    let validators = ACTIVE_VALIDATORS
        .read()
        .map_err(|_| BlockchainError::Storage("Failed to read active validators".to_string()))?
        .clone();

    let hashes = NODE_HASH_CACHE
        .read()
        .map_err(|_| BlockchainError::Storage("Failed to read node hash cache".to_string()))?
        .clone();

    // Serialize data with error handling
    let nodes_data = bincode::serialize(&nodes).map_err(|e| {
        BlockchainError::Storage(format!("Failed to serialize staking nodes: {}", e))
    })?;

    let pool_data = bincode::serialize(&pool).map_err(|e| {
        BlockchainError::Storage(format!("Failed to serialize staking pool: {}", e))
    })?;

    let validators_data = bincode::serialize(&validators).map_err(|e| {
        BlockchainError::Storage(format!("Failed to serialize active validators: {}", e))
    })?;

    let hashes_data = bincode::serialize(&hashes).map_err(|e| {
        BlockchainError::Storage(format!("Failed to serialize node hash cache: {}", e))
    })?;

    // Save data with proper error handling
    let kari_dir = common::get_kari_dir();
    let db_path = kari_dir.join("storage").join("blockchain_db");
    let storage = mona_storage::RocksDBStorage::new(db_path)
        .map_err(|e| BlockchainError::Storage(format!("Failed to open storage: {}", e)))?;

    storage
        .save_data(b"staking_nodes", &nodes_data)
        .map_err(|e| BlockchainError::Storage(format!("Failed to save staking nodes: {}", e)))?;

    storage
        .save_data(b"staking_pool", &pool_data)
        .map_err(|e| BlockchainError::Storage(format!("Failed to save staking pool: {}", e)))?;

    storage
        .save_data(b"active_validators", &validators_data)
        .map_err(|e| {
            BlockchainError::Storage(format!("Failed to save active validators: {}", e))
        })?;

    storage
        .save_data(b"node_hash_cache", &hashes_data)
        .map_err(|e| BlockchainError::Storage(format!("Failed to save node hash cache: {}", e)))?;

    storage
        .flush()
        .map_err(|e| BlockchainError::Storage(format!("Failed to flush storage: {}", e)))?;

    Ok(())
}

pub fn load_staking_state() -> Result<(), BlockchainError> {
    let kari_dir = common::get_kari_dir();
    let db_path = kari_dir.join("storage").join("blockchain_db");
    let storage = mona_storage::RocksDBStorage::new(db_path)
        .map_err(|e| BlockchainError::Storage(format!("Failed to open storage: {}", e)))?;

    // Load nodes with error handling
    if let Ok(Some(nodes_data)) = storage.load_data(b"staking_nodes") {
        if let Ok(nodes) = bincode::deserialize::<HashMap<String, StakedNode>>(&nodes_data) {
            *STAKING_NODES.write().map_err(|_| {
                BlockchainError::Storage("Failed to write staking nodes".to_string())
            })? = nodes;
            debug!(
                "Loaded {} staked nodes",
                STAKING_NODES.read().unwrap().len()
            );
        }
    }

    // Load pool with error handling
    if let Ok(Some(pool_data)) = storage.load_data(b"staking_pool") {
        if let Ok(pool) = bincode::deserialize::<StakingPool>(&pool_data) {
            *STAKING_POOL.lock().map_err(|_| {
                BlockchainError::Storage("Failed to write staking pool".to_string())
            })? = pool.clone();
            debug!(
                "Loaded staking pool: {} total staked, {} rewards distributed",
                pool.total_staked, pool.total_rewards_distributed
            );
        }
    }

    // Load validators with error handling
    if let Ok(Some(validators_data)) = storage.load_data(b"active_validators") {
        if let Ok(validators) = bincode::deserialize::<HashSet<String>>(&validators_data) {
            *ACTIVE_VALIDATORS.write().map_err(|_| {
                BlockchainError::Storage("Failed to write active validators".to_string())
            })? = validators;
            debug!(
                "Loaded {} active validators",
                ACTIVE_VALIDATORS.read().unwrap().len()
            );
        }
    }

    // Load node hash cache with error handling
    if let Ok(Some(hashes_data)) = storage.load_data(b"node_hash_cache") {
        if let Ok(hashes) = bincode::deserialize::<HashMap<String, String>>(&hashes_data) {
            *NODE_HASH_CACHE.write().map_err(|_| {
                BlockchainError::Storage("Failed to write node hash cache".to_string())
            })? = hashes;
            debug!(
                "Loaded {} node hashes",
                NODE_HASH_CACHE.read().unwrap().len()
            );
        }
    }

    Ok(())
}
