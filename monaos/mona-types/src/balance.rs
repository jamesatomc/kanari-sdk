use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

/// A Supply of T. Used for minting and burning.
/// Corresponds to `kanari_framework::balance::Supply<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Supply<T> {
    pub value: u64,
    _phantom: PhantomData<T>,
}

/// Storable balance - corresponds to a Coin's inner balance.
/// Corresponds to `kanari_framework::balance::Balance<T>` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Balance<T> {
    pub value: u64,
    _phantom: PhantomData<T>,
}

impl<T> Supply<T> {
    /// Create a new supply with zero value
    pub fn new() -> Self {
        Self {
            value: 0,
            _phantom: PhantomData,
        }
    }

    /// Create a supply with a specific value
    pub fn with_value(value: u64) -> Self {
        Self {
            value,
            _phantom: PhantomData,
        }
    }

    /// Get the supply value
    pub fn value(&self) -> u64 {
        self.value
    }

    /// Increase supply by value and create a new Balance<T>
    pub fn increase_supply(&mut self, value: u64) -> Result<Balance<T>, BalanceError> {
        if value > (u64::MAX - self.value) {
            return Err(BalanceError::Overflow);
        }
        self.value += value;
        Ok(Balance::with_value(value))
    }

    /// Burn a Balance<T> and decrease Supply<T>
    pub fn decrease_supply(&mut self, balance: Balance<T>) -> Result<u64, BalanceError> {
        let value = balance.value;
        if self.value < value {
            return Err(BalanceError::Overflow);
        }
        self.value -= value;
        Ok(value)
    }
}

impl<T> Balance<T> {
    /// Create a zero balance
    pub fn zero() -> Self {
        Self {
            value: 0,
            _phantom: PhantomData,
        }
    }

    /// Create a balance with a specific value
    pub fn with_value(value: u64) -> Self {
        Self {
            value,
            _phantom: PhantomData,
        }
    }

    /// Get the balance value
    pub fn value(&self) -> u64 {
        self.value
    }

    /// Join two balances together
    pub fn join(&mut self, other: Balance<T>) -> u64 {
        self.value += other.value;
        self.value
    }

    /// Split a balance and take a sub balance from it
    pub fn split(&mut self, value: u64) -> Result<Balance<T>, BalanceError> {
        if self.value < value {
            return Err(BalanceError::NotEnough);
        }
        self.value -= value;
        Ok(Balance::with_value(value))
    }

    /// Withdraw all balance
    pub fn withdraw_all(&mut self) -> Balance<T> {
        let value = self.value;
        self.value = 0;
        Balance::with_value(value)
    }

    /// Destroy a zero balance
    pub fn destroy_zero(self) -> Result<(), BalanceError> {
        if self.value != 0 {
            return Err(BalanceError::NonZero);
        }
        Ok(())
    }

    /// Check if balance is zero
    pub fn is_zero(&self) -> bool {
        self.value == 0
    }
}

impl<T> Default for Supply<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Default for Balance<T> {
    fn default() -> Self {
        Self::zero()
    }
}

/// Balance operation errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BalanceError {
    #[error("Balance is not zero")]
    NonZero,
    #[error("Overflow in balance operation")]
    Overflow,
    #[error("Not enough balance")]
    NotEnough,
    #[error("Not system address")]
    NotSystemAddress,
}

/// Balance error constants matching Move constants
pub mod error_constants {
    /// For when trying to destroy a non-zero balance.
    pub const E_NON_ZERO: u64 = 0;
    /// For when an overflow is happening on Supply operations.
    pub const E_OVERFLOW: u64 = 1;
    /// For when trying to withdraw more than there is.
    pub const E_NOT_ENOUGH: u64 = 2;
    /// Sender is not @0x0 the system address.
    pub const E_NOT_SYSTEM_ADDRESS: u64 = 3;
}
