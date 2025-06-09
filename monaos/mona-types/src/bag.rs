// Dynamic bag data structure
// Corresponds to `kanari_framework::bag` module
use serde::{Deserialize, Serialize};
use crate::object::UID;
use std::collections::HashMap;

/// A heterogeneous map-like collection that can store values of different types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bag {
    pub id: UID,
    pub size: u64,
    // In a real implementation, this would use dynamic fields
    // For simulation, we use HashMap with serialized values
    pub data: HashMap<String, Vec<u8>>, // key -> serialized value
}

impl Bag {
    /// Create a new empty bag
    pub fn new(id: UID) -> Self {
        Self {
            id,
            size: 0,
            data: HashMap::new(),
        }
    }

    /// Add a key-value pair to the bag
    pub fn add<K, V>(&mut self, key: K, value: V) -> Result<(), BagError>
    where
        K: ToString,
        V: serde::Serialize,
    {
        let key_str = key.to_string();
        
        if self.data.contains_key(&key_str) {
            return Err(BagError::KeyAlreadyExists);
        }

        let serialized = bcs::to_bytes(&value)
            .map_err(|_| BagError::SerializationFailed)?;
        
        self.data.insert(key_str, serialized);
        self.size += 1;
        
        Ok(())
    }

    /// Borrow a value from the bag
    pub fn borrow<K, V>(&self, key: K) -> Result<V, BagError>
    where
        K: ToString,
        V: serde::de::DeserializeOwned,
    {
        let key_str = key.to_string();
        let data = self.data.get(&key_str)
            .ok_or(BagError::KeyNotFound)?;
        
        bcs::from_bytes(data)
            .map_err(|_| BagError::DeserializationFailed)
    }

    /// Remove a key-value pair from the bag
    pub fn remove<K, V>(&mut self, key: K) -> Result<V, BagError>
    where
        K: ToString,
        V: serde::de::DeserializeOwned,
    {
        let key_str = key.to_string();
        let data = self.data.remove(&key_str)
            .ok_or(BagError::KeyNotFound)?;
        
        self.size -= 1;
        
        bcs::from_bytes(&data)
            .map_err(|_| BagError::DeserializationFailed)
    }

    /// Check if the bag contains a key
    pub fn contains<K>(&self, key: K) -> bool
    where
        K: ToString,
    {
        self.data.contains_key(&key.to_string())
    }

    /// Check if the bag is empty
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Get the number of items in the bag
    pub fn length(&self) -> u64 {
        self.size
    }

    /// Destroy the bag, which must be empty
    pub fn destroy_empty(self) -> Result<(), BagError> {
        if !self.is_empty() {
            return Err(BagError::BagNotEmpty);
        }
        Ok(())
    }

    /// Get all keys in the bag
    pub fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    /// Clear all items from the bag
    pub fn clear(&mut self) {
        self.data.clear();
        self.size = 0;
    }
}

/// Bag operation errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BagError {
    #[error("Key already exists in bag")]
    KeyAlreadyExists,
    #[error("Key not found in bag")]
    KeyNotFound,
    #[error("Bag is not empty")]
    BagNotEmpty,
    #[error("Serialization failed")]
    SerializationFailed,
    #[error("Deserialization failed")]
    DeserializationFailed,
    #[error("Type mismatch")]
    TypeMismatch,
}

pub type BagResult<T> = Result<T, BagError>;
