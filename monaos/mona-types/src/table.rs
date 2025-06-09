// Table data structure for key-value storage
// Corresponds to `kanari_framework::table` module
use serde::{Deserialize, Serialize};
use crate::object::UID;
use std::collections::HashMap;

/// A homogeneous map-like collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Table<K, V>
where
    K: Clone + std::hash::Hash + Eq,
    V: Clone,
{
    pub id: UID,
    pub size: u64,
    pub data: HashMap<K, V>,
}

impl<K, V> Table<K, V>
where
    K: Clone + std::hash::Hash + Eq + serde::Serialize + serde::de::DeserializeOwned,
    V: Clone + serde::Serialize + serde::de::DeserializeOwned,
{
    /// Create a new empty table
    pub fn new(id: UID) -> Self {
        Self {
            id,
            size: 0,
            data: HashMap::new(),
        }
    }

    /// Add a key-value pair to the table
    pub fn add(&mut self, key: K, value: V) -> Result<(), TableError> {
        if self.data.contains_key(&key) {
            return Err(TableError::KeyAlreadyExists);
        }

        self.data.insert(key, value);
        self.size += 1;
        Ok(())
    }

    /// Borrow a value from the table
    pub fn borrow(&self, key: &K) -> Result<&V, TableError> {
        self.data.get(key).ok_or(TableError::KeyNotFound)
    }

    /// Mutably borrow a value from the table
    pub fn borrow_mut(&mut self, key: &K) -> Result<&mut V, TableError> {
        self.data.get_mut(key).ok_or(TableError::KeyNotFound)
    }

    /// Remove a key-value pair from the table
    pub fn remove(&mut self, key: &K) -> Result<V, TableError> {
        let value = self.data.remove(key).ok_or(TableError::KeyNotFound)?;
        self.size -= 1;
        Ok(value)
    }

    /// Check if the table contains a key
    pub fn contains(&self, key: &K) -> bool {
        self.data.contains_key(key)
    }

    /// Check if the table is empty
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Get the number of items in the table
    pub fn length(&self) -> u64 {
        self.size
    }

    /// Destroy the table, which must be empty
    pub fn destroy_empty(self) -> Result<(), TableError> {
        if !self.is_empty() {
            return Err(TableError::TableNotEmpty);
        }
        Ok(())
    }

    /// Get all keys in the table
    pub fn keys(&self) -> Vec<K> {
        self.data.keys().cloned().collect()
    }

    /// Get all values in the table
    pub fn values(&self) -> Vec<V> {
        self.data.values().cloned().collect()
    }

    /// Clear all items from the table
    pub fn clear(&mut self) {
        self.data.clear();
        self.size = 0;
    }

    /// Iterate over key-value pairs
    pub fn iter(&self) -> std::collections::hash_map::Iter<K, V> {
        self.data.iter()
    }
}

/// Linked table for ordered key-value storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedTable<K, V>
where
    K: Clone + std::hash::Hash + Eq,
    V: Clone,
{
    pub id: UID,
    pub size: u64,
    pub head: Option<K>,
    pub tail: Option<K>,
    pub data: HashMap<K, LinkedNode<K, V>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedNode<K, V> {
    pub value: V,
    pub next: Option<K>,
    pub prev: Option<K>,
}

impl<K, V> LinkedTable<K, V>
where
    K: Clone + std::hash::Hash + Eq + serde::Serialize + serde::de::DeserializeOwned,
    V: Clone + serde::Serialize + serde::de::DeserializeOwned,
{
    /// Create a new empty linked table
    pub fn new(id: UID) -> Self {
        Self {
            id,
            size: 0,
            head: None,
            tail: None,
            data: HashMap::new(),
        }
    }

    /// Push a key-value pair to the back
    pub fn push_back(&mut self, key: K, value: V) -> Result<(), TableError> {
        if self.data.contains_key(&key) {
            return Err(TableError::KeyAlreadyExists);
        }

        let node = LinkedNode {
            value,
            next: None,
            prev: self.tail.clone(),
        };

        if let Some(ref tail_key) = self.tail {
            if let Some(tail_node) = self.data.get_mut(tail_key) {
                tail_node.next = Some(key.clone());
            }
        }

        if self.head.is_none() {
            self.head = Some(key.clone());
        }

        self.tail = Some(key.clone());
        self.data.insert(key, node);
        self.size += 1;

        Ok(())
    }

    /// Push a key-value pair to the front
    pub fn push_front(&mut self, key: K, value: V) -> Result<(), TableError> {
        if self.data.contains_key(&key) {
            return Err(TableError::KeyAlreadyExists);
        }

        let node = LinkedNode {
            value,
            next: self.head.clone(),
            prev: None,
        };

        if let Some(ref head_key) = self.head {
            if let Some(head_node) = self.data.get_mut(head_key) {
                head_node.prev = Some(key.clone());
            }
        }

        if self.tail.is_none() {
            self.tail = Some(key.clone());
        }

        self.head = Some(key.clone());
        self.data.insert(key, node);
        self.size += 1;

        Ok(())
    }

    /// Get the front key
    pub fn front(&self) -> Option<&K> {
        self.head.as_ref()
    }

    /// Get the back key
    pub fn back(&self) -> Option<&K> {
        self.tail.as_ref()
    }

    /// Remove and return the front element
    pub fn pop_front(&mut self) -> Option<(K, V)> {
        let head_key = self.head.clone()?;
        let node = self.data.remove(&head_key)?;
        
        self.head = node.next.clone();
        if let Some(ref new_head) = self.head {
            if let Some(new_head_node) = self.data.get_mut(new_head) {
                new_head_node.prev = None;
            }
        } else {
            self.tail = None;
        }

        self.size -= 1;
        Some((head_key, node.value))
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Get length
    pub fn length(&self) -> u64 {
        self.size
    }
}

/// Table operation errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TableError {
    #[error("Key already exists in table")]
    KeyAlreadyExists,
    #[error("Key not found in table")]
    KeyNotFound,
    #[error("Table is not empty")]
    TableNotEmpty,
    #[error("Invalid operation")]
    InvalidOperation,
}

pub type TableResult<T> = Result<T, TableError>;
