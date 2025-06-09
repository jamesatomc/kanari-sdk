use crate::chain_id::CHAIN_ID;
use consensus_pos::HashAlgorithm;
use serde::{Deserialize, Serialize};
use mona_types::address::Address;
use mona_crypto::verify_signature; // Add mona-crypto dependency
use log;

// Transaction types for different operations
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TransactionType {
    Transfer,
    ContractDeploy,
    ContractCall,
    ContractUpgrade,
}

// Move smart contract transaction data
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ContractTransaction {
    pub contract_address: Option<Address>, // None for deployment
    pub function_name: Option<String>, // None for deployment
    pub arguments: Vec<Vec<u8>>,
    pub bytecode: Option<Vec<u8>>, // For deployment
    pub dependencies: Vec<String>, // Module dependencies
    pub gas_limit: u64,
}

// Define the Transaction struct
#[derive(Serialize, Deserialize, Clone)]
pub struct Transaction {
    pub transaction_id: String, // New field for transaction ID
    pub sender: Address,
    pub receiver: Address,
    pub amount: u64,
    pub gas_fee: u64, // Add gas fee field
    pub timestamp: u64,
    pub signature: Vec<u8>, // Changed from Option<String> to Vec<u8>
    pub data: Option<Vec<u8>>, // Re-add data field as Option<Vec<u8>>
    pub transaction_type: TransactionType, // Transaction type
    pub contract_data: Option<ContractTransaction>, // Smart contract data
    pub kari_gas_used: u64, // Kari amount used for gas
}

// Add methods to the Transaction struct
impl Transaction {    // Create a message representation of the transaction for signing/verification
    pub fn to_signable_message(&self) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(self.transaction_id.as_bytes());
        message.extend_from_slice(self.sender.to_string().as_bytes());
        message.extend_from_slice(self.receiver.to_string().as_bytes());
        message.extend_from_slice(&self.amount.to_le_bytes());
        message.extend_from_slice(&self.gas_fee.to_le_bytes()); // Include gas fee in the signed message
        message.extend_from_slice(&self.timestamp.to_le_bytes());
        message.extend_from_slice(&self.kari_gas_used.to_le_bytes()); // Include Kari gas usage
        
        // Include contract data if present
        if let Some(contract_data) = &self.contract_data {
            if let Some(contract_addr) = &contract_data.contract_address {
                message.extend_from_slice(contract_addr.as_bytes());
            }
            if let Some(function_name) = &contract_data.function_name {
                message.extend_from_slice(function_name.as_bytes());
            }
            message.extend_from_slice(&contract_data.gas_limit.to_le_bytes());
        }
        
        // For debugging
        log::debug!("Generated message for signing/verification: tx_id={}, len={}", 
                   self.transaction_id, message.len());
        
        message
    }

    // Verify the transaction signature
    pub fn verify(&self) -> bool {
        // Check if signature exists
        if self.signature.is_empty() {
            log::warn!("Transaction {} has no signature", self.transaction_id);
            return false;
        }

        // Generate the message that was originally signed
        let message = self.to_signable_message();
        
        // Verify signature using mona-crypto
        match verify_signature(&self.sender.to_string(), &message, &self.signature) {
            Ok(is_valid) => {
                if !is_valid {
                    log::warn!("Invalid signature for transaction {}", self.transaction_id);
                }
                is_valid
            },
            Err(err) => {
                log::error!("Error verifying signature for transaction {}: {}", 
                    self.transaction_id, err);
                false
            }
        }
    }
    
    // Check if the transaction is a VM transaction
    pub fn is_vm_transaction(&self) -> bool {
        if let Some(data) = &self.data {
            if let Ok(data_str) = std::str::from_utf8(data) {
                return data_str.starts_with("VM:") || data_str.contains("::");
            }
        }
        false
    }
    
    // Check if the transaction is a VM module deployment
    pub fn is_vm_module_deployment(&self) -> bool {
        if let Some(data) = &self.data {
            if let Ok(data_str) = std::str::from_utf8(data) {
                return data_str.starts_with("VM_MODULE:");
            }
        }
        false
    }
    
    // Get transaction type as a string for better logging
    pub fn get_transaction_type(&self) -> &'static str {
        if self.is_vm_module_deployment() {
            "VM_MODULE_DEPLOYMENT"
        } else if self.is_vm_transaction() {
            "VM_FUNCTION_CALL"
        } else if self.data.is_some() {
            "DATA_TRANSACTION"
        } else {
            "TOKEN_TRANSFER"
        }
    }

    /// Create a new contract deployment transaction
    pub fn new_contract_deployment(
        transaction_id: String,
        deployer: Address,
        bytecode: Vec<u8>,
        dependencies: Vec<String>,
        gas_limit: u64,
        kari_gas_amount: u64,
        timestamp: u64,
    ) -> Self {
        Self {
            transaction_id,
            sender: deployer,
            receiver: Address::zero(), // No specific receiver for deployment
            amount: 0, // No token transfer in deployment
            gas_fee: 0, // Will be calculated based on gas usage
            timestamp,
            signature: Vec::new(), // Will be set when signing
            data: Some(bytecode.clone()),
            transaction_type: TransactionType::ContractDeploy,
            contract_data: Some(ContractTransaction {
                contract_address: None, // Will be determined after deployment
                function_name: None,
                arguments: Vec::new(),
                bytecode: Some(bytecode),
                dependencies,
                gas_limit,
            }),
            kari_gas_used: kari_gas_amount,
        }
    }

    /// Create a new contract function call transaction
    pub fn new_contract_call(
        transaction_id: String,
        caller: Address,
        contract_address: Address,
        function_name: String,
        arguments: Vec<Vec<u8>>,
        gas_limit: u64,
        kari_gas_amount: u64,
        timestamp: u64,
    ) -> Self {
        Self {
            transaction_id,
            sender: caller,
            receiver: contract_address,
            amount: 0, // No direct token transfer
            gas_fee: 0, // Will be calculated
            timestamp,
            signature: Vec::new(),
            data: None,
            transaction_type: TransactionType::ContractCall,
            contract_data: Some(ContractTransaction {
                contract_address: Some(contract_address),
                function_name: Some(function_name),
                arguments,
                bytecode: None,
                dependencies: Vec::new(),
                gas_limit,
            }),
            kari_gas_used: kari_gas_amount,
        }
    }

    /// Check if this is a Move contract transaction
    pub fn is_contract_transaction(&self) -> bool {
        matches!(self.transaction_type, TransactionType::ContractDeploy | TransactionType::ContractCall | TransactionType::ContractUpgrade)
    }

    /// Get the contract address if this is a contract transaction
    pub fn get_contract_address(&self) -> Option<Address> {
        self.contract_data.as_ref().and_then(|data| data.contract_address)
    }

    /// Get the function name if this is a contract call
    pub fn get_function_name(&self) -> Option<&String> {
        self.contract_data.as_ref().and_then(|data| data.function_name.as_ref())
    }
}

// Define the Block struct
#[derive(Serialize, Deserialize, Clone)]
pub struct Block<T: HashAlgorithm> {
    pub chain_id: String,
    pub index: u32,
    pub timestamp: u64,
    pub data: Vec<u8>,
    pub hash: String,
    pub prev_hash: String,
    pub tokens: u64,
    pub transactions: Vec<Transaction>,
    pub address: String,
    pub hasher: T,
}

// Implement the Block struct
impl<T: HashAlgorithm> Block<T> {

    pub fn new(
        index: u32,
        data: Vec<u8>,
        prev_hash: String,
        tokens: u64,
        transactions: Vec<Transaction>,
        address: String,
        hasher: T,
    ) -> Block<T> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut block = Block {
            chain_id: CHAIN_ID.to_string(),
            index,
            timestamp,
            data,
            hash: String::new(),
            prev_hash,
            tokens,
            transactions,
            address,
            hasher,
        };
        block.hash = block.calculate_hash();
        block
    }

    // Add a method to calculate the hash of the block
    pub fn calculate_hash(&self) -> String {
        let mut input = Vec::new();
        input.extend_from_slice(self.chain_id.as_bytes());
        input.extend_from_slice(&self.index.to_le_bytes());
        input.extend_from_slice(&self.timestamp.to_le_bytes());
        input.extend_from_slice(&self.data);
        input.extend_from_slice(self.prev_hash.as_bytes());
        input.extend_from_slice(&self.tokens.to_le_bytes());

        // Serialize transactions
        let transactions_serialized = serde_json::to_string(&self.transactions).unwrap();
        input.extend_from_slice(transactions_serialized.as_bytes());

        // self.hasher.log_input(&input);
        self.hasher.hash(&input)
    }

    // Add a method to verify the block
    pub fn verify(&self, prev_block: &Block<T>) -> bool {
        if self.index != prev_block.index + 1 {
            return false;
        }
        if self.prev_hash != prev_block.hash {
            return false;
        }

        let calculated_hash = self.calculate_hash();
        if self.hash != calculated_hash {
            return false;
        }
        true
    }
}

