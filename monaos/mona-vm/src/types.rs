//! Common types used throughout the VM

use serde::{Serialize, Deserialize};
use mona_types::address::Address;
use thiserror::Error;

/// Type alias for contract addresses
pub type ContractAddress = Address;

/// Execution statistics for monitoring and debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionStats {
    pub gas_used: u64,
    pub execution_time_ms: u64,
    pub instructions_executed: u64,
    pub memory_used: u64,
    pub storage_reads: u64,
    pub storage_writes: u64,
}

/// Smart contract deployment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeploymentInfo {
    pub contract_address: ContractAddress,
    pub deployer: Address,
    pub bytecode_hash: String,
    pub gas_used: u64,
    pub timestamp: u64,
    pub module_name: String,
}

/// Move function call information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub contract_address: ContractAddress,
    pub module_name: String,
    pub function_name: String,
    pub caller: Address,
    pub args: Vec<Vec<u8>>,
    pub gas_limit: u64,
}

/// Transaction receipt for contract interactions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionReceipt {
    pub transaction_hash: String,
    pub contract_address: Option<ContractAddress>,
    pub caller: Address,
    pub gas_used: u64,
    pub gas_price: u64,
    pub kari_spent: u64,
    pub success: bool,
    pub return_data: Vec<u8>,
    pub events: Vec<ContractEvent>,
    pub error_message: Option<String>,
    pub execution_time_ms: u64,
    pub move_gas_breakdown: Option<serde_json::Value>,
}

/// Contract event emitted during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEvent {
    pub contract_address: ContractAddress,
    pub event_type: String,
    pub data: Vec<u8>,
    pub indexed_data: Vec<Vec<u8>>,
}

/// VM error types
#[derive(Error, Debug, Clone, Serialize, Deserialize)]
pub enum VMError {
    #[error("Gas limit exceeded: used {used}, limit {limit}")]
    GasLimitExceeded { used: u64, limit: u64 },
    
    #[error("Execution timeout after {duration_ms}ms")]
    ExecutionTimeout { duration_ms: u64 },
    
    #[error("Memory limit exceeded: used {used}, limit {limit}")]
    MemoryLimitExceeded { used: u64, limit: u64 },
    
    #[error("Storage operation limit exceeded")]
    StorageOperationLimitExceeded,
    
    #[error("Move compilation error: {message}")]
    CompilationError { message: String },
    
    #[error("Move execution error: {message}")]
    ExecutionError { message: String },
    
    #[error("Contract not found: {address}")]
    ContractNotFound { address: String },
    
    #[error("Function not found: {module}.{function}")]
    FunctionNotFound { module: String, function: String },
    
    #[error("Invalid bytecode: {message}")]
    InvalidBytecode { message: String },
    
    #[error("Storage error: {message}")]
    StorageError { message: String },
    
    #[error("Invalid arguments: {message}")]
    InvalidArguments { message: String },
    
    #[error("Access denied: {message}")]
    AccessDenied { message: String },
    
    #[error("Internal VM error: {message}")]
    InternalError { message: String },

    #[error("Contract error: {message}")]
    ContractError { message: String },

    #[error("Runtime error: {message}")]
    RuntimeError { message: String },

}

/// VM result type
pub type VMResult<T> = Result<T, VMError>;

/// VM event for monitoring and debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VMEvent {
    ContractDeployed {
        address: ContractAddress,
        deployer: Address,
        gas_used: u64,
        timestamp: u64,
    },
    FunctionCalled {
        contract: ContractAddress,
        function: String,
        caller: Address,
        gas_used: u64,
        success: bool,
        timestamp: u64,
    },
    StateChanged {
        contract: ContractAddress,
        storage_writes: u64,
        timestamp: u64,
    },
    GasConsumed {
        operation: String,
        amount: u64,
        remaining: u64,
        timestamp: u64,
    },
}

/// VM configuration parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VMConfig {
    /// Maximum gas limit per transaction
    pub max_gas_per_transaction: u64,
    /// Maximum execution time per transaction (milliseconds)
    pub max_execution_time_ms: u64,
    /// Maximum memory usage per transaction (bytes)
    pub max_memory_usage: u64,
    /// Maximum number of storage operations per transaction
    pub max_storage_operations: u64,
    /// Enable execution tracing for debugging
    pub enable_tracing: bool,
    /// Cache size for compiled modules
    pub module_cache_size: usize,
}

impl Default for VMConfig {
    fn default() -> Self {
        Self {
            max_gas_per_transaction: 10_000_000, // 10M gas units
            max_execution_time_ms: 30_000,       // 30 seconds
            max_memory_usage: 128 * 1024 * 1024, // 128 MB
            max_storage_operations: 10_000,
            enable_tracing: false,
            module_cache_size: 1024,             // 1K modules
        }
    }
}
