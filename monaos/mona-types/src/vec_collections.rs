// VecMap and VecSet data structures
// Corresponds to `kanari_framework::vec_map` and `kanari_framework::vec_set` modules
use serde::{Deserialize, Serialize};

/// A map data structure backed by a vector
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VecMap<K, V> {
    pub contents: Vec<Entry<K, V>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry<K, V> {
    pub key: K,
    pub value: V,
}

impl<K, V> VecMap<K, V>
where
    K: Clone + PartialEq + serde::Serialize + serde::de::DeserializeOwned,
    V: Clone + serde::Serialize + serde::de::DeserializeOwned,
{
    /// Create a new empty VecMap
    pub fn empty() -> Self {
        Self {
            contents: Vec::new(),
        }
    }

    /// Insert a key-value pair
    pub fn insert(&mut self, key: K, value: V) {
        if let Some(entry) = self.contents.iter_mut().find(|entry| entry.key == key) {
            entry.value = value;
        } else {
            self.contents.push(Entry { key, value });
        }
    }

    /// Remove a key-value pair
    pub fn remove(&mut self, key: &K) -> Option<(K, V)> {
        if let Some(index) = self.contents.iter().position(|entry| &entry.key == key) {
            let entry = self.contents.remove(index);
            Some((entry.key, entry.value))
        } else {
            None
        }
    }

    /// Get a value by key
    pub fn get(&self, key: &K) -> Option<&V> {
        self.contents
            .iter()
            .find(|entry| &entry.key == key)
            .map(|entry| &entry.value)
    }

    /// Get a mutable value by key
    pub fn get_mut(&mut self, key: &K) -> Option<&mut V> {
        self.contents
            .iter_mut()
            .find(|entry| &entry.key == key)
            .map(|entry| &mut entry.value)
    }

    /// Check if the map contains a key
    pub fn contains(&self, key: &K) -> bool {
        self.contents.iter().any(|entry| &entry.key == key)
    }

    /// Get the size of the map
    pub fn size(&self) -> u64 {
        self.contents.len() as u64
    }

    /// Check if the map is empty
    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    /// Destroy the map and return its contents
    pub fn into_keys_values(self) -> (Vec<K>, Vec<V>) {
        let mut keys = Vec::new();
        let mut values = Vec::new();
        
        for entry in self.contents {
            keys.push(entry.key);
            values.push(entry.value);
        }
        
        (keys, values)
    }

    /// Get all keys
    pub fn keys(&self) -> Vec<K> {
        self.contents.iter().map(|entry| entry.key.clone()).collect()
    }

    /// Get all values
    pub fn values(&self) -> Vec<V> {
        self.contents.iter().map(|entry| entry.value.clone()).collect()
    }
}

/// A set data structure backed by a vector
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VecSet<K> {
    pub contents: Vec<K>,
}

impl<K> VecSet<K>
where
    K: Clone + PartialEq + serde::Serialize + serde::de::DeserializeOwned,
{
    /// Create a new empty VecSet
    pub fn empty() -> Self {
        Self {
            contents: Vec::new(),
        }
    }

    /// Insert an element
    pub fn insert(&mut self, key: K) {
        if !self.contents.contains(&key) {
            self.contents.push(key);
        }
    }

    /// Remove an element
    pub fn remove(&mut self, key: &K) -> bool {
        if let Some(index) = self.contents.iter().position(|k| k == key) {
            self.contents.remove(index);
            true
        } else {
            false
        }
    }

    /// Check if the set contains an element
    pub fn contains(&self, key: &K) -> bool {
        self.contents.contains(key)
    }

    /// Get the size of the set
    pub fn size(&self) -> u64 {
        self.contents.len() as u64
    }

    /// Check if the set is empty
    pub fn is_empty(&self) -> bool {
        self.contents.is_empty()
    }

    /// Convert to vector
    pub fn into_keys(self) -> Vec<K> {
        self.contents
    }

    /// Get all keys as a reference
    pub fn keys(&self) -> &Vec<K> {
        &self.contents
    }

    /// Check if this set is a subset of another
    pub fn is_subset(&self, other: &VecSet<K>) -> bool {
        self.contents.iter().all(|key| other.contains(key))
    }

    /// Union with another set
    pub fn union(&self, other: &VecSet<K>) -> VecSet<K> {
        let mut result = self.clone();
        for key in &other.contents {
            result.insert(key.clone());
        }
        result
    }

    /// Intersection with another set
    pub fn intersection(&self, other: &VecSet<K>) -> VecSet<K> {
        let mut result = VecSet::empty();
        for key in &self.contents {
            if other.contains(key) {
                result.insert(key.clone());
            }
        }
        result
    }
}

/// Priority queue implementation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriorityQueue<T> {
    pub entries: Vec<Entry<u64, T>>, // priority, value pairs
}

impl<T> PriorityQueue<T>
where
    T: Clone + serde::Serialize + serde::de::DeserializeOwned,
{
    /// Create a new empty priority queue
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Insert an element with priority
    pub fn insert(&mut self, priority: u64, value: T) {
        let entry = Entry {
            key: priority,
            value,
        };

        // Insert in sorted order (highest priority first)
        let pos = self
            .entries
            .binary_search_by(|e| e.key.cmp(&priority).reverse())
            .unwrap_or_else(|e| e);
        
        self.entries.insert(pos, entry);
    }

    /// Remove and return the highest priority element
    pub fn pop_max(&mut self) -> Option<(u64, T)> {
        if self.entries.is_empty() {
            None
        } else {
            let entry = self.entries.remove(0);
            Some((entry.key, entry.value))
        }
    }

    /// Peek at the highest priority element
    pub fn peek_max(&self) -> Option<(&u64, &T)> {
        self.entries.first().map(|entry| (&entry.key, &entry.value))
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get the size
    pub fn size(&self) -> u64 {
        self.entries.len() as u64
    }
}

impl<T> Default for PriorityQueue<T>
where
    T: Clone + serde::Serialize + serde::de::DeserializeOwned,
{
    fn default() -> Self {
        Self::new()
    }
}
