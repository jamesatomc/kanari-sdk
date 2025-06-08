//! Mona VM - Move virtual machine implementation for Kanari blockchain
//!
//! This module provides the core Move VM functionality for executing smart contracts
//! on the Kanari blockchain, with Kari token as the gas currency.

pub mod types;
pub mod vm;
pub mod executor;
pub mod state;
pub mod gas;
pub mod move_adapter;
pub mod storage;

// Re-export main types
pub use types::{
    VMConfig, VMError, VMResult, VMEvent,
    ContractAddress, ExecutionStats, DeploymentInfo, 
    FunctionCall, TransactionReceipt, ContractEvent
};
pub use executor::{ExecutionContext, ExecutionResult};
pub use state::{StateManager, ContractState};
pub use gas::{GasParameters, GasCosts, MoveCosts, StorageCosts, BaseCosts};
pub use move_adapter::{MoveAdapter, MoveModuleInfo, MoveFunction, MoveFunctionVisibility};
pub use storage::VMStorage;

#[cfg(test)]
mod tests {
    use super::*;
    use mona_types::address::Address;

    #[test]
    fn test_vm_types() {
        // Test that all types are properly defined and usable
        let _stats = ExecutionStats {
            gas_used: 100,
            execution_time_ms: 50,
            instructions_executed: 1000,
            memory_used: 2048,
            storage_reads: 5,
            storage_writes: 2,
        };

        let _deployment = DeploymentInfo {
            contract_address: Address::zero(),
            deployer: Address::zero(),
            bytecode_hash: "test_hash".to_string(),
            gas_used: 1000,
            timestamp: 1234567890,
            module_name: "test_module".to_string(),
        };
    }
}