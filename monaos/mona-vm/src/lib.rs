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
pub mod recovery;
pub mod diagnostics;

#[cfg(test)]
pub mod tests;

// Re-export main types
pub use types::{
    VMConfig, VMError, VMResult, VMEvent,
    ContractAddress, ExecutionStats, DeploymentInfo, 
    FunctionCall, TransactionReceipt, ContractEvent
};
pub use vm::MonaVM;
pub use executor::{ExecutionContext, ExecutionResult};
pub use state::{StateManager, ContractState};
pub use gas::{GasParameters, GasCosts, MoveCosts, StorageCosts, BaseCosts};
pub use move_adapter::{MoveAdapter, MoveModuleInfo, MoveFunction, MoveFunctionVisibility};
pub use storage::VMStorage;
pub use recovery::{RecoveryManager, RecoveryStrategy, ProblemClassification, RecoveryResult};
pub use diagnostics::{DiagnosticManager, DiagnosticReport, PerformanceMetrics, Alert};