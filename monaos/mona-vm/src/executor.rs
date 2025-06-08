//! Execution context and results for VM operations

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use mona_types::address::Address;
use crate::types::{ContractAddress, ContractEvent};

/// Execution context passed to VM operations
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Address that initiated the transaction
    pub caller: Address,
    /// Contract being executed (for function calls)
    pub contract_address: ContractAddress,
    /// Gas limit for this execution
    pub gas_limit: u64,
    /// Gas used so far
    pub gas_used: u64,
    /// Current block timestamp
    pub timestamp: u64,
    /// Current block height
    pub block_height: u64,
    /// Transaction hash
    pub transaction_hash: String,
    /// Extra context data
    pub context_data: HashMap<String, Vec<u8>>,
}

impl ExecutionContext {
    /// Create a new execution context
    pub fn new(caller: Address, contract_address: ContractAddress, gas_limit: u64, timestamp: u64) -> Self {
        Self {
            caller,
            contract_address,
            gas_limit,
            gas_used: 0,
            timestamp,
            block_height: 0,
            transaction_hash: String::new(),
            context_data: HashMap::new(),
        }
    }

    /// Check if we have enough gas remaining
    pub fn has_gas(&self, amount: u64) -> bool {
        self.gas_used + amount <= self.gas_limit
    }

    /// Consume gas from the context
    pub fn consume_gas(&mut self, amount: u64) -> Result<(), String> {
        if self.gas_used + amount > self.gas_limit {
            return Err(format!("Out of gas: used {}, limit {}", self.gas_used + amount, self.gas_limit));
        }
        self.gas_used += amount;
        Ok(())
    }

    /// Get remaining gas
    pub fn remaining_gas(&self) -> u64 {
        self.gas_limit.saturating_sub(self.gas_used)
    }

    /// Set context data
    pub fn set_context_data(&mut self, key: String, value: Vec<u8>) {
        self.context_data.insert(key, value);
    }

    /// Get context data
    pub fn get_context_data(&self, key: &str) -> Option<&Vec<u8>> {
        self.context_data.get(key)
    }
}

/// Result of VM execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Whether execution was successful
    pub success: bool,
    /// Return value from the function (if any)
    pub return_value: Vec<u8>,
    /// Gas consumed during execution
    pub gas_used: u64,
    /// Events emitted during execution
    pub events: Vec<ContractEvent>,
    /// Error message if execution failed
    pub error: Option<String>,
}

impl ExecutionResult {
    /// Create a successful execution result
    pub fn success(return_value: Vec<u8>, gas_used: u64, events: Vec<ContractEvent>) -> Self {
        Self {
            success: true,
            return_value,
            gas_used,
            events,
            error: None,
        }
    }

    /// Create a failed execution result
    pub fn failure(error: String, gas_used: u64) -> Self {
        Self {
            success: false,
            return_value: Vec::new(),
            gas_used,
            events: Vec::new(),
            error: Some(error),
        }
    }

    /// Create an out-of-gas result
    pub fn out_of_gas(gas_limit: u64) -> Self {
        Self {
            success: false,
            return_value: Vec::new(),
            gas_used: gas_limit,
            events: Vec::new(),
            error: Some("Out of gas".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_context() {
        let mut ctx = ExecutionContext::new(
            Address::zero(),
            Address::zero(),
            1000,
            1234567890,
        );

        assert_eq!(ctx.remaining_gas(), 1000);
        assert!(ctx.has_gas(500));
        assert!(!ctx.has_gas(1500));

        ctx.consume_gas(300).unwrap();
        assert_eq!(ctx.gas_used, 300);
        assert_eq!(ctx.remaining_gas(), 700);

        // Test gas exhaustion
        let result = ctx.consume_gas(800);
        assert!(result.is_err());
    }

    #[test]
    fn test_execution_result() {
        let success_result = ExecutionResult::success(
            vec![1, 2, 3],
            500,
            Vec::new(),
        );
        assert!(success_result.success);
        assert_eq!(success_result.gas_used, 500);

        let failure_result = ExecutionResult::failure(
            "Test error".to_string(),
            200,
        );
        assert!(!failure_result.success);
        assert_eq!(failure_result.error, Some("Test error".to_string()));
    }
}
