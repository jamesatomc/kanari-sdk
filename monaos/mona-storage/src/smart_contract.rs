//! Smart contract storage layer for Move VM integration

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Serialize, Deserialize};
use bincode;
use log::{debug, info, warn};
use serde_json;

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

/// Gas accounting entry for contract execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasUsageRecord {
    pub contract_address: SmartContractAddress,
    pub transaction_hash: String,
    pub operation_type: String, // "deploy", "call", "read", "write"
    pub kari_amount: u64,
    pub gas_price: u64,
    pub timestamp: u64,
    pub block_height: u64,
}

/// Move resource information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveResourceInfo {
    pub resource_type: String,
    pub module_address: SmartContractAddress,
    pub module_name: String,
    pub struct_name: String,
    pub data: Vec<u8>,
    pub last_modified: u64,
}

/// Contract execution trace for debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub transaction_hash: String,
    pub contract_address: SmartContractAddress,
    pub function_name: String,
    pub arguments: Vec<Vec<u8>>,
    pub return_values: Vec<Vec<u8>>,
    pub gas_used: u64,
    pub kari_spent: u64,
    pub execution_time_ms: u64,
    pub status: String, // "success", "failed", "aborted"
    pub error_message: Option<String>,
    pub events: Vec<SmartContractEvent>,
    pub storage_changes: HashMap<Vec<u8>, Vec<u8>>,
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

impl SmartContractStorage {    /// Create a new smart contract storage manager
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

    /// Helper method to create contract-specific keys
    fn make_contract_key(&self, address: &SmartContractAddress, suffix: &str) -> String {
        format!("contract:{}:{}", address.to_hex_literal(), suffix)
    }

    /// Helper method to create storage keys
    fn make_storage_key(&self, address: &SmartContractAddress, storage_key: &[u8]) -> String {
        format!("contract_storage:{}:{}", address.to_hex_literal(), hex::encode(storage_key))
    }

    /// Update contract cache with size management
    fn update_contract_cache(&self, address: SmartContractAddress, bytecode: Vec<u8>) -> Result<(), StorageError> {
        if let Ok(mut cache) = self.contract_cache.write() {
            if let Ok(mut current_size) = self.current_cache_size.write() {
                let new_size = bytecode.len();
                
                // Check if we need to make room
                while *current_size + new_size > self.cache_size_limit && !cache.is_empty() {
                    if let Some((_, removed_bytecode)) = cache.iter().next() {
                        let removed_size = removed_bytecode.len();
                        let key_to_remove = cache.keys().next().cloned();
                        if let Some(key) = key_to_remove {
                            cache.remove(&key);
                            *current_size = current_size.saturating_sub(removed_size);
                        }
                    } else {
                        break;
                    }
                }
                
                // Add new entry
                cache.insert(address, bytecode);
                *current_size += new_size;
            }
        }
        Ok(())
    }

    /// Get bytecode from contract cache
    fn get_from_contract_cache(&self, address: &SmartContractAddress) -> Option<Vec<u8>> {
        if let Ok(cache) = self.contract_cache.read() {
            cache.get(address).cloned()
        } else {
            None
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

    /// Store gas usage record for Kari gas accounting
    pub fn store_gas_usage(
        &self,
        gas_record: &GasUsageRecord,
    ) -> Result<(), StorageError> {
        let key = format!("gas_usage:{}:{}", 
                         gas_record.contract_address.to_hex_literal(),
                         gas_record.transaction_hash);
        let serialized = bincode::serialize(gas_record)?;
        
        debug!("Storing gas usage record for tx {}", gas_record.transaction_hash);
        self.storage.save_data(key.as_bytes(), &serialized)
    }

    /// Load gas usage records for a contract within a time range  
    pub fn load_gas_usage_records(
        &self,
        address: &SmartContractAddress,
        from_timestamp: u64,
        to_timestamp: u64,
    ) -> Result<Vec<GasUsageRecord>, StorageError> {
        let prefix = format!("gas_usage:{}:", address.to_hex_literal());
        let keys = self.storage.list_keys_with_prefix(prefix.as_bytes())?;
        let mut records = Vec::new();
        
        for key in keys {
            if let Some(data) = self.storage.load_data(&key)? {
                if let Ok(record) = bincode::deserialize::<GasUsageRecord>(&data) {
                    if record.timestamp >= from_timestamp && record.timestamp <= to_timestamp {
                        records.push(record);
                    }
                }
            }
        }
        
        debug!("Loaded {} gas usage records for contract {}", 
               records.len(), address.to_hex_literal());
        Ok(records)
    }

    /// Store Move resource data
    pub fn store_move_resource(
        &self,
        address: &SmartContractAddress,
        resource_info: &MoveResourceInfo,
    ) -> Result<(), StorageError> {
        let key = format!("move_resource:{}:{}:{}:{}", 
                         address.to_hex_literal(),
                         resource_info.module_name,
                         resource_info.struct_name,
                         resource_info.resource_type);
        let serialized = bincode::serialize(resource_info)?;
        
        debug!("Storing Move resource {} for contract {}", 
               resource_info.resource_type, address.to_hex_literal());
        self.storage.save_data(key.as_bytes(), &serialized)
    }

    /// Load Move resource data
    pub fn load_move_resource(
        &self,
        address: &SmartContractAddress,
        module_name: &str,
        struct_name: &str,
        resource_type: &str,
    ) -> Result<Option<MoveResourceInfo>, StorageError> {
        let key = format!("move_resource:{}:{}:{}:{}", 
                         address.to_hex_literal(),
                         module_name,
                         struct_name,
                         resource_type);
        
        debug!("Loading Move resource {} for contract {}", 
               resource_type, address.to_hex_literal());
        
        match self.storage.load_data(key.as_bytes())? {
            Some(data) => {
                let resource_info: MoveResourceInfo = bincode::deserialize(&data)?;
                Ok(Some(resource_info))
            },
            None => Ok(None),
        }
    }

    /// List all Move resources for a contract
    pub fn list_move_resources(
        &self,
        address: &SmartContractAddress,
    ) -> Result<Vec<MoveResourceInfo>, StorageError> {
        let prefix = format!("move_resource:{}:", address.to_hex_literal());
        let keys = self.storage.list_keys_with_prefix(prefix.as_bytes())?;
        let mut resources = Vec::new();
        
        for key in keys {
            if let Some(data) = self.storage.load_data(&key)? {
                if let Ok(resource) = bincode::deserialize::<MoveResourceInfo>(&data) {
                    resources.push(resource);
                }
            }
        }
        
        debug!("Found {} Move resources for contract {}", 
               resources.len(), address.to_hex_literal());
        Ok(resources)
    }

    /// Store execution trace for debugging and analysis
    pub fn store_execution_trace(
        &self,
        trace: &ExecutionTrace,
    ) -> Result<(), StorageError> {
        let key = format!("execution_trace:{}:{}", 
                         trace.contract_address.to_hex_literal(),
                         trace.transaction_hash);
        let serialized = bincode::serialize(trace)?;
        
        debug!("Storing execution trace for tx {} on contract {}", 
               trace.transaction_hash, trace.contract_address.to_hex_literal());
        self.storage.save_data(key.as_bytes(), &serialized)
    }

    /// Load execution trace by transaction hash
    pub fn load_execution_trace(
        &self,
        contract_address: &SmartContractAddress,
        transaction_hash: &str,
    ) -> Result<Option<ExecutionTrace>, StorageError> {
        let key = format!("execution_trace:{}:{}", 
                         contract_address.to_hex_literal(),
                         transaction_hash);
        
        debug!("Loading execution trace for tx {}", transaction_hash);
        
        match self.storage.load_data(key.as_bytes())? {
            Some(data) => {
                let trace: ExecutionTrace = bincode::deserialize(&data)?;
                Ok(Some(trace))
            },
            None => Ok(None),
        }
    }

    /// Get total Kari spent by a contract (gas accounting)
    pub fn get_contract_kari_spent(
        &self,
        address: &SmartContractAddress,
        from_timestamp: u64,
        to_timestamp: u64,
    ) -> Result<u64, StorageError> {
        let gas_records = self.load_gas_usage_records(address, from_timestamp, to_timestamp)?;
        let total_kari = gas_records.iter().map(|r| r.kari_amount).sum();
        
        debug!("Contract {} spent {} Kari between {} and {}", 
               address.to_hex_literal(), total_kari, from_timestamp, to_timestamp);
        Ok(total_kari)
    }

    /// Store Kari gas consumption for Move contract execution
    pub fn store_move_gas_consumption(
        &self,
        contract_address: &SmartContractAddress,
        transaction_hash: &str,
        function_name: &str,
        gas_used: u64,
        kari_spent: u64,
        execution_time_ms: u64,
    ) -> Result<(), StorageError> {
        let gas_record = GasUsageRecord {
            contract_address: contract_address.clone(),
            transaction_hash: transaction_hash.to_string(),
            operation_type: format!("move_function_call:{}", function_name),
            kari_amount: kari_spent,
            gas_price: if gas_used > 0 { kari_spent / gas_used } else { 0 },
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            block_height: 0, // Will be updated when integrated with blockchain
        };

        self.store_gas_usage(&gas_record)?;        // Store detailed execution metrics
        let metrics_key = format!("move_execution_metrics:{}:{}", 
                                 contract_address.to_hex_literal(),
                                 transaction_hash);
        let metrics = serde_json::json!({
            "function_name": function_name,
            "gas_used": gas_used,
            "kari_spent": kari_spent,
            "execution_time_ms": execution_time_ms,
            "gas_efficiency": if execution_time_ms > 0 { gas_used as f64 / execution_time_ms as f64 } else { 0.0 },
            "kari_efficiency": if execution_time_ms > 0 { kari_spent as f64 / execution_time_ms as f64 } else { 0.0 }
        });        let serialized = serde_json::to_vec(&metrics).map_err(|e| {
            StorageError::SerializationError(Box::new(bincode::ErrorKind::Custom(
                format!("JSON serialization error: {}", e)
            )))
        })?;

        self.storage.save_data(metrics_key.as_bytes(), &serialized)?;

        debug!("Stored Move gas consumption: {} gas, {} Kari for function {} on contract {}", 
               gas_used, kari_spent, function_name, contract_address.to_hex_literal());
        Ok(())
    }

    /// Load Move execution metrics for performance analysis
    pub fn load_move_execution_metrics(
        &self,
        contract_address: &SmartContractAddress,
        start_time: u64,
        end_time: u64,
    ) -> Result<Vec<u8>, StorageError> {
        let prefix = format!("move_execution_metrics:{}:", contract_address.to_hex_literal());
        let keys = self.storage.list_keys_with_prefix(prefix.as_bytes())?;
        
        let mut all_metrics = Vec::new();
        for key in keys {
            if let Some(data) = self.storage.load_data(&key)? {
                // Parse JSON to check timestamp range
                if let Ok(metrics_value) = serde_json::from_slice::<serde_json::Value>(&data) {
                    if let Some(timestamp) = metrics_value.get("timestamp").and_then(|v| v.as_u64()) {
                        if timestamp >= start_time && timestamp <= end_time {
                            all_metrics.extend_from_slice(&data);
                        }
                    }
                }
            }
        }
        
        Ok(all_metrics)
    }

    /// Store Move compilation artifacts (source code, ABI, dependencies)
    pub fn store_move_compilation_artifacts(
        &self,
        contract_address: &SmartContractAddress,
        source_code: &str,
        abi_data: &[u8],
        dependencies: &[String],
    ) -> Result<(), StorageError> {
        let artifacts = serde_json::json!({
            "source_code": source_code,
            "abi_data": abi_data,
            "dependencies": dependencies,
            "compiled_at": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        });        let key = format!("move_compilation_artifacts:{}", contract_address.to_hex_literal());
        let serialized = serde_json::to_vec(&artifacts).map_err(|e| {
            StorageError::SerializationError(Box::new(bincode::ErrorKind::Custom(
                format!("JSON serialization error: {}", e)
            )))
        })?;

        self.storage.save_data(key.as_bytes(), &serialized)?;
        debug!("Stored Move compilation artifacts for contract {}", contract_address.to_hex_literal());
        Ok(())
    }

    /// Load Move compilation artifacts
    pub fn load_move_compilation_artifacts(
        &self,
        contract_address: &SmartContractAddress,
    ) -> Result<Option<(String, Vec<u8>, Vec<String>)>, StorageError> {
        let key = format!("move_compilation_artifacts:{}", contract_address.to_hex_literal());
        
        if let Some(data) = self.storage.load_data(key.as_bytes())? {
            if let Ok(artifacts) = serde_json::from_slice::<serde_json::Value>(&data) {
                let source_code = artifacts.get("source_code")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                
                let abi_data = artifacts.get("abi_data")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect())
                    .unwrap_or_default();
                
                let dependencies = artifacts.get("dependencies")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default();
                
                return Ok(Some((source_code, abi_data, dependencies)));
            }
        }
        
        Ok(None)
    }

    /// Store Move resource snapshot for debugging and analysis
    pub fn store_move_resource_snapshot(
        &self,
        contract_address: &SmartContractAddress,
        resource_type: &str,
        resource_data: &[u8],
        timestamp: u64,
    ) -> Result<(), StorageError> {
        let key = format!("move_resource_snapshot:{}:{}:{}", 
                         contract_address.to_hex_literal(),
                         resource_type,
                         timestamp);
        
        let snapshot = serde_json::json!({
            "resource_type": resource_type,
            "resource_data": resource_data,
            "timestamp": timestamp,
            "contract_address": contract_address.to_hex_literal()
        });        let serialized = serde_json::to_vec(&snapshot).map_err(|e| {
            StorageError::SerializationError(Box::new(bincode::ErrorKind::Custom(
                format!("JSON serialization error: {}", e)
            )))
        })?;

        self.storage.save_data(key.as_bytes(), &serialized)?;
        debug!("Stored Move resource snapshot for contract {} resource {}", 
               contract_address.to_hex_literal(), resource_type);
        Ok(())
    }

    /// Get detailed Move contract gas analytics
    pub fn get_move_contract_gas_analytics(
        &self,
        contract_address: &SmartContractAddress,
        days: u32,
    ) -> Result<String, StorageError> {
        let end_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let start_time = end_time - (days as u64 * 24 * 60 * 60);

        let gas_records = self.load_gas_usage_records(contract_address, start_time, end_time)?;
        
        let total_gas = gas_records.iter().map(|r| r.kari_amount).sum::<u64>();
        let avg_gas = if !gas_records.is_empty() { total_gas / gas_records.len() as u64 } else { 0 };
        let max_gas = gas_records.iter().map(|r| r.kari_amount).max().unwrap_or(0);
        let min_gas = gas_records.iter().map(|r| r.kari_amount).min().unwrap_or(0);

        let analytics = serde_json::json!({
            "contract_address": contract_address.to_hex_literal(),
            "analysis_period_days": days,
            "total_transactions": gas_records.len(),
            "total_kari_spent": total_gas,
            "average_kari_per_transaction": avg_gas,
            "max_kari_per_transaction": max_gas,
            "min_kari_per_transaction": min_gas,
            "analysis_timestamp": end_time
        });        serde_json::to_string(&analytics).map_err(|e| {
            StorageError::SerializationError(Box::new(bincode::ErrorKind::Custom(
                format!("JSON serialization error: {}", e)
            )))
        })
    }
}
