use bincode;
use consensus_pos::Blake3Algorithm;
use log::{info, warn};
use mona_storage::{BlockchainStorage, RocksDBStorage, StorageError};
use serde::{Deserialize, Serialize};

use lazy_static::lazy_static;
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, RwLock, atomic::{AtomicU64, Ordering}};
use std::path::PathBuf;

use crate::address::Address;
use crate::storage::block::Block;
use crate::{Transaction};

pub mod block;

// Local implementation of get_kari_dir
fn get_kari_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".kari")
}

// Define improved thread-safe blockchain globals
lazy_static! {
    pub static ref BLOCKCHAIN_DATA: BlockchainData = BlockchainData::new();        
    pub static ref BALANCES: Mutex<HashMap<String, u64>> = Mutex::new(HashMap::new());
    pub static ref PENDING_TRANSACTIONS: Mutex<VecDeque<Transaction>> = Mutex::new(VecDeque::new());
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
        self.total_tokens.fetch_add(block.tokens, Ordering::Relaxed);
        
        // Insert into cache before adding to chain
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
    
    // Add method to get latest block without cloning entire chain
    pub fn latest_block(&self) -> Option<Block<Blake3Algorithm>> {
        self.chain.read().unwrap().back().cloned()
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

// Optimized save function with reduced memory allocations
pub fn save_blockchain() -> Result<(), StorageError> {
    let kari_dir = get_kari_dir();
    let db_path = kari_dir.join("storage").join("blockchain_db");
    let storage = RocksDBStorage::new(db_path)?;

    // Save blockchain data - use reference to avoid cloning
    {
        let chain_guard = BLOCKCHAIN_DATA.chain.read().unwrap();
        let data = bincode::serialize(&*chain_guard)?;
        storage.save_data(b"blockchain", &data)?;
    } // Release lock early

    // Save balances separately - use reference to avoid cloning
    {
        let balances_guard = BALANCES.lock().unwrap();
        let balances_data = bincode::serialize(&*balances_guard)?;
        storage.save_data(b"balances", &balances_data)?;
    } // Release lock early
    
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
    // Convert string address to Address struct using the from_hex method or similar
    Address::from_hex(address).map_err(|e| BlockchainError::InvalidAddress(format!("Failed to parse address: {}", e)))
}

// Add a function to handle Address directly
pub fn get_hex_from_address(address: &Address) -> String {
    address.to_string()
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
    get_balance(&address.to_string())
}


/// Enhanced function to prioritize VM function calls with reduced memory usage
pub fn get_next_block_transactions(max_count: usize) -> Vec<Transaction> {
    let mut result = Vec::with_capacity(max_count.min(1000)); // Pre-allocate with reasonable size
    
    // Try to get pending transactions
    if let Ok(mut queue) = PENDING_TRANSACTIONS.lock() {
        let queue_size = queue.len();
        if queue_size == 0 {
            return result;
        }
        
        info!("Processing pending transaction queue, size: {}", queue_size);
        
        // Use iterators to reduce memory allocation - process in-place
        let mut processed = 0;
        let mut remaining_txs = Vec::with_capacity(queue_size);
        
        // Single pass: categorize and select transactions efficiently
        while let Some(tx) = queue.pop_front() {
            if result.len() >= max_count {
                remaining_txs.push(tx);
                continue;
            }
            
            // Priority check with early optimization
            let should_include = if let Some(data) = &tx.data {
                if let Ok(data_str) = std::str::from_utf8(data) {
                    // VM modules get highest priority
                    if data_str.starts_with("VM_MODULE:") {
                        true
                    } 
                    // VM function calls get medium priority
                    else if data_str.starts_with("VM:") || data_str.contains("::") {
                        result.len() < max_count.saturating_sub(max_count / 4) // Reserve space for VM modules
                    }
                    // Regular transactions get lowest priority
                    else {
                        result.len() < max_count.saturating_sub(max_count / 2) // Reserve space for VM operations
                    }
                } else {
                    result.len() < max_count.saturating_sub(max_count / 2)
                }
            } else {
                result.len() < max_count.saturating_sub(max_count / 2)
            };
            
            if should_include {
                result.push(tx);
                processed += 1;
            } else {
                remaining_txs.push(tx);
            }
        }
        
        // Return unused transactions to queue efficiently
        for tx in remaining_txs.into_iter().rev() {
            queue.push_front(tx);
        }
        
        info!("Selected {} transactions for next block ({} processed, {} remain)", 
             result.len(), processed, queue.len());
    } else {
        warn!("Failed to lock transaction queue, creating empty block");
    }
    
    result
}

// Optimized function to get pending transactions with reduced allocations
pub fn get_pending_transactions(max_count: usize) -> Vec<Transaction> {
    let max_count = max_count.min(10000); // Prevent excessive memory allocation
    let mut result = Vec::with_capacity(max_count);
    
    if let Ok(mut queue) = PENDING_TRANSACTIONS.lock() {
        // Use drain to avoid extra allocations
        let to_drain = queue.len().min(max_count);
        result.extend(queue.drain(..to_drain));
    }
    
    result
}

// Optimized load function with better memory management
pub fn load_blockchain() -> Result<(), StorageError> {
    let kari_dir = get_kari_dir();
    let db_path = kari_dir.join("storage").join("blockchain_db");
    let storage = RocksDBStorage::new(db_path)?;
    init_blockchain_state();
    
    // Load balances first to avoid duplicate work
    let loaded_balances = if let Ok(Some(balances_data)) = storage.load_data(b"balances") {
        match bincode::deserialize::<HashMap<String, u64>>(&balances_data) {
            Ok(balances) => {
                log::info!("Loaded {} balances from dedicated storage", balances.len());
                Some(balances)
            }
            Err(e) => {
                log::warn!("Failed to deserialize balances: {}", e);
                None
            }
        }
    } else {
        None
    };

    match storage.load_data(b"blockchain")? {
        Some(value) => {
            let loaded_chain: VecDeque<Block<Blake3Algorithm>> = bincode::deserialize(&value)?;
            let chain_len = loaded_chain.len();
            
            // Pre-allocate collections with known sizes
            let mut balances = loaded_balances.unwrap_or_else(|| HashMap::with_capacity(chain_len));
            let mut total_tokens = 0u64;
            let mut block_height_cache = HashMap::with_capacity(chain_len);
            
            // Process blocks efficiently
            if balances.is_empty() {
                // Only process transactions if we don't have cached balances
                for (height, block) in loaded_chain.iter().enumerate() {
                    total_tokens = total_tokens.saturating_add(block.tokens);
                    block_height_cache.insert(block.hash.clone(), height);
                    
                    // Process miner rewards
                    if let Ok(addr) = normalize_address(&block.address) {
                        let miner_address = addr.to_string();
                        *balances.entry(miner_address).or_insert(0) += block.tokens;
                    }

                    // Process transactions in batch
                    for tx in &block.transactions {
                        let tx_sender = tx.sender.to_string();
                        let tx_receiver = tx.receiver.to_string();
                        
                        // Use entry API to reduce lookups
                        *balances.entry(tx_sender).or_insert(0) = 
                            balances.get(&tx.sender.to_string()).unwrap_or(&0).saturating_sub(tx.amount);
                        *balances.entry(tx_receiver).or_insert(0) += tx.amount;
                    }
                }
            } else {
                // Just build cache since we have balances
                for (height, block) in loaded_chain.iter().enumerate() {
                    total_tokens = total_tokens.saturating_add(block.tokens);
                    block_height_cache.insert(block.hash.clone(), height);
                }
            }

            // Update global state efficiently
            {
                let mut chain = BLOCKCHAIN_DATA.chain.write().unwrap();
                *chain = loaded_chain;
            }
            
            BLOCKCHAIN_DATA.total_tokens.store(total_tokens, Ordering::Relaxed);
            
            {
                let mut cache = BLOCKCHAIN_DATA.block_height_cache.write().unwrap();
                *cache = block_height_cache;
            }
            
            {
                let mut global_balances = BALANCES.lock().unwrap();
                *global_balances = balances;
            }

            log::info!("Blockchain loaded successfully with {} blocks and {} accounts", 
                chain_len, BALANCES.lock().unwrap().len());
        }
        None => {
            log::info!("No blockchain data found, initializing new chain");
            // Initialize empty collections
            *BLOCKCHAIN_DATA.chain.write().unwrap() = VecDeque::new();
            BLOCKCHAIN_DATA.total_tokens.store(0, Ordering::Relaxed);
            *BALANCES.lock().unwrap() = HashMap::new();
            *BLOCKCHAIN_DATA.block_height_cache.write().unwrap() = HashMap::new();
        }
    }

    storage.flush()?;
    Ok(())
}
