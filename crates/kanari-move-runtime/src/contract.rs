use anyhow::Result;
use move_core_types::{
    account_address::AccountAddress,
    identifier::Identifier,
    language_storage::{ModuleId, TypeTag},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Contract deployment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractInfo {
    /// Contract address (module publisher)
    pub address: String,

    /// Module name
    pub module_name: String,

    /// Module bytecode
    pub bytecode: Vec<u8>,

    /// Deployment transaction hash
    pub deployment_tx: Vec<u8>,

    /// Block height when deployed
    pub deployed_at: u64,

    /// ABI (function signatures)
    pub abi: ContractABI,

    /// Contract metadata
    pub metadata: ContractMetadata,
}

/// Contract ABI (Application Binary Interface)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractABI {
    /// Public functions
    pub functions: Vec<FunctionSignature>,

    /// Public structs
    pub structs: Vec<StructSignature>,
}

impl ContractABI {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            structs: Vec::new(),
        }
    }

    /// Add a function to the ABI
    pub fn add_function(&mut self, func: FunctionSignature) {
        self.functions.push(func);
    }

    /// Find function by name
    pub fn get_function(&self, name: &str) -> Option<&FunctionSignature> {
        self.functions.iter().find(|f| f.name == name)
    }

    /// List all function names
    pub fn list_functions(&self) -> Vec<String> {
        self.functions.iter().map(|f| f.name.clone()).collect()
    }
}

impl Default for ContractABI {
    fn default() -> Self {
        Self::new()
    }
}

/// Function signature in ABI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSignature {
    /// Function name
    pub name: String,

    /// Is entry function (can be called externally)
    pub is_entry: bool,

    /// Type parameters
    pub type_params: Vec<String>,

    /// Parameters
    pub parameters: Vec<ParameterInfo>,

    /// Return types
    pub returns: Vec<String>,

    /// Function documentation
    pub doc: Option<String>,
}

/// Parameter information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterInfo {
    pub name: String,
    pub type_name: String,
}

/// Struct signature in ABI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructSignature {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    pub abilities: Vec<String>,
}

/// Field information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInfo {
    pub name: String,
    pub type_name: String,
}

/// Contract metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractMetadata {
    /// Contract name (human-readable)
    pub name: String,

    /// Contract version
    pub version: String,

    /// Author/Publisher
    pub author: String,

    /// Description
    pub description: String,

    /// Source code URL (optional)
    pub source_url: Option<String>,

    /// License
    pub license: Option<String>,

    /// Tags/Categories
    pub tags: Vec<String>,
}

impl ContractMetadata {
    pub fn new(name: String, version: String, author: String) -> Self {
        Self {
            name,
            version,
            author,
            description: String::new(),
            source_url: None,
            license: None,
            tags: Vec::new(),
        }
    }

    pub fn with_description(mut self, desc: String) -> Self {
        self.description = desc;
        self
    }

    pub fn with_source(mut self, url: String) -> Self {
        self.source_url = Some(url);
        self
    }

    pub fn with_license(mut self, license: String) -> Self {
        self.license = Some(license);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Contract registry for tracking deployed contracts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractRegistry {
    /// Map: (address, module_name) -> ContractInfo
    contracts: HashMap<(String, String), ContractInfo>,

    /// Map: address -> list of module names
    address_modules: HashMap<String, Vec<String>>,
}

impl ContractRegistry {
    pub fn new() -> Self {
        Self {
            contracts: HashMap::new(),
            address_modules: HashMap::new(),
        }
    }

    /// Register a deployed contract
    pub fn register(&mut self, contract: ContractInfo) {
        let key = (contract.address.clone(), contract.module_name.clone());

        // Add to address_modules index
        self.address_modules
            .entry(contract.address.clone())
            .or_insert_with(Vec::new)
            .push(contract.module_name.clone());

        // Add to contracts
        self.contracts.insert(key, contract);
    }

    /// Get contract info
    pub fn get_contract(&self, address: &str, module_name: &str) -> Option<&ContractInfo> {
        self.contracts
            .get(&(address.to_string(), module_name.to_string()))
    }

    /// List all contracts by address
    pub fn get_contracts_by_address(&self, address: &str) -> Vec<&ContractInfo> {
        if let Some(modules) = self.address_modules.get(address) {
            modules
                .iter()
                .filter_map(|module| self.get_contract(address, module))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// List all deployed contracts
    pub fn list_all(&self) -> Vec<&ContractInfo> {
        self.contracts.values().collect()
    }

    /// Search contracts by tag
    pub fn search_by_tag(&self, tag: &str) -> Vec<&ContractInfo> {
        self.contracts
            .values()
            .filter(|c| c.metadata.tags.contains(&tag.to_string()))
            .collect()
    }

    /// Get total number of contracts
    pub fn count(&self) -> usize {
        self.contracts.len()
    }
}

impl Default for ContractRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Contract interaction builder
pub struct ContractCall {
    /// Target module
    pub module_id: ModuleId,

    /// Function to call
    pub function: String,

    /// Type arguments
    pub type_args: Vec<TypeTag>,

    /// Function arguments (BCS-encoded)
    pub args: Vec<Vec<u8>>,

    /// Sender address
    pub sender: AccountAddress,

    /// Gas configuration
    pub gas_limit: u64,
    pub gas_price: u64,
}

impl ContractCall {
    /// Create a new contract call
    pub fn new(address: &str, module: &str, function: &str, sender: &str) -> Result<Self> {
        let module_addr = AccountAddress::from_hex_literal(address)?;
        let module_name = Identifier::new(module)?;
        let module_id = ModuleId::new(module_addr, module_name);
        let sender_addr = AccountAddress::from_hex_literal(sender)?;

        Ok(Self {
            module_id,
            function: function.to_string(),
            type_args: Vec::new(),
            args: Vec::new(),
            sender: sender_addr,
            gas_limit: 100_000,
            gas_price: 1000,
        })
    }

    /// Add type argument
    pub fn with_type_arg(mut self, type_tag: TypeTag) -> Self {
        self.type_args.push(type_tag);
        self
    }

    /// Add argument (BCS-encoded)
    pub fn with_arg(mut self, arg: Vec<u8>) -> Self {
        self.args.push(arg);
        self
    }

    /// Set gas limit
    pub fn with_gas_limit(mut self, limit: u64) -> Self {
        self.gas_limit = limit;
        self
    }

    /// Set gas price
    pub fn with_gas_price(mut self, price: u64) -> Self {
        self.gas_price = price;
        self
    }

    /// Get module address as string
    pub fn module_address(&self) -> String {
        format!("0x{}", hex::encode(self.module_id.address().to_vec()))
    }

    /// Get module name
    pub fn module_name(&self) -> String {
        self.module_id.name().to_string()
    }
}

/// Contract deployment builder
pub struct ContractDeployment {
    /// Module bytecode
    pub bytecode: Vec<u8>,

    /// Module name
    pub module_name: String,

    /// Publisher address
    pub publisher: AccountAddress,

    /// Contract metadata
    pub metadata: ContractMetadata,

    /// Gas configuration
    pub gas_limit: u64,
    pub gas_price: u64,
}

impl ContractDeployment {
    /// Create new contract deployment
    pub fn new(
        bytecode: Vec<u8>,
        module_name: String,
        publisher: &str,
        metadata: ContractMetadata,
    ) -> Result<Self> {
        let publisher_addr = AccountAddress::from_hex_literal(publisher)?;

        Ok(Self {
            bytecode,
            module_name,
            publisher: publisher_addr,
            metadata,
            gas_limit: 500_000, // Higher default for module publishing
            gas_price: 1000,
        })
    }

    /// Set gas limit
    pub fn with_gas_limit(mut self, limit: u64) -> Self {
        self.gas_limit = limit;
        self
    }

    /// Set gas price
    pub fn with_gas_price(mut self, price: u64) -> Self {
        self.gas_price = price;
        self
    }

    /// Get publisher address as string
    pub fn publisher_address(&self) -> String {
        format!("0x{}", hex::encode(self.publisher.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contract_abi() {
        let mut abi = ContractABI::new();

        let func = FunctionSignature {
            name: kanari_types::transfer::TransferModule::function_names()
                .public_transfer
                .to_string(),
            is_entry: true,
            type_params: vec![],
            parameters: vec![
                ParameterInfo {
                    name: "to".to_string(),
                    type_name: "address".to_string(),
                },
                ParameterInfo {
                    name: "amount".to_string(),
                    type_name: "u64".to_string(),
                },
            ],
            returns: vec![],
            doc: Some("Transfer tokens to recipient".to_string()),
        };

        abi.add_function(func);

        assert_eq!(abi.functions.len(), 1);
        let transfer_fn = kanari_types::transfer::TransferModule::function_names().public_transfer;
        assert!(abi.get_function(transfer_fn).is_some());
        assert_eq!(abi.list_functions(), vec![transfer_fn.to_string()]);
    }

    #[test]
    fn test_contract_registry() {
        let mut registry = ContractRegistry::new();

        let contract = ContractInfo {
            address: "0x1".to_string(),
            module_name: kanari_types::coin::CoinModule::COIN_MODULE.to_string(),
            bytecode: vec![1, 2, 3],
            deployment_tx: vec![4, 5, 6],
            deployed_at: 100,
            abi: ContractABI::new(),
            metadata: ContractMetadata::new(
                "Coin".to_string(),
                "1.0.0".to_string(),
                "0x1".to_string(),
            ),
        };

        registry.register(contract);

        assert_eq!(registry.count(), 1);
        assert!(
            registry
                .get_contract("0x1", kanari_types::coin::CoinModule::COIN_MODULE)
                .is_some()
        );
        assert_eq!(registry.get_contracts_by_address("0x1").len(), 1);
    }

    #[test]
    fn test_contract_metadata() {
        let metadata = ContractMetadata::new(
            "MyToken".to_string(),
            "1.0.0".to_string(),
            "0x123".to_string(),
        )
        .with_description("A test token".to_string())
        .with_license("MIT".to_string())
        .with_tags(vec!["token".to_string(), "defi".to_string()]);

        assert_eq!(metadata.name, "MyToken");
        assert_eq!(metadata.version, "1.0.0");
        assert_eq!(metadata.description, "A test token");
        assert_eq!(metadata.license, Some("MIT".to_string()));
        assert_eq!(metadata.tags.len(), 2);
    }

    #[test]
    fn test_contract_call_builder() -> Result<()> {
        let call = ContractCall::new(
            "0x1",
            kanari_types::coin::CoinModule::COIN_MODULE,
            kanari_types::transfer::TransferModule::function_names().public_transfer,
            "0x2",
        )?
        .with_gas_limit(200_000)
        .with_gas_price(2000);

        assert_eq!(
            call.function,
            kanari_types::transfer::TransferModule::function_names()
                .public_transfer
                .to_string()
        );
        assert_eq!(call.gas_limit, 200_000);
        assert_eq!(call.gas_price, 2000);
        assert_eq!(
            call.module_name(),
            kanari_types::coin::CoinModule::COIN_MODULE.to_string()
        );

        Ok(())
    }

    #[test]
    fn test_contract_deployment_builder() -> Result<()> {
        let metadata = ContractMetadata::new(
            "TestModule".to_string(),
            "1.0.0".to_string(),
            "0x1".to_string(),
        );

        let deployment =
            ContractDeployment::new(vec![1, 2, 3, 4], "test_module".to_string(), "0x1", metadata)?
                .with_gas_limit(1_000_000);

        assert_eq!(deployment.module_name, "test_module");
        assert_eq!(deployment.gas_limit, 1_000_000);
        assert_eq!(deployment.bytecode.len(), 4);

        Ok(())
    }
}
