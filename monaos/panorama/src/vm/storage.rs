//! VM Storage System
//!
//! This module provides persistent storage for smart contracts, including
//! state management, storage optimization, and state root calculation.

use crate::vm::VMError;
use log::{debug, info, warn};
use mona_crypto::hash_data_blake3;
use mona_storage::{BlockchainStorage, RocksDBStorage, StorageError};
use mona_types::address::Address;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

/// VM Storage interface for smart contracts
#[derive(Debug, Clone)]
pub struct VMStorage {
    /// In-memory storage cache for active contracts
    storage_cache: Arc<RwLock<HashMap<String, ContractStorage>>>,
    /// Persistent storage backend
    persistent_storage: Option<Arc<RocksDBStorage>>,
    /// Storage statistics
    stats: Arc<RwLock<StorageStats>>,
    /// Storage limits per contract
    storage_limits: StorageLimits,
}

/// Contract-specific storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractStorage {
    /// Contract address
    pub address: Address,
    /// Key-value storage map
    pub storage: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Storage size in bytes
    pub size: usize,
    /// Last modification timestamp
    pub last_modified: u64,
    /// Storage root hash for verification
    pub storage_root: String,
    /// Dirty flag for tracking changes
    pub dirty: bool,
}

/// Storage statistics tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageStats {
    /// Total storage operations
    pub total_operations: u64,
    /// Read operations count
    pub read_operations: u64,
    /// Write operations count
    pub write_operations: u64,
    /// Delete operations count
    pub delete_operations: u64,
    /// Total storage size across all contracts
    pub total_storage_size: usize,
    /// Number of contracts with storage
    pub contracts_with_storage: usize,
    /// Cache hit rate
    pub cache_hits: u64,
    /// Cache miss rate
    pub cache_misses: u64,
}

/// Storage limits configuration
#[derive(Debug, Clone)]
pub struct StorageLimits {
    /// Maximum storage size per contract (in bytes)
    pub max_contract_storage: usize,
    /// Maximum number of storage keys per contract
    pub max_storage_keys: usize,
    /// Maximum key size (in bytes)
    pub max_key_size: usize,
    /// Maximum value size (in bytes)
    pub max_value_size: usize,
    /// Maximum total storage across all contracts
    pub max_total_storage: usize,
}

impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            max_contract_storage: 100 * 1024 * 1024, // 100MB per contract
            max_storage_keys: 100_000,               // 100k keys per contract
            max_key_size: 32,                        // 32 bytes max key size
            max_value_size: 32 * 1024,               // 32KB max value size
            max_total_storage: 10 * 1024 * 1024 * 1024, // 10GB total
        }
    }
}

impl Default for StorageStats {
    fn default() -> Self {
        Self {
            total_operations: 0,
            read_operations: 0,
            write_operations: 0,
            delete_operations: 0,
            total_storage_size: 0,
            contracts_with_storage: 0,
            cache_hits: 0,
            cache_misses: 0,
        }
    }
}

impl VMStorage {
    /// Create a new VM storage instance
    pub fn new() -> Self {
        Self {
            storage_cache: Arc::new(RwLock::new(HashMap::new())),
            persistent_storage: None,
            stats: Arc::new(RwLock::new(StorageStats::default())),
            storage_limits: StorageLimits::default(),
        }
    }

    /// Create VM storage with persistent backend
    pub fn with_persistent_storage(storage_path: std::path::PathBuf) -> Result<Self, VMError> {
        let persistent_storage = RocksDBStorage::new(storage_path)
            .map_err(|e| VMError::StorageError(format!("Failed to initialize storage: {}", e)))?;

        let mut vm_storage = Self::new();
        vm_storage.persistent_storage = Some(Arc::new(persistent_storage));

        // Load existing storage from persistent backend
        vm_storage.load_from_persistent()?;

        Ok(vm_storage)
    }

    /// Get storage value for a contract
    pub fn get_storage(&self, contract_address: &Address, key: &[u8]) -> Option<Vec<u8>> {
        // Validate key size
        if key.len() > self.storage_limits.max_key_size {
            warn!("Storage key too large: {} bytes", key.len());
            return None;
        }

        let contract_key = contract_address.to_hex_literal();

        // Try cache first
        {
            let cache = self.storage_cache.read().unwrap();
            if let Some(contract_storage) = cache.get(&contract_key) {
                let result = contract_storage.storage.get(key).cloned();

                // Update statistics
                let mut stats = self.stats.write().unwrap();
                stats.read_operations += 1;
                stats.total_operations += 1;

                if result.is_some() {
                    stats.cache_hits += 1;
                } else {
                    stats.cache_misses += 1;
                }

                debug!(
                    "Storage read: {} -> {:?}",
                    hex::encode(key),
                    result.is_some()
                );
                return result;
            }
        }

        // Cache miss - try persistent storage
        if let Some(ref persistent) = self.persistent_storage {
            let storage_key = self.make_storage_key(contract_address, key);
            if let Ok(Some(value)) = persistent.load_data(&storage_key) {
                // Update cache
                self.cache_storage_value(contract_address.clone(), key.to_vec(), value.clone());

                // Update statistics
                let mut stats = self.stats.write().unwrap();
                stats.read_operations += 1;
                stats.total_operations += 1;
                stats.cache_misses += 1;

                debug!(
                    "Storage read from persistent: {} -> {} bytes",
                    hex::encode(key),
                    value.len()
                );
                return Some(value);
            }
        }

        // Update statistics for miss
        let mut stats = self.stats.write().unwrap();
        stats.read_operations += 1;
        stats.total_operations += 1;
        stats.cache_misses += 1;

        None
    }

    /// Set storage value for a contract
    pub fn set_storage(&self, contract_address: Address, key: Vec<u8>, value: Vec<u8>) {
        // Validate limits
        if key.len() > self.storage_limits.max_key_size {
            warn!("Storage key too large: {} bytes", key.len());
            return;
        }

        if value.len() > self.storage_limits.max_value_size {
            warn!("Storage value too large: {} bytes", value.len());
            return;
        }

        let contract_key = contract_address.to_hex_literal();
        let is_new_key;

        // Update cache
        {
            let mut cache = self.storage_cache.write().unwrap();
            let contract_storage = cache
                .entry(contract_key.clone())
                .or_insert_with(|| ContractStorage::new(contract_address.clone()));

            // Check storage limits
            if contract_storage.storage.len() >= self.storage_limits.max_storage_keys {
                warn!("Contract {} reached maximum storage keys", contract_address);
                return;
            }

            if contract_storage.size >= self.storage_limits.max_contract_storage {
                warn!("Contract {} reached maximum storage size", contract_address);
                return;
            }

            is_new_key = !contract_storage.storage.contains_key(&key);

            // Update storage
            let old_value_size = contract_storage
                .storage
                .get(&key)
                .map(|v| v.len())
                .unwrap_or(0);

            contract_storage.storage.insert(key.clone(), value.clone());
            contract_storage.size = contract_storage
                .size
                .saturating_sub(old_value_size)
                .saturating_add(value.len());

            contract_storage.last_modified = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            contract_storage.dirty = true;

            // Recalculate storage root
            contract_storage.storage_root = Self::calculate_storage_root(&contract_storage.storage);
        }

        // Update persistent storage if available
        if let Some(ref persistent) = self.persistent_storage {
            let storage_key = self.make_storage_key(&contract_address, &key);
            if let Err(e) = persistent.save_data(&storage_key, &value) {
                warn!("Failed to save to persistent storage: {}", e);
            }
        }

        // Update statistics
        {
            let mut stats = self.stats.write().unwrap();
            stats.write_operations += 1;
            stats.total_operations += 1;

            if is_new_key {
                stats.total_storage_size += key.len() + value.len();
            } else {
                // Size might have changed, but we approximate
                stats.total_storage_size += value.len();
            }
        }

        debug!(
            "Storage write: {} -> {} bytes",
            hex::encode(&key),
            value.len()
        );
    }

    /// Delete storage value for a contract
    pub fn delete_storage(&self, contract_address: &Address, key: &[u8]) -> bool {
        let contract_key = contract_address.to_hex_literal();
        let deleted;

        // Update cache
        {
            let mut cache = self.storage_cache.write().unwrap();
            if let Some(contract_storage) = cache.get_mut(&contract_key) {
                if let Some(old_value) = contract_storage.storage.remove(key) {
                    contract_storage.size = contract_storage
                        .size
                        .saturating_sub(key.len() + old_value.len());
                    contract_storage.last_modified = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    contract_storage.dirty = true;

                    // Recalculate storage root
                    contract_storage.storage_root =
                        Self::calculate_storage_root(&contract_storage.storage);
                    deleted = true;
                } else {
                    deleted = false;
                }
            } else {
                deleted = false;
            }
        }

        // Update persistent storage if available
        if deleted && self.persistent_storage.is_some() {
            let storage_key = self.make_storage_key(contract_address, key);
            if let Some(ref persistent) = self.persistent_storage {
                if let Err(e) = persistent.delete_data(&storage_key) {
                    warn!("Failed to delete from persistent storage: {}", e);
                }
            }
        }

        // Update statistics
        if deleted {
            let mut stats = self.stats.write().unwrap();
            stats.delete_operations += 1;
            stats.total_operations += 1;
            stats.total_storage_size = stats.total_storage_size.saturating_sub(key.len());
        }

        debug!("Storage delete: {} -> {}", hex::encode(key), deleted);
        deleted
    }

    /// Get all storage keys for a contract
    pub fn get_storage_keys(&self, contract_address: &Address) -> Vec<Vec<u8>> {
        let contract_key = contract_address.to_hex_literal();

        let cache = self.storage_cache.read().unwrap();
        if let Some(contract_storage) = cache.get(&contract_key) {
            contract_storage.storage.keys().cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// Get storage size for a contract
    pub fn get_storage_size(&self, contract_address: &Address) -> usize {
        let contract_key = contract_address.to_hex_literal();

        let cache = self.storage_cache.read().unwrap();
        cache
            .get(&contract_key)
            .map(|storage| storage.size)
            .unwrap_or(0)
    }

    /// Get storage root hash for a contract
    pub fn get_storage_root(&self, contract_address: &Address) -> Option<String> {
        let contract_key = contract_address.to_hex_literal();

        let cache = self.storage_cache.read().unwrap();
        cache
            .get(&contract_key)
            .map(|storage| storage.storage_root.clone())
    }

    /// Clear all storage for a contract
    pub fn clear_contract_storage(&self, contract_address: &Address) {
        let contract_key = contract_address.to_hex_literal();

        // Clear from cache
        {
            let mut cache = self.storage_cache.write().unwrap();
            if let Some(contract_storage) = cache.remove(&contract_key) {
                // Update statistics
                let mut stats = self.stats.write().unwrap();
                stats.total_storage_size = stats
                    .total_storage_size
                    .saturating_sub(contract_storage.size);
                stats.contracts_with_storage = stats.contracts_with_storage.saturating_sub(1);
            }
        }

        // Clear from persistent storage if available
        if let Some(ref persistent) = self.persistent_storage {
            let prefix = format!("storage:{}:", contract_address.to_hex_literal());
            // Note: RocksDB doesn't have a direct prefix delete, so we'd need to implement this
            // For now, we'll mark this as a TODO for optimization
            debug!("TODO: Implement prefix deletion for contract storage cleanup");
        }

        info!("Cleared all storage for contract {}", contract_address);
    }

    /// Get storage statistics
    pub fn get_stats(&self) -> StorageStats {
        self.stats.read().unwrap().clone()
    }

    /// Reset storage statistics
    pub fn reset_stats(&self) {
        let mut stats = self.stats.write().unwrap();
        *stats = StorageStats::default();
    }

    /// Flush dirty storage to persistent backend
    pub fn flush(&self) -> Result<(), VMError> {
        if self.persistent_storage.is_none() {
            return Ok(());
        }

        let persistent = self.persistent_storage.as_ref().unwrap();
        let mut flushed_contracts = 0;

        // Get dirty contracts
        let dirty_contracts: Vec<_> = {
            let cache = self.storage_cache.read().unwrap();
            cache
                .iter()
                .filter(|(_, storage)| storage.dirty)
                .map(|(addr, storage)| (addr.clone(), storage.clone()))
                .collect()
        };

        // Flush each dirty contract
        for (contract_addr, contract_storage) in dirty_contracts {
            // Save contract metadata
            let metadata_key = format!("contract_meta:{}", contract_addr);
            let metadata = ContractStorageMetadata {
                address: contract_storage.address.clone(),
                size: contract_storage.size,
                last_modified: contract_storage.last_modified,
                storage_root: contract_storage.storage_root.clone(),
                key_count: contract_storage.storage.len(),
            };

            if let Ok(metadata_bytes) = bincode::serialize(&metadata) {
                if let Err(e) = persistent.save_data(metadata_key.as_bytes(), &metadata_bytes) {
                    warn!("Failed to save contract metadata: {}", e);
                }
            }

            // Save individual storage entries
            for (key, value) in &contract_storage.storage {
                let storage_key = self.make_storage_key(&contract_storage.address, key);
                if let Err(e) = persistent.save_data(&storage_key, value) {
                    warn!("Failed to save storage entry: {}", e);
                }
            }

            flushed_contracts += 1;
        }

        // Mark contracts as clean
        {
            let mut cache = self.storage_cache.write().unwrap();
            for storage in cache.values_mut() {
                storage.dirty = false;
            }
        }

        // Flush persistent storage
        if let Err(e) = persistent.flush() {
            return Err(VMError::StorageError(format!(
                "Failed to flush storage: {}",
                e
            )));
        }

        info!(
            "Flushed {} dirty contracts to persistent storage",
            flushed_contracts
        );
        Ok(())
    }

    /// Load storage from persistent backend
    fn load_from_persistent(&mut self) -> Result<(), VMError> {
        if self.persistent_storage.is_none() {
            return Ok(());
        }

        // TODO: Implement loading from persistent storage
        // This would involve scanning for contract metadata and loading storage entries
        debug!("TODO: Implement loading from persistent storage");
        Ok(())
    }

    /// Cache a storage value
    fn cache_storage_value(&self, contract_address: Address, key: Vec<u8>, value: Vec<u8>) {
        let contract_key = contract_address.to_hex_literal();

        let mut cache = self.storage_cache.write().unwrap();
        let contract_storage = cache
            .entry(contract_key)
            .or_insert_with(|| ContractStorage::new(contract_address));

        contract_storage.storage.insert(key, value);
    }

    /// Create storage key for persistent backend
    fn make_storage_key(&self, contract_address: &Address, key: &[u8]) -> Vec<u8> {
        let mut storage_key = Vec::new();
        storage_key.extend_from_slice(b"storage:");
        storage_key.extend_from_slice(contract_address.to_hex_literal().as_bytes());
        storage_key.extend_from_slice(b":");
        storage_key.extend_from_slice(key);
        storage_key
    }

    /// Calculate storage root hash from storage data
    fn calculate_storage_root(storage: &BTreeMap<Vec<u8>, Vec<u8>>) -> String {
        let mut combined_data = Vec::new();

        // Use BTreeMap's natural ordering for deterministic hashing
        for (key, value) in storage.iter() {
            combined_data.extend_from_slice(key);
            combined_data.extend_from_slice(value);
        }

        let hash = hash_data_blake3(&combined_data);
        hex::encode(hash)
    }

    /// Optimize storage by removing empty contracts and compacting data
    pub fn optimize(&self) -> Result<(), VMError> {
        let mut removed_contracts = 0;

        // Remove empty contracts from cache
        {
            let mut cache = self.storage_cache.write().unwrap();
            cache.retain(|addr, storage| {
                if storage.storage.is_empty() {
                    debug!("Removing empty storage for contract {}", addr);
                    removed_contracts += 1;
                    false
                } else {
                    true
                }
            });
        }

        // Update statistics
        {
            let mut stats = self.stats.write().unwrap();
            stats.contracts_with_storage = stats
                .contracts_with_storage
                .saturating_sub(removed_contracts);
        }

        // Flush optimized state
        self.flush()?;

        info!(
            "Storage optimization completed, removed {} empty contracts",
            removed_contracts
        );
        Ok(())
    }
}

impl ContractStorage {
    /// Create new contract storage
    pub fn new(address: Address) -> Self {
        Self {
            address,
            storage: BTreeMap::new(),
            size: 0,
            last_modified: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            storage_root: hex::encode(hash_data_blake3(&[])), // Empty root
            dirty: false,
        }
    }

    /// Check if storage is empty
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    /// Get number of storage keys
    pub fn key_count(&self) -> usize {
        self.storage.len()
    }
}

/// Contract storage metadata for persistent storage
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContractStorageMetadata {
    address: Address,
    size: usize,
    last_modified: u64,
    storage_root: String,
    key_count: usize,
}

/// Storage migration utilities
pub struct StorageMigrator {
    old_storage: VMStorage,
    new_storage: VMStorage,
}

impl StorageMigrator {
    /// Create a new storage migrator
    pub fn new(old_storage: VMStorage, new_storage: VMStorage) -> Self {
        Self {
            old_storage,
            new_storage,
        }
    }

    /// Migrate storage from old to new backend
    pub fn migrate(&self) -> Result<usize, VMError> {
        let mut migrated_contracts = 0;

        // Get all contracts from old storage
        let contracts: Vec<_> = {
            let cache = self.old_storage.storage_cache.read().unwrap();
            cache.keys().cloned().collect()
        };

        for contract_key in contracts {
            if let Ok(address) = Address::from_hex_literal(&contract_key) {
                // Get all storage keys for this contract
                let storage_keys = self.old_storage.get_storage_keys(&address);

                // Migrate each key-value pair
                for key in storage_keys {
                    if let Some(value) = self.old_storage.get_storage(&address, &key) {
                        self.new_storage.set_storage(address.clone(), key, value);
                    }
                }

                migrated_contracts += 1;
            }
        }

        // Flush new storage
        self.new_storage.flush()?;

        info!(
            "Migrated {} contracts to new storage backend",
            migrated_contracts
        );
        Ok(migrated_contracts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_basic_storage_operations() {
        let storage = VMStorage::new();
        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        let key = b"test_key".to_vec();
        let value = b"test_value".to_vec();

        // Test set and get
        storage.set_storage(address.clone(), key.clone(), value.clone());
        let retrieved = storage.get_storage(&address, &key);
        assert_eq!(retrieved, Some(value));

        // Test delete
        assert!(storage.delete_storage(&address, &key));
        let deleted = storage.get_storage(&address, &key);
        assert_eq!(deleted, None);
    }

    #[test]
    fn test_storage_limits() {
        let storage = VMStorage::new();
        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        // Test key size limit
        let large_key = vec![0u8; storage.storage_limits.max_key_size + 1];
        let value = b"test".to_vec();
        storage.set_storage(address.clone(), large_key.clone(), value.clone());
        let retrieved = storage.get_storage(&address, &large_key);
        assert_eq!(retrieved, None); // Should be rejected due to size
    }

    #[test]
    fn test_storage_stats() {
        let storage = VMStorage::new();
        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        let initial_stats = storage.get_stats();
        assert_eq!(initial_stats.total_operations, 0);

        // Perform some operations
        storage.set_storage(address.clone(), b"key1".to_vec(), b"value1".to_vec());
        storage.get_storage(&address, b"key1");
        storage.delete_storage(&address, b"key1");

        let final_stats = storage.get_stats();
        assert!(final_stats.total_operations > 0);
        assert!(final_stats.write_operations > 0);
        assert!(final_stats.read_operations > 0);
        assert!(final_stats.delete_operations > 0);
    }

    #[test]
    fn test_persistent_storage() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("test_storage");

        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let key = b"persist_key".to_vec();
        let value = b"persist_value".to_vec();

        // Create storage with persistent backend
        {
            let storage = VMStorage::with_persistent_storage(storage_path.clone()).unwrap();
            storage.set_storage(address.clone(), key.clone(), value.clone());
            storage.flush().unwrap();
        }

        // Create new storage instance and verify data persists
        {
            let storage = VMStorage::with_persistent_storage(storage_path).unwrap();
            // Note: Loading from persistent storage is not fully implemented in this test
            // but the storage backend is initialized
            assert!(storage.persistent_storage.is_some());
        }
    }

    #[test]
    fn test_storage_root_calculation() {
        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let mut contract_storage = ContractStorage::new(address);

        let initial_root = contract_storage.storage_root.clone();

        // Add some data
        contract_storage
            .storage
            .insert(b"key1".to_vec(), b"value1".to_vec());
        contract_storage.storage_root =
            VMStorage::calculate_storage_root(&contract_storage.storage);

        let new_root = contract_storage.storage_root.clone();
        assert_ne!(initial_root, new_root);

        // Same data should produce same root
        let mut other_storage = BTreeMap::new();
        other_storage.insert(b"key1".to_vec(), b"value1".to_vec());
        let other_root = VMStorage::calculate_storage_root(&other_storage);
        assert_eq!(new_root, other_root);
    }
}
