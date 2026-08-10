// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Data types for key rotation policy and tracking.

use serde::{Deserialize, Serialize};

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
            max_age_days: 90,
            auto_rotate: true,
            min_rotation_interval_hours: 24,
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
    /// Returns u32::MAX if timestamp is invalid.
    pub fn age_days(&self) -> u32 {
        let now = crate::get_current_timestamp();

        if self.created_at == 0 || now == 0 {
            return u32::MAX;
        }

        if now < self.created_at {
            return u32::MAX;
        }

        let age_seconds = now.saturating_sub(self.created_at);
        let age_days = age_seconds / 86400;
        age_days.min(u32::MAX as u64) as u32
    }

    /// Get time since last rotation in hours.
    pub fn hours_since_last_rotation(&self) -> Option<u64> {
        self.last_rotated_at.map(|last_rotated| {
            let now = crate::get_current_timestamp();
            let age_seconds = now.saturating_sub(last_rotated);
            age_seconds / 3600
        })
    }

    /// Check if key should be rotated based on policy.
    pub fn should_rotate(&self, policy: &KeyRotationPolicy) -> bool {
        let age = self.age_days();

        if age == u32::MAX {
            return false;
        }

        if age as u64 >= policy.max_age_days {
            return true;
        }

        if let Some(hours_since) = self.hours_since_last_rotation()
            && hours_since < policy.min_rotation_interval_hours
        {
            return false;
        }

        self.rotation_due
    }

    /// Mark key for rotation.
    pub fn mark_for_rotation(&mut self) {
        self.rotation_due = true;
    }

    /// Record successful rotation.
    pub fn record_rotation(&mut self) {
        let now = crate::get_current_timestamp();

        self.last_rotated_at = Some(now);
        self.rotation_count = self.rotation_count.saturating_add(1);
        self.rotation_due = false;
    }
}

/// Statistics about key rotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationStatistics {
    pub total_keys: usize,
    pub keys_due_for_rotation: usize,
    pub total_rotations: u64,
    pub average_key_age_days: u64,
}
