// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use log::{debug, error, info};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::SystemTime;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct FileStorage {
    pub id: Uuid,
    pub metadata: FileMetadata,
    pub path: PathBuf,
    pub created_at: SystemTime,
    #[serde(skip)]
    access_lock: Arc<RwLock<()>>, // For concurrent access control
}

#[derive(Error, Debug)]
pub enum StorageError2 {
    #[error("Invalid file ID")]
    InvalidId,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("File not found")]
    NotFound,

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("UUID error: {0}")]
    UuidError(#[from] uuid::Error),

    #[error("Lock error: failed to acquire {0} lock")]
    LockError(String),

    #[error("Unknown error")]
    Unknown,

    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Path canonicalization failed: {0}")]
    PathError(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileMetadata {
    pub filename: String,
    pub size: u64,
    pub content_type: String,
    pub uploaded_at: SystemTime,
}

// Global lock for file operations
lazy_static::lazy_static! {
    static ref FILE_SYSTEM_LOCK: Mutex<()> = Mutex::new(());
}

const KARI_DIR: &str = ".kari";
const STORAGE_DIR: &str = "storage";

fn get_storage_path() -> PathBuf {
    let home_dir = dirs::home_dir().expect("Could not find home directory");
    let kari_path = home_dir.join(KARI_DIR);
    let storage_path = kari_path.join(STORAGE_DIR);

    // Create directories if they don't exist
    if !kari_path.exists() {
        debug!("Creating .kari directory");
        if let Err(e) = std::fs::create_dir_all(&kari_path) {
            error!("Failed to create .kari directory: {}", e);
        }
    }
    if !storage_path.exists() {
        debug!("Creating storage directory");
        if let Err(e) = std::fs::create_dir_all(&storage_path) {
            error!("Failed to create storage directory: {}", e);
        }
    }

    storage_path
}

// Implement methods for FileStorage
impl FileStorage {
    pub fn init_storage() -> std::io::Result<()> {
        info!("Initializing file storage system");
        let home = dirs::home_dir().expect("Could not find home directory");
        let kari_dir = home.join(".kari");
        let storage_dir = kari_dir.join("storage");

        // Create .kari directory if it doesn't exist
        if !kari_dir.exists() {
            debug!("Creating .kari directory");
            fs::create_dir_all(&kari_dir)?;
        }

        // Create storage directory if it doesn't exist
        if !storage_dir.exists() {
            debug!("Creating storage directory");
            fs::create_dir_all(&storage_dir)?;
        }

        Ok(())
    }

    pub fn new() -> Result<Self, StorageError2> {
        let path = get_storage_path();
        debug!("Creating new FileStorage instance at {:?}", path);

        Ok(FileStorage {
            id: Uuid::new_v4(),
            metadata: FileMetadata {
                filename: String::from(""),
                size: 0,
                content_type: String::from(""),
                uploaded_at: SystemTime::now(),
            },
            path,
            created_at: SystemTime::now(),
            access_lock: Arc::new(RwLock::new(())),
        })
    }

    pub fn store(&self, filename: &str, data: &[u8]) -> Result<FileStorage, StorageError2> {
        // Acquire read lock to ensure thread safety
        let _read_guard = self
            .access_lock
            .read()
            .map_err(|_| StorageError2::LockError("read".to_string()))?;

        let file_path = self.path.join(filename);
        debug!(
            "Storing file: {} ({} bytes) at {:?}",
            filename,
            data.len(),
            file_path
        );

        // Use a global lock for the actual file write
        let _fs_guard = FILE_SYSTEM_LOCK
            .lock()
            .map_err(|_| StorageError2::LockError("filesystem".to_string()))?;
        fs::write(&file_path, data)?;

        let content_type = mime_guess::from_path(filename)
            .first_or_octet_stream()
            .to_string();

        info!(
            "Successfully stored file: {} ({} bytes)",
            filename,
            data.len()
        );

        Ok(FileStorage {
            id: self.id,
            metadata: FileMetadata {
                filename: filename.to_string(),
                size: data.len() as u64,
                content_type,
                uploaded_at: SystemTime::now(),
            },
            path: file_path,
            created_at: SystemTime::now(),
            access_lock: Arc::new(RwLock::new(())),
        })
    }

    pub fn read(&self, filename: &str) -> Result<Vec<u8>, StorageError2> {
        // Acquire read lock to ensure thread safety
        let _read_guard = self
            .access_lock
            .read()
            .map_err(|_| StorageError2::LockError("read".to_string()))?;

        let file_path = self.path.join(filename);
        debug!("Reading file: {} from {:?}", filename, file_path);

        match fs::read(&file_path) {
            Ok(data) => {
                debug!(
                    "Successfully read file: {} ({} bytes)",
                    filename,
                    data.len()
                );
                Ok(data)
            }
            Err(e) => {
                error!("Failed to read file {}: {}", filename, e);
                Err(StorageError2::Io(e))
            }
        }
    }

    // Check if file exists in storage with improved error handling
    pub fn check_file_exists(&self, file_path: &Path) -> bool {
        debug!("Checking if file exists: {:?}", file_path);
        match file_path.canonicalize() {
            Ok(canonical_path) => {
                let exists = canonical_path.exists();
                debug!("File exists check: {}", exists);
                exists
            }
            Err(e) => {
                debug!("Path canonicalization failed: {}", e);
                false
            }
        }
    }

    // Get complete storage path for a file
    pub fn get_file_path(&self, filename: &str) -> PathBuf {
        let path = get_storage_path().join(filename);
        debug!("Full file path for {}: {:?}", filename, path);
        path
    }

    // Upload file with improved concurrency handling
    pub fn upload(source_path: impl AsRef<Path>, filename: String) -> Result<Self, StorageError2> {
        let source_path = source_path.as_ref();
        debug!(
            "Uploading file from {:?} with name {}",
            source_path, filename
        );

        // Check source file
        if !source_path.exists() {
            let err_msg = format!("Source file does not exist: {:?}", source_path);
            error!("{}", err_msg);
            return Err(StorageError2::FileNotFound(
                source_path.to_string_lossy().to_string(),
            ));
        }

        // Initialize storage
        if let Err(e) = FileStorage::init_storage() {
            error!("Failed to initialize storage: {}", e);
            return Err(StorageError2::Io(e));
        }

        // Generate new UUID for file
        let id = Uuid::new_v4();
        let storage_path = get_storage_path();

        // Create paths
        let file_ext = source_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        let dest_filename = format!("{}.{}", id, file_ext);
        let dest_path = storage_path.join(&dest_filename);

        // Get file metadata
        let file_size = match fs::metadata(source_path) {
            Ok(metadata) => metadata.len(),
            Err(e) => {
                error!("Failed to get source file metadata: {}", e);
                return Err(StorageError2::Io(e));
            }
        };

        // Create metadata
        let metadata = FileMetadata {
            filename,
            size: file_size,
            content_type: mime_guess::from_path(source_path)
                .first_or_octet_stream()
                .to_string(),
            uploaded_at: SystemTime::now(),
        };

        // Use a lock for file operations
        let _lock = FILE_SYSTEM_LOCK
            .lock()
            .map_err(|_| StorageError2::LockError("filesystem".to_string()))?;

        // Save file
        info!("Copying file to storage: {} bytes", file_size);
        fs::copy(source_path, &dest_path)?;

        // Save metadata
        let metadata_path = storage_path.join(format!("{}.json", id));
        let metadata_json = serde_json::to_string(&metadata)?;
        fs::write(&metadata_path, metadata_json)?;

        info!("File successfully uploaded: {}", id);
        Ok(FileStorage {
            id,
            metadata,
            path: dest_path,
            created_at: SystemTime::now(),
            access_lock: Arc::new(RwLock::new(())),
        })
    }

    // Get file by ID with improved error handling
    pub fn get_by_id(id_str: &str) -> Result<Self, StorageError2> {
        debug!("Retrieving file with ID: {}", id_str);

        // Parse UUID
        let id = Uuid::parse_str(id_str).map_err(|e| {
            error!("Invalid UUID format: {}", e);
            StorageError2::InvalidId
        })?;

        // Get storage path
        let storage_path = get_storage_path();

        // Find file by looking for metadata first
        let metadata_path = storage_path.join(format!("{}.json", id_str));
        if !metadata_path.exists() {
            let err_msg = format!("Metadata file not found: {:?}", metadata_path);
            error!("{}", err_msg);
            return Err(StorageError2::NotFound);
        }

        // Load metadata with proper error handling
        let metadata_content = match std::fs::read_to_string(&metadata_path) {
            Ok(content) => content,
            Err(e) => {
                error!("Failed to read metadata file: {}", e);
                return Err(StorageError2::Io(e));
            }
        };

        let metadata: FileMetadata = match serde_json::from_str(&metadata_content) {
            Ok(data) => data,
            Err(e) => {
                error!("Failed to parse metadata JSON: {}", e);
                return Err(StorageError2::Serialization(e));
            }
        };

        // Find actual file by extension
        let file_ext = Path::new(&metadata.filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");
        let file_path = storage_path.join(format!("{}.{}", id_str, file_ext));

        if !file_path.exists() {
            let err_msg = format!("File not found at expected path: {:?}", file_path);
            error!("{}", err_msg);
            return Err(StorageError2::NotFound);
        }

        info!("Successfully retrieved file: {}", id_str);
        Ok(Self {
            id,
            path: file_path,
            metadata,
            created_at: SystemTime::now(),
            access_lock: Arc::new(RwLock::new(())),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_file_storage_basic() {
        // Create temporary directory
        let temp_dir = tempdir().unwrap();
        let test_file_path = temp_dir.path().join("test_file.txt");

        // Create test file
        let test_content = b"Hello, world!";
        let mut file = File::create(&test_file_path).unwrap();
        file.write_all(test_content).unwrap();

        // Test upload
        let storage = FileStorage::new().unwrap();
        let filename = "test_file.txt";
        let stored = storage.store(filename, test_content).unwrap();

        // Verify metadata
        assert_eq!(stored.metadata.size, test_content.len() as u64);
        assert_eq!(stored.metadata.filename, filename);

        // Test read
        let read_content = storage.read(filename).unwrap();
        assert_eq!(read_content, test_content);

        // Test existence check
        assert!(storage.check_file_exists(&storage.get_file_path(filename)));
    }
}
