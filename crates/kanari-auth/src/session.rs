// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Session management for authenticated users
//!
//! This module handles user sessions including creation, validation,
//! expiration, and secure storage of session tokens.

use chrono::{DateTime, Utc};
use kanari_crypto::keys::CurveType;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::AuthError;

/// Default session timeout duration (24 hours)
const DEFAULT_SESSION_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// Maximum session timeout (7 days)
const MAX_SESSION_TIMEOUT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Represents an authenticated user session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier
    pub session_id: String,

    /// Email address of the authenticated user
    pub email: String,

    /// Wallet address associated with this session
    pub wallet_address: String,

    /// Decrypted private key held in memory for authenticated operations.
    /// Never serialized; it is zeroized when the session is invalidated or dropped.
    #[serde(skip)]
    pub private_key: Option<String>,

    /// Curve type for this wallet
    pub curve_type: CurveType,

    /// Session creation timestamp
    pub created_at: DateTime<Utc>,

    /// Session expiration timestamp
    pub expires_at: DateTime<Utc>,

    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,

    /// Whether the session is still valid
    #[serde(skip)]
    pub is_valid: bool,
}

impl Session {
    /// Create a new session for a user
    ///
    /// # Arguments
    /// * `email` - User's email address
    /// * `wallet_address` - Associated wallet address
    /// * `timeout` - Optional custom timeout duration
    ///
    /// # Returns
    /// A new Session instance
    pub fn new(
        email: String,
        wallet_address: String,
        private_key: Option<String>,
        curve_type: CurveType,
        timeout: Option<Duration>,
    ) -> Self {
        let now = Utc::now();
        let timeout_duration = timeout.unwrap_or(DEFAULT_SESSION_TIMEOUT);

        // Cap timeout at maximum allowed.
        let actual_timeout = if timeout_duration > MAX_SESSION_TIMEOUT {
            MAX_SESSION_TIMEOUT
        } else {
            timeout_duration
        };

        let session_id = Uuid::new_v4().to_string();

        Self {
            session_id,
            email,
            wallet_address,
            private_key,
            curve_type,
            created_at: now,
            expires_at: now + chrono::Duration::from_std(actual_timeout).unwrap(),
            last_activity: now,
            is_valid: true,
        }
    }

    /// Check if the session is still valid (not expired)
    ///
    /// # Returns
    /// `true` if the session is valid and not expired
    pub fn is_expired(&self) -> bool {
        !self.is_valid || Utc::now() >= self.expires_at
    }

    /// Update the last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Utc::now();
    }

    /// Invalidate the session and immediately erase any decrypted key material.
    pub fn invalidate(&mut self) {
        if let Some(private_key) = self.private_key.as_mut() {
            private_key.zeroize();
        }
        self.private_key = None;
        self.is_valid = false;
    }

    /// Get remaining time until session expiration
    ///
    /// # Returns
    /// Duration until expiration, or None if already expired
    pub fn time_remaining(&self) -> Option<Duration> {
        if self.is_expired() {
            return None;
        }

        let now = Utc::now();
        let remaining = self.expires_at.signed_duration_since(now);

        if remaining.num_seconds() <= 0 {
            None
        } else {
            Some(Duration::from_secs(remaining.num_seconds() as u64))
        }
    }

    /// Serialize session to JSON string for storage
    ///
    /// # Returns
    /// JSON representation of the session
    pub fn to_json(&self) -> Result<String, AuthError> {
        serde_json::to_string(self).map_err(|e| {
            AuthError::SerializationError(format!("Failed to serialize session: {}", e))
        })
    }

    /// Deserialize session from JSON string
    ///
    /// # Arguments
    /// * `json` - JSON string representation
    ///
    /// # Returns
    /// Deserialized Session
    pub fn from_json(json: &str) -> Result<Self, AuthError> {
        serde_json::from_str(json).map_err(|e| {
            AuthError::SerializationError(format!("Failed to deserialize session: {}", e))
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(private_key) = self.private_key.as_mut() {
            private_key.zeroize();
        }
    }
}

/// Session manager that handles multiple active sessions
#[derive(Debug)]
pub struct SessionManager {
    /// Active sessions indexed by session ID
    sessions: std::collections::HashMap<String, Session>,

    /// Sessions indexed by email for quick lookup
    email_sessions: std::collections::HashMap<String, Vec<String>>,

    /// Maximum number of concurrent sessions per user
    max_sessions_per_user: usize,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            sessions: std::collections::HashMap::new(),
            email_sessions: std::collections::HashMap::new(),
            max_sessions_per_user: 5,
        }
    }

    /// Create a new session for a user
    ///
    /// # Arguments
    /// * `email` - User's email
    /// * `wallet_address` - Associated wallet address
    /// * `timeout` - Optional custom timeout
    ///
    /// # Returns
    /// The created session
    pub fn create_session(
        &mut self,
        email: String,
        wallet_address: String,
        private_key: Option<String>,
        curve_type: CurveType,
        timeout: Option<Duration>,
    ) -> Session {
        // Clean up expired sessions for this user first
        self.cleanup_expired_sessions(&email);

        // Check session limit
        if let Some(user_sessions) = self.email_sessions.get(&email)
            && user_sessions.len() >= self.max_sessions_per_user
        {
            // Removing the oldest session drops it and zeroizes its private key.
            if let Some(oldest_id) = user_sessions.first() {
                self.sessions.remove(oldest_id);
            }
        }

        let session = Session::new(
            email.clone(),
            wallet_address,
            private_key,
            curve_type,
            timeout,
        );
        let session_id = session.session_id.clone();

        // Store session
        self.sessions.insert(session_id.clone(), session.clone());

        // Index by email
        self.email_sessions
            .entry(email)
            .or_default()
            .push(session_id);

        session
    }

    /// Validate and retrieve a session by ID
    ///
    /// # Arguments
    /// * `session_id` - The session identifier
    ///
    /// # Returns
    /// Reference to the session if valid, error otherwise
    pub fn validate_session(&mut self, session_id: &str) -> Result<&Session, AuthError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or(AuthError::InvalidSession)?;

        if session.is_expired() {
            return Err(AuthError::SessionExpired);
        }

        Ok(session)
    }

    /// Get mutable reference to a session and update activity
    pub fn get_session_mut(&mut self, session_id: &str) -> Result<&mut Session, AuthError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or(AuthError::InvalidSession)?;

        if session.is_expired() {
            return Err(AuthError::SessionExpired);
        }

        session.touch();
        Ok(session)
    }

    /// Invalidate a specific session
    pub fn invalidate_session(&mut self, session_id: &str) -> Result<(), AuthError> {
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.invalidate();

            // Remove from email index
            let email = session.email.clone();
            if let Some(user_sessions) = self.email_sessions.get_mut(&email) {
                user_sessions.retain(|id| id != session_id);
            }

            self.sessions.remove(session_id);
            Ok(())
        } else {
            Err(AuthError::InvalidSession)
        }
    }

    /// Invalidate all sessions for a user
    pub fn invalidate_all_user_sessions(&mut self, email: &str) {
        if let Some(session_ids) = self.email_sessions.remove(email) {
            for session_id in session_ids {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.invalidate();
                }
                self.sessions.remove(&session_id);
            }
        }
    }

    /// Clean up expired sessions for a specific user
    fn cleanup_expired_sessions(&mut self, email: &str) {
        if let Some(session_ids) = self.email_sessions.get_mut(email) {
            let expired_ids: Vec<String> = session_ids
                .iter()
                .filter(|id| {
                    self.sessions
                        .get(*id)
                        .map(|s| s.is_expired())
                        .unwrap_or(true)
                })
                .cloned()
                .collect();

            for expired_id in expired_ids {
                if let Some(session) = self.sessions.get_mut(&expired_id) {
                    session.invalidate();
                }
                self.sessions.remove(&expired_id);
                session_ids.retain(|id| id != &expired_id);
            }
        }
    }

    /// Get count of active sessions for a user
    pub fn get_user_session_count(&self, email: &str) -> usize {
        self.email_sessions
            .get(email)
            .map(|ids| ids.len())
            .unwrap_or(0)
    }

    /// List all active session IDs for a user
    pub fn list_user_sessions(&self, email: &str) -> Vec<String> {
        self.email_sessions.get(email).cloned().unwrap_or_default()
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let session = Session::new(
            "user@example.com".to_string(),
            "0x123".to_string(),
            Some("kanari_test".to_string()),
            CurveType::Ed25519,
            None,
        );

        assert!(!session.session_id.is_empty());
        assert_eq!(session.email, "user@example.com");
        assert_eq!(session.wallet_address, "0x123");
        assert!(!session.is_expired());
        assert!(session.time_remaining().unwrap() <= DEFAULT_SESSION_TIMEOUT);
    }

    #[test]
    fn test_session_expiration() {
        // Create session with very short timeout
        let mut session = Session::new(
            "user@example.com".to_string(),
            "0x123".to_string(),
            Some("kanari_test".to_string()),
            CurveType::Ed25519,
            Some(Duration::from_secs(0)),
        );

        // Manually set expiration to past
        session.expires_at = Utc::now() - chrono::Duration::seconds(1);

        assert!(session.is_expired());
        assert!(session.time_remaining().is_none());
    }

    #[test]
    fn test_session_timeout_is_capped() {
        let session = Session::new(
            "user@example.com".to_string(),
            "0x123".to_string(),
            Some("kanari_test".to_string()),
            CurveType::Ed25519,
            Some(Duration::from_secs(30 * 24 * 60 * 60)),
        );

        assert!(session.time_remaining().unwrap() <= MAX_SESSION_TIMEOUT);
    }

    #[test]
    fn test_invalidate_erases_private_key() {
        let mut session = Session::new(
            "user@example.com".to_string(),
            "0x123".to_string(),
            Some("kanari_test".to_string()),
            CurveType::Ed25519,
            None,
        );

        session.invalidate();
        assert!(session.private_key.is_none());
        assert!(session.is_expired());
    }

    #[test]
    fn test_session_manager() {
        let mut manager = SessionManager::new();

        let session = manager.create_session(
            "user@example.com".to_string(),
            "0x123".to_string(),
            Some("kanari_test".to_string()),
            CurveType::Ed25519,
            None,
        );

        let session_id = session.session_id.clone();

        // Validate session
        assert!(manager.validate_session(&session_id).is_ok());

        // Check session count
        assert_eq!(manager.get_user_session_count("user@example.com"), 1);

        // Invalidate session
        assert!(manager.invalidate_session(&session_id).is_ok());
        assert!(manager.validate_session(&session_id).is_err());
    }

    #[test]
    fn test_session_serialization() {
        let session = Session::new(
            "user@example.com".to_string(),
            "0x123".to_string(),
            Some("kanari_test".to_string()),
            CurveType::Ed25519,
            None,
        );

        let json = session.to_json().unwrap();
        assert!(!json.contains("kanari_test"));
        assert!(!json.contains("private_key"));

        let restored = Session::from_json(&json).unwrap();

        assert_eq!(session.session_id, restored.session_id);
        assert_eq!(session.email, restored.email);
        assert_eq!(session.wallet_address, restored.wallet_address);
        assert!(restored.private_key.is_none());
    }
}
