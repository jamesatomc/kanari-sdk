//! Smart Contract definition and management
//!
//! This module provides the core Contract structure and related functionality
//! for managing smart contracts in the Kanari VM.

use crate::vm::{VMError, VMStorage};
use log::{debug, info, warn};
use mona_crypto::hash_data_blake3;
use mona_types::address::Address;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// Smart Contract representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    /// Contract address
    pub address: Address,
    /// Contract deployer address
    pub deployer: Address,
    /// Contract bytecode
    pub bytecode: Vec<u8>,
    /// Contract runtime bytecode (after constructor execution)
    pub runtime_bytecode: Option<Vec<u8>>,
    /// Contract version
    pub version: u32,
    /// Contract ABI (Application Binary Interface)
    pub abi: Option<ContractABI>,
    /// Contract metadata
    pub metadata: ContractMetadata,
    /// Contract state root hash
    pub state_root: String,
    /// Creation timestamp
    pub created_at: u64,
    /// Last updated timestamp
    pub updated_at: u64,
    /// Contract status
    pub status: ContractStatus,
}

/// Contract Application Binary Interface
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractABI {
    /// Contract functions
    pub functions: HashMap<String, ABIFunction>,
    /// Contract events
    pub events: HashMap<String, ABIEvent>,
    /// Contract constructor
    pub constructor: Option<ABIFunction>,
    /// Contract fallback function
    pub fallback: Option<ABIFunction>,
    /// Contract receive function (for receiving tokens)
    pub receive: Option<ABIFunction>,
}

/// ABI Function definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABIFunction {
    /// Function name
    pub name: String,
    /// Function selector (first 4 bytes of keccak256 hash)
    pub selector: [u8; 4],
    /// Input parameters
    pub inputs: Vec<ABIParameter>,
    /// Output parameters
    pub outputs: Vec<ABIParameter>,
    /// Function mutability
    pub state_mutability: StateMutability,
    /// Function visibility
    pub visibility: Visibility,
    /// Gas estimate
    pub gas_estimate: Option<u64>,
}

/// ABI Event definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABIEvent {
    /// Event name
    pub name: String,
    /// Event signature hash
    pub signature: String,
    /// Event parameters
    pub inputs: Vec<ABIParameter>,
    /// Whether the event is anonymous
    pub anonymous: bool,
}

/// ABI Parameter definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABIParameter {
    /// Parameter name
    pub name: String,
    /// Parameter type
    pub param_type: String,
    /// Whether parameter is indexed (for events)
    pub indexed: bool,
    /// Internal type information
    pub internal_type: Option<String>,
}

/// Function state mutability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StateMutability {
    /// Function reads state but doesn't modify it
    View,
    /// Function doesn't read or modify state
    Pure,
    /// Function can modify state (default)
    Nonpayable,
    /// Function can receive tokens
    Payable,
}

/// Function visibility
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    External,
    Internal,
    Private,
}

/// Contract metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractMetadata {
    /// Contract name
    pub name: Option<String>,
    /// Contract description
    pub description: Option<String>,
    /// Contract author
    pub author: Option<String>,
    /// Contract version string
    pub version_string: Option<String>,
    /// Contract license
    pub license: Option<String>,
    /// Source code hash
    pub source_hash: Option<String>,
    /// Compiler version
    pub compiler_version: Option<String>,
    /// Compilation settings
    pub compilation_settings: HashMap<String, String>,
    /// Custom metadata
    pub custom: HashMap<String, String>,
}

/// Contract status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ContractStatus {
    /// Contract is active and can be called
    Active,
    /// Contract is paused (if it supports pausing)
    Paused,
    /// Contract has been destroyed
    Destroyed,
    /// Contract deployment failed
    Failed,
}

impl Contract {
    /// Create a new contract
    pub fn new(
        address: Address,
        deployer: Address,
        bytecode: Vec<u8>,
        version: u32,
    ) -> Result<Self, VMError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Calculate initial state root
        let state_root = Self::calculate_state_root(&HashMap::new());

        Ok(Contract {
            address,
            deployer,
            bytecode,
            runtime_bytecode: None,
            version,
            abi: None,
            metadata: ContractMetadata::default(),
            state_root,
            created_at: now,
            updated_at: now,
            status: ContractStatus::Active,
        })
    }

    /// Create a contract with full metadata
    pub fn new_with_metadata(
        address: Address,
        deployer: Address,
        bytecode: Vec<u8>,
        version: u32,
        abi: Option<ContractABI>,
        metadata: ContractMetadata,
    ) -> Result<Self, VMError> {
        let mut contract = Self::new(address, deployer, bytecode, version)?;
        contract.abi = abi;
        contract.metadata = metadata;
        Ok(contract)
    }

    /// Set runtime bytecode (after constructor execution)
    pub fn set_runtime_bytecode(&mut self, runtime_bytecode: Vec<u8>) {
        self.runtime_bytecode = Some(runtime_bytecode);
        self.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    /// Get function by selector
    pub fn get_function_by_selector(&self, selector: [u8; 4]) -> Option<&ABIFunction> {
        if let Some(ref abi) = self.abi {
            abi.functions
                .values()
                .find(|func| func.selector == selector)
        } else {
            None
        }
    }

    /// Get function by name
    pub fn get_function_by_name(&self, name: &str) -> Option<&ABIFunction> {
        if let Some(ref abi) = self.abi {
            abi.functions.get(name)
        } else {
            None
        }
    }

    /// Check if contract has a specific function
    pub fn has_function(&self, selector: [u8; 4]) -> bool {
        self.get_function_by_selector(selector).is_some()
    }

    /// Get all function selectors
    pub fn get_function_selectors(&self) -> Vec<[u8; 4]> {
        if let Some(ref abi) = self.abi {
            abi.functions.values().map(|func| func.selector).collect()
        } else {
            Vec::new()
        }
    }

    /// Update contract state root
    pub fn update_state_root(&mut self, state_data: &HashMap<Vec<u8>, Vec<u8>>) {
        self.state_root = Self::calculate_state_root(state_data);
        self.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    /// Calculate state root hash from state data
    fn calculate_state_root(state_data: &HashMap<Vec<u8>, Vec<u8>>) -> String {
        let mut combined_data = Vec::new();

        // Sort keys for deterministic hash
        let mut sorted_keys: Vec<&Vec<u8>> = state_data.keys().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            if let Some(value) = state_data.get(key) {
                combined_data.extend_from_slice(key);
                combined_data.extend_from_slice(value);
            }
        }

        let hash = hash_data_blake3(&combined_data);
        hex::encode(hash)
    }

    /// Set contract status
    pub fn set_status(&mut self, status: ContractStatus) {
        self.status = status;
        self.updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }

    /// Check if contract is active
    pub fn is_active(&self) -> bool {
        self.status == ContractStatus::Active
    }

    /// Get contract size (bytecode length)
    pub fn size(&self) -> usize {
        self.bytecode.len()
    }

    /// Get runtime size
    pub fn runtime_size(&self) -> usize {
        self.runtime_bytecode
            .as_ref()
            .map(|code| code.len())
            .unwrap_or(0)
    }

    /// Verify contract bytecode hash
    pub fn verify_bytecode_hash(&self, expected_hash: &str) -> bool {
        let actual_hash = hex::encode(hash_data_blake3(&self.bytecode));
        actual_hash == expected_hash
    }

    /// Get contract interface summary
    pub fn get_interface_summary(&self) -> ContractInterface {
        let mut interface = ContractInterface {
            address: self.address.clone(),
            functions: Vec::new(),
            events: Vec::new(),
            has_fallback: false,
            has_receive: false,
        };

        if let Some(ref abi) = self.abi {
            interface.functions = abi.functions.keys().cloned().collect();
            interface.events = abi.events.keys().cloned().collect();
            interface.has_fallback = abi.fallback.is_some();
            interface.has_receive = abi.receive.is_some();
        }

        interface
    }

    /// Estimate gas for function call
    pub fn estimate_gas_for_function(&self, function_name: &str) -> Option<u64> {
        self.get_function_by_name(function_name)
            .and_then(|func| func.gas_estimate)
    }
}

/// Contract interface summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractInterface {
    pub address: Address,
    pub functions: Vec<String>,
    pub events: Vec<String>,
    pub has_fallback: bool,
    pub has_receive: bool,
}

impl Default for ContractMetadata {
    fn default() -> Self {
        Self {
            name: None,
            description: None,
            author: None,
            version_string: None,
            license: None,
            source_hash: None,
            compiler_version: None,
            compilation_settings: HashMap::new(),
            custom: HashMap::new(),
        }
    }
}

impl ContractABI {
    /// Create a new empty ABI
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
            events: HashMap::new(),
            constructor: None,
            fallback: None,
            receive: None,
        }
    }

    /// Add a function to the ABI
    pub fn add_function(&mut self, function: ABIFunction) {
        self.functions.insert(function.name.clone(), function);
    }

    /// Add an event to the ABI
    pub fn add_event(&mut self, event: ABIEvent) {
        self.events.insert(event.name.clone(), event);
    }

    /// Get function selector by name
    pub fn get_function_selector(&self, name: &str) -> Option<[u8; 4]> {
        self.functions.get(name).map(|func| func.selector)
    }

    /// Generate function selector from signature
    pub fn generate_function_selector(signature: &str) -> [u8; 4] {
        let hash = hash_data_blake3(signature.as_bytes());
        let mut selector = [0u8; 4];
        selector.copy_from_slice(&hash[0..4]);
        selector
    }
}

impl ABIFunction {
    /// Create a new ABI function
    pub fn new(
        name: String,
        inputs: Vec<ABIParameter>,
        outputs: Vec<ABIParameter>,
        state_mutability: StateMutability,
        visibility: Visibility,
    ) -> Self {
        // Generate function signature
        let input_types: Vec<String> = inputs.iter().map(|p| p.param_type.clone()).collect();
        let signature = format!("{}({})", name, input_types.join(","));
        let selector = ContractABI::generate_function_selector(&signature);

        Self {
            name,
            selector,
            inputs,
            outputs,
            state_mutability,
            visibility,
            gas_estimate: None,
        }
    }

    /// Set gas estimate
    pub fn with_gas_estimate(mut self, gas_estimate: u64) -> Self {
        self.gas_estimate = Some(gas_estimate);
        self
    }

    /// Check if function is payable
    pub fn is_payable(&self) -> bool {
        matches!(self.state_mutability, StateMutability::Payable)
    }

    /// Check if function is view
    pub fn is_view(&self) -> bool {
        matches!(
            self.state_mutability,
            StateMutability::View | StateMutability::Pure
        )
    }

    /// Check if function modifies state
    pub fn modifies_state(&self) -> bool {
        matches!(
            self.state_mutability,
            StateMutability::Nonpayable | StateMutability::Payable
        )
    }
}

impl ABIEvent {
    /// Create a new ABI event
    pub fn new(name: String, inputs: Vec<ABIParameter>, anonymous: bool) -> Self {
        // Generate event signature
        let input_types: Vec<String> = inputs.iter().map(|p| p.param_type.clone()).collect();
        let signature_string = format!("{}({})", name, input_types.join(","));
        let signature = hex::encode(hash_data_blake3(signature_string.as_bytes()));

        Self {
            name,
            signature,
            inputs,
            anonymous,
        }
    }

    /// Get indexed parameters
    pub fn get_indexed_params(&self) -> Vec<&ABIParameter> {
        self.inputs.iter().filter(|p| p.indexed).collect()
    }

    /// Get non-indexed parameters
    pub fn get_data_params(&self) -> Vec<&ABIParameter> {
        self.inputs.iter().filter(|p| !p.indexed).collect()
    }
}

impl ABIParameter {
    /// Create a new ABI parameter
    pub fn new(name: String, param_type: String) -> Self {
        Self {
            name,
            param_type,
            indexed: false,
            internal_type: None,
        }
    }

    /// Create an indexed parameter (for events)
    pub fn indexed(mut self) -> Self {
        self.indexed = true;
        self
    }

    /// Set internal type
    pub fn with_internal_type(mut self, internal_type: String) -> Self {
        self.internal_type = Some(internal_type);
        self
    }
}

/// Contract registry for managing deployed contracts
#[derive(Debug)]
pub struct ContractRegistry {
    contracts: HashMap<String, Contract>,
    by_deployer: HashMap<String, Vec<String>>,
}

impl ContractRegistry {
    /// Create a new contract registry
    pub fn new() -> Self {
        Self {
            contracts: HashMap::new(),
            by_deployer: HashMap::new(),
        }
    }

    /// Register a new contract
    pub fn register(&mut self, contract: Contract) {
        let address = contract.address.to_hex_literal();
        let deployer = contract.deployer.to_hex_literal();

        // Add to main registry
        self.contracts.insert(address.clone(), contract);

        // Add to deployer index
        self.by_deployer
            .entry(deployer)
            .or_insert_with(Vec::new)
            .push(address);
    }

    /// Get contract by address
    pub fn get(&self, address: &str) -> Option<&Contract> {
        self.contracts.get(address)
    }

    /// Get mutable contract by address
    pub fn get_mut(&mut self, address: &str) -> Option<&mut Contract> {
        self.contracts.get_mut(address)
    }

    /// Get contracts deployed by a specific address
    pub fn get_by_deployer(&self, deployer: &str) -> Vec<&Contract> {
        if let Some(addresses) = self.by_deployer.get(deployer) {
            addresses
                .iter()
                .filter_map(|addr| self.contracts.get(addr))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Check if contract exists
    pub fn exists(&self, address: &str) -> bool {
        self.contracts.contains_key(address)
    }

    /// Remove a contract
    pub fn remove(&mut self, address: &str) -> Option<Contract> {
        if let Some(contract) = self.contracts.remove(address) {
            let deployer = contract.deployer.to_hex_literal();
            if let Some(addresses) = self.by_deployer.get_mut(&deployer) {
                addresses.retain(|addr| addr != address);
                if addresses.is_empty() {
                    self.by_deployer.remove(&deployer);
                }
            }
            Some(contract)
        } else {
            None
        }
    }

    /// Get all contract addresses
    pub fn get_all_addresses(&self) -> Vec<String> {
        self.contracts.keys().cloned().collect()
    }

    /// Get contract count
    pub fn count(&self) -> usize {
        self.contracts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_creation() {
        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let deployer =
            Address::from_hex_literal("0xabcdef1234567890abcdef1234567890abcdef12").unwrap();
        let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3];

        let contract = Contract::new(address.clone(), deployer, bytecode, 1).unwrap();

        assert_eq!(contract.address, address);
        assert_eq!(contract.version, 1);
        assert_eq!(contract.status, ContractStatus::Active);
        assert!(contract.is_active());
    }

    #[test]
    fn test_abi_function_creation() {
        let function = ABIFunction::new(
            "transfer".to_string(),
            vec![
                ABIParameter::new("to".to_string(), "address".to_string()),
                ABIParameter::new("amount".to_string(), "uint256".to_string()),
            ],
            vec![ABIParameter::new("success".to_string(), "bool".to_string())],
            StateMutability::Nonpayable,
            Visibility::Public,
        );

        assert_eq!(function.name, "transfer");
        assert_eq!(function.inputs.len(), 2);
        assert_eq!(function.outputs.len(), 1);
        assert!(!function.is_payable());
        assert!(!function.is_view());
        assert!(function.modifies_state());
    }

    #[test]
    fn test_contract_registry() {
        let mut registry = ContractRegistry::new();

        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let deployer =
            Address::from_hex_literal("0xabcdef1234567890abcdef1234567890abcdef12").unwrap();
        let contract =
            Contract::new(address.clone(), deployer.clone(), vec![0x60, 0x00], 1).unwrap();

        registry.register(contract);

        assert!(registry.exists(&address.to_hex_literal()));
        assert_eq!(registry.count(), 1);

        let contracts_by_deployer = registry.get_by_deployer(&deployer.to_hex_literal());
        assert_eq!(contracts_by_deployer.len(), 1);
    }
}
