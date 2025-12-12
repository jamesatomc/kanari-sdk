//! Backup and restore functionality for keystore
//!
//! This module provides secure backup and restore capabilities for the keystore,
//! including encryption and verification.

use serde::{Deserialize, Serialize};
use std::fs::{self};
use std::io::{self};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::Keystore;
use crate::encryption::{EncryptedData, decrypt_data, encrypt_data};

/// Errors related to backup/restore operations
#[derive(Error, Debug)]
pub enum BackupError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Encryption error: {0}")]
    EncryptionError(String),

    #[error("Decryption error: {0}")]
    DecryptionError(String),

    #[error("Invalid backup format")]
    InvalidFormat,

    #[error("Backup verification failed: {0}")]
    VerificationFailed(String),

    #[error("Keystore error: {0}")]
    KeystoreError(String),

    #[error("Backup file not found: {0}")]
    NotFound(String),
}

/// Backup metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMetadata {
    /// Backup creation timestamp (Unix timestamp)
    pub created_at: u64,
    /// Version of the backup format
    pub version: String,
    /// Number of keys in the backup
    pub key_count: usize,
    /// Whether mnemonic is included
    pub has_mnemonic: bool,
    /// Checksum for verification (SHA3-256)
    pub checksum: String,
    /// Optional description
    pub description: Option<String>,
}

impl BackupMetadata {
    /// Create new backup metadata
    pub fn new(key_count: usize, has_mnemonic: bool, checksum: String) -> Self {
        let timestamp = crate::get_current_timestamp();

        Self {
            created_at: timestamp,
            version: env!("CARGO_PKG_VERSION").to_string(),
            key_count,
            has_mnemonic,
            checksum,
            description: None,
        }
    }

    /// Set description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// Encrypted backup structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBackup {
    /// Metadata about the backup
    pub metadata: BackupMetadata,
    /// Encrypted keystore data
    pub encrypted_data: EncryptedData,
}

/// Backup manager
pub struct BackupManager {
    backup_dir: PathBuf,
}

impl Default for BackupManager {
    fn default() -> Self {
        let mut backup_dir = kanari_common::get_kari_dir();
        backup_dir.push("backups");
        Self { backup_dir }
    }
}

impl BackupManager {
    /// Create new backup manager
    pub fn new(backup_dir: PathBuf) -> Self {
        Self { backup_dir }
    }

    /// Ensure backup directory exists
    fn ensure_backup_dir(&self) -> Result<(), BackupError> {
        if !self.backup_dir.exists() {
            fs::create_dir_all(&self.backup_dir)?;
        }
        Ok(())
    }

    /// Create backup of keystore
    pub fn create_backup(
        &self,
        password: &str,
        description: Option<String>,
    ) -> Result<PathBuf, BackupError> {
        self.ensure_backup_dir()?;

        // Load current keystore
        let keystore = Keystore::load().map_err(|e| BackupError::KeystoreError(e.to_string()))?;

        // Serialize keystore
        let keystore_json = serde_json::to_vec(&keystore)
            .map_err(|e| BackupError::SerializationError(e.to_string()))?;

        // Calculate checksum
        let checksum = hex::encode(crate::hash_data(&keystore_json));

        // Create metadata
        let metadata = BackupMetadata::new(keystore.keys.len(), keystore.has_mnemonic(), checksum);

        let metadata = if let Some(desc) = description {
            metadata.with_description(desc)
        } else {
            metadata
        };

        // Encrypt keystore data
        let encrypted_data = encrypt_data(&keystore_json, password)
            .map_err(|e| BackupError::EncryptionError(e.to_string()))?;

        // Create backup structure
        let backup = EncryptedBackup {
            metadata: metadata.clone(),
            encrypted_data,
        };

        // Generate backup filename with timestamp from metadata (ensures consistency)
        let filename = format!("keystore_backup_{}.kbak", metadata.created_at);
        let backup_path = self.backup_dir.join(&filename);

        // Write backup to file
        let backup_json = serde_json::to_string_pretty(&backup)
            .map_err(|e| BackupError::SerializationError(e.to_string()))?;
        fs::write(&backup_path, backup_json)?;

        Ok(backup_path)
    }

    /// Validate backup file before loading (size and basic checks)
    fn validate_backup_file(&self, path: &Path) -> Result<(), BackupError> {
        const MAX_BACKUP_SIZE: u64 = 50 * 1024 * 1024; // 50MB
        
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_BACKUP_SIZE {
            return Err(BackupError::VerificationFailed(
                "Backup file size exceeds maximum allowed".to_string()
            ));
        }
        
        Ok(())
    }

    /// Restore keystore from backup
    pub fn restore_backup(
        &self,
        backup_path: &Path,
        password: &str,
        verify: bool,
    ) -> Result<(), BackupError> {
        // Check if backup file exists
        if !backup_path.exists() {
            return Err(BackupError::NotFound(backup_path.display().to_string()));
        }

        // Validate file size first
        self.validate_backup_file(backup_path)?;

        // Read backup file
        let backup_data = fs::read_to_string(backup_path)?;

        // Deserialize backup
        let backup: EncryptedBackup = serde_json::from_str(&backup_data)
            .map_err(|e| BackupError::SerializationError(e.to_string()))?;

        // Decrypt keystore data
        let decrypted_data = decrypt_data(&backup.encrypted_data, password)
            .map_err(|e| BackupError::DecryptionError(e.to_string()))?;

        // Verify checksum if requested
        if verify {
            let checksum = hex::encode(crate::hash_data(&decrypted_data));
            if checksum != backup.metadata.checksum {
                return Err(BackupError::VerificationFailed(
                    "Checksum mismatch".to_string(),
                ));
            }
        }

        // Deserialize keystore
        let mut keystore: Keystore = serde_json::from_slice(&decrypted_data)
            .map_err(|e| BackupError::SerializationError(e.to_string()))?;

        // Verify restored keystore structure
        if verify {
            if keystore.keys.len() != backup.metadata.key_count {
                return Err(BackupError::VerificationFailed(format!(
                    "Key count mismatch: expected {}, got {}",
                    backup.metadata.key_count,
                    keystore.keys.len()
                )));
            }

            if keystore.has_mnemonic() != backup.metadata.has_mnemonic {
                return Err(BackupError::VerificationFailed(
                    "Mnemonic presence mismatch".to_string(),
                ));
            }
        }

        // Save restored keystore
        keystore
            .save()
            .map_err(|e| BackupError::KeystoreError(e.to_string()))?;

        Ok(())
    }

    /// List all available backups
    pub fn list_backups(&self) -> Result<Vec<BackupInfo>, BackupError> {
        self.ensure_backup_dir()?;

        let mut backups = Vec::new();

        for entry in fs::read_dir(&self.backup_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("kbak")
                && let Ok(data) = fs::read_to_string(&path)
                && let Ok(backup) = serde_json::from_str::<EncryptedBackup>(&data)
            {
                backups.push(BackupInfo {
                    path: path.clone(),
                    metadata: backup.metadata,
                    file_size: entry.metadata()?.len(),
                });
            }
        }

        // Sort by creation time (newest first)
        backups.sort_by(|a, b| b.metadata.created_at.cmp(&a.metadata.created_at));

        Ok(backups)
    }

    /// Delete a backup file
    pub fn delete_backup(&self, backup_path: &Path) -> Result<(), BackupError> {
        if !backup_path.exists() {
            return Err(BackupError::NotFound(backup_path.display().to_string()));
        }

        fs::remove_file(backup_path)?;
        Ok(())
    }

    /// Get backup directory path
    pub fn get_backup_dir(&self) -> &Path {
        &self.backup_dir
    }

    /// Clean old backups (keep only N most recent)
    pub fn clean_old_backups(&self, keep_count: usize) -> Result<usize, BackupError> {
        let mut backups = self.list_backups()?;

        if backups.len() <= keep_count {
            return Ok(0);
        }

        // Keep only the most recent backups
        let to_delete = backups.split_off(keep_count);
        let deleted_count = to_delete.len();

        for backup in to_delete {
            self.delete_backup(&backup.path)?;
        }

        Ok(deleted_count)
    }
}

/// Backup information
#[derive(Debug, Clone)]
pub struct BackupInfo {
    /// Path to backup file
    pub path: PathBuf,
    /// Backup metadata
    pub metadata: BackupMetadata,
    /// File size in bytes
    pub file_size: u64,
}

impl BackupInfo {
    /// Get formatted creation time
    pub fn created_at_formatted(&self) -> String {
        chrono::DateTime::from_timestamp(self.metadata.created_at as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| format!("timestamp:{}", self.metadata.created_at))
    }

    /// Get human-readable file size
    pub fn file_size_formatted(&self) -> String {
        let size = self.file_size as f64;
        if size < 1024.0 {
            format!("{:.0} B", size)
        } else if size < 1024.0 * 1024.0 {
            format!("{:.2} KB", size / 1024.0)
        } else {
            format!("{:.2} MB", size / (1024.0 * 1024.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_backup_metadata_creation() {
        let metadata =
            BackupMetadata::new(5, true, "abc123".to_string()).with_description("Test backup");

        assert_eq!(metadata.key_count, 5);
        assert!(metadata.has_mnemonic);
        assert_eq!(metadata.checksum, "abc123");
        assert_eq!(metadata.description, Some("Test backup".to_string()));
    }

    #[test]
    fn test_backup_manager_creation() {
        let temp_dir = TempDir::new().unwrap();
        let manager = BackupManager::new(temp_dir.path().to_path_buf());

        assert_eq!(manager.get_backup_dir(), temp_dir.path());
    }

    #[test]
    fn test_list_empty_backups() {
        let temp_dir = TempDir::new().unwrap();
        let manager = BackupManager::new(temp_dir.path().to_path_buf());

        let backups = manager.list_backups().unwrap();
        assert_eq!(backups.len(), 0);
    }
}
