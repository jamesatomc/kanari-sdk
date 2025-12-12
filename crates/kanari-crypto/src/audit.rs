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

    /// Builder method to set details
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
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
        serde_json::to_string(self).map_err(|e| AuditError::SerializationError(e.to_string()))
    }

    /// Format as human-readable string
    pub fn to_string_formatted(&self) -> String {
        let timestamp = chrono::Utc
            .timestamp_opt(self.timestamp as i64, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| format!("timestamp:{}", self.timestamp));

        let resource = self
            .resource_id
            .as_ref()
            .map(|r| format!(" resource={}", r))
            .unwrap_or_default();

        let actor = self
            .actor
            .as_ref()
            .map(|a| format!(" actor={}", a))
            .unwrap_or_default();

        let details = self
            .details
            .as_ref()
            .map(|d| format!(" details={}", d))
            .unwrap_or_default();

        let status = if self.success { "SUCCESS" } else { "FAILURE" };

        format!(
            "[{}] {:?} severity={:?} status={}{}{}{}",
            timestamp, self.event, self.severity, status, resource, actor, details
        )
    }
}

/// Audit logger
pub struct AuditLogger {
    log_path: PathBuf,
    min_severity: EventSeverity,
    console_output: bool,
}

impl AuditLogger {
    /// Create new audit logger
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            log_path,
            min_severity: EventSeverity::Info,
            console_output: false,
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
