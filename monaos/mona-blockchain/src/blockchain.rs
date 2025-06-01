use crate::block::{self, Block};
use bincode;
use consensus_pos::Blake3Algorithm;
// Replace with common import
use common::get_kari_dir;
use log::{info, warn};
use mona_storage::{BlockchainStorage, RocksDBStorage, StorageError};
use mona_types::address::Address;
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
        // Validate block first
        if block.hash.is_empty() {
            log::error!("Cannot add block with empty hash");
            return false;
        }

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
            log::warn!("Block with hash {} already exists", block.hash);
            return false;
        }
        
        // Use saturating_add to prevent overflow
        self.total_tokens.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_add(block.tokens))
        }).unwrap();
        
        {
            let mut cache = self.block_height_cache.write().unwrap();
            cache.insert(block.hash.clone(), height);
        }
        
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
    let db_path = kari_dir.join("storage").join("blockchain_db");
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

pub fn save_mvsm() -> Result<(), StorageError> {
    let kari_dir = get_kari_dir();
    let db_path = kari_dir.join("storage").join("mvsm_db");
    let storage = RocksDBStorage::new(db_path)?;
    
    // Save basic system state without accessing VM directly to avoid circular dependency
    let system_metadata = serde_json::json!({
        "status": "system_save",
        "blockchain_height": BLOCKCHAIN_DATA.len(),
        "total_tokens": BLOCKCHAIN_DATA.get_total_tokens(),
        "pending_transactions": {
            "count": match PENDING_TRANSACTIONS.lock() {
                Ok(queue) => queue.len(),
                Err(_) => 0
            }
        },
        "system_info": {
            "version": "2.0",
            "save_timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        },
        "note": "VM state will be saved separately by mona-vm to avoid circular dependencies"
    });
    
    let metadata_bytes = system_metadata.to_string().into_bytes();
    storage.save_data(b"blockchain_metadata", &metadata_bytes)?;
    
    // Save blockchain transaction statistics
    let blockchain_stats = serde_json::json!({
        "total_blocks": BLOCKCHAIN_DATA.len(),
        "total_tokens": BLOCKCHAIN_DATA.get_total_tokens(),
        "account_count": match BALANCES.lock() {
            Ok(balances) => balances.len(),
            Err(_) => 0
        },
        "last_saved": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    });
    
    storage.save_data(b"blockchain_stats", &blockchain_stats.to_string().into_bytes())?;
    
    storage.flush()?;
    log::debug!("Blockchain metadata saved successfully to secure storage");
    
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

    // Validate address format first
    let normalized_address = if address.trim().is_empty() {
        return Err(BlockchainError::InvalidAddress("Empty address provided".to_string()));
    } else if !address.starts_with("0x") {
        format!("0x{}", address)
    } else {
        address.to_string()
    };

    // Validate hex format
    if normalized_address.len() < 3 || !normalized_address[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(BlockchainError::InvalidAddress(format!("Invalid hex address: {}", address)));
    }

    log::debug!("Getting balance for normalized address: {}", normalized_address);

    while attempts < max_retries {
        match BALANCES.lock() {
            Ok(guard) => {
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
    // Validate transaction first - Allow VM transactions with amount = 0
    let tx_type = transaction.get_transaction_type();
    
    if transaction.amount == 0 && 
       tx_type != "VM_FUNCTION_CALL" && 
       tx_type != "VM_MODULE_DEPLOYMENT" && 
       tx_type != "MINING" {
        return Err(BlockchainError::Transaction("Invalid transaction amount".to_string()));
    }

    // Check for sufficient balance for non-mining and non-VM transactions
    if tx_type != "MINING" && tx_type != "VM_MODULE_DEPLOYMENT" && tx_type != "VM_FUNCTION_CALL" {
        let sender_balance = get_balance(&transaction.sender.to_hex_literal())?;
        let total_cost = transaction.amount + transaction.gas_fee;
        if sender_balance < total_cost {
            return Err(BlockchainError::InsufficientFunds(
                format!("Insufficient balance: {} < {} (amount: {} + gas: {})", 
                    sender_balance, total_cost, transaction.amount, transaction.gas_fee)
            ));
        }
    } else if tx_type == "VM_MODULE_DEPLOYMENT" {
        // For VM module deployment, allow even with zero balance but warn
        let sender_balance = get_balance(&transaction.sender.to_hex_literal())?;
        if sender_balance < transaction.gas_fee {
            log::warn!("VM module deployment proceeding with insufficient balance: {} < {}", sender_balance, transaction.gas_fee);
            // Don't return error - allow deployment to proceed
        }
    } else if tx_type == "VM_FUNCTION_CALL" {
        // For VM function calls, only check gas fee
        let sender_balance = get_balance(&transaction.sender.to_hex_literal())?;
        if sender_balance < transaction.gas_fee {
            return Err(BlockchainError::InsufficientFunds(
                format!("Insufficient balance for gas: {} < {}", sender_balance, transaction.gas_fee)
            ));
        }
    }
    
    log::info!(
        "Submitting transaction: {} (type: {}, id: {})",
        tx_type,
        transaction.transaction_id,
        hex::encode(&transaction.transaction_id.as_bytes()[..8.min(transaction.transaction_id.len())])
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
        } else if tx_type == "VM_MODULE_DEPLOYMENT" {
            if let Some((address, module_name)) = transaction.get_vm_module_info() {
                log::info!("VM module deployment: {} at address: {}", module_name, address);
            }
        }
    }
    
    // Add to pending transaction queue
    let mut transactions = match PENDING_TRANSACTIONS.lock() {
        Ok(t) => t,
        Err(_) => return Err(BlockchainError::Transaction("Failed to lock pending transactions".to_string()))
    };
    
    // Check for duplicate transactions
    if transactions.iter().any(|tx| tx.transaction_id == transaction.transaction_id) {
        return Err(BlockchainError::Transaction("Duplicate transaction ID".to_string()));
    }
    
    transactions.push_back(transaction);
    log::info!("Transaction added to pending queue. Queue size: {}", transactions.len());
    
    Ok(())
}

// Enhanced function to prioritize VM function calls and deployments
pub fn get_next_block_transactions(max_count: usize) -> Vec<block::Transaction> {
    let mut result = Vec::new();
    
    if let Ok(mut queue) = PENDING_TRANSACTIONS.lock() {
        info!("Processing pending transaction queue, size: {}", queue.len());
        
        let mut vm_module_deployments = VecDeque::new();
        let mut vm_function_calls = VecDeque::new();
        let mut regular_txs = VecDeque::new();
        
        // Sort transactions by priority
        while let Some(tx) = queue.pop_front() {
            match tx.get_transaction_type() {
                "VM_MODULE_DEPLOYMENT" => {
                    info!("Found VM module deployment transaction: {}", tx.transaction_id);
                    if let Some((address, module_name)) = tx.get_vm_module_info() {
                        info!("  Module: {} at address: {}", module_name, address);
                    }
                    vm_module_deployments.push_back(tx);
                },
                "VM_FUNCTION_CALL" => {
                    info!("Found VM function call transaction: {}", tx.transaction_id);
                    if let Some((module_id, function)) = tx.get_vm_function_info() {
                        info!("  Calling: {}::{}", module_id, function);
                    }
                    vm_function_calls.push_back(tx);
                },
                _ => {
                    regular_txs.push_back(tx);
                }
            }
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
            result.push(regular_txs.pop_front().unwrap());
        }
        
        // Return unused transactions to queue in priority order
        for tx in vm_module_deployments {
            queue.push_front(tx);
        }
        for tx in vm_function_calls {
            queue.push_back(tx);
        }
        for tx in regular_txs {
            queue.push_back(tx);
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
    let db_path = kari_dir.join("storage").join("blockchain_db");
    let storage = RocksDBStorage::new(db_path)?;
    init_blockchain_state();
    
    let mut loaded_balances = HashMap::new();
    if let Ok(Some(balances_data)) = storage.load_data(b"balances") {
        if let Ok(balances) = bincode::deserialize::<HashMap<String, u64>>(&balances_data) {
            loaded_balances = balances;
            log::info!("Loaded {} balances from dedicated storage", loaded_balances.len());
        }
    }

    match storage.load_data(b"blockchain")? {
        Some(value) => {
            let loaded_chain: VecDeque<Block<Blake3Algorithm>> = bincode::deserialize(&value)?;
            
            let mut balances = if loaded_balances.is_empty() {
                HashMap::new()
            } else {
                loaded_balances.clone()
            };
            
            let mut total_tokens = 0u64;
            let mut block_height_cache = HashMap::new();
            
            let mut chain = BLOCKCHAIN_DATA.chain.write().unwrap();
            *chain = loaded_chain;
            
            if loaded_balances.is_empty() {
                for (height, block) in chain.iter().enumerate() {
                    // Prevent overflow
                    total_tokens = total_tokens.saturating_add(block.tokens);
                    
                    let miner_address = match normalize_address(&block.address) {
                        Ok(addr) => addr.to_hex_literal(),
                        Err(_) => {
                            log::warn!("Invalid miner address in block {}: {}", height, block.address);
                            continue;
                        }
                    };
                    
                    let current_balance = balances.entry(miner_address).or_insert(0);
                    *current_balance = current_balance.saturating_add(block.tokens);
                    block_height_cache.insert(block.hash.clone(), height);

                    for tx in &block.transactions {
                        let tx_sender = tx.sender.to_hex_literal();
                        let tx_receiver = tx.receiver.to_hex_literal();
                        
                        // Prevent underflow on sender balance
                        let sender_balance = balances.entry(tx_sender).or_insert(0);
                        *sender_balance = sender_balance.saturating_sub(tx.amount);
                        
                        // Prevent overflow on receiver balance
                        let receiver_balance = balances.entry(tx_receiver).or_insert(0);
                        *receiver_balance = receiver_balance.saturating_add(tx.amount);
                    }
                }
            } else {
                for (height, block) in chain.iter().enumerate() {
                    total_tokens = total_tokens.saturating_add(block.tokens);
                    block_height_cache.insert(block.hash.clone(), height);
                }
            }

            BLOCKCHAIN_DATA.total_tokens.store(total_tokens, Ordering::Relaxed);
            *BLOCKCHAIN_DATA.block_height_cache.write().unwrap() = block_height_cache;
            
            // Use scope to ensure lock is released quickly
            {
                let mut global_balances = BALANCES.lock().unwrap();
                *global_balances = balances;
            }

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
