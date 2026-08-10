// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Security audit logging for sensitive operations
//!
//! This module provides comprehensive audit logging for all cryptographic
//! operations and security-sensitive events.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use thiserror::Error;

mod event;
pub use event::{AuditEntry, EventSeverity, SecurityEvent};

// Maximum entries in rate limiter before cleanup (prevent memory leak)
const MAX_RATE_LIMITER_ENTRIES: usize = 1000;

/// Errors related to audit logging
#[derive(Error, Debug)]
pub enum AuditError {
    #[error("IO error: {0}")]
    IoError(#[from] io::Error),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Invalid audit log path")]
    InvalidPath,
}

/// Audit logger
pub struct AuditLogger {
    log_path: PathBuf,
    min_severity: EventSeverity,
    console_output: bool,
    max_file_size: u64,
    max_files: usize,
    last_log_time: std::sync::Mutex<std::collections::HashMap<String, u64>>,
    rate_limit_secs: u64,
}

impl AuditLogger {
    /// Create new audit logger
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            log_path,
            min_severity: EventSeverity::Info,
            console_output: false,
            max_file_size: 10 * 1024 * 1024, // 10MB default
            max_files: 5,
            last_log_time: std::sync::Mutex::new(std::collections::HashMap::new()),
            rate_limit_secs: 1, // 1 second between identical logs
        }
    }

    /// Set minimum severity level for logging
    pub fn with_min_severity(mut self, severity: EventSeverity) -> Self {
        self.min_severity = severity;
        self
    }

    /// Enable console output
    pub fn with_console_output(mut self, enabled: bool) -> Self {
        self.console_output = enabled;
        self
    }

    /// Log an audit entry
    pub fn log(&self, entry: &AuditEntry) -> Result<(), AuditError> {
        // Check if severity meets minimum threshold
        if entry.severity < self.min_severity {
            return Ok(());
        }

        // Rate limiting: prevent log flooding
        let entry_key = format!("{:?}:{:?}", entry.event, entry.resource_id);
        let now = crate::get_current_timestamp();

        // Handle mutex poisoning by recovering from poisoned state
        let mut last_times = self.last_log_time.lock().unwrap_or_else(|poisoned| {
            // Recover from poisoned mutex
            poisoned.into_inner()
        });

        // Cleanup expired entries if too many accumulated (prevent memory leak)
        if last_times.len() > MAX_RATE_LIMITER_ENTRIES {
            last_times.retain(|_, &mut last_time| {
                now.saturating_sub(last_time) < self.rate_limit_secs * 2
            });
        }

        if let Some(&last_time) = last_times.get(&entry_key)
            && now.saturating_sub(last_time) < self.rate_limit_secs
        {
            return Ok(()); // Skip duplicate within rate limit window
        }
        last_times.insert(entry_key, now);
        drop(last_times); // Explicitly drop lock before file operations

        // Check file size and rotate if needed
        self.rotate_if_needed()?;

        // Ensure log directory exists
        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Open log file in append mode
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;

        // Write JSON line
        let json_line = entry.to_json_line()?;
        writeln!(file, "{}", json_line)?;

        // Console output if enabled
        if self.console_output {
            log::info!("{}", entry.to_string_formatted());
        }

        Ok(())
    }

    /// Rotate log file if it exceeds max size
    fn rotate_if_needed(&self) -> Result<(), AuditError> {
        if !self.log_path.exists() {
            return Ok(());
        }

        let metadata = std::fs::metadata(&self.log_path)?;
        if metadata.len() < self.max_file_size {
            return Ok(());
        }

        // Use advisory lock to prevent race conditions during rotation
        use fs2::FileExt;
        let lock_path = self.log_path.with_extension("rotate.lock");
        let lock_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)?;

        // Try to acquire exclusive lock - if another process is rotating, skip
        if lock_file.try_lock_exclusive().is_err() {
            return Ok(()); // Another process is rotating
        }

        // Rotate existing logs
        for i in (1..self.max_files).rev() {
            let old_path = self.log_path.with_extension(format!("log.{}", i));
            let new_path = self.log_path.with_extension(format!("log.{}", i + 1));
            if old_path.exists() {
                let _ = std::fs::rename(&old_path, &new_path);
            }
        }

        // Rotate current log to .log.1
        let rotated_path = self.log_path.with_extension("log.1");
        std::fs::rename(&self.log_path, &rotated_path)?;

        // Lock is automatically released when lock_file is dropped
        drop(lock_file);
        let _ = std::fs::remove_file(&lock_path); // Clean up lock file

        Ok(())
    }

    /// Log a simple event
    pub fn log_event(&self, event: SecurityEvent) -> Result<(), AuditError> {
        let entry = AuditEntry::new(event);
        self.log(&entry)
    }

    /// Get path to audit log file
    pub fn get_log_path(&self) -> &PathBuf {
        &self.log_path
    }
}

/// Get default audit log path
pub fn get_default_audit_log_path() -> PathBuf {
    let mut path = kanari_common::get_kanari_config_path();
    // Use the config directory's parent as base (same approach as keystore)
    path.pop();
    path.push("audit");
    path.push("security.log");
    path
}

/// Create default audit logger
pub fn create_default_logger() -> AuditLogger {
    AuditLogger::new(get_default_audit_log_path())
        .with_min_severity(EventSeverity::Info)
        .with_console_output(false)
}

#[cfg(test)]
#[path = "../tests/unit/audit_test.rs"]
mod tests;
