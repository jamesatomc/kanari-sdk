// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Audit event model and redaction.

use super::AuditError;
use chrono::TimeZone;
use serde::{Deserialize, Serialize};

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
        let redacted = self.redacted();
        serde_json::to_string(&redacted).map_err(|e| AuditError::SerializationError(e.to_string()))
    }

    /// Format as human-readable string
    pub fn to_string_formatted(&self) -> String {
        let timestamp_i64 = self.timestamp.min(i64::MAX as u64) as i64;
        let timestamp = chrono::Utc
            .timestamp_opt(timestamp_i64, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| format!("timestamp:{}", self.timestamp));

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

                    if lower.starts_with("kanari")
                        || lower.starts_with("kanapqc")
                        || lower.starts_with("kanamldsa")
                        || lower.starts_with("kanaslh")
                        || lower.starts_with("kanahybrid")
                    {
                        return Some("[REDACTED]".to_string());
                    }

                    if v.len() >= 16 && v.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
                        return Some("[REDACTED]".to_string());
                    }

                    if v.len() >= 20 && v.len() % 4 == 0 {
                        let chars: Vec<char> = v.chars().collect();
                        let all_valid = chars
                            .iter()
                            .all(|c| c.is_alphanumeric() || *c == '+' || *c == '/' || *c == '=');

                        let padding_valid = chars.iter().rev().take_while(|&&c| c == '=').count()
                            <= 2
                            && chars
                                .iter()
                                .take(chars.len().saturating_sub(2))
                                .all(|&c| c != '=');

                        let has_b64_chars = v.contains('+') || v.contains('/') || v.contains('=');

                        if all_valid && padding_valid && has_b64_chars {
                            return Some("[REDACTED]".to_string());
                        }
                    }

                    if v.split_whitespace().count() >= 6 {
                        return Some("[REDACTED]".to_string());
                    }

                    if lower.starts_with("0x") && v.len() >= 16 {
                        return Some("[REDACTED]".to_string());
                    }

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
