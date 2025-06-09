use crate::block::{self, Block};
use bincode;
use consensus_pos::Blake3Algorithm;
// Replace with common import
use common::get_kari_dir;
use log::{info, warn};
use mona_storage::{BlockchainStorage, RocksDBStorage, StorageError};
use mona_types::address::Address;
use mona_vm::types::FunctionCall;
use serde::{Deserialize, Serialize};

use lazy_static::lazy_static;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, RwLock, atomic::{AtomicU64, Ordering}};

// Define improved thread-safe blockchain globals
lazy_static! {
    pub static ref BLOCKCHAIN_DATA: BlockchainData = BlockchainData::new();
    pub static ref BALANCES: Mutex<HashMap<String, u64>> = Mutex::new(HashMap::new());
    pub static ref PENDING_TRANSACTIONS: Mutex<VecDeque<block::Transaction>> = Mutex::new(VecDeque::new());
}

/// Improved blockchain data container with thread-safety and performance features
pub struct BlockchainData {
    chain: RwLock<VecDeque<Block<Blake3Algorithm>>>,
    total_tokens: AtomicU64,
    // Can be extended with cache and other performance features
    block_height_cache: RwLock<HashMap<String, usize>>, // Hash -> Height mapping
}

impl BlockchainData {
    pub fn new() -> Self {
        BlockchainData {
            chain: RwLock::new(VecDeque::new()),
            total_tokens: AtomicU64::new(0),
            block_height_cache: RwLock::new(HashMap::new()),
        }
    }
    
    pub fn get_total_tokens(&self) -> u64 {
        self.total_tokens.load(Ordering::Relaxed)
    }
    
    pub fn add_tokens(&self, amount: u64) {
        self.total_tokens.fetch_add(amount, Ordering::Relaxed);
    }
    
    pub fn get_block(&self, index: usize) -> Option<Block<Blake3Algorithm>> {
        self.chain.read().unwrap().get(index).cloned()
    }
    
    // Add method to check if a block with given hash exists
    pub fn has_block_with_hash(&self, hash: &str) -> bool {
        self.block_height_cache.read().unwrap().contains_key(hash)
    }
    
    // Add method to get a block by its hash
    pub fn get_block_by_hash(&self, hash: &str) -> Option<Block<Blake3Algorithm>> {
        let cache = self.block_height_cache.read().unwrap();
        if let Some(&height) = cache.get(hash) {
            return self.get_block(height);
        }
        None
    }
    
    // Modified to return bool indicating success
    pub fn add_block(&self, mut block: Block<Blake3Algorithm>) -> bool {
        let mut chain = self.chain.write().unwrap();
        let height = chain.len();
        
        // If block has no transactions, check if there are any pending
        if block.transactions.is_empty() {
            let pending_txs = get_next_block_transactions(100);
            if !pending_txs.is_empty() {
                log::info!("Adding {} pending transactions to block {}", pending_txs.len(), block.index);
                block.transactions = pending_txs;
                
                // Recalculate block hash since we modified it
                block.hash = block.calculate_hash();
            }
        }

        // Check if block already exists
        if self.has_block_with_hash(&block.hash) {
            return false;
        }
        
        // Update token count
        self.total_tokens.fetch_add(block.tokens, Ordering::Relaxed);
        
        // Update cache
        self.block_height_cache.write().unwrap().insert(block.hash.clone(), height);
        
        // Add block to chain
        chain.push_back(block);
        
        true
    }
    
    pub fn len(&self) -> usize {
        self.chain.read().unwrap().len()
    }
    
    pub fn is_empty(&self) -> bool {
        self.chain.read().unwrap().is_empty()
    }
    
    pub fn iter(&self) -> Vec<Block<Blake3Algorithm>> {
        self.chain.read().unwrap().iter().cloned().collect()
    }

    /// Process Move contract transactions in a block
    pub fn process_contract_transactions(
        &self,
        block: &Block<Blake3Algorithm>,
        vm: Option<&mona_vm::MonaVM>,
    ) -> Vec<ContractExecutionResult> {
        let mut results = Vec::new();
        
        for transaction in &block.transactions {
            if transaction.is_contract_transaction() {
                match self.execute_contract_transaction(transaction, vm) {
                    Ok(result) => results.push(result),
                    Err(e) => {
                        warn!("Contract execution failed for tx {}: {}", 
                              transaction.transaction_id, e);
                        
                        // Create failed execution result
                        if let Some(contract_addr) = transaction.get_contract_address() {
                            results.push(ContractExecutionResult {
                                transaction_hash: transaction.transaction_id.clone(),
                                contract_address: contract_addr,
                                gas_used: 0,
                                kari_spent: 0,
                                success: false,
                                error_message: Some(e),
                                events: Vec::new(),
                                storage_changes: HashMap::new(),
                            });
                        }
                    }
                }
            }
        }
        
        info!("Processed {} contract transactions in block {}", 
              results.len(), block.index);
        results
    }

    /// Execute a single contract transaction
    fn execute_contract_transaction(
        &self,
        transaction: &block::Transaction,
        vm: Option<&mona_vm::MonaVM>,
    ) -> Result<ContractExecutionResult, String> {
        let vm = vm.ok_or("VM not available for contract execution")?;
        
        match transaction.transaction_type {
            block::TransactionType::ContractDeploy => {
                self.execute_contract_deployment(transaction, vm)
            },
            block::TransactionType::ContractCall => {
                self.execute_contract_call(transaction, vm)
            },
            _ => Err("Not a contract transaction".to_string()),
        }
    }

    /// Execute contract deployment
    fn execute_contract_deployment(
        &self,
        transaction: &block::Transaction,
        vm: &mona_vm::MonaVM,
    ) -> Result<ContractExecutionResult, String> {
        let contract_data = transaction.contract_data.as_ref()
            .ok_or("Missing contract data for deployment")?;
        
        let bytecode = contract_data.bytecode.as_ref()
            .ok_or("Missing bytecode for deployment")?;

        // Convert bytecode to source code (simplified for now)
        let source_code = String::from_utf8_lossy(bytecode).to_string();
        
        // Deploy contract using VM
        let deployment_info = vm.deploy_contract(
            source_code,
            contract_data.dependencies.clone(),
            transaction.sender,
            contract_data.gas_limit,
        ).map_err(|e| format!("Deployment failed: {:?}", e))?;

        Ok(ContractExecutionResult {
            transaction_hash: transaction.transaction_id.clone(),
            contract_address: deployment_info.contract_address,
            gas_used: deployment_info.gas_used,
            kari_spent: deployment_info.gas_used, // 1:1 ratio
            success: true,
            error_message: None,
            events: Vec::new(), // TODO: Convert VM events
            storage_changes: HashMap::new(),
        })
    }

    /// Execute contract function call
    fn execute_contract_call(
        &self,
        transaction: &block::Transaction,
        vm: &mona_vm::MonaVM,
    ) -> Result<ContractExecutionResult, String> {
        let contract_data = transaction.contract_data.as_ref()
            .ok_or("Missing contract data for call")?;
        
        let contract_address = contract_data.contract_address
            .ok_or("Missing contract address for call")?;
          let function_name = contract_data.function_name.as_ref()
            .ok_or("Missing function name for call")?;

        // Get module name from contract metadata
        let module_name = match vm.get_contract_info(contract_address) {
            Ok(Some(contract_state)) => contract_state.metadata.module_name,
            Ok(None) => return Err("Contract not found".to_string()),
            Err(_) => "default_module".to_string(), // Fallback for compatibility
        };

        // Create FunctionCall struct for VM
        let function_call = FunctionCall {
            contract_address,
            module_name,
            function_name: function_name.clone(),
            caller: transaction.sender,
            args: contract_data.arguments.clone(),
            gas_limit: contract_data.gas_limit,
        };

        // Execute function call using VM
        let result = vm.call_function(function_call)
            .map_err(|e| format!("Function call failed: {:?}", e))?;

        Ok(ContractExecutionResult {
            transaction_hash: transaction.transaction_id.clone(),
            contract_address,
            gas_used: result.gas_used,
            kari_spent: result.gas_used, // 1:1 ratio
            success: result.success,
            error_message: result.error_message,
            events: result.events.into_iter().map(|e| ContractEvent {
                event_type: e.event_type,
                data: e.data,
                contract_address,
            }).collect(),
            storage_changes: HashMap::new(), // TODO: Track storage changes
        })
    }

}

// Helper functions to maintain backward compatibility

pub fn load_blockchain_with_retry() -> Result<(), StorageError> {
    // First attempt - simple load
    let result = load_blockchain();
    if result.is_ok() {
        return result;
    }
    
    // Second attempt after cleanup
    let result = load_blockchain();
    if result.is_ok() {
        return result;
    }
    
    // One more attempt with delay
    std::thread::sleep(std::time::Duration::from_millis(500));
    load_blockchain()
}

// Improved save function that ensures balances are saved
pub fn save_blockchain() -> Result<(), StorageError> {
    let kari_dir = get_kari_dir();
    let db_path = kari_dir.join("blockchain_db");
    let storage = RocksDBStorage::new(db_path)?;

    // Save blockchain data
    let data = bincode::serialize(&BLOCKCHAIN_DATA.chain.read().unwrap().clone())?;
    storage.save_data(b"blockchain", &data)?;

    // Save balances separately for better reliability
    let balances = BALANCES.lock().unwrap().clone();
    let balances_data = bincode::serialize(&balances)?;
    storage.save_data(b"balances", &balances_data)?;
    
    storage.flush()?;
    log::debug!("Blockchain and balances saved successfully");
    
    Ok(())
}

pub fn init_blockchain_state() {
    // BALANCES already initialized by lazy_static
    let balances = BALANCES.lock().unwrap();
    if balances.is_empty() {
        // Add any initial balances here if needed
    }
    
    // No need to initialize BLOCKCHAIN_DATA as it's created by lazy_static
}

// Modified BlockchainError to store StorageError as a string
#[derive(Debug, Serialize, Deserialize)]
pub enum BlockchainError {
    Storage(String), 
    Balance(String),
    Initialization(String),
    Transaction(String),
    InsufficientFunds(String),
    InvalidAddress(String),
    IO(String), // Changed from std::io::Error to String to support serialization
    NotFound(String),
    Network(String),
    
}

impl From<StorageError> for BlockchainError {
    fn from(error: StorageError) -> Self {
        BlockchainError::Storage(format!("{}", error))
    }
}

impl From<std::io::Error> for BlockchainError {
    fn from(error: std::io::Error) -> Self {
        BlockchainError::IO(format!("{}", error)) // Convert io::Error to String
    }
}

impl std::fmt::Display for BlockchainError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            BlockchainError::Storage(e) => write!(f, "Storage error: {}", e),
            BlockchainError::Balance(e) => write!(f, "Balance error: {}", e),
            BlockchainError::Initialization(e) => write!(f, "Initialization error: {}", e),
            BlockchainError::Transaction(e) => write!(f, "Transaction error: {}", e),
            BlockchainError::InsufficientFunds(e) => write!(f, "Insufficient funds: {}", e),
            BlockchainError::InvalidAddress(e) => write!(f, "Invalid address: {}", e),
            BlockchainError::IO(e) => write!(f, "IO error: {}", e),
            BlockchainError::NotFound(e) => write!(f, "Not found error: {}", e),
            BlockchainError::Network(e) => write!(f, "Network error: {}", e),
        }
    }
}

// Helper function to normalize addresses
pub fn normalize_address(address: &str) -> Result<Address, BlockchainError> {
    Address::from_hex_literal(address)
        .map_err(|_| BlockchainError::InvalidAddress(format!("Invalid address format: {}", address)))
}

// Add a function to handle Address directly
pub fn get_hex_from_address(address: &Address) -> String {
    address.to_hex_literal()
}

pub fn get_balance(address: &str) -> Result<u64, BlockchainError> {
    let max_retries = 3;
    let mut attempts = 0;

    // Normalize address format to ensure consistent lookup
    let normalized_address = if !address.starts_with("0x") {
        format!("0x{}", address)
    } else {
        address.to_string()
    };

    // Debug log the address we're checking
    log::debug!("Getting balance for normalized address: {}", normalized_address);

    while attempts < max_retries {
        match BALANCES.lock() {
            Ok(guard) => {
                // Try both with and without 0x prefix to ensure we find the balance
                let balance = guard.get(&normalized_address)
                    .or_else(|| {
                        let no_prefix = normalized_address.trim_start_matches("0x");
                        guard.get(no_prefix)
                    })
                    .unwrap_or(&0);
                
                return Ok(*balance);
            }
            Err(_) => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
        }
    }

    // Add error handling for the case where all attempts fail
    Err(BlockchainError::Balance(
        "Failed to acquire balance lock after multiple attempts".into(),
     ))
}

// Add a new function that accepts Address directly
pub fn get_address_balance(address: &Address) -> Result<u64, BlockchainError> {
    get_balance(&address.to_hex_literal())
}

// Improved submit_transaction function with better logging
pub fn submit_transaction(transaction: block::Transaction) -> Result<(), BlockchainError> {
    // Get transaction type for better logging
    let tx_type = transaction.get_transaction_type();
    
    log::info!(
        "Submitting transaction: {} (type: {}, id: {})",
        tx_type,
        transaction.transaction_id,
        hex::encode(&transaction.transaction_id.as_bytes()[..8])
    );
    
    // Provide detailed VM transaction info if applicable
    if tx_type == "VM_FUNCTION_CALL" {
        if let Some(data) = &transaction.data {
            if let Ok(data_str) = std::str::from_utf8(data) {
                if data_str.starts_with("VM:") {
                    let parts: Vec<&str> = data_str.split(':').collect();
                    if parts.len() >= 3 {
                        log::info!(
                            "VM function call: module={}, function={}", 
                            parts.get(1).unwrap_or(&"unknown"), 
                            parts.get(2).unwrap_or(&"unknown")
                        );
                    }
                }
            }
        }
    }
    
    // Add to pending transaction queue
    let mut transactions = match PENDING_TRANSACTIONS.lock() {
        Ok(t) => t,
        Err(_) => return Err(BlockchainError::Transaction("Failed to lock pending transactions".to_string()))
    };
    
    transactions.push_back(transaction);
    log::info!("Transaction added to pending queue. Queue size: {}", transactions.len());
    
    Ok(())
}

// Enhanced function to prioritize VM function calls
pub fn get_next_block_transactions(max_count: usize) -> Vec<block::Transaction> {
    let mut result = Vec::new();
    
    // Try to get pending transactions
    if let Ok(mut queue) = PENDING_TRANSACTIONS.lock() {
        // Log queue size for debugging
        info!("Processing pending transaction queue, size: {}", queue.len());
        
        // First pass: prioritize modules, then VM function calls
        let mut vm_module_deployments = VecDeque::new();
        let mut vm_function_calls = VecDeque::new();
        let mut regular_txs = VecDeque::new();
        
        // Scan through all transactions to sort by priority
        while let Some(tx) = queue.pop_front() {
            // Check transaction type and prioritize accordingly
            if let Some(data) = &tx.data {
                if let Ok(data_str) = std::str::from_utf8(data) {
                    // Highest priority - module deployments
                    if data_str.starts_with("VM_MODULE:") {
                        info!("Found VM module deployment transaction: {}", tx.transaction_id);
                        vm_module_deployments.push_back(tx);
                        continue;
                    }
                    // Medium priority - VM function calls
                    else if data_str.starts_with("VM:") || data_str.contains("::") {
                        info!("Found VM function call transaction: {}", tx.transaction_id);
                        vm_function_calls.push_back(tx);
                        continue;
                    }
                }
            }
            
            // Lowest priority - regular transactions
            regular_txs.push_back(tx);
        }
        
        // Add VM module deployments first (highest priority)
        while !vm_module_deployments.is_empty() && result.len() < max_count {
            if let Some(tx) = vm_module_deployments.pop_front() {
                info!("Including VM module deployment: {}", tx.transaction_id);
                result.push(tx);
            }
        }
        
        // Add VM function calls next (medium priority)
        while !vm_function_calls.is_empty() && result.len() < max_count {
            if let Some(tx) = vm_function_calls.pop_front() {
                info!("Including VM function call: {}", tx.transaction_id);
                result.push(tx);
            }
        }
        
        // Finally add regular transactions (lowest priority)
        while !regular_txs.is_empty() && result.len() < max_count {
            if let Some(tx) = regular_txs.pop_front() {
                result.push(tx);
            }
        }
        
        // Return any unused transactions back to the queue in order of priority
        for tx in vm_module_deployments {
            queue.push_front(tx); // Add back to front for highest priority
        }
        
        for tx in vm_function_calls {
            queue.push_back(tx); // Medium priority
        }
        
        for tx in regular_txs {
            queue.push_back(tx); // Lowest priority
        }
        
        info!("Selected {} transactions for next block ({} remain in queue)", 
             result.len(), queue.len());
    } else {
        warn!("Failed to lock transaction queue, creating empty block");
    }
    
    result
}

// Make sure PENDING_TRANSACTIONS is properly exposed to be processed
pub fn get_pending_transactions(max_count: usize) -> Vec<block::Transaction> {
    let mut result = Vec::new();
    
    if let Ok(mut queue) = PENDING_TRANSACTIONS.lock() {
        while let Some(tx) = queue.pop_front() {
            result.push(tx);
            if result.len() >= max_count {
                break;
            }
        }
    }
    
    result
}

// Modified load method to ensure balances are properly loaded
pub fn load_blockchain() -> Result<(), StorageError> {
    let kari_dir = get_kari_dir();
    let db_path = kari_dir.join("blockchain_db");
    let storage = RocksDBStorage::new(db_path)?;
    init_blockchain_state();
    
    // First, try to load dedicated balances if they exist
    let mut loaded_balances = HashMap::new();
    if let Ok(Some(balances_data)) = storage.load_data(b"balances") {
        if let Ok(balances) = bincode::deserialize::<HashMap<String, u64>>(&balances_data) {
            loaded_balances = balances;
            log::info!("Loaded {} balances from dedicated storage", loaded_balances.len());
        }
    }

    // Then load blockchain data and calculate balances as a fallback
    match storage.load_data(b"blockchain")? {
        Some(value) => {
            let loaded_chain: VecDeque<Block<Blake3Algorithm>> = bincode::deserialize(&value)?;
            
            // Calculate balances and total tokens if we didn't load from dedicated storage
            let mut balances = if loaded_balances.is_empty() {
                HashMap::new()
            } else {
                loaded_balances.clone()
            };
            
            let mut total_tokens = 0;
            let mut block_height_cache = HashMap::new();
            
            // Update blockchain data
            let mut chain = BLOCKCHAIN_DATA.chain.write().unwrap();
            *chain = loaded_chain;
            
            // Process blocks for balances and caching
            if loaded_balances.is_empty() {
                for (height, block) in chain.iter().enumerate() {
                    total_tokens += block.tokens;
                    
                    // Ensure addresses are always stored with 0x prefix
                    let miner_address = normalize_address(&block.address).unwrap().to_hex_literal();
                    
                    *balances.entry(miner_address).or_insert(0) += block.tokens;
                    block_height_cache.insert(block.hash.clone(), height);

                    for tx in &block.transactions {
                        // Get hex string directly from Address
                        let tx_sender = tx.sender.to_hex_literal();
                        let tx_receiver = tx.receiver.to_hex_literal();
                        
                        *balances.entry(tx_sender).or_insert(0) -= tx.amount;
                        *balances.entry(tx_receiver).or_insert(0) += tx.amount;
                    }
                }
            } else {
                // Just calculate total tokens and build cache
                for (height, block) in chain.iter().enumerate() {
                    total_tokens += block.tokens;
                    block_height_cache.insert(block.hash.clone(), height);
                }
            }

            // Update BLOCKCHAIN_DATA
            BLOCKCHAIN_DATA.total_tokens.store(total_tokens, Ordering::Relaxed);
            *BLOCKCHAIN_DATA.block_height_cache.write().unwrap() = block_height_cache;
            
            // Update balances
            *BALANCES.lock().unwrap() = balances;

            log::info!("Blockchain loaded successfully with {} blocks and {} accounts", 
                chain.len(), BALANCES.lock().unwrap().len());
        }
        None => {
            log::info!("No blockchain data found, initializing new chain");
            *BLOCKCHAIN_DATA.chain.write().unwrap() = VecDeque::new();
            BLOCKCHAIN_DATA.total_tokens.store(0, Ordering::Relaxed);
            *BALANCES.lock().unwrap() = HashMap::new();
        }
    }

    storage.flush()?;
    Ok(())
}

/// Move contract execution result for blockchain processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractExecutionResult {
    pub transaction_hash: String,
    pub contract_address: Address,
    pub gas_used: u64,
    pub kari_spent: u64,
    pub success: bool,
    pub error_message: Option<String>,
    pub events: Vec<ContractEvent>,
    pub storage_changes: HashMap<Vec<u8>, Vec<u8>>,
}

/// Contract event for blockchain storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEvent {
    pub event_type: String,
    pub data: Vec<u8>,
    pub contract_address: Address,
}
