// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Main authentication manager
//!
//! This module provides the primary interface for email-based authentication
//! and transaction signing operations.

use std::str::FromStr;
use std::time::Duration;

use kanari_crypto::{keys::CurveType, wallet};
use kanari_types::transaction::{ObjectRef, SignedTransaction, Transaction};
use move_core_types::account_address::AccountAddress;

use crate::private_key_crypto::{decrypt_private_key, encrypt_private_key};
use crate::{
    AuthError, AuthResult, Session, UserStore, email_validator, session::SessionManager,
    user_store::UserRecord,
};

#[derive(Debug, Clone)]
pub struct TwoFactorStatus {
    pub secret: String,
    pub enabled: bool,
    pub backup_codes: Vec<String>,
}

/// Main authentication manager that coordinates user registration,
/// login, and transaction signing operations.
pub struct AuthManager {
    /// User storage
    user_store: UserStore,

    /// Session manager
    session_manager: SessionManager,
}

impl AuthManager {
    /// Create a new AuthManager with in-memory SQLite database
    pub fn new() -> Self {
        Self {
            user_store: UserStore::new(None).expect("Failed to create in-memory UserStore"),
            session_manager: SessionManager::new(),
        }
    }

    /// Create an AuthManager with persistent SQLite storage
    ///
    /// # Arguments
    /// * `db_path` - Path to SQLite database file
    ///
    /// # Returns
    /// Configured AuthManager
    pub fn with_persistence(db_path: std::path::PathBuf) -> AuthResult<Self> {
        Ok(Self {
            user_store: UserStore::new(Some(db_path))?,
            session_manager: SessionManager::new(),
        })
    }

    pub fn has_legacy_two_factor_secrets(&self) -> AuthResult<bool> {
        self.user_store.has_legacy_two_factor_secrets()
    }
    /// Register a new user with email and password
    ///
    /// This creates a new wallet and associates it with the user's email.
    ///
    /// # Arguments
    /// * `email` - User's email address
    /// * `password` - User's password (must meet strength requirements)
    /// * `curve_type` - Optional cryptographic curve type (defaults to Ed25519)
    ///
    /// # Returns
    /// The created wallet information
    ///
    /// # Examples
    /// ```ignore
    /// use kanari_auth::AuthManager;
    ///
    /// let mut auth = AuthManager::new();
    /// let address = auth.register_user("user@example.com", "SecurePassword123!", None)?;
    /// println!("Wallet created: {}", address);
    /// ```
    pub fn register_user(
        &mut self,
        email: &str,
        password: &str,
        curve_type: Option<CurveType>,
    ) -> AuthResult<wallet::Wallet> {
        // Validate email format
        email_validator::validate_email(email)?;

        let normalized_email = email_validator::normalize_email(email);

        // Check if user already exists
        if self.user_store.user_exists(&normalized_email)? {
            return Err(AuthError::UserAlreadyExists(normalized_email));
        }

        // Generate new wallet
        let curve = curve_type.unwrap_or(CurveType::Ed25519Dilithium3);
        log::info!("Generating keypair with curve type: {:?}", curve);
        let keypair = kanari_crypto::keys::generate_keypair(curve)
            .map_err(|e| AuthError::CryptoError(format!("Failed to generate keypair: {}", e)))?;

        let address = AccountAddress::from_str(&keypair.address)
            .map_err(|e| AuthError::CryptoError(format!("Invalid address format: {}", e)))?;

        // Create wallet - need to convert Zeroizing<String> to String
        let wallet = wallet::Wallet::new(
            address,
            keypair.private_key.to_string(),
            String::new(), // Seed phrase not available from keypair
            None,
            curve,
        );

        let wallet_address = wallet.address.to_hex_literal();

        // Create user record with hashed password
        let mut user_record = UserRecord::new(
            normalized_email.clone(),
            password,
            wallet_address,
            curve.to_string(),
        )?;

        let encrypted_private_key = encrypt_private_key(keypair.private_key.as_ref(), password)?;
        user_record.set_encrypted_private_key(encrypted_private_key);

        // Save user to store
        self.user_store.add_user(user_record)?;

        log::info!("User registered successfully: {}", normalized_email);

        Ok(wallet)
    }

    /// Authenticate a user with email and password
    ///
    /// # Arguments
    /// * `email` - User's email address
    /// * `password` - User's password
    /// * `session_timeout` - Optional custom session timeout duration
    ///
    /// # Returns
    /// A valid session if authentication succeeds
    ///
    /// # Examples
    /// ```ignore
    /// use kanari_auth::AuthManager;
    ///
    /// let mut auth = AuthManager::new();
    /// let session = auth.login("user@example.com", "SecurePassword123!", None)?;
    /// println!("Session ID: {}", session.session_id);
    /// ```
    pub fn login(
        &mut self,
        email: &str,
        password: &str,
        session_timeout: Option<Duration>,
    ) -> AuthResult<Session> {
        let normalized_email = email_validator::normalize_email(email);

        // Get user record and check account status
        let (wallet_address, curve_type, decrypted_private_key) = {
            let mut user = self
                .user_store
                .get_user(&normalized_email)?
                .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;

            // Check if account is locked
            if user.is_locked() {
                return Err(AuthError::AccountLocked);
            }

            // Verify password
            if !user.verify_password(password)? {
                user.record_failed_attempt();
                self.user_store.update_user(&user)?;
                log::warn!("Failed login attempt for: {}", normalized_email);
                return Err(AuthError::AuthenticationFailed);
            }

            // Record successful login
            user.record_successful_login();
            self.user_store.update_user(&user)?;

            let decrypted_private_key = decrypt_private_key(
                user.encrypted_private_key.as_deref().ok_or_else(|| {
                    AuthError::CryptoError("Encrypted private key not found".to_string())
                })?,
                password,
            )?;
            let curve_type = CurveType::from_str(&user.curve_type)
                .map_err(|e| AuthError::CryptoError(format!("Invalid stored curve type: {e}")))?;

            (
                user.wallet_address.clone(),
                curve_type,
                decrypted_private_key,
            )
        };

        // Create session (outside the borrow scope)
        let session = self.session_manager.create_session(
            normalized_email.clone(),
            wallet_address,
            Some(decrypted_private_key),
            curve_type,
            session_timeout,
        );

        log::info!("User logged in successfully: {}", normalized_email);

        Ok(session)
    }

    /// Logout and invalidate a session
    ///
    /// # Arguments
    /// * `session_id` - The session identifier to invalidate
    pub fn logout(&mut self, session_id: &str) -> AuthResult<()> {
        self.session_manager.invalidate_session(session_id)?;
        log::info!("Session invalidated");
        Ok(())
    }

    /// Logout all sessions for a user
    ///
    /// # Arguments
    /// * `email` - User's email address
    pub fn logout_all(&mut self, email: &str) -> AuthResult<()> {
        let normalized_email = email_validator::normalize_email(email);
        self.session_manager
            .invalidate_all_user_sessions(&normalized_email);
        log::info!("All sessions invalidated for: {}", normalized_email);
        Ok(())
    }

    /// Validate a session token
    ///
    /// # Arguments
    /// * `session_id` - The session identifier
    ///
    /// # Returns
    /// Reference to the session if valid
    pub fn validate_session(&mut self, session_id: &str) -> AuthResult<&Session> {
        self.session_manager.validate_session(session_id)
    }

    /// Sign a transaction using session credentials
    ///
    /// This loads the user's wallet and signs the transaction.
    ///
    /// # Arguments
    /// * `session` - Valid session reference
    /// * `transaction` - Transaction to sign
    ///
    /// # Returns
    /// Signed transaction ready for submission
    ///
    /// # Examples
    /// ```ignore
    /// use kanari_auth::AuthManager;
    /// use kanari_types::transaction::{ObjectRef, Transaction};
    ///
    /// let mut auth = AuthManager::new();
    /// let session = auth.login("user@example.com", "SecurePassword123!", None)?;
    ///
    /// // Create your transaction here
    /// let tx = Transaction::new_transfer_with_object_ref(
    ///     session.wallet_address.clone(),
    ///     ObjectRef::new("0x2", Some(1), Some("0xdigest".to_string())),
    ///     "0x3".to_string(),
    ///     1_000,
    ///     0,
    /// );
    /// let signed_tx = auth.sign_transaction(&session, tx)?;
    /// ```
    pub fn sign_transaction(
        &self,
        session: &Session,
        transaction: Transaction,
    ) -> AuthResult<SignedTransaction> {
        // Validate session
        if session.is_expired() {
            return Err(AuthError::SessionExpired);
        }

        // Get user to retrieve wallet credentials
        let private_key = session.private_key.as_ref().ok_or_else(|| {
            AuthError::WalletError(kanari_crypto::wallet::WalletError::NotFound(
                "Session private key not found".to_string(),
            ))
        })?;

        // Parse wallet address
        let address = AccountAddress::from_str(&session.wallet_address)
            .map_err(|e| AuthError::CryptoError(format!("Invalid wallet address: {}", e)))?;

        let wallet = wallet::Wallet::new(
            address,
            private_key.clone(),
            String::new(), // Seed phrase would be needed for full reconstruction
            None,
            session.curve_type,
        );

        // Sign the transaction
        let mut signed_tx = SignedTransaction::new(transaction);
        let signing_curve = wallet
            .validated_signing_curve()
            .map_err(AuthError::WalletError)?;
        signed_tx
            .sign(&wallet.private_key, signing_curve)
            .map_err(|e| AuthError::SigningError(format!("Transaction signing failed: {}", e)))?;

        log::info!(
            "Transaction signed for user: {}, hash: {:?}",
            session.email,
            signed_tx.transaction.hash()
        );

        Ok(signed_tx)
    }

    /// Sign a transfer transaction (convenience method)
    ///
    /// # Arguments
    /// * `session` - Valid session reference
    /// * `to` - Recipient address
    /// * `amount` - Amount in Mist
    /// * `gas_limit` - Gas limit (optional, defaults to 100,000)
    /// * `gas_price` - Gas price in Mist (optional, defaults to 1,000)
    ///
    /// # Returns
    /// Signed transfer transaction
    pub fn sign_transfer(
        &self,
        session: &Session,
        coin_object_ref: ObjectRef,
        to: &str,
        amount: u64,
        gas_limit: Option<u64>,
        gas_price: Option<u64>,
    ) -> AuthResult<SignedTransaction> {
        let from = session.wallet_address.clone();

        let mut transaction = Transaction::new_transfer_with_object_ref(
            from,
            coin_object_ref,
            to.to_string(),
            amount,
            0,
        );
        if let Transaction::ExecuteFunction {
            gas_limit: tx_gas_limit,
            gas_price: tx_gas_price,
            ..
        } = &mut transaction
        {
            *tx_gas_limit = gas_limit.unwrap_or(100_000);
            *tx_gas_price = gas_price.unwrap_or(1_000);
        }

        self.sign_transaction(session, transaction)
    }

    /// Get user information by session
    ///
    /// # Arguments
    /// * `session` - Valid session reference
    ///
    /// # Returns
    /// User's email and wallet address
    pub fn get_user_info(&self, session: &Session) -> AuthResult<(String, String)> {
        if session.is_expired() {
            return Err(AuthError::SessionExpired);
        }

        let user = self
            .user_store
            .get_user(&session.email)?
            .ok_or(AuthError::UserNotFound(session.email.clone()))?;

        Ok((user.email.clone(), user.wallet_address.clone()))
    }

    /// Get user's encrypted private key by email (for wallet restoration)
    ///
    /// # Arguments
    /// * `email` - User's email
    ///
    /// # Returns
    /// User's email, wallet address, and encrypted private key (if available)
    pub fn get_user_encrypted_key(
        &self,
        email: &str,
    ) -> AuthResult<(String, String, String, Option<String>)> {
        let user = self
            .user_store
            .get_user(email)?
            .ok_or(AuthError::UserNotFound(email.to_string()))?;

        if !user.is_active {
            return Err(AuthError::AccountLocked);
        }

        Ok((
            user.email.clone(),
            user.wallet_address.clone(),
            user.curve_type.clone(),
            user.encrypted_private_key.clone(),
        ))
    }

    /// Change user password
    ///
    /// # Arguments
    /// * `email` - User's email
    /// * `old_password` - Current password
    /// * `new_password` - New password
    pub fn change_password(
        &mut self,
        email: &str,
        old_password: &str,
        new_password: &str,
    ) -> AuthResult<()> {
        let normalized_email = email_validator::normalize_email(email);

        let mut user = self
            .user_store
            .get_user(&normalized_email)?
            .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;

        // Verify old password
        if !user.verify_password(old_password)? {
            return Err(AuthError::AuthenticationFailed);
        }

        // Validate new password
        UserRecord::validate_password(new_password)?;

        let encrypted_private_key = user
            .encrypted_private_key
            .clone()
            .ok_or_else(|| AuthError::CryptoError("Encrypted private key not found".to_string()))?;
        let decrypted_private_key = decrypt_private_key(&encrypted_private_key, old_password)?;

        // Hash and update password
        let new_hash = UserRecord::hash_password(new_password)?;
        user.password_hash = new_hash;
        user.encrypted_private_key =
            Some(encrypt_private_key(&decrypted_private_key, new_password)?);

        if let Some(secret) = user.totp_secret.take() {
            let plaintext = if secret.starts_with('{') {
                decrypt_private_key(&secret, old_password)?
            } else {
                secret
            };
            user.totp_secret = Some(encrypt_private_key(&plaintext, new_password)?);
        }

        // Persist the password change
        self.user_store.update_user(&user)?;

        // Invalidate all sessions for security
        self.logout_all(&normalized_email)?;

        log::info!("Password changed for user: {}", normalized_email);

        Ok(())
    }

    /// Save or refresh a pending 2FA enrollment for the given user.
    pub fn save_two_factor_setup(
        &mut self,
        email: &str,
        secret: String,
        backup_codes: Vec<String>,
        password: &str,
    ) -> AuthResult<()> {
        let normalized_email = email_validator::normalize_email(email);
        let mut user = self
            .user_store
            .get_user(&normalized_email)?
            .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;
        if user.totp_enabled {
            return Err(AuthError::ValidationError(
                "Disable existing 2FA before creating a new setup".to_string(),
            ));
        }

        user.set_two_factor_setup(secret, backup_codes, password)?;
        self.user_store.update_user(&user)?;
        Ok(())
    }

    /// Return current 2FA state, including pending setup information.
    pub fn get_two_factor_status(
        &mut self,
        email: &str,
        password: &str,
    ) -> AuthResult<Option<TwoFactorStatus>> {
        let normalized_email = email_validator::normalize_email(email);
        let mut user = self
            .user_store
            .get_user(&normalized_email)?
            .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;

        let secret = match user.totp_secret.clone() {
            Some(secret) if secret.starts_with('{') => {
                Some(decrypt_private_key(&secret, password)?)
            }
            Some(secret) => {
                if !user.verify_password(password)? {
                    return Err(AuthError::AuthenticationFailed);
                }
                user.totp_secret = Some(encrypt_private_key(&secret, password)?);
                self.user_store.update_user(&user)?;
                Some(secret)
            }
            None => None,
        };

        Ok(secret.map(|secret| TwoFactorStatus {
            secret,
            enabled: user.totp_enabled,
            backup_codes: user.backup_codes,
        }))
    }

    /// Mark the stored 2FA setup as enabled.
    pub fn enable_two_factor(&mut self, email: &str) -> AuthResult<()> {
        let normalized_email = email_validator::normalize_email(email);
        let mut user = self
            .user_store
            .get_user(&normalized_email)?
            .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;

        if user.totp_secret.is_none() {
            return Err(AuthError::ValidationError(
                "No pending 2FA setup found".to_string(),
            ));
        }

        user.enable_two_factor();
        self.user_store.update_user(&user)?;
        Ok(())
    }

    /// Disable 2FA and clear persisted recovery data.
    pub fn disable_two_factor(&mut self, email: &str) -> AuthResult<bool> {
        let normalized_email = email_validator::normalize_email(email);
        let mut user = self
            .user_store
            .get_user(&normalized_email)?
            .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;

        if user.totp_secret.is_none() && !user.totp_enabled && user.backup_codes.is_empty() {
            return Ok(false);
        }

        user.disable_two_factor();
        self.user_store.update_user(&user)?;
        Ok(true)
    }

    /// Consume a persisted one-time backup code.
    pub fn consume_backup_code(&mut self, email: &str, backup_code: &str) -> AuthResult<bool> {
        let normalized_email = email_validator::normalize_email(email);
        let mut user = self
            .user_store
            .get_user(&normalized_email)?
            .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;

        if !user.totp_enabled {
            return Ok(false);
        }

        let consumed = user.consume_backup_code(backup_code);
        if consumed {
            self.user_store.update_user(&user)?;
        }

        Ok(consumed)
    }

    /// Delete a user account
    ///
    /// # Arguments
    /// * `email` - User's email
    /// * `password` - Password for verification
    pub fn delete_account(&mut self, email: &str, password: &str) -> AuthResult<()> {
        let normalized_email = email_validator::normalize_email(email);

        let user = self
            .user_store
            .get_user(&normalized_email)?
            .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;

        // Verify password before deletion
        if !user.verify_password(password)? {
            return Err(AuthError::AuthenticationFailed);
        }

        // Invalidate all sessions
        self.logout_all(&normalized_email)?;

        // Remove user
        self.user_store.remove_user(&normalized_email)?;

        log::info!("Account deleted: {}", normalized_email);

        Ok(())
    }

    /// List all registered users
    pub fn list_users(&self) -> Vec<String> {
        self.user_store.list_users().unwrap_or_default()
    }

    /// Get total number of registered users
    pub fn user_count(&self) -> usize {
        self.user_store.user_count().unwrap_or(0)
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_login() {
        let mut auth = AuthManager::new();

        // Register user
        let wallet = auth
            .register_user(
                "test@example.com",
                "SecurePass123!",
                Some(CurveType::Ed25519),
            )
            .unwrap();

        assert!(!wallet.address.to_string().is_empty());

        // Login
        let session = auth
            .login("test@example.com", "SecurePass123!", None)
            .unwrap();

        assert!(!session.session_id.is_empty());
        assert_eq!(session.email, "test@example.com");
    }

    #[test]
    fn test_duplicate_registration() {
        let mut auth = AuthManager::new();

        auth.register_user(
            "test@example.com",
            "SecurePass123!",
            Some(CurveType::Ed25519),
        )
        .unwrap();

        // Second registration should fail
        let result = auth.register_user(
            "test@example.com",
            "AnotherPass456!",
            Some(CurveType::Ed25519),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_login() {
        let mut auth = AuthManager::new();

        auth.register_user(
            "test@example.com",
            "SecurePass123!",
            Some(CurveType::Ed25519),
        )
        .unwrap();

        // Wrong password
        let result = auth.login("test@example.com", "WrongPassword", None);
        assert!(result.is_err());

        // Non-existent user
        let result = auth.login("nonexistent@example.com", "SecurePass123!", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_logout() {
        let mut auth = AuthManager::new();

        auth.register_user(
            "test@example.com",
            "SecurePass123!",
            Some(CurveType::Ed25519),
        )
        .unwrap();

        let session = auth
            .login("test@example.com", "SecurePass123!", None)
            .unwrap();

        let session_id = session.session_id.clone();

        // Logout
        assert!(auth.logout(&session_id).is_ok());

        // Session should be invalid
        assert!(auth.validate_session(&session_id).is_err());
    }

    #[test]
    fn test_password_change() {
        let mut auth = AuthManager::new();

        auth.register_user(
            "test@example.com",
            "SecurePass123!",
            Some(CurveType::Ed25519),
        )
        .unwrap();

        // Change password
        assert!(
            auth.change_password("test@example.com", "SecurePass123!", "NewPass456!")
                .is_ok()
        );

        // Old password should fail
        assert!(
            auth.login("test@example.com", "SecurePass123!", None)
                .is_err()
        );

        // New password should work
        assert!(auth.login("test@example.com", "NewPass456!", None).is_ok());
    }

    #[test]
    fn test_legacy_totp_secret_is_migrated_after_password_verification() {
        let mut auth = AuthManager::new();
        auth.register_user(
            "legacy@example.com",
            "SecurePass123!",
            Some(CurveType::Ed25519),
        )
        .unwrap();

        let mut user = auth
            .user_store
            .get_user("legacy@example.com")
            .unwrap()
            .unwrap();
        user.totp_secret = Some("LEGACYPLAINTEXT".to_string());
        user.totp_enabled = true;
        auth.user_store.update_user(&user).unwrap();
        assert!(auth.has_legacy_two_factor_secrets().unwrap());

        assert!(
            auth.get_two_factor_status("legacy@example.com", "WrongPassword123!")
                .is_err()
        );
        assert!(auth.has_legacy_two_factor_secrets().unwrap());

        let status = auth
            .get_two_factor_status("legacy@example.com", "SecurePass123!")
            .unwrap()
            .unwrap();
        assert_eq!(status.secret, "LEGACYPLAINTEXT");
        assert!(!auth.has_legacy_two_factor_secrets().unwrap());
    }
    #[test]
    fn test_enabled_two_factor_cannot_be_overwritten() {
        let mut auth = AuthManager::new();
        auth.register_user(
            "protected@example.com",
            "SecurePass123!",
            Some(CurveType::Ed25519),
        )
        .unwrap();
        auth.save_two_factor_setup(
            "protected@example.com",
            "ORIGINALSECRET".to_string(),
            vec!["ORIGINAL1".to_string()],
            "SecurePass123!",
        )
        .unwrap();
        auth.enable_two_factor("protected@example.com").unwrap();

        let result = auth.save_two_factor_setup(
            "protected@example.com",
            "ATTACKERSECRET".to_string(),
            vec!["ATTACKER1".to_string()],
            "SecurePass123!",
        );
        assert!(matches!(result, Err(AuthError::ValidationError(_))));
        let status = auth
            .get_two_factor_status("protected@example.com", "SecurePass123!")
            .unwrap()
            .unwrap();
        assert_eq!(status.secret, "ORIGINALSECRET");
        assert!(status.enabled);
    }
    #[test]
    fn test_persisted_two_factor_lifecycle() {
        let mut auth = AuthManager::new();
        auth.register_user(
            "test@example.com",
            "SecurePass123!",
            Some(CurveType::Ed25519),
        )
        .unwrap();

        auth.save_two_factor_setup(
            "test@example.com",
            "BASE32SECRET".to_string(),
            vec!["CODE1234".to_string()],
            "SecurePass123!",
        )
        .unwrap();

        let status = auth
            .get_two_factor_status("test@example.com", "SecurePass123!")
            .unwrap()
            .unwrap();
        assert!(!status.enabled);
        assert_eq!(status.secret, "BASE32SECRET");

        auth.enable_two_factor("test@example.com").unwrap();

        assert!(
            auth.consume_backup_code("test@example.com", "CODE1234")
                .unwrap()
        );
        assert!(
            !auth
                .consume_backup_code("test@example.com", "CODE1234")
                .unwrap()
        );

        assert!(auth.disable_two_factor("test@example.com").unwrap());
        assert!(
            auth.get_two_factor_status("test@example.com", "SecurePass123!")
                .unwrap()
                .is_none()
        );
    }
}
