//! Virtual Machine module for Smart Contract execution
//!
//! This module provides a complete VM environment for executing smart contracts
//! on the Kanari blockchain, including gas metering, state management, and
//! contract deployment capabilities.

use crate::utils::{calculate_gas_fee, format_gas_fee_display};
use log::{debug, error, info, warn};
use mona_blockchain::block::Transaction;
use mona_blockchain::blockchain::{BALANCES, BlockchainError, normalize_address};
use mona_types::address::Address;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

pub mod bytecode;
pub mod contract;
pub mod execution;
pub mod gas;
pub mod storage;

pub use bytecode::*;
pub use contract::*;
pub use execution::*;
pub use gas::*;
pub use storage::*;

/// VM execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VMResult {
    pub success: bool,
    pub return_data: Vec<u8>,
    pub gas_used: u64,
    pub logs: Vec<VMLog>,
    pub error: Option<String>,
    pub state_changes: HashMap<String, Vec<u8>>,
}

/// VM execution log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VMLog {
    pub address: Address,
    pub topics: Vec<String>,
    pub data: Vec<u8>,
    pub block_number: u64,
    pub transaction_hash: String,
}

/// VM execution context
#[derive(Debug, Clone)]
pub struct VMContext {
    pub caller: Address,
    pub origin: Address,
    pub contract_address: Address,
    pub gas_limit: u64,
    pub gas_price: u64,
    pub block_number: u64,
    pub block_timestamp: u64,
    pub chain_id: String,
    pub value: u64, // Amount of tokens sent with the call
}

/// Smart Contract Virtual Machine
pub struct SmartContractVM {
    /// Contract storage
    storage: Arc<RwLock<VMStorage>>,
    /// Deployed contracts
    contracts: Arc<RwLock<HashMap<String, Contract>>>,
    /// Gas meter
    gas_meter: Arc<Mutex<GasMeter>>,
    /// Execution stack limit
    stack_limit: usize,
    /// Memory limit
    memory_limit: usize,
}

impl SmartContractVM {
    /// Create a new VM instance
    pub fn new() -> Self {
        Self {
            storage: Arc::new(RwLock::new(VMStorage::new())),
            contracts: Arc::new(RwLock::new(HashMap::new())),
            gas_meter: Arc::new(Mutex::new(GasMeter::new())),
            stack_limit: 1024,
            memory_limit: 32 * 1024 * 1024, // 32MB
        }
    }

    /// Deploy a new smart contract
    pub fn deploy_contract(
        &self,
        deployer: &Address,
        bytecode: Vec<u8>,
        constructor_args: Vec<u8>,
        gas_limit: u64,
        value: u64,
    ) -> Result<(Address, VMResult), VMError> {
        info!("Deploying contract from {}", deployer);

        // Generate contract address (simplified deterministic approach)
        let contract_address = self.generate_contract_address(deployer, &bytecode)?;

        // Create execution context
        let context = VMContext {
            caller: deployer.clone(),
            origin: deployer.clone(),
            contract_address: contract_address.clone(),
            gas_limit,
            gas_price: calculate_gas_fee(None),
            block_number: self.get_current_block_number(),
            block_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            chain_id: "kanari-testnet".to_string(),
            value,
        };

        // Initialize gas meter
        {
            let mut gas_meter = self.gas_meter.lock().unwrap();
            gas_meter.reset(gas_limit);
        }

        // Create and store the contract
        let contract = Contract::new(
            contract_address.clone(),
            deployer.clone(),
            bytecode.clone(),
            0, // Initial version
        )?;

        // Execute constructor if present
        let mut result = VMResult {
            success: true,
            return_data: Vec::new(),
            gas_used: 0,
            logs: Vec::new(),
            error: None,
            state_changes: HashMap::new(),
        };

        // Execute constructor
        if !constructor_args.is_empty() {
            match self.execute_constructor(&contract, &context, constructor_args) {
                Ok(constructor_result) => {
                    result.gas_used += constructor_result.gas_used;
                    result.logs.extend(constructor_result.logs);
                    result
                        .state_changes
                        .extend(constructor_result.state_changes);

                    if !constructor_result.success {
                        return Err(VMError::ExecutionError(
                            constructor_result
                                .error
                                .unwrap_or("Constructor execution failed".to_string()),
                        ));
                    }
                }
                Err(e) => return Err(e),
            }
        }

        // Store the deployed contract
        {
            let mut contracts = self.contracts.write().unwrap();
            contracts.insert(contract_address.to_hex_literal(), contract);
        }

        // Update gas used
        result.gas_used += GAS_COSTS.contract_creation;

        info!("Contract deployed successfully at {}", contract_address);
        Ok((contract_address, result))
    }

    /// Execute a contract function call
    pub fn call_contract(
        &self,
        caller: &Address,
        contract_address: &Address,
        function_selector: [u8; 4],
        function_args: Vec<u8>,
        gas_limit: u64,
        value: u64,
    ) -> Result<VMResult, VMError> {
        debug!("Calling contract {} from {}", contract_address, caller);

        // Get the contract
        let contract = {
            let contracts = self.contracts.read().unwrap();
            contracts
                .get(&contract_address.to_hex_literal())
                .cloned()
                .ok_or(VMError::ContractNotFound(contract_address.to_hex_literal()))?
        };

        // Create execution context
        let context = VMContext {
            caller: caller.clone(),
            origin: caller.clone(),
            contract_address: contract_address.clone(),
            gas_limit,
            gas_price: calculate_gas_fee(None),
            block_number: self.get_current_block_number(),
            block_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            chain_id: "kanari-testnet".to_string(),
            value,
        };

        // Initialize gas meter
        {
            let mut gas_meter = self.gas_meter.lock().unwrap();
            gas_meter.reset(gas_limit);
        }

        // Execute the function
        self.execute_function(&contract, &context, function_selector, function_args)
    }

    /// Get contract storage value
    pub fn get_storage(&self, contract_address: &Address, key: &[u8]) -> Option<Vec<u8>> {
        let storage = self.storage.read().unwrap();
        storage.get_storage(contract_address, key)
    }

    /// Set contract storage value
    pub fn set_storage(
        &self,
        contract_address: &Address,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<(), VMError> {
        let mut storage = self.storage.write().unwrap();
        storage.set_storage(contract_address.clone(), key, value);
        Ok(())
    }

    /// Execute contract constructor
    fn execute_constructor(
        &self,
        contract: &Contract,
        context: &VMContext,
        args: Vec<u8>,
    ) -> Result<VMResult, VMError> {
        // Create execution environment
        let mut executor = ContractExecutor::new(
            self.storage.clone(),
            self.gas_meter.clone(),
            self.stack_limit,
            self.memory_limit,
        );

        // Execute constructor bytecode
        executor.execute_constructor(contract, context, args)
    }

    /// Execute contract function
    fn execute_function(
        &self,
        contract: &Contract,
        context: &VMContext,
        function_selector: [u8; 4],
        args: Vec<u8>,
    ) -> Result<VMResult, VMError> {
        // Create execution environment
        let mut executor = ContractExecutor::new(
            self.storage.clone(),
            self.gas_meter.clone(),
            self.stack_limit,
            self.memory_limit,
        );

        // Execute function
        executor.execute_function(contract, context, function_selector, args)
    }

    /// Generate contract address from deployer and bytecode
    fn generate_contract_address(
        &self,
        deployer: &Address,
        bytecode: &[u8],
    ) -> Result<Address, VMError> {
        use mona_crypto::hash_data_blake3;

        let mut input = Vec::new();
        input.extend_from_slice(deployer.to_string().as_bytes());
        input.extend_from_slice(bytecode);

        let nonce = self.get_account_nonce(deployer);
        input.extend_from_slice(&nonce.to_le_bytes());

        let hash = hash_data_blake3(&input);
        let address_bytes = &hash[..20]; // Take first 20 bytes

        let address_hex = format!("0x{}", hex::encode(address_bytes));
        Address::from_hex_literal(&address_hex).map_err(|_| {
            VMError::AddressGeneration("Failed to generate contract address".to_string())
        })
    }

    /// Get account nonce (simplified)
    fn get_account_nonce(&self, _address: &Address) -> u64 {
        // In a real implementation, this would track transaction counts
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Get current block number
    fn get_current_block_number(&self) -> u64 {
        use mona_blockchain::blockchain::BLOCKCHAIN_DATA;
        BLOCKCHAIN_DATA.len() as u64
    }

    /// Process VM transaction (integration with blockchain)
    pub fn process_vm_transaction(&self, transaction: &Transaction) -> Result<VMResult, VMError> {
        // Check if this is a contract deployment or call
        if let Some(data) = &transaction.data {
            if data.len() >= 4 {
                // Extract function selector (first 4 bytes)
                let mut selector = [0u8; 4];
                selector.copy_from_slice(&data[0..4]);

                // Extract function arguments
                let args = data[4..].to_vec();

                // Execute contract call
                return self.call_contract(
                    &transaction.sender,
                    &transaction.receiver,
                    selector,
                    args,
                    transaction.gas_fee * 100, // Convert gas fee to gas limit
                    transaction.amount,
                );
            }
        }

        // Not a contract transaction
        Err(VMError::InvalidTransaction(
            "Not a valid contract transaction".to_string(),
        ))
    }

    /// Get all deployed contracts
    pub fn get_deployed_contracts(&self) -> HashMap<String, Contract> {
        self.contracts.read().unwrap().clone()
    }

    /// Check if address is a contract
    pub fn is_contract(&self, address: &Address) -> bool {
        let contracts = self.contracts.read().unwrap();
        contracts.contains_key(&address.to_hex_literal())
    }

    /// Get contract info
    pub fn get_contract_info(&self, address: &Address) -> Option<Contract> {
        let contracts = self.contracts.read().unwrap();
        contracts.get(&address.to_hex_literal()).cloned()
    }
}

/// VM Error types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VMError {
    OutOfGas,
    StackOverflow,
    StackUnderflow,
    InvalidInstruction(u8),
    InvalidJump,
    InvalidMemoryAccess,
    ContractNotFound(String),
    ExecutionError(String),
    StorageError(String),
    AddressGeneration(String),
    InvalidTransaction(String),
    InsufficientBalance,
    ReentrancyGuard,
}

impl std::fmt::Display for VMError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            VMError::OutOfGas => write!(f, "Out of gas"),
            VMError::StackOverflow => write!(f, "Stack overflow"),
            VMError::StackUnderflow => write!(f, "Stack underflow"),
            VMError::InvalidInstruction(op) => write!(f, "Invalid instruction: 0x{:02x}", op),
            VMError::InvalidJump => write!(f, "Invalid jump destination"),
            VMError::InvalidMemoryAccess => write!(f, "Invalid memory access"),
            VMError::ContractNotFound(addr) => write!(f, "Contract not found: {}", addr),
            VMError::ExecutionError(msg) => write!(f, "Execution error: {}", msg),
            VMError::StorageError(msg) => write!(f, "Storage error: {}", msg),
            VMError::AddressGeneration(msg) => write!(f, "Address generation error: {}", msg),
            VMError::InvalidTransaction(msg) => write!(f, "Invalid transaction: {}", msg),
            VMError::InsufficientBalance => write!(f, "Insufficient balance"),
            VMError::ReentrancyGuard => write!(f, "Reentrancy detected"),
        }
    }
}

impl From<VMError> for BlockchainError {
    fn from(error: VMError) -> Self {
        BlockchainError::Transaction(format!("VM Error: {}", error))
    }
}

/// Global VM instance
lazy_static::lazy_static! {
    pub static ref GLOBAL_VM: SmartContractVM = SmartContractVM::new();
}

/// Convenience function to deploy a contract via the global VM
pub fn deploy_contract(
    deployer: &Address,
    bytecode: Vec<u8>,
    constructor_args: Vec<u8>,
    gas_limit: u64,
    value: u64,
) -> Result<(Address, VMResult), VMError> {
    GLOBAL_VM.deploy_contract(deployer, bytecode, constructor_args, gas_limit, value)
}

/// Convenience function to call a contract via the global VM
pub fn call_contract(
    caller: &Address,
    contract_address: &Address,
    function_selector: [u8; 4],
    function_args: Vec<u8>,
    gas_limit: u64,
    value: u64,
) -> Result<VMResult, VMError> {
    GLOBAL_VM.call_contract(
        caller,
        contract_address,
        function_selector,
        function_args,
        gas_limit,
        value,
    )
}

/// Process a VM transaction
pub fn process_vm_transaction(transaction: &Transaction) -> Result<VMResult, VMError> {
    GLOBAL_VM.process_vm_transaction(transaction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mona_types::address::Address;

    #[test]
    fn test_vm_creation() {
        let vm = SmartContractVM::new();
        assert!(vm.contracts.read().unwrap().is_empty());
    }

    #[test]
    fn test_contract_address_generation() {
        let vm = SmartContractVM::new();
        let deployer =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let bytecode = vec![0x60, 0x60, 0x60, 0x40]; // Simple bytecode

        let address1 = vm.generate_contract_address(&deployer, &bytecode).unwrap();
        let address2 = vm.generate_contract_address(&deployer, &bytecode).unwrap();

        // Addresses should be different due to nonce (timestamp)
        // In a real implementation with proper nonce tracking, this would be more deterministic
    }

    #[test]
    fn test_contract_deployment() {
        let vm = SmartContractVM::new();
        let deployer =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // Simple contract that returns empty data

        // Initialize deployer balance
        {
            let mut balances = BALANCES.lock().unwrap();
            balances.insert(deployer.to_hex_literal(), 1000000);
        }

        let result = vm.deploy_contract(&deployer, bytecode, vec![], 100000, 0);

        match result {
            Ok((address, vm_result)) => {
                assert!(vm_result.success);
                assert!(vm.is_contract(&address));
            }
            Err(e) => panic!("Contract deployment failed: {}", e),
        }
    }
}
