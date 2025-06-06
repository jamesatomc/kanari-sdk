use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use lazy_static::lazy_static;

use mona_crypto::hash_data_blake3;
use mona_storage::BlockchainStorage;
use mona_types::address::Address;
use mona_types::kari::{
    KA_PER_KARI,
    POOL_ADDRESS
};
use mona_types::move_bridge::{StakingRequest, StakedPosition, StakingPoolState, ValidatorInfo, ObjectID};
use log::{debug, info, warn};
use serde::{Serialize, Deserialize};
use bincode;
use std::time::{SystemTime, UNIX_EPOCH};
use  mona_types::storage::{BlockchainError, BALANCES, normalize_address};


// Enhanced constants aligned with Move staking module
pub const MIN_NODE_STAKE: u64 = 200_000_000_000; // 200 KARI in KA (matches Move)
pub const MIN_VALIDATOR_STAKE: u64 = 32_000_000_000; // 32 KARI in KA (matches Move)
pub const STAKING_LOCK_PERIOD_SECONDS: u64 = 86400; // 24 hours (matches Move LOCK_PERIOD_MS / 1000)
pub const EARLY_UNSTAKE_PENALTY_BASIS_POINTS: u64 = 1000; // 10% (matches Move)
pub const VALIDATOR_COMMISSION_BASIS_POINTS: u64 = 500; // 5% (matches Move)
pub const REWARD_RATE_BASIS_POINTS: u64 = 1; // 0.01% annual (matches Move)
pub const REWARDS_DISTRIBUTION_INTERVAL: u64 = 86400; // Daily distribution (matches Move epochs)
pub const MAX_VALIDATORS: usize = 100;

// Enhanced staking data structures with Move integration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnhancedStakedNode {
    pub address: Address,
    pub staked_amount: u64,
    pub is_validator: bool,
    pub staked_at: u64,
    pub unlock_time: u64,
    pub last_reward_time: u64,
    pub accumulated_rewards: u64,
    pub security_hash: String,
    pub move_object_id: Option<ObjectID>, // Link to Move object
    pub commission_rate: u64, // For validators
    pub delegated_stake: u64, // Amount delegated to this validator
    pub is_active: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StakingPoolManager {
    pub pool_state: StakingPoolState,
    pub pending_rewards: u64,
    pub last_distribution: u64,
    pub total_delegated: u64,
    pub move_pool_id: Option<ObjectID>,
}

// Thread-safe globals with enhanced structure
lazy_static! {
    pub static ref ENHANCED_STAKING_NODES: RwLock<HashMap<String, EnhancedStakedNode>> = RwLock::new(HashMap::new());
    pub static ref VALIDATOR_REGISTRY: RwLock<HashMap<String, ValidatorInfo>> = RwLock::new(HashMap::new());
    pub static ref STAKING_POOL_MANAGER: Mutex<StakingPoolManager> = Mutex::new(StakingPoolManager::default());
    pub static ref DELEGATION_MAP: RwLock<HashMap<String, Vec<String>>> = RwLock::new(HashMap::new()); // validator -> delegators
    pub static ref MOVE_INTEGRATION: RwLock<bool> = RwLock::new(false);
}

impl Default for StakingPoolManager {
    fn default() -> Self {
        Self {
            pool_state: StakingPoolState {
                pool_id: ObjectID::new(Address::ZERO),
                total_staked: 0,
                validator_count: 0,
                node_count: 0,
                reward_rate: REWARD_RATE_BASIS_POINTS, // Use Move-aligned reward rate
                last_reward_epoch: 0,
            },
            pending_rewards: 0,
            last_distribution: 0,
            total_delegated: 0,
            move_pool_id: None,
        }
    }
}

// Enhanced staking functions with Move-aligned validation
pub fn stake_tokens_enhanced(
    address: &Address,
    amount: u64,
    wants_to_validate: bool,
    use_move_integration: bool,
) -> Result<EnhancedStakedNode, BlockchainError> {
    // Validate staking amount using Move constants
    if amount < MIN_NODE_STAKE {
        return Err(BlockchainError::Transaction(
            format!("Insufficient stake amount: {} < {} (MIN_NODE_STAKE)", amount, MIN_NODE_STAKE)
        ));
    }
    
    // Create staking request
    let staking_request = StakingRequest::new(address.clone(), amount, wants_to_validate);
    
    // Validate with KARI config
    let kari_config = mona_types::kari::KARI::default();
    staking_request.validate(&kari_config)
        .map_err(|e| BlockchainError::Transaction(e))?;
    
    let address_str = address.to_hex_literal();
    
    // Check if already staking
    {
        let staking_nodes = ENHANCED_STAKING_NODES.read().unwrap();
        if staking_nodes.contains_key(&address_str) {
            return Err(BlockchainError::Transaction(
                format!("Address {} is already staking", address_str)
            ));
        }
    }
    
    // Verify balance
    let balance = mona_types::storage::get_balance(&address_str)?;
    if balance < amount {
        return Err(BlockchainError::InsufficientFunds(
            format!("Insufficient balance for staking: {} < {}", balance, amount)
        ));
    }
    
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    // Use Move-aligned validator validation
    let is_validator = wants_to_validate && amount >= MIN_VALIDATOR_STAKE;
    
    // Generate security hash
    let node_identity = format!("{}:{}:{}", address_str, amount, current_time);
    let security_hash = hex::encode(hash_data_blake3(node_identity.as_bytes()));
    
    // Create Move object ID if using Move integration
    let move_object_id = if use_move_integration {
        Some(ObjectID::new(address.clone()))
    } else {
        None
    };
    
    // Create enhanced staked node with Move-aligned unlock time
    let staked_node = EnhancedStakedNode {
        address: address.clone(),
        staked_amount: amount,
        is_validator,
        staked_at: current_time,
        unlock_time: current_time + STAKING_LOCK_PERIOD_SECONDS, // Matches Move lock period
        last_reward_time: current_time,
        accumulated_rewards: 0,
        security_hash,
        move_object_id,
        commission_rate: if is_validator { VALIDATOR_COMMISSION_BASIS_POINTS } else { 0 },
        delegated_stake: 0,
        is_active: true,
    };
    
    // Update balances atomically
    {
        let mut balances = BALANCES.lock().unwrap();
        if let Some(user_balance) = balances.get_mut(&address_str) {
            if *user_balance < amount {
                return Err(BlockchainError::InsufficientFunds(
                    "Insufficient balance during staking lock".to_string()
                ));
            }
            *user_balance = user_balance.saturating_sub(amount);
        } else {
            return Err(BlockchainError::InsufficientFunds(
                "User balance not found".to_string()
            ));
        }
    }
    
    // Update enhanced staking data
    {
        let mut nodes = ENHANCED_STAKING_NODES.write().unwrap();
        let mut pool_manager = STAKING_POOL_MANAGER.lock().unwrap();
        let mut validators = VALIDATOR_REGISTRY.write().unwrap();
        
        // Update pool state
        pool_manager.pool_state.total_staked = pool_manager.pool_state.total_staked.saturating_add(amount);
        pool_manager.pool_state.node_count += 1;
        
        if is_validator {
            // Check validator limit
            if validators.len() >= MAX_VALIDATORS {
                return Err(BlockchainError::Transaction(
                    format!("Maximum validator limit reached: {}", MAX_VALIDATORS)
                ));
            }
            
            pool_manager.pool_state.validator_count += 1;
            
            // Register validator
            let validator_info = ValidatorInfo {
                address: address.clone(),
                stake_amount: amount,
                commission_rate: VALIDATOR_COMMISSION_BASIS_POINTS,
                is_active: true,
                last_epoch_reward: 0,
            };
            
            validators.insert(address_str.clone(), validator_info);
        }
        
        nodes.insert(address_str.clone(), staked_node.clone());
    }
    
    info!(
        "Enhanced staking: {} staked {} KARI. Validator: {}, Move integration: {}",
        address_str,
        amount as f64 / KA_PER_KARI as f64,
        is_validator,
        use_move_integration
    );
    
    // Save state
    save_enhanced_staking_state()?;
    
    Ok(staked_node)
}

// Enhanced unstaking with penalty calculation
pub fn unstake_tokens_enhanced(
    address: &Address,
    force_unstake: bool,
) -> Result<(u64, u64, u64), BlockchainError> {
    let address_str = address.to_hex_literal();
    
    // Get node info
    let node_info = {
        let nodes = ENHANCED_STAKING_NODES.read().unwrap();
        match nodes.get(&address_str) {
            Some(node) => node.clone(),
            None => return Err(BlockchainError::Transaction(
                format!("Address {} is not staking", address_str)
            )),
        }
    };
    
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    // Use Move-aligned penalty calculation
    let mut penalty = 0u64;
    let mut withdrawal_amount = node_info.staked_amount;
    
    if current_time < node_info.unlock_time && !force_unstake {
        return Err(BlockchainError::Transaction(
            format!("Stake is still locked. Unlock time: {}", node_info.unlock_time)
        ));
    }
    
    if current_time < node_info.unlock_time {
        penalty = calculate_early_unstake_penalty(node_info.staked_amount);
        withdrawal_amount = withdrawal_amount.saturating_sub(penalty);
        
        warn!(
            "Early unstaking penalty applied: {} KARI for {}",
            penalty as f64 / KA_PER_KARI as f64,
            address_str
        );
    }
    
    // Update staking data
    {
        let mut nodes = ENHANCED_STAKING_NODES.write().unwrap();
        let mut pool_manager = STAKING_POOL_MANAGER.lock().unwrap();
        let mut validators = VALIDATOR_REGISTRY.write().unwrap();
        
        // Update pool state
        pool_manager.pool_state.total_staked = pool_manager.pool_state.total_staked.saturating_sub(node_info.staked_amount);
        pool_manager.pool_state.node_count = pool_manager.pool_state.node_count.saturating_sub(1);
        
        if node_info.is_validator {
            pool_manager.pool_state.validator_count = pool_manager.pool_state.validator_count.saturating_sub(1);
            validators.remove(&address_str);
        }
        
        nodes.remove(&address_str);
    }
    
    // Return funds to user
    {
        let mut balances = BALANCES.lock().unwrap();
        let user_balance = balances.entry(address_str.clone()).or_insert(0);
        *user_balance = user_balance
            .saturating_add(withdrawal_amount)
            .saturating_add(node_info.accumulated_rewards);
    }
    
    info!(
        "Enhanced unstaking: {} withdrew {} KARI, penalty: {} KARI, rewards: {} KARI",
        address_str,
        withdrawal_amount as f64 / KA_PER_KARI as f64,
        penalty as f64 / KA_PER_KARI as f64,
        node_info.accumulated_rewards as f64 / KA_PER_KARI as f64
    );
    
    save_enhanced_staking_state()?;
    
    Ok((withdrawal_amount, node_info.accumulated_rewards, penalty))
}

// Enhanced reward distribution with Move-aligned calculation
pub fn distribute_rewards_enhanced(block_height: u32) -> Result<u64, BlockchainError> {
    if block_height == 0 {
        return Ok(0);
    }
    
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    // Check if it's time to distribute rewards (daily like Move epochs)
    let should_distribute = {
        let pool_manager = STAKING_POOL_MANAGER.lock().unwrap();
        current_time >= pool_manager.last_distribution + REWARDS_DISTRIBUTION_INTERVAL
    };
    
    if !should_distribute {
        return Ok(0);
    }
    
    // Get pool balance for rewards
    let pool_addr_str = match normalize_address(POOL_ADDRESS) {
        Ok(addr) => addr.to_hex_literal(),
        Err(_) => return Err(BlockchainError::Transaction("Invalid pool address".to_string())),
    };
    
    let pool_balance = mona_types::storage::get_balance(&pool_addr_str)?;
    if pool_balance == 0 {
        info!("No funds in reward pool");
        return Ok(0);
    }
    
    // Calculate total rewards to distribute
    let validators = VALIDATOR_REGISTRY.read().unwrap();
    if validators.is_empty() {
        return Ok(0);
    }
    
    let total_validator_stake: u64 = validators.values().map(|v| v.stake_amount).sum();
    if total_validator_stake == 0 {
        return Ok(0);
    }
    
    // Calculate rewards per validator based on Move reward rate
    let reward_rate = (REWARD_RATE_BASIS_POINTS as f64) / 10000.0 / 365.0; // Daily rate from annual basis points
    let total_rewards = (total_validator_stake as f64 * reward_rate) as u64;
    let actual_rewards = total_rewards.min(pool_balance);
    
    if actual_rewards == 0 {
        return Ok(0);
    }
    
    // Distribute rewards proportionally
    let mut total_distributed = 0u64;
    {
        let mut nodes = ENHANCED_STAKING_NODES.write().unwrap();
        let mut balances = BALANCES.lock().unwrap();
        let mut pool_manager = STAKING_POOL_MANAGER.lock().unwrap();
        
        for (address_str, validator_info) in validators.iter() {
            if let Some(node) = nodes.get_mut(address_str) {
                if !node.is_validator || !node.is_active {
                    continue;
                }
                
                // Calculate proportional reward
                let validator_reward = (actual_rewards * validator_info.stake_amount) / total_validator_stake;
                
                // Apply commission (validator keeps commission, rest goes to delegators)
                let commission = (validator_reward * validator_info.commission_rate) / 10000;
                let _delegator_reward = validator_reward - commission;
                
                // Add to validator's accumulated rewards
                node.accumulated_rewards = node.accumulated_rewards.saturating_add(commission);
                node.last_reward_time = current_time;
                
                total_distributed = total_distributed.saturating_add(validator_reward);
                
                debug!(
                    "Validator {} earned {} KARI reward (commission: {} KARI)",
                    address_str,
                    validator_reward as f64 / KA_PER_KARI as f64,
                    commission as f64 / KA_PER_KARI as f64
                );
            }
        }
        
        // Update pool state
        if let Some(pool_balance_ref) = balances.get_mut(&pool_addr_str) {
            *pool_balance_ref = pool_balance_ref.saturating_sub(total_distributed);
        }
        
        pool_manager.last_distribution = current_time;
        pool_manager.pool_state.last_reward_epoch = current_time / 86400; // Daily epochs
    }
    
    save_enhanced_staking_state()?;
    
    Ok(total_distributed)
}

// Add Move-aligned penalty calculation
pub fn calculate_early_unstake_penalty(amount: u64) -> u64 {
    (amount * EARLY_UNSTAKE_PENALTY_BASIS_POINTS) / 10000
}

// Add Move-aligned validation functions
pub fn can_unstake_without_penalty(node: &EnhancedStakedNode, current_time: u64) -> bool {
    current_time >= node.unlock_time
}

pub fn validate_unlock_timing(node: &EnhancedStakedNode, current_time: u64) -> Result<(), BlockchainError> {
    if !can_unstake_without_penalty(node, current_time) {
        return Err(BlockchainError::Transaction(
            format!("Stake is still locked until {}", node.unlock_time)
        ));
    }
    Ok(())
}

// Enhanced view functions
pub fn get_enhanced_staking_info(address: &Address) -> Option<EnhancedStakedNode> {
    let nodes = ENHANCED_STAKING_NODES.read().ok()?;
    nodes.get(&address.to_hex_literal()).cloned()
}

pub fn get_validator_list() -> Vec<ValidatorInfo> {
    match VALIDATOR_REGISTRY.read() {
        Ok(validators) => validators.values().cloned().collect(),
        Err(_) => Vec::new(),
    }
}

pub fn get_pool_state() -> StakingPoolState {
    match STAKING_POOL_MANAGER.lock() {
        Ok(manager) => manager.pool_state.clone(),
        Err(_) => StakingPoolState {
            pool_id: ObjectID::new(Address::ZERO),
            total_staked: 0,
            validator_count: 0,
            node_count: 0,
            reward_rate: 0,
            last_reward_epoch: 0,
        },
    }
}

// Enhanced storage functions
fn save_enhanced_staking_state() -> Result<(), BlockchainError> {
    let nodes = ENHANCED_STAKING_NODES.read()
        .map_err(|_| BlockchainError::Storage("Failed to read enhanced nodes".to_string()))?
        .clone();
    
    let validators = VALIDATOR_REGISTRY.read()
        .map_err(|_| BlockchainError::Storage("Failed to read validators".to_string()))?
        .clone();
    
    let pool_manager = STAKING_POOL_MANAGER.lock()
        .map_err(|_| BlockchainError::Storage("Failed to read pool manager".to_string()))?
        .clone();
    
    // Serialize data
    let nodes_data = bincode::serialize(&nodes)
        .map_err(|e| BlockchainError::Storage(format!("Failed to serialize nodes: {}", e)))?;
    
    let validators_data = bincode::serialize(&validators)
        .map_err(|e| BlockchainError::Storage(format!("Failed to serialize validators: {}", e)))?;
    
    let pool_data = bincode::serialize(&pool_manager)
        .map_err(|e| BlockchainError::Storage(format!("Failed to serialize pool: {}", e)))?;
    
    // Save to storage
    let kari_dir = common::get_kari_dir();
    let db_path = kari_dir.join("storage").join("blockchain_db");
    let storage = mona_storage::RocksDBStorage::new(db_path)
        .map_err(|e| BlockchainError::Storage(format!("Failed to open storage: {}", e)))?;
    
    storage.save_data(b"enhanced_staking_nodes", &nodes_data)?;
    storage.save_data(b"validator_registry", &validators_data)?;
    storage.save_data(b"staking_pool_manager", &pool_data)?;
    storage.flush()?;
    
    Ok(())
}

pub fn load_enhanced_staking_state() -> Result<(), BlockchainError> {
    let kari_dir = common::get_kari_dir();
    let db_path = kari_dir.join("storage").join("blockchain_db");
    let storage = mona_storage::RocksDBStorage::new(db_path)
        .map_err(|e| BlockchainError::Storage(format!("Failed to open storage: {}", e)))?;
    
    // Load enhanced nodes
    if let Ok(Some(nodes_data)) = storage.load_data(b"enhanced_staking_nodes") {
        if let Ok(nodes) = bincode::deserialize::<HashMap<String, EnhancedStakedNode>>(&nodes_data) {
            *ENHANCED_STAKING_NODES.write()
                .map_err(|_| BlockchainError::Storage("Failed to write nodes".to_string()))? = nodes;
        }
    }
    
    // Load validators
    if let Ok(Some(validators_data)) = storage.load_data(b"validator_registry") {
        if let Ok(validators) = bincode::deserialize::<HashMap<String, ValidatorInfo>>(&validators_data) {
            *VALIDATOR_REGISTRY.write()
                .map_err(|_| BlockchainError::Storage("Failed to write validators".to_string()))? = validators;
        }
    }
    
    // Load pool manager
    if let Ok(Some(pool_data)) = storage.load_data(b"staking_pool_manager") {
        if let Ok(pool_manager) = bincode::deserialize::<StakingPoolManager>(&pool_data) {
            *STAKING_POOL_MANAGER.lock()
                .map_err(|_| BlockchainError::Storage("Failed to write pool manager".to_string()))? = pool_manager;
        }
    }
    
    info!("Enhanced staking state loaded successfully");
    Ok(())
}
