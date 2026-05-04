// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::storage::shared_db::get_or_open_db;
use rocksdb::DB;

/// Custom error type for PersistentStore operations
#[derive(Debug)]
pub enum PersistentStoreError {
    RocksDB(rocksdb::Error),
    Serialization(bcs::Error),
    Internal(String),
    Io(std::io::Error),
}

impl std::fmt::Display for PersistentStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistentStoreError::RocksDB(e) => write!(f, "RocksDB error: {}", e),
            PersistentStoreError::Serialization(e) => write!(f, "Serialization error: {}", e),
            PersistentStoreError::Internal(e) => write!(f, "Internal error: {}", e),
            PersistentStoreError::Io(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for PersistentStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PersistentStoreError::RocksDB(e) => Some(e),
            PersistentStoreError::Serialization(e) => Some(e),
            PersistentStoreError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rocksdb::Error> for PersistentStoreError {
    fn from(e: rocksdb::Error) -> Self {
        PersistentStoreError::RocksDB(e)
    }
}

impl From<bcs::Error> for PersistentStoreError {
    fn from(e: bcs::Error) -> Self {
        PersistentStoreError::Serialization(e)
    }
}

impl From<std::io::Error> for PersistentStoreError {
    fn from(e: std::io::Error) -> Self {
        PersistentStoreError::Io(e)
    }
}

/// Type alias for in-memory store to reduce complexity
type MemoryStore = Arc<RwLock<HashMap<Vec<u8>, Vec<u8>>>>;

/// Lightweight persistent BCS-backed store for runtime state using RocksDB.
#[derive(Debug)]
pub struct PersistentStore {
    db: Option<Arc<DB>>,
    // In-memory fallback for when RocksDB is not used (e.g. tests, Miri)
    memory_store: Option<MemoryStore>,
}

impl PersistentStore {
    /// Open default store path (same behavior as previous implementation).
    pub fn open_default() -> Result<Self> {
        let db_path = std::env::var("KANARI_STATE_DB").ok().map(PathBuf::from);
        Self::open_with_path(db_path)
    }

    /// Open store using an explicit path (None -> default).
    pub fn open_with_path(path_opt: Option<PathBuf>) -> Result<Self> {
        let db = get_or_open_db(path_opt)?;
        Ok(PersistentStore {
            db: Some(db),
            memory_store: None,
        })
    }

    /// Open an in-memory-only store that performs no filesystem operations.
    /// Useful for tests and Miri where disk APIs may be unavailable.
    pub fn open_in_memory() -> Result<Self> {
        Ok(PersistentStore {
            db: None,
            memory_store: Some(Arc::new(RwLock::new(HashMap::new()))),
        })
    }

    /// Save raw bytes under `key`.
    pub fn save_raw(
        &self,
        key: &[u8],
        value: &[u8],
    ) -> std::result::Result<(), PersistentStoreError> {
        if let Some(db) = &self.db {
            db.put(key, value)?;
        } else if let Some(store) = &self.memory_store {
            store
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key.to_vec(), value.to_vec());
        }
        Ok(())
    }

    /// Save a serializable value under `key`.
    pub fn save<T: Serialize + ?Sized>(
        &self,
        key: &[u8],
        value: &T,
    ) -> std::result::Result<(), PersistentStoreError> {
        let bytes = bcs::to_bytes(value)?;

        if let Some(db) = &self.db {
            db.put(key, &bytes)?;
        } else if let Some(store) = &self.memory_store {
            store
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key.to_vec(), bytes);
        }

        Ok(())
    }

    /// Load a deserializable value from `key`.
    pub fn load<T: DeserializeOwned>(
        &self,
        key: &[u8],
    ) -> std::result::Result<Option<T>, PersistentStoreError> {
        if let Some(db) = &self.db {
            match db.get(key)? {
                Some(bytes) => {
                    let value = bcs::from_bytes(&bytes)?;
                    Ok(Some(value))
                }
                None => Ok(None),
            }
        } else if let Some(store) = &self.memory_store {
            let guard = store.read().unwrap_or_else(|e| e.into_inner());
            match guard.get(key) {
                Some(bytes) => {
                    let value = bcs::from_bytes(bytes)?;
                    Ok(Some(value))
                }
                None => Ok(None),
            }
        } else {
            Ok(None)
        }
    }

    /// Flush all pending writes to the backing store synchronously.
    pub fn flush(&self) -> std::result::Result<(), PersistentStoreError> {
        // RocksDB writes are synchronous in this implementation
        // In-memory writes are immediate
        Ok(())
    }

    /// Delete a key from the store.
    pub fn delete(&self, key: &[u8]) -> std::result::Result<(), PersistentStoreError> {
        if let Some(db) = &self.db {
            db.delete(key)?;
        } else if let Some(store) = &self.memory_store {
            store.write().unwrap_or_else(|e| e.into_inner()).remove(key);
        }
        Ok(())
    }

    /// Expose underlying RocksDB instance for other components (e.g. SMT)
    pub fn get_db(&self) -> Option<Arc<DB>> {
        self.db.clone()
    }

    /// Apply a write batch atomically.
    pub fn apply_batch(
        &self,
        batch: rocksdb::WriteBatch,
    ) -> std::result::Result<(), PersistentStoreError> {
        if let Some(db) = &self.db {
            db.write(batch)?;
        }
        // Note: In-memory batch application is not supported directly via RocksDB batch type
        // For in-memory, callers should use save_raw individually or implement a custom batch
        Ok(())
    }
}
