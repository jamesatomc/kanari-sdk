// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak,
};

use rocksdb::{DB, IteratorMode, WriteBatch};

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
type RawKeyValue = (Vec<u8>, Vec<u8>);

/// One genesis lock per shared RocksDB handle. `open_or_get_db` returns one
/// `Arc<DB>` for repeated opens of a path, so all wrappers around that DB share
/// this lock without serializing unrelated databases.
static DB_GENESIS_LOCKS: OnceLock<Mutex<HashMap<usize, Weak<Mutex<()>>>>> = OnceLock::new();

fn genesis_lock_for_db(db: &Arc<DB>) -> Arc<Mutex<()>> {
    let key = Arc::as_ptr(db) as usize;
    let registry = DB_GENESIS_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = registry.get(&key).and_then(Weak::upgrade) {
        return lock;
    }

    let lock = Arc::new(Mutex::new(()));
    registry.insert(key, Arc::downgrade(&lock));
    lock
}

/// Lightweight persistent BCS-backed store for runtime state using RocksDB.
#[derive(Debug)]
pub struct PersistentStore {
    db: Option<Arc<DB>>,
    // In-memory store for when RocksDB is not used (e.g. tests, Miri)
    memory_store: Option<MemoryStore>,
    transaction_lock: Mutex<()>,
    // Serializes first-time genesis check, initialization, and commit for the
    // underlying database without blocking unrelated databases in this process.
    genesis_init_lock: Arc<Mutex<()>>,
}

impl PersistentStore {
    /// Serialize read-modify-write transactions that maintain secondary indexes.
    pub(crate) fn transaction_guard(&self) -> MutexGuard<'_, ()> {
        self.transaction_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Serialize first-time genesis initialization for this store's database.
    pub(crate) fn genesis_init_guard(&self) -> MutexGuard<'_, ()> {
        self.genesis_init_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Open default store path (same behavior as previous implementation).
    pub fn open_default() -> Result<Self> {
        let db_path = std::env::var("KANARI_STATE_DB").ok().map(PathBuf::from);
        Self::open_with_path(db_path)
    }

    /// Open store using an explicit path (None -> default).
    pub fn open_with_path(path_opt: Option<PathBuf>) -> Result<Self> {
        let db = kanari_db_common::open_or_get_db(path_opt)?;
        let genesis_init_lock = genesis_lock_for_db(&db);
        Ok(PersistentStore {
            db: Some(db),
            memory_store: None,
            transaction_lock: Mutex::new(()),
            genesis_init_lock,
        })
    }

    /// Open an in-memory-only store that performs no filesystem operations.
    /// Useful for tests and Miri where disk APIs may be unavailable.
    pub fn open_in_memory() -> Result<Self> {
        Ok(PersistentStore {
            db: None,
            memory_store: Some(Arc::new(RwLock::new(HashMap::new()))),
            transaction_lock: Mutex::new(()),
            genesis_init_lock: Arc::new(Mutex::new(())),
        })
    }

    fn read_memory_store(store: &MemoryStore) -> RwLockReadGuard<'_, HashMap<Vec<u8>, Vec<u8>>> {
        store
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_memory_store(store: &MemoryStore) -> RwLockWriteGuard<'_, HashMap<Vec<u8>, Vec<u8>>> {
        store
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn read_raw(&self, key: &[u8]) -> std::result::Result<Option<Vec<u8>>, PersistentStoreError> {
        if let Some(db) = &self.db {
            return Ok(db.get(key)?);
        }

        if let Some(store) = &self.memory_store {
            return Ok(Self::read_memory_store(store).get(key).cloned());
        }

        Ok(None)
    }

    fn write_raw(
        &self,
        key: &[u8],
        value: Vec<u8>,
    ) -> std::result::Result<(), PersistentStoreError> {
        if let Some(db) = &self.db {
            db.put(key, value)?;
        } else if let Some(store) = &self.memory_store {
            Self::write_memory_store(store).insert(key.to_vec(), value);
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
        self.write_raw(key, bytes)
    }

    /// Load a deserializable value from `key`.
    pub fn load<T: DeserializeOwned>(
        &self,
        key: &[u8],
    ) -> std::result::Result<Option<T>, PersistentStoreError> {
        match self.read_raw(key)? {
            Some(bytes) => Ok(Some(bcs::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Delete a key from the store.
    pub fn delete(&self, key: &[u8]) -> std::result::Result<(), PersistentStoreError> {
        if let Some(db) = &self.db {
            db.delete(key)?;
        } else if let Some(store) = &self.memory_store {
            Self::write_memory_store(store).remove(key);
        }
        Ok(())
    }

    /// Return a stable snapshot of logical state entries, excluding SMT internals.
    pub fn logical_entries(&self) -> std::result::Result<Vec<RawKeyValue>, PersistentStoreError> {
        let mut entries = Vec::new();

        if let Some(db) = &self.db {
            for item in db.iterator(IteratorMode::Start) {
                let (key, value) = item?;
                if Self::is_internal_smt_key(&key) {
                    continue;
                }
                entries.push((key.to_vec(), value.to_vec()));
            }
        } else if let Some(store) = &self.memory_store {
            entries.extend(
                Self::read_memory_store(store)
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries)
    }

    /// Apply raw state key updates/deletes atomically where the backend supports it.
    pub fn apply_raw_changes(
        &self,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
    ) -> std::result::Result<(), PersistentStoreError> {
        if let Some(db) = &self.db {
            let mut batch = WriteBatch::default();
            for (key, value) in updates {
                batch.put(key, value);
            }
            for key in deletes {
                batch.delete(key);
            }
            db.write(batch)?;
        } else if let Some(store) = &self.memory_store {
            let mut guard = Self::write_memory_store(store);
            for (key, value) in updates {
                guard.insert(key.clone(), value.clone());
            }
            for key in deletes {
                guard.remove(key);
            }
        }
        Ok(())
    }

    /// Expose underlying RocksDB instance for other components (e.g. SMT)
    pub fn get_db(&self) -> Option<Arc<DB>> {
        self.db.clone()
    }

    /// Flush pending RocksDB writes before a node snapshot or clean shutdown.
    pub fn flush(&self) -> std::result::Result<(), PersistentStoreError> {
        if let Some(db) = &self.db {
            db.flush()?;
        }
        Ok(())
    }

    /// Request a full-range compaction. This only reclaims obsolete RocksDB
    /// versions; it never prunes canonical blockchain keys.
    pub fn compact(&self) {
        if let Some(db) = &self.db {
            db.compact_range::<&[u8], &[u8]>(None, None);
        }
    }

    /// Read a RocksDB string property for benchmark/operational diagnostics.
    pub fn rocksdb_property(
        &self,
        name: &str,
    ) -> std::result::Result<Option<String>, PersistentStoreError> {
        let Some(db) = &self.db else {
            return Ok(None);
        };
        Ok(db.property_value(name)?)
    }

    /// Read a RocksDB integer property for benchmark/operational diagnostics.
    pub fn rocksdb_int_property(
        &self,
        name: &str,
    ) -> std::result::Result<Option<u64>, PersistentStoreError> {
        let Some(db) = &self.db else {
            return Ok(None);
        };
        Ok(db.property_int_value(name)?)
    }

    fn is_internal_smt_key(key: &[u8]) -> bool {
        key.starts_with(b"n:") || key.starts_with(b"d:")
    }
}
