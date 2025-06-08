//! Smart contract storage layer for Move VM integration

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Serialize, Deserialize};
use bincode;
use log::{debug, info, warn};

use crate::{BlockchainStorage, StorageError};

/// Smart contract address representation
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SmartContractAddress {
    pub bytes: [u8; 32],
}

impl SmartContractAddress {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    pub fn from_hex(hex_str: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(hex_str.trim_start_matches("0x"))?;
        if bytes.len() != 32 {
            return Err(hex::FromHexError::InvalidStringLength);
        }
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(Self::new(array))
    }

    pub fn to_hex_literal(&self) -> String {
        format!("0x{}", hex::encode(self.bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Smart contract metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartContractMetadata {
    pub name: String,
    pub version: String,
    pub deployed_at: u64, // timestamp
    pub deployer: SmartContractAddress,
    pub bytecode_size: usize,
    pub source_hash: Option<String>,
    pub compiler_version: String,
    pub gas_used_deployment: u64,
}

/// Move module information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveModuleInfo {
    pub module_id: String,
    pub address: SmartContractAddress,
    pub name: String,
    pub friends: Vec<String>,
    pub structs: Vec<String>,
    pub functions: Vec<String>,
    pub constants: Vec<String>,
}

/// Smart contract state snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractStateSnapshot {
    pub address: SmartContractAddress,
    pub storage_items: HashMap<Vec<u8>, Vec<u8>>,
    pub snapshot_time: u64,
    pub gas_used: u64,
}

/// Event log entry for smart contract execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartContractEvent {
    pub contract_address: SmartContractAddress,
    pub event_type: String,
    pub event_data: Vec<u8>,
    pub transaction_hash: String,
    pub block_height: u64,
    pub timestamp: u64,
}

/// Smart contract storage manager
pub struct SmartContractStorage {
    /// Underlying storage
    storage: Arc<dyn BlockchainStorage + Send + Sync>,
    /// Cache for frequently accessed contracts
    contract_cache: Arc<RwLock<HashMap<SmartContractAddress, Vec<u8>>>>,
    /// Cache for contract metadata
    metadata_cache: Arc<RwLock<HashMap<SmartContractAddress, SmartContractMetadata>>>,
    /// Cache size limit
    cache_size_limit: usize,
    /// Current cache size
    current_cache_size: Arc<RwLock<usize>>,
}

impl SmartContractStorage {
    /// Create a new smart contract storage manager
    pub fn new(
        storage: Arc<dyn BlockchainStorage + Send + Sync>,
        cache_size_limit: usize,
    ) -> Self {
        Self {
            storage,
            contract_cache: Arc::new(RwLock::new(HashMap::new())),
            metadata_cache: Arc::new(RwLock::new(HashMap::new())),
            cache_size_limit,
            current_cache_size: Arc::new(RwLock::new(0)),
        }
    }

    /// Store smart contract bytecode
    pub fn store_contract_bytecode(
        &self,
        address: &SmartContractAddress,
        bytecode: &[u8],
    ) -> Result<(), StorageError> {
        let key = self.make_contract_key(address, "bytecode");
        debug!("Storing contract bytecode for {}, size: {} bytes", 
               address.to_hex_literal(), bytecode.len());
        
        self.storage.save_data(key.as_bytes(), bytecode)?;
        
        // Update cache
        self.update_contract_cache(address.clone(), bytecode.to_vec())?;
        
        info!("Successfully stored contract bytecode for {}", address.to_hex_literal());
        Ok(())
    }

    /// Load smart contract bytecode
    pub fn load_contract_bytecode(
        &self,
        address: &SmartContractAddress,
    ) -> Result<Option<Vec<u8>>, StorageError> {
        // Check cache first
        if let Some(bytecode) = self.get_from_contract_cache(address) {
            debug!("Contract bytecode cache hit for {}", address.to_hex_literal());
            return Ok(Some(bytecode));
        }

        let key = self.make_contract_key(address, "bytecode");
        debug!("Loading contract bytecode for {}", address.to_hex_literal());
        
        match self.storage.load_data(key.as_bytes())? {
            Some(bytecode) => {
                debug!("Loaded {} bytes of bytecode for {}", 
                       bytecode.len(), address.to_hex_literal());
                
                // Update cache
                self.update_contract_cache(address.clone(), bytecode.clone())?;
                Ok(Some(bytecode))
            },
            None => {
                debug!("No bytecode found for {}", address.to_hex_literal());
                Ok(None)
            }
        }
    }

    /// Store contract metadata
    pub fn store_contract_metadata(
        &self,
        address: &SmartContractAddress,
        metadata: &SmartContractMetadata,
    ) -> Result<(), StorageError> {
        let key = self.make_contract_key(address, "metadata");
        let serialized = bincode::serialize(metadata)?;
        
        debug!("Storing contract metadata for {}", address.to_hex_literal());
        self.storage.save_data(key.as_bytes(), &serialized)?;
        
        // Update metadata cache
        if let Ok(mut cache) = self.metadata_cache.write() {
            cache.insert(address.clone(), metadata.clone());
        }
        
        info!("Successfully stored contract metadata for {}", address.to_hex_literal());
        Ok(())
    }

    /// Load contract metadata
    pub fn load_contract_metadata(
        &self,
        address: &SmartContractAddress,
    ) -> Result<Option<SmartContractMetadata>, StorageError> {
        // Check cache first
        if let Ok(cache) = self.metadata_cache.read() {
            if let Some(metadata) = cache.get(address) {
                debug!("Contract metadata cache hit for {}", address.to_hex_literal());
                return Ok(Some(metadata.clone()));
            }
        }

        let key = self.make_contract_key(address, "metadata");
        debug!("Loading contract metadata for {}", address.to_hex_literal());
        
        match self.storage.load_data(key.as_bytes())? {
            Some(data) => {
                let metadata: SmartContractMetadata = bincode::deserialize(&data)?;
                
                // Update cache
                if let Ok(mut cache) = self.metadata_cache.write() {
                    cache.insert(address.clone(), metadata.clone());
                }
                
                Ok(Some(metadata))
            },
            None => Ok(None),
        }
    }

    /// Store contract storage item
    pub fn store_contract_storage(
        &self,
        address: &SmartContractAddress,
        storage_key: &[u8],
        value: &[u8],
    ) -> Result<(), StorageError> {
        let key = self.make_storage_key(address, storage_key);
        debug!("Storing contract storage item for {}, key: {}", 
               address.to_hex_literal(), hex::encode(storage_key));
        
        self.storage.save_data(key.as_bytes(), value)
    }

    /// Load contract storage item
    pub fn load_contract_storage(
        &self,
        address: &SmartContractAddress,
        storage_key: &[u8],
    ) -> Result<Option<Vec<u8>>, StorageError> {
        let key = self.make_storage_key(address, storage_key);
        debug!("Loading contract storage item for {}, key: {}", 
               address.to_hex_literal(), hex::encode(storage_key));
        
        self.storage.load_data(key.as_bytes())
    }

    /// Delete contract storage item
    pub fn delete_contract_storage(
        &self,
        address: &SmartContractAddress,
        storage_key: &[u8],
    ) -> Result<(), StorageError> {
        let key = self.make_storage_key(address, storage_key);
        debug!("Deleting contract storage item for {}, key: {}", 
               address.to_hex_literal(), hex::encode(storage_key));
        
        self.storage.delete_data(key.as_bytes())
    }

    /// List all storage keys for a contract
    pub fn list_contract_storage_keys(
        &self,
        address: &SmartContractAddress,
    ) -> Result<Vec<Vec<u8>>, StorageError> {
        let prefix = format!("contract_storage:{}:", address.to_hex_literal());
        debug!("Listing storage keys for contract {}", address.to_hex_literal());
        
        let keys = self.storage.list_keys_with_prefix(prefix.as_bytes())?;
        
        // Extract the actual storage keys by removing the prefix
        let storage_keys: Vec<Vec<u8>> = keys
            .into_iter()
            .filter_map(|key| {
                let key_str = String::from_utf8_lossy(&key);
                // Split on the last ':' to get the storage key part
                if let Some(pos) = key_str.rfind(':') {
                    let storage_key_hex = &key_str[pos + 1..];
                    hex::decode(storage_key_hex).ok()
                } else {
                    None
                }
            })
            .collect();
        
        debug!("Found {} storage keys for contract {}", 
               storage_keys.len(), address.to_hex_literal());
        Ok(storage_keys)
    }

    /// Store Move module information
    pub fn store_move_module(
        &self,
        module_info: &MoveModuleInfo,
    ) -> Result<(), StorageError> {
        let key = format!("move_module:{}:{}", 
                         module_info.address.to_hex_literal(), 
                         module_info.module_id);
        let serialized = bincode::serialize(module_info)?;
        
        debug!("Storing Move module info: {}", module_info.module_id);
        self.storage.save_data(key.as_bytes(), &serialized)
    }

    /// Load Move module information
    pub fn load_move_module(
        &self,
        address: &SmartContractAddress,
        module_id: &str,
    ) -> Result<Option<MoveModuleInfo>, StorageError> {
        let key = format!("move_module:{}:{}", address.to_hex_literal(), module_id);
        debug!("Loading Move module info: {}", module_id);
        
        match self.storage.load_data(key.as_bytes())? {
            Some(data) => {
                let module_info: MoveModuleInfo = bincode::deserialize(&data)?;
                Ok(Some(module_info))
            },
            None => Ok(None),
        }
    }

    /// Store contract state snapshot
    pub fn store_state_snapshot(
        &self,
        snapshot: &ContractStateSnapshot,
    ) -> Result<(), StorageError> {
        let key = format!("state_snapshot:{}:{}", 
                         snapshot.address.to_hex_literal(), 
                         snapshot.snapshot_time);
        let serialized = bincode::serialize(snapshot)?;
        
        debug!("Storing state snapshot for {} at time {}", 
               snapshot.address.to_hex_literal(), snapshot.snapshot_time);
        self.storage.save_data(key.as_bytes(), &serialized)
    }

    /// Load contract state snapshot
    pub fn load_state_snapshot(
        &self,
        address: &SmartContractAddress,
        timestamp: u64,
    ) -> Result<Option<ContractStateSnapshot>, StorageError> {
        let key = format!("state_snapshot:{}:{}", address.to_hex_literal(), timestamp);
        debug!("Loading state snapshot for {} at time {}", 
               address.to_hex_literal(), timestamp);
        
        match self.storage.load_data(key.as_bytes())? {
            Some(data) => {
                let snapshot: ContractStateSnapshot = bincode::deserialize(&data)?;
                Ok(Some(snapshot))
            },
            None => Ok(None),
        }
    }

    /// Store smart contract event
    pub fn store_contract_event(
        &self,
        event: &SmartContractEvent,
    ) -> Result<(), StorageError> {
        let key = format!("contract_event:{}:{}:{}", 
                         event.contract_address.to_hex_literal(),
                         event.block_height,
                         event.transaction_hash);
        let serialized = bincode::serialize(event)?;
        
        debug!("Storing contract event for {} in tx {}", 
               event.contract_address.to_hex_literal(), event.transaction_hash);
        self.storage.save_data(key.as_bytes(), &serialized)
    }

    /// Load contract events by address and block range
    pub fn load_contract_events(
        &self,
        address: &SmartContractAddress,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<SmartContractEvent>, StorageError> {
        let mut events = Vec::new();
        
        for block_height in from_block..=to_block {
            let prefix = format!("contract_event:{}:{}:", 
                               address.to_hex_literal(), block_height);
            let keys = self.storage.list_keys_with_prefix(prefix.as_bytes())?;
            
            for key in keys {
                if let Some(data) = self.storage.load_data(&key)? {
                    if let Ok(event) = bincode::deserialize::<SmartContractEvent>(&data) {
                        events.push(event);
                    }
                }
            }
        }
        
        debug!("Loaded {} events for contract {} in blocks {}-{}", 
               events.len(), address.to_hex_literal(), from_block, to_block);
        Ok(events)
    }

    /// Get contract storage size in bytes
    pub fn get_contract_storage_size(
        &self,
        address: &SmartContractAddress,
    ) -> Result<u64, StorageError> {
        let storage_keys = self.list_contract_storage_keys(address)?;
        let mut total_size = 0u64;
        
        for storage_key in storage_keys {
            if let Some(value) = self.load_contract_storage(address, &storage_key)? {
                total_size += storage_key.len() as u64 + value.len() as u64;
            }
        }
        
        debug!("Contract {} storage size: {} bytes", 
               address.to_hex_literal(), total_size);
        Ok(total_size)
    }

    /// Clear all data for a contract (for testing/debugging)
    pub fn clear_contract_data(
        &self,
        address: &SmartContractAddress,
    ) -> Result<(), StorageError> {
        warn!("Clearing all data for contract {}", address.to_hex_literal());
        
        // Clear bytecode
        let bytecode_key = self.make_contract_key(address, "bytecode");
        let _ = self.storage.delete_data(bytecode_key.as_bytes());
        
        // Clear metadata
        let metadata_key = self.make_contract_key(address, "metadata");
        let _ = self.storage.delete_data(metadata_key.as_bytes());
        
        // Clear storage items
        let storage_keys = self.list_contract_storage_keys(address)?;
        for storage_key in storage_keys {
            let key = self.make_storage_key(address, &storage_key);
            let _ = self.storage.delete_data(key.as_bytes());
        }
        
        // Clear from caches
        if let Ok(mut cache) = self.contract_cache.write() {
            cache.remove(address);
        }
        if let Ok(mut cache) = self.metadata_cache.write() {
            cache.remove(address);
        }
        
        info!("Cleared all data for contract {}", address.to_hex_literal());
        Ok(())
    }

    /// Flush all cached data to storage
    pub fn flush(&self) -> Result<(), StorageError> {
        debug!("Flushing smart contract storage");
        self.storage.flush()
    }

    // Private helper methods

    fn make_contract_key(&self, address: &SmartContractAddress, suffix: &str) -> String {
        format!("contract_{}:{}", suffix, address.to_hex_literal())
    }

    fn make_storage_key(&self, address: &SmartContractAddress, storage_key: &[u8]) -> String {
        format!("contract_storage:{}:{}", 
               address.to_hex_literal(), 
               hex::encode(storage_key))
    }

    fn update_contract_cache(
        &self,
        address: SmartContractAddress,
        bytecode: Vec<u8>,
    ) -> Result<(), StorageError> {
        if let Ok(mut cache) = self.contract_cache.write() {
            if let Ok(mut size) = self.current_cache_size.write() {
                let new_size = bytecode.len();
                
                // Check if adding this would exceed cache limit
                if *size + new_size > self.cache_size_limit {
                    // Simple eviction: clear half the cache
                    let keys_to_remove: Vec<_> = cache.keys().take(cache.len() / 2).cloned().collect();
                    for key in keys_to_remove {
                        if let Some(removed) = cache.remove(&key) {
                            *size = size.saturating_sub(removed.len());
                        }
                    }
                }
                
                cache.insert(address, bytecode);
                *size += new_size;
            }
        }
        Ok(())
    }

    fn get_from_contract_cache(&self, address: &SmartContractAddress) -> Option<Vec<u8>> {
        if let Ok(cache) = self.contract_cache.read() {
            cache.get(address).cloned()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::RocksDBStorage;

    use super::*;    use tempfile::tempdir;

    fn create_test_storage() -> Arc<RocksDBStorage> {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().to_path_buf();
        Arc::new(RocksDBStorage::new(db_path).unwrap())
    }

    #[test]
    fn test_smart_contract_address() {
        let bytes = [1u8; 32];
        let addr = SmartContractAddress::new(bytes);
        assert_eq!(addr.bytes, bytes);
        
        let hex_literal = addr.to_hex_literal();
        assert!(hex_literal.starts_with("0x"));
        
        let addr2 = SmartContractAddress::from_hex(&hex_literal).unwrap();
        assert_eq!(addr, addr2);
    }

    #[test]
    fn test_contract_bytecode_storage() {
        let storage = create_test_storage();
        let sc_storage = SmartContractStorage::new(storage, 1024 * 1024); // 1MB cache
        
        let address = SmartContractAddress::new([1u8; 32]);
        let bytecode = vec![0xde, 0xad, 0xbe, 0xef];
        
        // Store bytecode
        sc_storage.store_contract_bytecode(&address, &bytecode).unwrap();
        
        // Load bytecode
        let loaded = sc_storage.load_contract_bytecode(&address).unwrap();
        assert_eq!(loaded, Some(bytecode));
        
        // Test non-existent contract
        let nonexistent = SmartContractAddress::new([2u8; 32]);
        let result = sc_storage.load_contract_bytecode(&nonexistent).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_contract_storage_operations() {
        let storage = create_test_storage();
        let sc_storage = SmartContractStorage::new(storage, 1024 * 1024);
        
        let address = SmartContractAddress::new([1u8; 32]);
        let storage_key = b"test_key";
        let value = b"test_value";
        
        // Store storage item
        sc_storage.store_contract_storage(&address, storage_key, value).unwrap();
        
        // Load storage item
        let loaded = sc_storage.load_contract_storage(&address, storage_key).unwrap();
        assert_eq!(loaded, Some(value.to_vec()));
        
        // List storage keys
        let keys = sc_storage.list_contract_storage_keys(&address).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], storage_key);
        
        // Delete storage item
        sc_storage.delete_contract_storage(&address, storage_key).unwrap();
        let deleted = sc_storage.load_contract_storage(&address, storage_key).unwrap();
        assert_eq!(deleted, None);
    }
}
