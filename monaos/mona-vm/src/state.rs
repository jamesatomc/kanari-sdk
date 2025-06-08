//! State management for smart contracts

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Serialize, Deserialize};
use log::{debug, error};

use mona_storage::{BlockchainStorage, RocksDBStorage};
use mona_types::address::Address;
use crate::types::{ContractAddress, VMResult, VMError};

/// Contract state information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractState {
    /// Contract address
    pub address: ContractAddress,
    /// Contract bytecode
    pub bytecode: Vec<u8>,
    /// Contract storage (key-value pairs)
    pub storage: HashMap<Vec<u8>, Vec<u8>>,
    /// Contract balance in KA
    pub balance: u64,
    /// Contract metadata
    pub metadata: ContractMetadata,
}

/// Contract metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractMetadata {
    /// Module name
    pub module_name: String,
    /// Deployer address
    pub deployer: Address,
    /// Deployment timestamp
    pub deployed_at: u64,
    /// Contract version
    pub version: String,
    /// Available public functions
    pub public_functions: Vec<String>,
}

/// State manager for managing contract states
pub struct StateManager {
    /// In-memory contract cache
    contracts: HashMap<ContractAddress, ContractState>,
    /// Persistent storage backend
    storage: Arc<RocksDBStorage>,
    /// Read-write lock for thread safety
    lock: Arc<RwLock<()>>,
}

impl StateManager {
    /// Create a new state manager
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self {
            contracts: HashMap::new(),
            storage,
            lock: Arc::new(RwLock::new(())),
        }
    }

    /// Check if a contract exists
    pub fn contract_exists(&self, address: ContractAddress) -> VMResult<bool> {
        let _lock = self.lock.read().unwrap();
        
        // Check in-memory cache first
        if self.contracts.contains_key(&address) {
            return Ok(true);
        }

        // Check persistent storage
        let key = format!("contract:{}", address.to_hex_literal());
        match self.storage.load_data(key.as_bytes()) {
            Ok(Some(_)) => Ok(true),
            Ok(None) => Ok(false),
            Err(e) => Err(VMError::StorageError {
                message: format!("Failed to check contract existence: {}", e),
            }),
        }
    }

    /// Create a new contract state
    pub fn create_contract_state(&mut self, address: ContractAddress) -> VMResult<()> {
        let _lock = self.lock.write().unwrap();

        if self.contracts.contains_key(&address) {
            return Err(VMError::ContractError {
                message: "Contract already exists".to_string(),
            });
        }

        let state = ContractState {
            address,
            bytecode: Vec::new(),
            storage: HashMap::new(),
            balance: 0,
            metadata: ContractMetadata {
                module_name: String::new(),
                deployer: Address::zero(),
                deployed_at: 0,
                version: "1.0.0".to_string(),
                public_functions: Vec::new(),
            },
        };

        self.contracts.insert(address, state);
        debug!("Created contract state for address: {}", address.to_hex_literal());
        Ok(())
    }

    /// Get contract state
    pub fn get_contract_state(&self, address: ContractAddress) -> VMResult<Option<ContractState>> {
        let _lock = self.lock.read().unwrap();

        // Check in-memory cache first
        if let Some(state) = self.contracts.get(&address) {
            return Ok(Some(state.clone()));
        }

        // Load from persistent storage
        let key = format!("contract:{}", address.to_hex_literal());
        match self.storage.load_data(key.as_bytes()) {
            Ok(Some(data)) => {
                match bincode::deserialize::<ContractState>(&data) {
                    Ok(state) => Ok(Some(state)),
                    Err(e) => Err(VMError::StorageError {
                        message: format!("Failed to deserialize contract state: {}", e),
                    }),
                }
            },
            Ok(None) => Ok(None),
            Err(e) => Err(VMError::StorageError {
                message: format!("Failed to load contract state: {}", e),
            }),
        }
    }

    /// Update contract state
    pub fn update_contract_state(&mut self, state: ContractState) -> VMResult<()> {
        let _lock = self.lock.write().unwrap();

        let address = state.address;
        
        // Update in-memory cache
        self.contracts.insert(address, state.clone());

        // Persist to storage
        let key = format!("contract:{}", address.to_hex_literal());
        match bincode::serialize(&state) {
            Ok(data) => {
                if let Err(e) = self.storage.save_data(key.as_bytes(), &data) {
                    error!("Failed to persist contract state: {}", e);
                    return Err(VMError::StorageError {
                        message: format!("Failed to persist contract state: {}", e),
                    });
                }
            },
            Err(e) => {
                return Err(VMError::StorageError {
                    message: format!("Failed to serialize contract state: {}", e),
                });
            }
        }

        debug!("Updated contract state for address: {}", address.to_hex_literal());
        Ok(())
    }

    /// Get contract storage value
    pub fn get_storage(&self, address: ContractAddress, key: &[u8]) -> VMResult<Option<Vec<u8>>> {
        if let Some(state) = self.get_contract_state(address)? {
            Ok(state.storage.get(key).cloned())
        } else {
            Ok(None)
        }
    }

    /// Set contract storage value
    pub fn set_storage(&mut self, address: ContractAddress, key: Vec<u8>, value: Vec<u8>) -> VMResult<()> {
        let mut state = self.get_contract_state(address)?
            .ok_or_else(|| VMError::ContractNotFound {
                address: address.to_string(),
            })?;

        state.storage.insert(key, value);
        self.update_contract_state(state)
    }

    /// Delete contract storage value
    pub fn delete_storage(&mut self, address: ContractAddress, key: &[u8]) -> VMResult<()> {
        let mut state = self.get_contract_state(address)?
            .ok_or_else(|| VMError::ContractNotFound {
                address: address.to_string(),
            })?;

        state.storage.remove(key);
        self.update_contract_state(state)
    }

    /// Get contract balance
    pub fn get_balance(&self, address: ContractAddress) -> VMResult<u64> {
        if let Some(state) = self.get_contract_state(address)? {
            Ok(state.balance)
        } else {
            Err(VMError::ContractNotFound {
                address: address.to_string(),
            })
        }
    }

    /// Update contract balance
    pub fn set_balance(&mut self, address: ContractAddress, balance: u64) -> VMResult<()> {
        let mut state = self.get_contract_state(address)?
            .ok_or_else(|| VMError::ContractNotFound {
                address: address.to_string(),
            })?;

        state.balance = balance;
        self.update_contract_state(state)
    }

    /// Get all contracts (for debugging/monitoring)
    pub fn get_all_contracts(&self) -> Vec<ContractAddress> {
        let _lock = self.lock.read().unwrap();
        self.contracts.keys().cloned().collect()
    }

    /// Get storage size for a contract
    pub fn get_storage_size(&self, address: ContractAddress) -> VMResult<u64> {
        if let Some(state) = self.get_contract_state(address)? {
            let size = state.storage.iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>() as u64;
            Ok(size)
        } else {
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::get_kari_dir;

    fn setup_test_storage() -> Arc<RocksDBStorage> {
        let mut test_dir = get_kari_dir();
        test_dir.push("test_vm_storage");
        Arc::new(RocksDBStorage::new(test_dir).unwrap())
    }    #[test]
    fn test_contract_state_creation() {
        let storage = setup_test_storage();
        let mut state_manager = StateManager::new(storage);
        let address = Address::from_hex_literal("0x1234567890123456789012345678901234567890123456789012345678901234").unwrap();

        assert!(!state_manager.contract_exists(address).unwrap());
        
        state_manager.create_contract_state(address).unwrap();
        assert!(state_manager.contract_exists(address).unwrap());

        let state = state_manager.get_contract_state(address).unwrap().unwrap();
        assert_eq!(state.address, address);
        assert_eq!(state.balance, 0);
    }

    #[test]
    fn test_contract_storage() {
        let storage = setup_test_storage();
        let mut state_manager = StateManager::new(storage);
        let address = Address::zero();

        state_manager.create_contract_state(address).unwrap();

        let key = b"test_key".to_vec();
        let value = b"test_value".to_vec();

        // Set storage
        state_manager.set_storage(address, key.clone(), value.clone()).unwrap();

        // Get storage
        let retrieved = state_manager.get_storage(address, &key).unwrap().unwrap();
        assert_eq!(retrieved, value);

        // Delete storage
        state_manager.delete_storage(address, &key).unwrap();
        let deleted = state_manager.get_storage(address, &key).unwrap();
        assert!(deleted.is_none());
    }
}
