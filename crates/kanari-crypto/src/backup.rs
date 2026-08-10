// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Backup and restore functionality for keystore
//!
//! This module provides secure backup and restore capabilities for the keystore,
//! including encryption and verification.

use std::fs::{self};
use std::io::{self};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::Keystore;
use crate::encryption::{decrypt_data, encrypt_data};
use hmac::{KeyInit, Mac, SimpleHmac};
use sha3::{Digest, Sha3_256};

mod types;
pub use types::{BackupInfo, BackupMetadata, EncryptedBackup};

type HmacSha3_256 = SimpleHmac<Sha3_256>;

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

/// Backup manager
pub struct BackupManager {
    backup_dir: PathBuf,
}

impl Default for BackupManager {
    fn default() -> Self {
        let mut backup_dir = kanari_common::get_kanari_dir();
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

        // Calculate HMAC for integrity
        // Derive HMAC key from password using HKDF for better security
        let password_zero = zeroize::Zeroizing::new(password.as_bytes().to_vec());

        // Use HKDF to derive a proper HMAC key
        let hkdf_salt = b"kanari-backup-hmac-v1"; // Version-specific salt
        let mut derived_key = vec![0u8; 32]; // 256-bit key
        let mut hasher = Sha3_256::new();
        hasher.update(hkdf_salt);
        hasher.update(&password_zero[..]);
        derived_key.copy_from_slice(&hasher.finalize()[..]);
        let derived_key_zero = zeroize::Zeroizing::new(derived_key);

        let mut mac = HmacSha3_256::new_from_slice(&derived_key_zero)
            .map_err(|e| BackupError::EncryptionError(format!("HMAC error: {}", e)))?;
        mac.update(&keystore_json);
        let hmac_result = mac.finalize();
        let checksum = hex::encode(hmac_result.into_bytes());

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
        // Sanitize to prevent path traversal attacks
        let filename = format!("keystore_backup_{}.kbak", metadata.created_at);

        // Validate filename doesn't contain dangerous characters
        // Check for path separators, null bytes, and control characters
        if filename.contains(std::path::MAIN_SEPARATOR)
            || filename.contains('/')
            || filename.contains('\\')
            || filename.contains('\0')
            || filename.chars().any(|c| c.is_control())
            || filename.contains("..")
        {
            return Err(BackupError::SerializationError(
                "Invalid backup filename".to_string(),
            ));
        }

        let backup_path = self.backup_dir.join(&filename);

        // Ensure the resolved path is still within backup_dir (prevent traversal)
        if !backup_path.starts_with(&self.backup_dir) {
            return Err(BackupError::SerializationError(
                "Path traversal detected".to_string(),
            ));
        }

        // Write backup to file
        let backup_json = serde_json::to_string_pretty(&backup)
            .map_err(|e| BackupError::SerializationError(e.to_string()))?;
        fs::write(&backup_path, backup_json)?;

        // Set secure file permissions (owner read/write only) - Unix systems
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&backup_path)?.permissions();
            perms.set_mode(0o600); // rw------- (owner only)
            fs::set_permissions(&backup_path, perms)?;
        }

        Ok(backup_path)
    }

    /// Validate backup file before loading (size and basic checks)
    fn validate_backup_file(&self, path: &Path) -> Result<(), BackupError> {
        const MAX_BACKUP_SIZE: u64 = 50 * 1024 * 1024; // 50MB

        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_BACKUP_SIZE {
            return Err(BackupError::VerificationFailed(
                "Backup file size exceeds maximum allowed".to_string(),
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
        // Validate file size first (before checking existence to avoid TOCTOU)
        self.validate_backup_file(backup_path)?;

        // Read backup file atomically
        let backup_data = fs::read_to_string(backup_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                BackupError::NotFound(backup_path.display().to_string())
            } else {
                BackupError::IoError(e)
            }
        })?;

        // Deserialize backup
        let backup: EncryptedBackup = serde_json::from_str(&backup_data)
            .map_err(|e| BackupError::SerializationError(e.to_string()))?;

        // Decrypt keystore data
        let decrypted_data = decrypt_data(&backup.encrypted_data, password)
            .map_err(|e| BackupError::DecryptionError(e.to_string()))?;

        // Verify HMAC if requested (more secure than simple checksum)
        if verify {
            // Derive HMAC key from password using HKDF (same as create_backup)
            let password_zero = zeroize::Zeroizing::new(password.as_bytes().to_vec());

            let hkdf_salt = b"kanari-backup-hmac-v1";
            let mut derived_key = vec![0u8; 32];
            let mut hasher = Sha3_256::new();
            hasher.update(hkdf_salt);
            hasher.update(&password_zero[..]);
            derived_key.copy_from_slice(&hasher.finalize()[..]);
            let derived_key_zero = zeroize::Zeroizing::new(derived_key);

            let mut mac = HmacSha3_256::new_from_slice(&derived_key_zero)
                .map_err(|e| BackupError::VerificationFailed(format!("HMAC error: {}", e)))?;
            mac.update(&decrypted_data);
            let hmac_result = mac.finalize();
            let calculated_hmac = hex::encode(hmac_result.into_bytes());

            if calculated_hmac != backup.metadata.checksum {
                return Err(BackupError::VerificationFailed(
                    "HMAC verification failed".to_string(),
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
    ///
    /// Note: This function reads backup metadata from disk. Large backup directories
    /// may consume significant memory. Files are processed sequentially to limit
    /// memory usage, and oversized files (>50MB) are automatically skipped.
    pub fn list_backups(&self) -> Result<Vec<BackupInfo>, BackupError> {
        self.ensure_backup_dir()?;

        let mut backups = Vec::new();
        const MAX_BACKUP_READ_SIZE: u64 = 50 * 1024 * 1024; // 50MB

        // Get canonical backup directory path for security validation
        let canonical_backup_dir = self
            .backup_dir
            .canonicalize()
            .map_err(BackupError::IoError)?;

        for entry in fs::read_dir(&self.backup_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Security: Block symlinks to prevent directory traversal
            let symlink_meta = path.symlink_metadata()?;
            if symlink_meta.is_symlink() {
                continue; // Skip symlinks
            }
            // Use regular metadata for file size and other checks
            let metadata = entry.metadata()?;

            // Security: Validate path is within backup directory
            if let Ok(canonical_path) = path.canonicalize() {
                if !canonical_path.starts_with(&canonical_backup_dir) {
                    continue; // Skip files outside backup directory
                }
            } else {
                continue; // Skip if can't canonicalize
            }

            if path.extension().and_then(|s| s.to_str()) == Some("kbak") {
                // Check file size before reading
                if metadata.len() > MAX_BACKUP_READ_SIZE {
                    continue; // Skip oversized files
                }

                if let Ok(data) = fs::read_to_string(&path)
                    && let Ok(backup) = serde_json::from_str::<EncryptedBackup>(&data)
                {
                    backups.push(BackupInfo {
                        path: path.clone(),
                        metadata: backup.metadata,
                        file_size: metadata.len(),
                    });
                }
            }
        }

        // Sort by creation time (newest first)
        backups.sort_by_key(|backup| std::cmp::Reverse(backup.metadata.created_at));

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

#[cfg(test)]
#[path = "../tests/unit/backup_test.rs"]
mod tests;
