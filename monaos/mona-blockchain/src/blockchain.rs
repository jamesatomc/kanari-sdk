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
        while let Some(tx) = queue.pop_front() {
            result.push(tx);
            if result.len() >= max_count {
                break;
            }
        }
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
