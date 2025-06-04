use std::time::{SystemTime, UNIX_EPOCH};
use crate::address::Address;
use crate::move_bridge::{ObjectID, KariMoveBridge, TransferData};


// Constants for Kari token
/// The amount of KA per Kari token based on the the fact that KA is
/// 10^-9 of a Kari token
pub const KA_PER_KARI: u64 = 1_000_000_000;

/// The total supply of Kari denominated in whole Kari tokens (100 Million)
pub const TOTAL_SUPPLY_KARI: u64 = 100_000_000;

/// The total supply of Kari denominated in KA (100 Million * 10^9)
pub const TOTAL_SUPPLY_KA: u64 = 100_000_000_000_000_000;

/// The amount of Kari reserved in the pool (40 Million)
pub const POOL_RESERVED_KARI: u64 = 40_000_000;

/// The amount of KA reserved in the pool (40 Million * 10^9)
pub const POOL_RESERVED_KA: u64 = POOL_RESERVED_KARI * KA_PER_KARI;

/// The pool address where reserved KARI is stored
pub const POOL_ADDRESS: &str = "0x47621776628ba3a5b9baaab38e61f4c98e893e124204bc4dad52e702e2b24ea1";

/// Minimum KARI required to run a node (200 KARI)
pub const NODE_STAKING_MINIMUM_KARI: u64 = 200;
/// Minimum KARI required to run a node in KA (200 * 10^9)
pub const NODE_STAKING_MINIMUM_KA: u64 = NODE_STAKING_MINIMUM_KARI * KA_PER_KARI;

/// Minimum KARI for validator staking (32 KARI)
pub const VALIDATOR_STAKING_MINIMUM_KARI: u64 = 32;
/// Minimum KARI for validator staking in KA (32 * 10^9)
pub const VALIDATOR_STAKING_MINIMUM_KA: u64 = VALIDATOR_STAKING_MINIMUM_KARI * KA_PER_KARI;

/// Annual staking reward percentage (0.01%)
pub const STAKING_REWARD_PERCENTAGE: f64 = 0.0001; // 0.01%

// Enhanced KARI structure with additional properties
#[derive(Clone, Debug)]
pub struct KARI {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub total_supply: u64,
    pub max_supply: u64,      // Maximum supply that will ever exist
    pub block_reward: u64,    // Reward per block if applicable
    pub created_at: u64,      // Timestamp when KARI was created
    pub pool_address: String, // Address where reserved tokens are stored
    pub pool_reserved: u64,   // Amount of tokens reserved in the pool
    pub staking_reward: f64,  // Staking reward percentage
    pub node_minimum: u64,    // Minimum amount to run a node
    pub validator_minimum: u64, // Minimum amount to be a validator
}

impl Default for KARI {
    fn default() -> Self {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
            
        KARI {
            name: "Kanari".to_string(),
            symbol: "KARI".to_string(),
            decimals: 9, // 9 decimals for KA units
            total_supply: TOTAL_SUPPLY_KA,
            max_supply: TOTAL_SUPPLY_KA, // Same as total supply for fixed supply
            block_reward: 0,             // No mining rewards in this implementation
            created_at: current_time,    // Current timestamp
            pool_address: POOL_ADDRESS.to_string(),
            pool_reserved: POOL_RESERVED_KA,
            staking_reward: STAKING_REWARD_PERCENTAGE,
            node_minimum: NODE_STAKING_MINIMUM_KA,
            validator_minimum: VALIDATOR_STAKING_MINIMUM_KA,
        }
    }
}

impl KARI {
    /// Create a bridge for Move operations
    pub fn create_move_bridge(&self) -> KariMoveBridge {
        KariMoveBridge {
            token_info: self.clone(),
        }
    }

    /// Get the pool address as an Address object
    pub fn pool_address_object(&self) -> Result<Address, crate::address::AddressParseError> {
        Address::from_hex_literal(&self.pool_address)
    }

    /// Get the pool address as an ObjectID for Move operations
    pub fn pool_object_id(&self) -> Result<ObjectID, crate::address::AddressParseError> {
        let addr = self.pool_address_object()?;
        Ok(ObjectID::new(addr))
    }

    /// Convert KARI to KA units
    pub fn kari_to_ka(&self, kari_amount: f64) -> u64 {
        (kari_amount * KA_PER_KARI as f64) as u64
    }

    /// Convert KA to KARI units
    pub fn ka_to_kari(&self, ka_amount: u64) -> f64 {
        ka_amount as f64 / KA_PER_KARI as f64
    }

    /// Create transfer data for Move operations
    pub fn create_transfer(&self, recipient: Address, amount: u64) -> TransferData {
        TransferData {
            recipient,
            amount,
            token_type: self.symbol.clone(),
        }
    }

    /// Validate transfer amount
    pub fn validate_transfer_amount(&self, amount: u64) -> bool {
        amount > 0 && amount <= self.total_supply
    }

    /// Check if address can stake as validator
    pub fn can_stake_as_validator(&self, amount: u64) -> bool {
        amount >= self.validator_minimum
    }

    /// Check if address can run a node
    pub fn can_run_node(&self, amount: u64) -> bool {
        amount >= self.node_minimum
    }

    /// Calculate staking rewards for a given amount and duration
    pub fn calculate_staking_reward(&self, staked_amount: u64, days: u64) -> u64 {
        let annual_reward = (staked_amount as f64 * self.staking_reward) as u64;
        (annual_reward * days) / 365
    }
}
