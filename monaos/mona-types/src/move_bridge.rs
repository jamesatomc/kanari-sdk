use crate::address::Address;
use crate::kari::KARI;
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Represents a Move object ID from the kanari_framework::object module
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectID {
    pub bytes: Address,
}

impl ObjectID {
    /// Create a new ObjectID from an Address
    pub fn new(address: Address) -> Self {
        Self { bytes: address }
    }

    /// Convert to Move AccountAddress for interaction with Move code
    pub fn to_account_address(&self) -> AccountAddress {
        self.bytes.into()
    }

    /// Create from Move AccountAddress
    pub fn from_account_address(addr: AccountAddress) -> Self {
        Self {
            bytes: Address::from(addr),
        }
    }

    /// Get the underlying address
    pub fn address(&self) -> &Address {
        &self.bytes
    }

    /// Convert to hex string representation
    pub fn to_hex(&self) -> String {
        self.bytes.to_hex_literal()
    }
}

/// Represents a Move UID from the kanari_framework::object module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UID {
    pub id: ObjectID,
}

impl UID {
    /// Create a new UID with the given ObjectID
    pub fn new(id: ObjectID) -> Self {
        Self { id }
    }

    /// Get the inner ObjectID
    pub fn inner(&self) -> &ObjectID {
        &self.id
    }

    /// Convert to address for storage operations
    pub fn to_address(&self) -> Address {
        self.id.bytes
    }
}

/// Represents the Receiving<T> struct from kanari_framework::transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receiving<T> {
    pub id: ObjectID,
    pub version: u64,
    pub phantom: std::marker::PhantomData<T>,
}

impl<T> Receiving<T> {
    /// Create a new Receiving object
    pub fn new(id: ObjectID, version: u64) -> Self {
        Self {
            id,
            version,
            phantom: std::marker::PhantomData,
        }
    }

    /// Get the receiving object ID
    pub fn receiving_object_id(&self) -> ObjectID {
        self.id
    }
}

/// Bridge for KARI token operations with Move
pub struct KariMoveBridge {
    pub token_info: KARI,
}

impl KariMoveBridge {
    /// Create a new bridge instance
    pub fn new() -> Self {
        Self {
            token_info: KARI::default(),
        }
    }

    /// Convert KARI amount to Move-compatible format
    pub fn kari_to_move_amount(&self, kari_amount: u64) -> u64 {
        kari_amount
    }

    /// Convert Move amount back to KARI
    pub fn move_amount_to_kari(&self, move_amount: u64) -> u64 {
        move_amount
    }

    /// Get the pool address as ObjectID
    pub fn pool_object_id(&self) -> Result<ObjectID, crate::address::AddressParseError> {
        let addr = Address::from_hex_literal(&self.token_info.pool_address)?;
        Ok(ObjectID::new(addr))
    }

    /// Create a transfer operation compatible with Move
    pub fn create_transfer_data(&self, recipient: Address, amount: u64) -> TransferData {
        TransferData {
            recipient,
            amount,
            token_type: "KARI".to_string(),
        }
    }
}

impl Default for KariMoveBridge {
    fn default() -> Self {
        Self::new()
    }
}

/// Data structure for Move transfer operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferData {
    pub recipient: Address,
    pub amount: u64,
    pub token_type: String,
}

/// Represents staking operations for Move integration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakingRequest {
    pub staker: Address,
    pub amount: u64,
    pub wants_validator: bool,
    pub lock_period: u64,
}

/// Represents a staked position in Move
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakedPosition {
    pub id: ObjectID,
    pub owner: Address,
    pub amount: u64,
    pub staked_at: u64,
    pub unlock_time: u64,
    pub is_validator: bool,
    pub accumulated_rewards: u64,
}

/// Staking pool state for Move operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakingPoolState {
    pub pool_id: ObjectID,
    pub total_staked: u64,
    pub validator_count: u64,
    pub node_count: u64,
    pub reward_rate: u64, // Annual reward rate in basis points
    pub last_reward_epoch: u64,
}

/// Validator registry for consensus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorInfo {
    pub address: Address,
    pub stake_amount: u64,
    pub commission_rate: u64, // In basis points
    pub is_active: bool,
    pub last_epoch_reward: u64,
}

impl StakingRequest {
    pub fn new(staker: Address, amount: u64, wants_validator: bool) -> Self {
        let _current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            staker,
            amount,
            wants_validator,
            lock_period: 86400, // 24 hours default
        }
    }

    pub fn validate(&self, kari_config: &KARI) -> Result<(), String> {
        if self.amount == 0 {
            return Err("Staking amount cannot be zero".to_string());
        }

        if !kari_config.can_run_node(self.amount) {
            return Err(format!(
                "Amount {} is below minimum node requirement {}",
                self.amount, kari_config.node_minimum
            ));
        }

        if self.wants_validator && !kari_config.can_stake_as_validator(self.amount) {
            return Err(format!(
                "Amount {} is below minimum validator requirement {}",
                self.amount, kari_config.validator_minimum
            ));
        }

        Ok(())
    }
}

impl StakedPosition {
    pub fn new(
        staker: Address,
        amount: u64,
        wants_validator: bool,
        kari_config: &KARI,
    ) -> Self {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let is_validator = wants_validator && kari_config.can_stake_as_validator(amount);

        Self {
            id: ObjectID::new(staker), // Simplified, should be unique
            owner: staker,
            amount,
            staked_at: current_time,
            unlock_time: current_time + 86400, // 24 hours
            is_validator,
            accumulated_rewards: 0,
        }
    }

    pub fn is_unlocked(&self) -> bool {
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        current_time >= self.unlock_time
    }

    pub fn calculate_reward(&self, reward_rate: f64, current_time: u64) -> u64 {
        if current_time <= self.staked_at || !self.is_validator {
            return 0;
        }

        let time_staked = current_time - self.staked_at;
        let days_staked = time_staked as f64 / 86400.0;
        let annual_reward = self.amount as f64 * reward_rate;
        let reward = (annual_reward * days_staked / 365.0) as u64;

        reward
    }
}

/// Convert Move constants to Rust
pub mod move_constants {
    use crate::address::Address;

    /// Convert Move address constants to Rust Address
    pub fn kari_system_state_object_id() -> Address {
        Address::from_hex_literal("0x5").expect("Valid system state address")
    }

    pub fn kari_clock_object_id() -> Address {
        Address::from_hex_literal("0x6").expect("Valid clock address")
    }

    pub fn kari_authenticator_state_id() -> Address {
        Address::from_hex_literal("0x7").expect("Valid authenticator state address")
    }

    pub fn kari_random_id() -> Address {
        Address::from_hex_literal("0x8").expect("Valid random address")
    }

    pub fn kari_deny_list_object_id() -> Address {
        Address::from_hex_literal("0x403").expect("Valid deny list address")
    }
}
