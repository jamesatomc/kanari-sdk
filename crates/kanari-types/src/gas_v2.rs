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

impl Default for GasConfig {
    fn default() -> Self {
        Self {
            base_price: 0,
            max_gas_per_tx: 100_000, // Keep limit to prevent infinite loops
            max_gas_per_block: 1_000_000,
            min_gas_price: 0,
            storage_price_per_byte: 0,
            storage_rebate_rate: 0,
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
        0 // All operations are free
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
        self.storage_bytes_written += bytes;
        Ok(())
    }

    /// Record storage rebate (refund)
    pub fn rebate_storage(&mut self, bytes: u64) {
        self.storage_bytes_deleted += bytes;
    }

    /// Calculate net storage fee in Mist
    pub fn net_storage_fee(&self, _config: &GasConfig) -> i64 {
        0
    }

    /// Consume gas for an operation
    pub fn consume(&mut self, _gas_units: u64) -> Result<(), GasError> {
        Ok(())
    }

    /// Calculate total gas cost in Mist
    pub fn total_cost(&self) -> u64 {
        0
    }

    /// Calculate remaining gas
    pub fn remaining(&self) -> u64 {
        self.gas_limit
    }

    /// Check if enough gas remains
    pub fn has_enough(&self, gas_units: u64) -> bool {
        self.remaining() >= gas_units
    }

    /// Get gas usage percentage
    pub fn usage_percentage(&self) -> f64 {
        0.0
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
        Self {
            gas_units,
            gas_price,
            total_cost_mist: 0,
            total_cost_kanari: 0.0,
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
        0
    }

    pub fn refund_amount(&self) -> u64 {
        0
    }

    pub fn net_cost(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_meter_consume() {
        let mut meter = GasMeter::new(100_000, 1000);

        // ตอนนี้การ consume จะไม่บันทึกค่า gas_used แล้ว และผ่านเสมอ
        assert!(meter.consume(21_000).is_ok());
        assert_eq!(meter.gas_used, 0);
        assert_eq!(meter.remaining(), 100_000);
    }

    #[test]
    fn test_gas_operation_costs() {
        // ค่า Gas ของทุก operation ต้องเป็น 0 ตามที่แก้ใหม่
        assert_eq!(GasOperation::Transfer.gas_units(), 0);
        assert_eq!(GasOperation::CreateAccount.gas_units(), 0);

        let publish = GasOperation::PublishModule { module_size: 1000 };
        assert_eq!(publish.gas_units(), 0);

        let execute = GasOperation::ExecuteFunction { complexity: 10 };
        assert_eq!(execute.gas_units(), 0);
    }

    #[test]
    fn test_gas_estimate() {
        let estimate = GasEstimate::new(21_000, 1000);
        assert_eq!(estimate.gas_units, 21_000);
        assert_eq!(estimate.total_cost_mist, 0); // คาดหวัง 0
        assert_eq!(estimate.total_cost_kanari, 0.0);
    }

    #[test]
    fn test_gas_meter_total_cost() {
        let mut meter = GasMeter::new(100_000, 1500);
        meter.consume(21_000).unwrap();

        assert_eq!(meter.total_cost(), 0); // ต้องเป็น 0
    }

    #[test]
    fn test_gas_usage_percentage() {
        let mut meter = GasMeter::new(100_000, 1000);
        meter.consume(25_000).unwrap();

        assert_eq!(meter.usage_percentage(), 0.0);
    }

    #[test]
    fn test_transaction_gas() {
        let mut tx_gas = TransactionGas::new(100_000, 1000);
        tx_gas.gas_used = 21_000;
        tx_gas.gas_refund = 5_000;

        assert_eq!(tx_gas.total_cost(), 0);
        assert_eq!(tx_gas.refund_amount(), 0);
        assert_eq!(tx_gas.net_cost(), 0);
    }
}
