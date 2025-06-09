use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::string::String as StdString;

use crate::address::Address;
use crate::balance::{Balance, Supply};

/// A coin of type T worth `value`. Transferable and storable.
/// Corresponds to `kanari_framework::coin::Coin<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coin<T> {
    pub id: ObjectId,
    pub balance: Balance<T>,
}

/// Object ID type used in coins and other objects
/// Corresponds to `kanari_framework::object::UID` in Move
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectId {
    pub id: Address,
}

/// Each Coin type T created through `create_currency` function will have a
/// unique instance of CoinMetadata<T> that stores the metadata for this coin type.
/// Corresponds to `kanari_framework::coin::CoinMetadata<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoinMetadata<T> {
    pub id: ObjectId,
    /// Number of decimal places the coin uses.
    pub decimals: u8,
    /// Name for the token
    pub name: StdString,
    /// Symbol for the token
    pub symbol: StdString,
    /// Description of the token
    pub description: StdString,
    /// URL for the token logo
    pub icon_url: Option<StdString>,
    _phantom: PhantomData<T>,
}

/// Capability allowing the bearer to mint and burn coins of type T.
/// Corresponds to `kanari_framework::coin::TreasuryCap<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreasuryCap<T> {
    pub id: ObjectId,
    pub total_supply: Supply<T>,
}

/// Capability allowing the bearer to freeze addresses
/// Corresponds to `kanari_framework::coin::DenyCap<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DenyCap<T> {
    pub id: ObjectId,
    _phantom: PhantomData<T>,
}

/// Regulated coin metadata for coins that use the DenyList
/// Corresponds to `kanari_framework::coin::RegulatedCoinMetadata<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegulatedCoinMetadata<T> {
    pub id: ObjectId,
    /// The ID of the coin's CoinMetadata object
    pub coin_metadata_object: ObjectId,
    /// The ID of the coin's DenyCap object
    pub deny_cap_object: ObjectId,
    _phantom: PhantomData<T>,
}

impl ObjectId {
    /// Create a new ObjectId from an address
    pub fn new(id: Address) -> Self {
        Self { id }
    }

    /// Get the address
    pub fn address(&self) -> Address {
        self.id
    }    /// Get the bytes representation
    pub fn to_bytes(&self) -> [u8; 32] {
        *self.id.to_bytes()
    }
}

impl<T> Coin<T> {
    /// Create a new coin from balance
    pub fn from_balance(balance: Balance<T>, id: ObjectId) -> Self {
        Self { id, balance }
    }

    /// Get the coin's value
    pub fn value(&self) -> u64 {
        self.balance.value()
    }

    /// Get immutable reference to the balance
    pub fn balance(&self) -> &Balance<T> {
        &self.balance
    }

    /// Get mutable reference to the balance
    pub fn balance_mut(&mut self) -> &mut Balance<T> {
        &mut self.balance
    }

    /// Convert coin into balance, consuming the coin
    pub fn into_balance(self) -> Balance<T> {
        self.balance
    }

    /// Join another coin into this one
    pub fn join(&mut self, other: Coin<T>) {
        self.balance.join(other.balance);
    }

    /// Split this coin into two, with the specified value going to the new coin
    pub fn split(&mut self, value: u64) -> Result<Coin<T>, CoinError> {
        let new_balance = self.balance.split(value)
            .map_err(|_| CoinError::NotEnough)?;
        
        // In a real implementation, you'd generate a new ObjectId
        // For now, we'll use the same id (this would need proper handling)
        Ok(Coin::from_balance(new_balance, self.id.clone()))
    }

    /// Check if the coin has zero value
    pub fn is_zero(&self) -> bool {
        self.balance.is_zero()
    }
}

impl<T> CoinMetadata<T> {
    /// Create new coin metadata
    pub fn new(
        id: ObjectId,
        decimals: u8,
        name: StdString,
        symbol: StdString,
        description: StdString,
        icon_url: Option<StdString>,
    ) -> Self {
        Self {
            id,
            decimals,
            name,
            symbol,
            description,
            icon_url,
            _phantom: PhantomData,
        }
    }
}

impl<T> TreasuryCap<T> {
    /// Create a new treasury cap
    pub fn new(id: ObjectId, total_supply: Supply<T>) -> Self {
        Self { id, total_supply }
    }

    /// Get the total supply value
    pub fn total_supply(&self) -> u64 {
        self.total_supply.value()
    }

    /// Get immutable reference to the supply
    pub fn supply_immut(&self) -> &Supply<T> {
        &self.total_supply
    }

    /// Get mutable reference to the supply
    pub fn supply_mut(&mut self) -> &mut Supply<T> {
        &mut self.total_supply
    }

    /// Convert treasury cap into supply
    pub fn into_supply(self) -> Supply<T> {
        self.total_supply
    }

    /// Mint new coins
    pub fn mint(&mut self, value: u64) -> Result<Balance<T>, CoinError> {
        self.total_supply.increase_supply(value)
            .map_err(|_| CoinError::Overflow)
    }

    /// Burn coins
    pub fn burn(&mut self, balance: Balance<T>) -> Result<u64, CoinError> {
        self.total_supply.decrease_supply(balance)
            .map_err(|_| CoinError::Overflow)
    }
}

/// Coin operation errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoinError {
    #[error("Bad witness type")]
    BadWitness,
    #[error("Invalid arguments")]
    InvalidArg,
    #[error("Not enough balance")]
    NotEnough,
    #[error("Overflow in coin operation")]
    Overflow,
}

/// Coin error constants matching Move constants
pub mod error_constants {
    /// A type passed to create_supply is not a one-time witness.
    pub const E_BAD_WITNESS: u64 = 0;
    /// Invalid arguments are passed to a function.
    pub const E_INVALID_ARG: u64 = 1;
    /// Trying to split a coin more times than its balance allows.
    pub const E_NOT_ENOUGH: u64 = 2;
}
