// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Key rotation mechanism for enhanced security
//!
//! This module provides automatic and manual key rotation capabilities
//! to ensure cryptographic keys are regularly updated.

use thiserror::Error;

mod types;
pub use types::{KeyMetadata, KeyRotationPolicy, RotationStatistics};

/// Errors related to key rotation
#[derive(Error, Debug)]
pub enum KeyRotationError {
    #[error("Key rotation failed: {0}")]
    RotationFailed(String),

    #[error("Invalid rotation policy: {0}")]
    InvalidPolicy(String),

    #[error("Key not due for rotation")]
    NotDue,

    #[error("Encryption error during rotation: {0}")]
    EncryptionError(String),

    #[error("Backup creation failed: {0}")]
    BackupFailed(String),
}

/// Key rotation manager
#[derive(Debug)]
pub struct KeyRotationManager {
    policy: KeyRotationPolicy,
    key_metadata: std::collections::HashMap<String, KeyMetadata>,
}

impl KeyRotationManager {
    /// Create new key rotation manager with default policy
    pub fn new() -> Self {
        Self {
            policy: KeyRotationPolicy::default(),
            key_metadata: std::collections::HashMap::new(),
        }
    }

    /// Create new key rotation manager with custom policy
    pub fn with_policy(policy: KeyRotationPolicy) -> Self {
        Self {
            policy,
            key_metadata: std::collections::HashMap::new(),
        }
    }

    /// Register a new key for rotation tracking
    pub fn register_key(&mut self, key_id: String) {
        let metadata = KeyMetadata::new(&key_id);
        self.key_metadata.insert(key_id, metadata);
    }

    /// Check if a key should be rotated
    pub fn should_rotate(&self, key_id: &str) -> bool {
        if let Some(metadata) = self.key_metadata.get(key_id) {
            metadata.should_rotate(&self.policy)
        } else {
            false
        }
    }

    /// Get list of keys that need rotation
    pub fn get_keys_due_for_rotation(&self) -> Vec<String> {
        self.key_metadata
            .iter()
            .filter(|(_, metadata)| metadata.should_rotate(&self.policy))
            .map(|(key_id, _)| key_id.clone())
            .collect()
    }

    /// Mark key as rotated
    pub fn record_rotation(&mut self, key_id: &str) -> Result<(), KeyRotationError> {
        if let Some(metadata) = self.key_metadata.get_mut(key_id) {
            metadata.record_rotation();
            Ok(())
        } else {
            Err(KeyRotationError::RotationFailed(format!(
                "Key not found: {}",
                key_id
            )))
        }
    }

    /// Get metadata for a specific key
    pub fn get_metadata(&self, key_id: &str) -> Option<&KeyMetadata> {
        self.key_metadata.get(key_id)
    }

    /// Update rotation policy
    pub fn update_policy(&mut self, policy: KeyRotationPolicy) {
        self.policy = policy;
    }

    /// Get current policy
    pub fn get_policy(&self) -> &KeyRotationPolicy {
        &self.policy
    }

    /// Get rotation statistics
    pub fn get_statistics(&self) -> RotationStatistics {
        let total_keys = self.key_metadata.len();
        let keys_due = self.get_keys_due_for_rotation().len();
        let total_rotations: u64 = self.key_metadata.values().map(|m| m.rotation_count).sum();

        let avg_age_days = if total_keys > 0 {
            let sum: u64 = self
                .key_metadata
                .values()
                .map(|m| m.age_days() as u64) // Convert u32 to u64 before sum
                .filter(|&age| age != u32::MAX as u64) // Filter out invalid timestamps
                .sum();
            let valid_count = self
                .key_metadata
                .values()
                .filter(|m| m.age_days() != u32::MAX)
                .count();

            if valid_count > 0 {
                sum / (valid_count as u64)
            } else {
                0 // No valid timestamps
            }
        } else {
            0
        };

        RotationStatistics {
            total_keys,
            keys_due_for_rotation: keys_due,
            total_rotations,
            average_key_age_days: avg_age_days,
        }
    }
}

impl Default for KeyRotationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "../tests/unit/key_rotation_test.rs"]
mod tests;
