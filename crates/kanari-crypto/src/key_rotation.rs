// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Key rotation mechanism for enhanced security
//!
//! This module provides automatic and manual key rotation capabilities
//! to ensure cryptographic keys are regularly updated.

use serde::{Deserialize, Serialize};
use thiserror::Error;

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

/// Key rotation policy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRotationPolicy {
    /// Maximum age of a key in days before rotation is required
    pub max_age_days: u64,
    /// Whether to automatically rotate keys
    pub auto_rotate: bool,
    /// Minimum time between rotations in hours
    pub min_rotation_interval_hours: u64,
    /// Keep backup of old keys
    pub keep_backup: bool,
    /// Number of backup versions to keep
    pub backup_versions: usize,
}

impl Default for KeyRotationPolicy {
    fn default() -> Self {
        Self {
            max_age_days: 90, // Rotate every 90 days by default
            auto_rotate: true,
            min_rotation_interval_hours: 24, // Don't rotate more than once per day
            keep_backup: true,
            backup_versions: 3,
        }
    }
}

/// Key metadata for rotation tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyMetadata {
    /// Key identifier
    pub key_id: String,
    /// Creation timestamp (Unix timestamp)
    pub created_at: u64,
    /// Last rotation timestamp
    pub last_rotated_at: Option<u64>,
    /// Number of times key has been rotated
    pub rotation_count: u64,
    /// Whether key is due for rotation
    pub rotation_due: bool,
}

impl KeyMetadata {
    /// Create new key metadata
    pub fn new(key_id: &str) -> Self {
        let now = crate::get_current_timestamp();

        Self {
            key_id: key_id.to_string(),
            created_at: now,
            last_rotated_at: None,
            rotation_count: 0,
            rotation_due: false,
        }
    }

    /// Get age of key in days (returns u32 to prevent overflow on 32-bit systems)
    /// Returns u32::MAX if timestamp is invalid
    /// Callers should check for u32::MAX and handle appropriately
    pub fn age_days(&self) -> u32 {
        let now = crate::get_current_timestamp();

        // Validate timestamps to prevent overflow
        // Return u32::MAX to signal error
        if self.created_at == 0 || now == 0 {
            return u32::MAX; // Signal invalid timestamp
        }

        if now < self.created_at {
            return u32::MAX; // Signal future timestamp (invalid)
        }

        let age_seconds = now.saturating_sub(self.created_at);
        let age_days = age_seconds / 86400; // Convert to days

        // Safely convert u64 to u32, capping at u32::MAX to prevent truncation
        age_days.min(u32::MAX as u64) as u32
    }

    /// Get time since last rotation in hours
    pub fn hours_since_last_rotation(&self) -> Option<u64> {
        self.last_rotated_at.map(|last_rotated| {
            let now = crate::get_current_timestamp();

            let age_seconds = now.saturating_sub(last_rotated);
            age_seconds / 3600 // Convert to hours
        })
    }

    /// Check if key should be rotated based on policy
    pub fn should_rotate(&self, policy: &KeyRotationPolicy) -> bool {
        let age = self.age_days();

        // Reject invalid ages
        if age == u32::MAX {
            return false; // Invalid timestamp, don't rotate
        }

        // Check if key age exceeds maximum (safely convert to u64 for comparison)
        if age as u64 >= policy.max_age_days {
            return true;
        }

        // Check if minimum rotation interval has passed since last rotation
        if let Some(hours_since) = self.hours_since_last_rotation()
            && hours_since < policy.min_rotation_interval_hours
        {
            return false;
        }

        self.rotation_due
    }

    /// Mark key for rotation
    pub fn mark_for_rotation(&mut self) {
        self.rotation_due = true;
    }

    /// Record successful rotation
    pub fn record_rotation(&mut self) {
        let now = crate::get_current_timestamp();

        self.last_rotated_at = Some(now);
        self.rotation_count = self.rotation_count.saturating_add(1);
        self.rotation_due = false;
    }
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

/// Statistics about key rotation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationStatistics {
    pub total_keys: usize,
    pub keys_due_for_rotation: usize,
    pub total_rotations: u64,
    pub average_key_age_days: u64,
}

#[cfg(test)]
#[path = "../tests/unit/key_rotation_test.rs"]
mod tests;
