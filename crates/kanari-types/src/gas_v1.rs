// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};

/// Gas configuration and pricing for the Kanari blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasConfig {
    /// Base gas price per unit (in Mist)
    pub base_price: u64,

    /// Maximum gas units per transaction
    pub max_gas_per_tx: u64,

    /// Maximum gas units per block
    pub max_gas_per_block: u64,

    /// Minimum gas price (in Mist)
    pub min_gas_price: u64,

    /// Cost per byte of storage written (in Mist)
    pub storage_price_per_byte: u64,

    /// Percentage of storage fee refunded when data is deleted (0-100)
    pub storage_rebate_rate: u8,
}

impl GasConfig {
    pub fn default_transaction_gas_limit(&self) -> u64 {
        self.max_gas_per_tx
    }

    pub fn default_transaction_gas_price(&self) -> u64 {
        self.base_price.max(self.min_gas_price)
    }

    pub fn validate_price(&self, gas_price: u64) -> Result<(), GasError> {
        if gas_price < self.min_gas_price {
            return Err(GasError::PriceTooLow {
                provided: gas_price,
                minimum: self.min_gas_price,
            });
        }
        Ok(())
    }
}

impl Default for GasConfig {
    fn default() -> Self {
        Self {
            base_price: 1,                // 1 Mist per gas unit (extremely low)
            max_gas_per_tx: 100_000,      // 100K gas per transaction
            max_gas_per_block: 1_000_000, // 1M gas per block
            min_gas_price: 1,             // 1 Mist minimum
            storage_price_per_byte: 1,    // 1 Mist per byte (extremely low)
            storage_rebate_rate: 99,      // 99% rebate (Sui-like)
        }
    }
}

/// Gas costs for different operations
#[derive(Debug, Clone, Copy)]
pub enum GasOperation {
    /// Transfer native tokens
    Transfer,
    /// Publish a Move module
    PublishModule { module_size: usize },
    /// Execute a Move function
    ExecuteFunction { complexity: u32 },
    /// Create new account
    CreateAccount,
    /// Update account state
    UpdateAccount,
}

impl GasOperation {
    /// Calculate gas units required for this operation
    pub fn gas_units(&self) -> u64 {
        match self {
            GasOperation::Transfer => 100, // ~0.0000001 KANARI (100 Mist)
            GasOperation::PublishModule { module_size } => {
                // Base cost + per-byte cost (very low)
                500 + (*module_size as u64)
            }
            GasOperation::ExecuteFunction { complexity } => {
                // Base cost + minimal complexity multiplier
                200 + (*complexity as u64 * 10)
            }
            GasOperation::CreateAccount => 150, // ~0.00000015 KANARI
            GasOperation::UpdateAccount => 50,  // ~0.00000005 KANARI
        }
    }

    /// Get operation name for logging
    pub fn name(&self) -> &str {
        match self {
            GasOperation::Transfer => "Transfer",
            GasOperation::PublishModule { .. } => "PublishModule",
            GasOperation::ExecuteFunction { .. } => "ExecuteFunction",
            GasOperation::CreateAccount => "CreateAccount",
            GasOperation::UpdateAccount => "UpdateAccount",
        }
    }
}

/// Gas meter for tracking gas usage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasMeter {
    /// Gas units used
    pub gas_used: u64,

    /// Gas price per unit (in Mist)
    pub gas_price: u64,

    /// Maximum gas allowed
    pub gas_limit: u64,

    /// Storage bytes written in this transaction
    pub storage_bytes_written: u64,

    /// Storage bytes deleted in this transaction
    pub storage_bytes_deleted: u64,
}

impl GasMeter {
    pub fn new(gas_limit: u64, gas_price: u64) -> Self {
        Self {
            gas_used: 0,
            gas_price,
            gas_limit,
            storage_bytes_written: 0,
            storage_bytes_deleted: 0,
        }
    }

    /// Charge for storage bytes written
    pub fn charge_storage(&mut self, bytes: u64, _config: &GasConfig) -> Result<(), GasError> {
        self.storage_bytes_written = self
            .storage_bytes_written
            .checked_add(bytes)
            .ok_or(GasError::Overflow)?;
        Ok(())
    }

    /// Record storage rebate (refund)
    pub fn rebate_storage(&mut self, bytes: u64) {
        self.storage_bytes_deleted = self.storage_bytes_deleted.saturating_add(bytes);
    }

    /// Calculate net storage fee in Mist
    pub fn net_storage_fee(&self, config: &GasConfig) -> i64 {
        let price = config.storage_price_per_byte as i128;
        let cost = self.storage_bytes_written as i128 * price;
        let rebate =
            self.storage_bytes_deleted as i128 * price * config.storage_rebate_rate as i128 / 100;
        (cost - rebate).clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }

    /// Consume gas for an operation
    pub fn consume(&mut self, gas_units: u64) -> Result<(), GasError> {
        let new_usage = self
            .gas_used
            .checked_add(gas_units)
            .ok_or(GasError::Overflow)?;

        if new_usage > self.gas_limit {
            return Err(GasError::OutOfGas {
                required: new_usage,
                limit: self.gas_limit,
            });
        }

        self.gas_used = new_usage;
        Ok(())
    }

    /// Calculate total gas cost in Mist
    pub fn total_cost(&self) -> u64 {
        self.gas_used.saturating_mul(self.gas_price)
    }

    /// Calculate remaining gas
    pub fn remaining(&self) -> u64 {
        self.gas_limit.saturating_sub(self.gas_used)
    }

    /// Check if enough gas remains
    pub fn has_enough(&self, gas_units: u64) -> bool {
        self.remaining() >= gas_units
    }

    /// Get gas usage percentage
    pub fn usage_percentage(&self) -> f64 {
        if self.gas_limit == 0 {
            return 0.0;
        }
        (self.gas_used as f64 / self.gas_limit as f64) * 100.0
    }
}

/// Gas estimation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasEstimate {
    pub gas_units: u64,
    pub gas_price: u64,
    pub total_cost_mist: u64,
    pub total_cost_kanari: f64,
}

impl GasEstimate {
    pub fn new(gas_units: u64, gas_price: u64) -> Self {
        let total_cost_mist = gas_units.saturating_mul(gas_price);
        let total_cost_kanari = total_cost_mist as f64 / 1_000_000_000.0;

        Self {
            gas_units,
            gas_price,
            total_cost_mist,
            total_cost_kanari,
        }
    }

    pub fn from_operation(operation: GasOperation, gas_price: u64) -> Self {
        Self::new(operation.gas_units(), gas_price)
    }
}

/// Gas-related errors
#[derive(Debug, Clone)]
pub enum GasError {
    OutOfGas { required: u64, limit: u64 },
    InsufficientBalance { required: u64, available: u64 },
    PriceTooLow { provided: u64, minimum: u64 },
    Overflow,
}

impl std::fmt::Display for GasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GasError::OutOfGas { required, limit } => {
                write!(
                    f,
                    "Out of gas: required {} but limit is {}",
                    required, limit
                )
            }
            GasError::InsufficientBalance {
                required,
                available,
            } => {
                write!(
                    f,
                    "Insufficient balance for gas: required {} Mist but only {} available",
                    required, available
                )
            }
            GasError::PriceTooLow { provided, minimum } => {
                write!(
                    f,
                    "Gas price too low: provided {} but minimum is {}",
                    provided, minimum
                )
            }
            GasError::Overflow => write!(f, "Gas calculation overflow"),
        }
    }
}

impl std::error::Error for GasError {}

/// Transaction gas info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionGas {
    pub gas_limit: u64,
    pub gas_price: u64,
    pub gas_used: u64,
    pub gas_refund: u64,
}

impl TransactionGas {
    pub fn new(gas_limit: u64, gas_price: u64) -> Self {
        Self {
            gas_limit,
            gas_price,
            gas_used: 0,
            gas_refund: 0,
        }
    }

    pub fn total_cost(&self) -> u64 {
        self.gas_used.saturating_mul(self.gas_price)
    }

    pub fn refund_amount(&self) -> u64 {
        self.gas_refund.saturating_mul(self.gas_price)
    }

    pub fn net_cost(&self) -> u64 {
        self.total_cost().saturating_sub(self.refund_amount())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gas_config_provides_valid_transaction_defaults() {
        let config = GasConfig::default();

        assert_eq!(
            config.default_transaction_gas_limit(),
            config.max_gas_per_tx
        );
        assert!(config.default_transaction_gas_price() >= config.min_gas_price);
    }

    #[test]
    fn gas_config_rejects_price_below_minimum() {
        let config = GasConfig::default();

        assert!(matches!(
            config.validate_price(0),
            Err(GasError::PriceTooLow { .. })
        ));
        assert!(config.validate_price(config.min_gas_price).is_ok());
    }

    #[test]
    fn storage_fee_is_not_added_to_execution_gas_units() {
        let config = GasConfig::default();
        let mut meter = GasMeter::new(100_000, 10);
        meter.consume(100).unwrap();
        meter.charge_storage(1_000, &config).unwrap();

        assert_eq!(meter.gas_used, 100);
        assert_eq!(meter.total_cost(), 1_000);
        assert_eq!(meter.net_storage_fee(&config), 1_000);
    }

    #[test]
    fn test_gas_meter_consume() {
        let mut meter = GasMeter::new(100_000, 1);

        assert!(meter.consume(100).is_ok());
        assert_eq!(meter.gas_used, 100);
        assert_eq!(meter.remaining(), 99_900);
    }

    #[test]
    fn test_gas_meter_out_of_gas() {
        let mut meter = GasMeter::new(50, 1);

        let result = meter.consume(100);
        assert!(result.is_err());
    }

    #[test]
    fn test_gas_operation_costs() {
        assert_eq!(GasOperation::Transfer.gas_units(), 100);
        assert_eq!(GasOperation::CreateAccount.gas_units(), 150);

        let publish = GasOperation::PublishModule { module_size: 1000 };
        assert_eq!(publish.gas_units(), 1_500); // 500 + 1000

        let execute = GasOperation::ExecuteFunction { complexity: 10 };
        assert_eq!(execute.gas_units(), 300); // 200 + 10 * 10
    }

    #[test]
    fn test_gas_estimate() {
        let estimate = GasEstimate::new(100, 1);
        assert_eq!(estimate.gas_units, 100);
        assert_eq!(estimate.total_cost_mist, 100);
        assert_eq!(estimate.total_cost_kanari, 0.0000001);
    }

    #[test]
    fn test_gas_meter_total_cost() {
        let mut meter = GasMeter::new(100_000, 1);
        meter.consume(100).unwrap();

        assert_eq!(meter.total_cost(), 100); // 100 * 1
    }

    #[test]
    fn test_gas_usage_percentage() {
        let mut meter = GasMeter::new(100_000, 1);
        meter.consume(25_000).unwrap();

        assert_eq!(meter.usage_percentage(), 25.0);
    }

    #[test]
    fn test_transaction_gas() {
        let mut tx_gas = TransactionGas::new(100_000, 1);
        tx_gas.gas_used = 100;
        tx_gas.gas_refund = 20;

        assert_eq!(tx_gas.total_cost(), 100);
        assert_eq!(tx_gas.refund_amount(), 20);
        assert_eq!(tx_gas.net_cost(), 80);
    }
}
