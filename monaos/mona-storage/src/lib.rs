use std::{fs, path::PathBuf, time::Duration};
use thiserror::Error;
use rocksdb::{DB, Error as RocksError, Options};
use bincode;
use log::{debug, info, warn, error};
pub mod file_storage;
pub mod smart_contract;

pub use file_storage::{
    FileStorage,
    StorageError2,
    FileMetadata
};

pub use smart_contract::{
    SmartContractStorage,
    SmartContractAddress,
    SmartContractMetadata,
    MoveModuleInfo,
    ContractStateSnapshot,
    SmartContractEvent,
};

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Database error: {0}")]
    DbError(#[from] RocksError),
    #[error("Serialization error: {0}")]
    SerializationError(#[from] bincode::Error),
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Lock file error: {0}")]
    LockFileError(String),
    #[error("DB initialization failed after {0} retries")]
    InitializationError(u32),
}

pub trait BlockchainStorage {
    fn save_data(&self, key: &[u8], value: &[u8]) -> Result<(), StorageError>;
    fn load_data(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError>;
    fn flush(&self) -> Result<(), StorageError>;
    fn delete_data(&self, key: &[u8]) -> Result<(), StorageError>;
    // New method to list keys with a prefix
    fn list_keys_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Vec<u8>>, StorageError>;
}

pub struct RocksDBStorage {
    db: DB,
    path: PathBuf,
}

impl RocksDBStorage {
    pub fn new(path: PathBuf) -> Result<Self, StorageError> {
        const MAX_RETRIES: u32 = 5;
        let mut backoff = Duration::from_millis(100);
        
        info!("Initializing RocksDB at: {:?}", path);
        let mut attempts = 0;
        while attempts < MAX_RETRIES {
            // Cleanup any stale lock files
            let lock_path = path.join("LOCK");
            if lock_path.exists() {
                debug!("Found stale lock file, attempting to remove");
                match fs::remove_file(&lock_path) {
                    Ok(_) => info!("Successfully removed stale lock file"),
                    Err(e) => {
                        warn!("Failed to remove lock file: {}", e);
                        // Continue anyway, the DB open might succeed
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }

            // Configure and open DB with optimized settings
            let mut opts = Options::default();
            opts.create_if_missing(true);
            opts.set_keep_log_file_num(1);
            opts.set_max_open_files(10);
            opts.set_use_fsync(true);
            opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
            opts.set_compaction_style(rocksdb::DBCompactionStyle::Level);
            
            match DB::open(&opts, &path) {
                Ok(db) => {
                    info!("RocksDB successfully opened at {:?}", path);
                    return Ok(Self { db, path });
                },
                Err(e) => {
                    attempts += 1;
                    warn!("Failed to open DB (attempt {}/{}): {}", attempts, MAX_RETRIES, e);
                    if attempts < MAX_RETRIES {
                        std::thread::sleep(backoff);
                        backoff *= 2; // Exponential backoff
                    }
                }
            }
        }

        error!("Failed to initialize RocksDB after {} attempts", MAX_RETRIES);
        Err(StorageError::InitializationError(MAX_RETRIES))
    }
    
    // Get the path to the database
    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for RocksDBStorage {
    fn drop(&mut self) {
        debug!("Flushing RocksDB before dropping");
        if let Err(e) = self.db.flush() {
            error!("Error flushing DB during drop: {}", e);
        }
    }
}

impl BlockchainStorage for RocksDBStorage {
    fn save_data(&self, key: &[u8], value: &[u8]) -> Result<(), StorageError> {
        debug!("Saving data with key of {} bytes", key.len());
        match self.db.put(key, value) {
            Ok(_) => {
                debug!("Successfully saved {} bytes of data", value.len());
                Ok(())
            },
            Err(e) => {
                error!("Failed to save data: {}", e);
                Err(StorageError::DbError(e))
            }
        }
    }

    fn load_data(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        debug!("Loading data with key of {} bytes", key.len());
        match self.db.get(key) {
            Ok(Some(data)) => {
                debug!("Successfully loaded {} bytes of data", data.len());
                Ok(Some(data))
            },
            Ok(None) => {
                debug!("No data found for key");
                Ok(None)
            },
            Err(e) => {
                error!("Failed to load data: {}", e);
                Err(StorageError::DbError(e))
            }
        }
    }

    fn flush(&self) -> Result<(), StorageError> {
        debug!("Flushing database to disk");
        match self.db.flush() {
            Ok(_) => {
                debug!("Database successfully flushed");
                Ok(())
            },
            Err(e) => {
                error!("Failed to flush database: {}", e);
                Err(StorageError::DbError(e))
            }
        }
    }
    
    fn delete_data(&self, key: &[u8]) -> Result<(), StorageError> {
        debug!("Deleting data with key of {} bytes", key.len());
        match self.db.delete(key) {
            Ok(_) => {
                debug!("Successfully deleted data");
                Ok(())
            },
            Err(e) => {
                error!("Failed to delete data: {}", e);
                Err(StorageError::DbError(e))
            }
        }
    }

    fn list_keys_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Vec<u8>>, StorageError> {
        debug!("Listing keys with prefix of {} bytes", prefix.len());
        let mut result = Vec::new();
        
        let iter = self.db.prefix_iterator(prefix);
        for item in iter {
            match item {
                Ok((key, _)) => {
                    result.push(key.to_vec());
                },
                Err(e) => {
                    error!("Error iterating over keys: {}", e);
                    return Err(StorageError::DbError(e));
                }
            }
        }
        
        debug!("Found {} keys with prefix", result.len());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    
    #[test]
    fn test_rocks_db_storage_basic() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().to_path_buf();
        
        // Create storage
        let storage = RocksDBStorage::new(db_path).unwrap();
        
        // Test saving data
        let key = b"test_key";
        let value = b"test_value";
        storage.save_data(key, value).unwrap();
        
        // Test loading data
        let loaded = storage.load_data(key).unwrap();
        assert_eq!(loaded, Some(value.to_vec()));
        
        // Test missing key
        let missing = storage.load_data(b"nonexistent").unwrap();
        assert_eq!(missing, None);
        
        // Test delete
        storage.delete_data(key).unwrap();
        let deleted = storage.load_data(key).unwrap();
        assert_eq!(deleted, None);
        
        // Test flush
        storage.flush().unwrap();
    }
}