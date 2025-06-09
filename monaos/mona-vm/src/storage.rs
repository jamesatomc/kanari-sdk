//! VM-specific storage operations

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Serialize, Deserialize};
use log::debug;

use mona_storage::{
    BlockchainStorage, RocksDBStorage,
    SmartContractStorage, SmartContractAddress, SmartContractMetadata,
    MoveModuleInfo, ContractStateSnapshot, SmartContractEvent
};
use crate::types::{VMResult, VMError, ContractAddress};

/// VM storage interface for contract data
pub struct VMStorage {
    /// Smart contract storage manager
    smart_contract_storage: Arc<SmartContractStorage>,
    /// In-memory cache for frequently accessed data
    cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    /// Cache size limit (in bytes)
    cache_size_limit: usize,
    /// Current cache size
    current_cache_size: Arc<RwLock<usize>>,
}

impl VMStorage {
    /// Create a new VM storage instance
    pub fn new(storage: Arc<RocksDBStorage>, cache_size_limit: usize) -> Self {
        let blockchain_storage: Arc<dyn BlockchainStorage + Send + Sync> = storage;
        let smart_contract_storage = Arc::new(SmartContractStorage::new(
            blockchain_storage,
            cache_size_limit,
        ));
        
        Self {
            smart_contract_storage,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_size_limit,
            current_cache_size: Arc::new(RwLock::new(0)),
        }
    }

    /// Get access to the underlying smart contract storage
    pub fn get_smart_contract_storage(&self) -> &Arc<SmartContractStorage> {
        &self.smart_contract_storage
    }
    
    /// Convert ContractAddress to SmartContractAddress
    fn to_smart_contract_address(&self, address: ContractAddress) -> SmartContractAddress {
        SmartContractAddress::new(address.into_bytes())
    }
    
    /// Store contract bytecode
    pub fn store_contract_bytecode(&self, address: ContractAddress, bytecode: &[u8]) -> VMResult<()> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .store_contract_bytecode(&smart_address, bytecode)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store contract bytecode: {}", e),
            })
    }

    /// Load contract bytecode
    pub fn load_contract_bytecode(&self, address: ContractAddress) -> VMResult<Option<Vec<u8>>> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .load_contract_bytecode(&smart_address)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to load contract bytecode: {}", e),
            })
    }    /// Store contract storage item
    pub fn store_contract_storage(
        &self,
        address: ContractAddress,
        storage_key: &[u8],
        value: &[u8],
    ) -> VMResult<()> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .store_contract_storage(&smart_address, storage_key, value)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store contract storage: {}", e),
            })
    }

    /// Load contract storage item
    pub fn load_contract_storage(
        &self,
        address: ContractAddress,
        storage_key: &[u8],
    ) -> VMResult<Option<Vec<u8>>> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .load_contract_storage(&smart_address, storage_key)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to load contract storage: {}", e),
            })
    }

    /// Delete contract storage item
    pub fn delete_contract_storage(
        &self,
        address: ContractAddress,
        storage_key: &[u8],
    ) -> VMResult<()> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .delete_contract_storage(&smart_address, storage_key)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to delete contract storage: {}", e),
            })
    }    /// Store contract metadata
    pub fn store_contract_metadata(
        &self,
        address: ContractAddress,
        metadata: &ContractMetadata,
    ) -> VMResult<()> {
        let smart_address = self.to_smart_contract_address(address);
        
        // Convert ContractMetadata to SmartContractMetadata
        let smart_metadata = SmartContractMetadata {
            name: metadata.module_name.clone(),
            version: metadata.version.clone(),
            deployed_at: metadata.deployed_at,
            deployer: SmartContractAddress::from_hex(&metadata.deployer)
                .unwrap_or_else(|_| SmartContractAddress::new([0u8; 32])),
            bytecode_size: 0, // Will be updated when bytecode is stored
            source_hash: Some(metadata.bytecode_hash.clone()),
            compiler_version: "move-compiler".to_string(),
            gas_used_deployment: 0, // Will be updated during deployment
        };
        
        self.smart_contract_storage
            .store_contract_metadata(&smart_address, &smart_metadata)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store contract metadata: {}", e),
            })
    }

    /// Load contract metadata
    pub fn load_contract_metadata(&self, address: ContractAddress) -> VMResult<Option<ContractMetadata>> {
        let smart_address = self.to_smart_contract_address(address);
        
        match self.smart_contract_storage
            .load_contract_metadata(&smart_address)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to load contract metadata: {}", e),
            })? {
            Some(smart_metadata) => {
                // Convert SmartContractMetadata back to ContractMetadata
                let metadata = ContractMetadata {
                    module_name: smart_metadata.name,
                    deployer: smart_metadata.deployer.to_hex_literal(),
                    deployed_at: smart_metadata.deployed_at,
                    bytecode_hash: smart_metadata.source_hash.unwrap_or_default(),
                    version: smart_metadata.version,
                };
                Ok(Some(metadata))
            },
            None => Ok(None),
        }
    }    /// Store execution result
    pub fn store_execution_result(
        &self,
        transaction_hash: &str,
        result: &crate::ExecutionResult,
    ) -> VMResult<()> {
        let key = format!("execution_result:{}", transaction_hash);
        let serialized = bincode::serialize(result)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to serialize execution result: {}", e),
            })?;
        
        // Store using cache mechanism for now
        self.update_cache(key.as_bytes(), Some(serialized))
    }

    /// Load execution result
    pub fn load_execution_result(&self, transaction_hash: &str) -> VMResult<Option<crate::ExecutionResult>> {
        let key = format!("execution_result:{}", transaction_hash);
        let key_str = hex::encode(key.as_bytes());
        
        // Check cache first
        {
            let cache = self.cache.read().unwrap();
            if let Some(value) = cache.get(&key_str) {
                let result = bincode::deserialize(value)
                    .map_err(|e| VMError::StorageError {
                        message: format!("Failed to deserialize execution result: {}", e),
                    })?;
                return Ok(Some(result));
            }
        }
        
        Ok(None) // For now, only cache-based storage for execution results
    }/// Get all storage keys for a contract
    pub fn get_contract_storage_keys(&self, address: ContractAddress) -> VMResult<Vec<Vec<u8>>> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .list_contract_storage_keys(&smart_address)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to get storage keys: {}", e),
            })
    }    /// Get contract storage size
    pub fn get_contract_storage_size(&self, address: ContractAddress) -> VMResult<u64> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .get_contract_storage_size(&smart_address)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to get storage size: {}", e),
            })
    }

    /// Clear all storage for a contract
    pub fn clear_contract_storage(&self, address: ContractAddress) -> VMResult<()> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .clear_contract_data(&smart_address)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to clear contract storage: {}", e),
            })
    }

    /// Store Move module information
    pub fn store_move_module(&self, module_info: &MoveModuleInfo) -> VMResult<()> {
        self.smart_contract_storage
            .store_move_module(module_info)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store Move module: {}", e),
            })
    }

    /// Load Move module information
    pub fn load_move_module(
        &self,
        address: ContractAddress,
        module_id: &str,
    ) -> VMResult<Option<MoveModuleInfo>> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .load_move_module(&smart_address, module_id)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to load Move module: {}", e),
            })
    }

    /// Store contract state snapshot
    pub fn store_state_snapshot(&self, snapshot: &ContractStateSnapshot) -> VMResult<()> {
        self.smart_contract_storage
            .store_state_snapshot(snapshot)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store state snapshot: {}", e),
            })
    }

    /// Load contract state snapshot
    pub fn load_state_snapshot(
        &self,
        address: ContractAddress,
        timestamp: u64,
    ) -> VMResult<Option<ContractStateSnapshot>> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .load_state_snapshot(&smart_address, timestamp)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to load state snapshot: {}", e),
            })
    }

    /// Store smart contract event
    pub fn store_contract_event(&self, event: &SmartContractEvent) -> VMResult<()> {
        self.smart_contract_storage
            .store_contract_event(event)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store contract event: {}", e),
            })
    }

    /// Load contract events by address and block range
    pub fn load_contract_events(
        &self,
        address: ContractAddress,
        from_block: u64,
        to_block: u64,
    ) -> VMResult<Vec<SmartContractEvent>> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .load_contract_events(&smart_address, from_block, to_block)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to load contract events: {}", e),
            })
    }    /// Flush all data to persistent storage
    pub fn flush(&self) -> VMResult<()> {
        self.smart_contract_storage
            .flush()
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to flush storage: {}", e),
            })
    }

    /// Update cache with size management (for execution results)
    fn update_cache(&self, key: &[u8], value: Option<Vec<u8>>) -> VMResult<()> {
        let key_str = hex::encode(key);
        let mut cache = self.cache.write().unwrap();
        let mut cache_size = self.current_cache_size.write().unwrap();

        match value {
            Some(v) => {
                let new_size = v.len();
                
                // Remove old value if exists
                if let Some(old_value) = cache.get(&key_str) {
                    *cache_size = cache_size.saturating_sub(old_value.len());
                }

                // Check if cache would exceed limit
                if *cache_size + new_size > self.cache_size_limit {
                    // Simple eviction: clear cache when full
                    cache.clear();
                    *cache_size = 0;
                }

                // Add new value
                cache.insert(key_str, v);
                *cache_size += new_size;
            },
            None => {
                // Remove from cache
                if let Some(old_value) = cache.remove(&key_str) {
                    *cache_size = cache_size.saturating_sub(old_value.len());
                }
            }
        }

        Ok(())
    }

    /// Get cache statistics
    pub fn get_cache_stats(&self) -> CacheStats {
        let cache = self.cache.read().unwrap();
        let cache_size = *self.current_cache_size.read().unwrap();

        CacheStats {
            entries: cache.len(),
            size_bytes: cache_size,
            size_limit: self.cache_size_limit,
        }
    }    /// Clear cache
    pub fn clear_cache(&self) {
        let mut cache = self.cache.write().unwrap();
        let mut cache_size = self.current_cache_size.write().unwrap();

        cache.clear();
        *cache_size = 0;
        debug!("Cache cleared");
    }    /// Store Move gas consumption data for analytics
    pub fn store_move_gas_consumption(
        &self,
        address: ContractAddress,
        transaction_hash: &str,
        function_name: &str,
        gas_used: u64,
        kari_spent: u64,
        execution_time_ms: u64,
    ) -> VMResult<()> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .store_move_gas_consumption(&smart_address, transaction_hash, function_name, gas_used, kari_spent, execution_time_ms)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store Move gas consumption: {}", e),
            })
    }

    /// Load Move execution metrics for performance analysis
    pub fn load_move_execution_metrics(
        &self,
        address: ContractAddress,
        start_time: u64,
        end_time: u64,
    ) -> VMResult<Vec<u8>> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .load_move_execution_metrics(&smart_address, start_time, end_time)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to load Move execution metrics: {}", e),
            })
    }

    /// Store Move compilation artifacts (source code, ABI, dependencies)
    pub fn store_move_compilation_artifacts(
        &self,
        address: ContractAddress,
        source_code: &str,
        abi_data: &[u8],
        dependencies: &[String],
    ) -> VMResult<()> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .store_move_compilation_artifacts(&smart_address, source_code, abi_data, dependencies)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store Move compilation artifacts: {}", e),
            })
    }

    /// Load Move compilation artifacts
    pub fn load_move_compilation_artifacts(
        &self,
        address: ContractAddress,
    ) -> VMResult<Option<(String, Vec<u8>, Vec<String>)>> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .load_move_compilation_artifacts(&smart_address)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to load Move compilation artifacts: {}", e),
            })
    }

    /// Store Move resource snapshot for debugging and analysis
    pub fn store_move_resource_snapshot(
        &self,
        address: ContractAddress,
        resource_type: &str,
        resource_data: &[u8],
        timestamp: u64,
    ) -> VMResult<()> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .store_move_resource_snapshot(&smart_address, resource_type, resource_data, timestamp)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to store Move resource snapshot: {}", e),
            })
    }

    /// Get detailed Move contract gas analytics
    pub fn get_move_contract_gas_analytics(
        &self,
        address: ContractAddress,
        days: u32,
    ) -> VMResult<String> {
        let smart_address = self.to_smart_contract_address(address);
        self.smart_contract_storage
            .get_move_contract_gas_analytics(&smart_address, days)
            .map_err(|e| VMError::StorageError {
                message: format!("Failed to get Move contract gas analytics: {}", e),
            })
    }
}

/// Contract metadata for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractMetadata {
    pub module_name: String,
    pub deployer: String,
    pub deployed_at: u64,
    pub bytecode_hash: String,
    pub version: String,
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entries: usize,
    pub size_bytes: usize,
    pub size_limit: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::get_kari_dir;
    use mona_types::address::Address;

    fn setup_test_storage() -> VMStorage {
        let mut test_dir = get_kari_dir();
        test_dir.push("test_vm_storage");
        let storage = Arc::new(RocksDBStorage::new(test_dir).unwrap());
        VMStorage::new(storage, 1024 * 1024) // 1MB cache
    }

    #[test]
    fn test_contract_bytecode_storage() {
        let vm_storage = setup_test_storage();
        let address = Address::zero();
        let bytecode = vec![1, 2, 3, 4, 5];

        // Store bytecode
        vm_storage.store_contract_bytecode(address, &bytecode).unwrap();

        // Load bytecode
        let loaded = vm_storage.load_contract_bytecode(address).unwrap().unwrap();
        assert_eq!(loaded, bytecode);
    }

    #[test]
    fn test_contract_storage() {
        let vm_storage = setup_test_storage();
        let address = Address::zero();
        let storage_key = b"test_key";
        let value = b"test_value";

        // Store value
        vm_storage.store_contract_storage(address, storage_key, value).unwrap();

        // Load value
        let loaded = vm_storage.load_contract_storage(address, storage_key).unwrap().unwrap();
        assert_eq!(loaded, value);

        // Delete value
        vm_storage.delete_contract_storage(address, storage_key).unwrap();

        // Verify deletion
        let deleted = vm_storage.load_contract_storage(address, storage_key).unwrap();
        assert!(deleted.is_none());
    }    #[test]
    fn test_cache_functionality() {
        let vm_storage = setup_test_storage();
        let transaction_hash = "test_tx_123";

        // Create and store execution result to populate cache
        let execution_result = crate::ExecutionResult::success(
            vec![1, 2, 3, 4],
            1000,
            Vec::new(),
        );
        
        vm_storage.store_execution_result(transaction_hash, &execution_result).unwrap();

        // Check cache stats
        let stats = vm_storage.get_cache_stats();
        assert!(stats.entries > 0);
        assert!(stats.size_bytes > 0);

        // Clear cache
        vm_storage.clear_cache();
        let stats_after_clear = vm_storage.get_cache_stats();
        assert_eq!(stats_after_clear.entries, 0);
        assert_eq!(stats_after_clear.size_bytes, 0);
    }
}
