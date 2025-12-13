//! Security audit logging for sensitive operations
//!
//! This module provides comprehensive audit logging for all cryptographic
//! operations and security-sensitive events.

use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use thiserror::Error;

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

/// Security event types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityEvent {
    /// Key generation
    KeyGenerated,
    /// Key accessed/loaded
    KeyAccessed,
    /// Key exported
    KeyExported,
    /// Key deleted
    KeyDeleted,
    /// Key rotated
    KeyRotated,
    /// Wallet created
    WalletCreated,
    /// Wallet accessed
    WalletAccessed,
    /// Wallet deleted
    WalletDeleted,
    /// Mnemonic created
    MnemonicCreated,
    /// Mnemonic accessed
    MnemonicAccessed,
    /// Mnemonic deleted
    MnemonicDeleted,
    /// Signature created
    SignatureCreated,
    /// Signature verified
    SignatureVerified,
    /// Encryption performed
    EncryptionPerformed,
    /// Decryption performed
    DecryptionPerformed,
    /// Authentication success
    AuthenticationSuccess,
    /// Authentication failure
    AuthenticationFailure,
    /// HSM operation
    HsmOperation,
    /// Backup created
    BackupCreated,
    /// Backup restored
    BackupRestored,
    /// Configuration changed
    ConfigurationChanged,
    /// Suspicious activity detected
    SuspiciousActivity,
}

impl SecurityEvent {
    /// Get severity level of the event
    pub fn severity(&self) -> EventSeverity {
        match self {
            SecurityEvent::KeyGenerated
            | SecurityEvent::WalletCreated
            | SecurityEvent::MnemonicCreated
            | SecurityEvent::SignatureCreated
            | SecurityEvent::EncryptionPerformed => EventSeverity::Info,

            SecurityEvent::KeyAccessed
            | SecurityEvent::WalletAccessed
            | SecurityEvent::MnemonicAccessed
            | SecurityEvent::SignatureVerified
            | SecurityEvent::DecryptionPerformed
            | SecurityEvent::AuthenticationSuccess => EventSeverity::Info,

            SecurityEvent::KeyExported
            | SecurityEvent::KeyRotated
            | SecurityEvent::BackupCreated
            | SecurityEvent::BackupRestored
            | SecurityEvent::HsmOperation => EventSeverity::Warning,

            SecurityEvent::KeyDeleted
            | SecurityEvent::WalletDeleted
            | SecurityEvent::MnemonicDeleted
            | SecurityEvent::ConfigurationChanged => EventSeverity::Warning,

            SecurityEvent::AuthenticationFailure => EventSeverity::Error,

            SecurityEvent::SuspiciousActivity => EventSeverity::Critical,
        }
    }
}

/// Event severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EventSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

/// Audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Timestamp (Unix timestamp in seconds)
    pub timestamp: u64,
    /// Event type
    pub event: SecurityEvent,
    /// Severity level
    pub severity: EventSeverity,
    /// Resource identifier (e.g., key ID, wallet address)
    pub resource_id: Option<String>,
    /// User/process identifier
    pub actor: Option<String>,
    /// Additional details
    pub details: Option<String>,
    /// Success or failure
    pub success: bool,
    /// IP address or source (if applicable)
    pub source: Option<String>,
}

impl AuditEntry {
    /// Create a new audit entry
    pub fn new(event: SecurityEvent) -> Self {
        let timestamp = crate::get_current_timestamp();

        Self {
            timestamp,
            event,
            severity: event.severity(),
            resource_id: None,
            actor: None,
            details: None,
            success: true,
            source: None,
        }
    }

    /// Builder method to set resource ID
    pub fn with_resource(mut self, resource_id: impl Into<String>) -> Self {
        self.resource_id = Some(resource_id.into());
        self
    }

    /// Builder method to set actor
    pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }

    /// Builder method to set details (supports error context for forensics)
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }

    /// Builder method to set error details (for failed operations)
    pub fn with_error(mut self, error: impl std::fmt::Display) -> Self {
        let error_details = format!("Error: {}", error);
        self.details = Some(error_details);
        self.success = false;
        self
    }

    /// Builder method to set success status
    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    /// Builder method to set source
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Format as JSON line
    pub fn to_json_line(&self) -> Result<String, AuditError> {
        // Serialize a redacted copy to avoid accidental logging of secrets
        let redacted = self.redacted();
        serde_json::to_string(&redacted).map_err(|e| AuditError::SerializationError(e.to_string()))
    }

    /// Format as human-readable string
    pub fn to_string_formatted(&self) -> String {
        // Safe conversion with overflow check (i64::MAX = year 292277026596)
        let timestamp_i64 = self.timestamp.min(i64::MAX as u64) as i64;
        let timestamp = chrono::Utc
            .timestamp_opt(timestamp_i64, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| format!("timestamp:{}", self.timestamp));

        // Use pre-allocated String with estimated capacity to reduce allocations
        let mut result = String::with_capacity(256);

        use std::fmt::Write;
        let _ = write!(
            result,
            "[{}] {:?} severity={:?} status={}",
            timestamp,
            self.event,
            self.severity,
            if self.success { "SUCCESS" } else { "FAILURE" }
        );

        if let Some(ref r) = self.resource_id {
            let _ = write!(result, " resource={}", r);
        }

        if let Some(ref a) = self.actor {
            let _ = write!(result, " actor={}", a);
        }

        if let Some(ref d) = self.details {
            let _ = write!(result, " details={}", d);
        }

        result
    }

    /// Return a redacted copy of this entry where likely-sensitive fields are masked
    pub fn redacted(&self) -> Self {
        fn redact_field(s: &Option<String>) -> Option<String> {
            match s {
                None => None,
                Some(v) => {
                    let lower = v.to_lowercase();

                    // Comprehensive redaction for sensitive data:
                    // 1. Known prefixes (strict word boundary check)
                    if lower.starts_with("kanari")
                        || lower.starts_with("kanapqc")
                        || lower.starts_with("kanahybrid")
                    {
                        return Some("[REDACTED]".to_string());
                    }

                    // 2. Hex strings (any length >= 16 chars)
                    if v.len() >= 16 && v.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
                        return Some("[REDACTED]".to_string());
                    }

                    // 3. Base64-encoded data (more specific validation)
                    if v.len() >= 20 && v.len() % 4 == 0 {
                        let chars: Vec<char> = v.chars().collect();
                        let all_valid = chars
                            .iter()
                            .all(|c| c.is_alphanumeric() || *c == '+' || *c == '/' || *c == '=');

                        // Check padding only at end
                        let padding_valid = chars.iter().rev().take_while(|&&c| c == '=').count()
                            <= 2
                            && chars
                                .iter()
                                .take(chars.len().saturating_sub(2))
                                .all(|&c| c != '=');

                        // Require at least one base64-specific character (+, /, or =)
                        let has_b64_chars = v.contains('+') || v.contains('/') || v.contains('=');

                        if all_valid && padding_valid && has_b64_chars {
                            return Some("[REDACTED]".to_string());
                        }
                    }

                    // 4. Mnemonic phrases (6+ words)
                    if v.split_whitespace().count() >= 6 {
                        return Some("[REDACTED]".to_string());
                    }

                    // 5. 0x-prefixed addresses (common format)
                    if lower.starts_with("0x") && v.len() >= 16 {
                        return Some("[REDACTED]".to_string());
                    }

                    // Otherwise return owned value to avoid clone
                    Some(v.to_string())
                }
            }
        }

        Self {
            timestamp: self.timestamp,
            event: self.event,
            severity: self.severity,
            resource_id: redact_field(&self.resource_id),
            actor: redact_field(&self.actor),
            details: redact_field(&self.details),
            success: self.success,
            source: redact_field(&self.source),
        }
    }
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

        if let Some(&last_time) = last_times.get(&entry_key) {
            if now.saturating_sub(last_time) < self.rate_limit_secs {
                return Ok(()); // Skip duplicate within rate limit window
            }
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
            println!("{}", entry.to_string_formatted());
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
mod tests {
    use super::*;

    #[test]
    fn test_audit_entry_creation() {
        let entry = AuditEntry::new(SecurityEvent::KeyGenerated)
            .with_resource("test-key")
            .with_actor("test-user")
            .with_details("Generated Ed25519 key");

        assert_eq!(entry.event, SecurityEvent::KeyGenerated);
        assert_eq!(entry.severity, EventSeverity::Info);
        assert!(entry.success);
        assert_eq!(entry.resource_id, Some("test-key".to_string()));
    }

    #[test]
    fn test_event_severity() {
        assert_eq!(SecurityEvent::KeyGenerated.severity(), EventSeverity::Info);
        assert_eq!(
            SecurityEvent::AuthenticationFailure.severity(),
            EventSeverity::Error
        );
        assert_eq!(
            SecurityEvent::SuspiciousActivity.severity(),
            EventSeverity::Critical
        );
    }

    #[test]
    fn test_audit_entry_json_serialization() {
        let entry = AuditEntry::new(SecurityEvent::WalletCreated)
            .with_resource("0x123")
            .with_success(true);

        let json = entry
            .to_json_line()
            .expect("Failed to serialize audit entry");
        assert!(json.contains("WalletCreated"));
        assert!(json.contains("0x123"));
    }
}
