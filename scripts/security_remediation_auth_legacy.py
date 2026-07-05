#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def save(path: str, text: str) -> None:
    (ROOT / path).write_text(text, encoding="utf-8")


def replace_required(text: str, old: str, new: str, label: str) -> str:
    if new in text:
        return text
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one source pattern, found {count}")
    return text.replace(old, new, 1)


# ---------------------------------------------------------------------------
# Session key custody: zeroizing internal key and redacted Clone semantics.
# ---------------------------------------------------------------------------
path = "crates/kanari-auth/src/session.rs"
s = load(path)
s = s.replace("use zeroize::Zeroize;", "use zeroize::{Zeroize, Zeroizing};")
s = s.replace(
    "#[derive(Debug, Clone, Serialize, Deserialize)]\npub struct Session",
    "#[derive(Debug, Serialize, Deserialize)]\npub struct Session",
)
s = s.replace(
    "pub private_key: Option<String>,",
    "pub private_key: Option<Zeroizing<String>>,")
s = replace_required(
    s,
    "            private_key,\n            curve_type,",
    "            private_key: private_key.map(Zeroizing::new),\n            curve_type,",
    "zeroizing session key",
)
if "impl Clone for Session" not in s:
    marker = "impl Session {"
    impl_clone = """/// Cloning a session always produces a public/redacted view. Decrypted key
/// material is never duplicated into handler responses or temporary values.
impl Clone for Session {
    fn clone(&self) -> Self {
        Self {
            session_id: self.session_id.clone(),
            email: self.email.clone(),
            wallet_address: self.wallet_address.clone(),
            private_key: None,
            curve_type: self.curve_type,
            created_at: self.created_at,
            expires_at: self.expires_at,
            last_activity: self.last_activity,
            is_valid: self.is_valid,
        }
    }
}

"""
    if marker not in s:
        raise RuntimeError("Session impl marker missing")
    s = s.replace(marker, impl_clone + marker, 1)
# Store original keyed session and return redacted clone.
old = """        let session_id = session.session_id.clone();

        // Store session
        self.sessions.insert(session_id.clone(), session.clone());

        // Index by email
        self.email_sessions
            .entry(email)
            .or_default()
            .push(session_id);

        session"""
new = """        let session_id = session.session_id.clone();
        let public_session = session.clone();

        // Store the only copy containing decrypted key material.
        self.sessions.insert(session_id.clone(), session);

        // Index by email
        self.email_sessions
            .entry(email)
            .or_default()
            .push(session_id);

        public_session"""
s = replace_required(s, old, new, "redacted public session")
save(path, s)

# ---------------------------------------------------------------------------
# AuthManager: bounded legacy migration, slow login verification outside lock,
# and core-level session-owner authorization for encrypted-key retrieval.
# ---------------------------------------------------------------------------
path = "crates/kanari-auth/src/auth_manager.rs"
s = load(path)
s = s.replace(
    "use crate::private_key_crypto::{decrypt_private_key, encrypt_private_key};",
    "use crate::private_key_crypto::{\n    decrypt_private_key, decrypt_private_key_with_migration, encrypt_private_key,\n};",
)
if "pub struct LoginPreparation" not in s:
    marker = "/// Main authentication manager that coordinates user registration,"
    types = """#[derive(Debug, Clone)]
pub struct LoginPreparation {
    normalized_email: String,
    user: UserRecord,
}

#[derive(Debug)]
pub enum LoginVerification {
    Authenticated {
        normalized_email: String,
        user: UserRecord,
        private_key: String,
        curve_type: CurveType,
    },
    Rejected {
        user: UserRecord,
        error: AuthError,
    },
}

"""
    if marker not in s:
        raise RuntimeError("AuthManager type insertion marker missing")
    s = s.replace(marker, types + marker, 1)
# Insert staged login methods before existing login docs.
marker = "    /// Authenticate a user with email and password"
if "pub fn prepare_login(" not in s:
    methods = """    /// Load only the account record while holding the manager lock. Slow
    /// password verification and key derivation happen in `verify_login_preparation`
    /// after the async handler releases its global manager guard.
    pub fn prepare_login(&self, email: &str) -> AuthResult<LoginPreparation> {
        let normalized_email = email_validator::normalize_email(email);
        let user = self
            .user_store
            .get_user(&normalized_email)?
            .ok_or(AuthError::UserNotFound(normalized_email.clone()))?;
        if user.is_locked() {
            return Err(AuthError::AccountLocked);
        }
        Ok(LoginPreparation {
            normalized_email,
            user,
        })
    }

    pub fn verify_login_preparation(
        mut preparation: LoginPreparation,
        password: &str,
    ) -> LoginVerification {
        match preparation.user.verify_password(password) {
            Ok(true) => {}
            Ok(false) => {
                preparation.user.record_failed_attempt();
                return LoginVerification::Rejected {
                    user: preparation.user,
                    error: AuthError::AuthenticationFailed,
                };
            }
            Err(error) => {
                return LoginVerification::Rejected {
                    user: preparation.user,
                    error,
                };
            }
        }

        let encrypted = match preparation.user.encrypted_private_key.as_deref() {
            Some(encrypted) => encrypted,
            None => {
                return LoginVerification::Rejected {
                    user: preparation.user,
                    error: AuthError::CryptoError(
                        "Encrypted private key not found".to_string(),
                    ),
                };
            }
        };
        let (private_key, migrated) = match decrypt_private_key_with_migration(encrypted, password) {
            Ok(result) => result,
            Err(error) => {
                return LoginVerification::Rejected {
                    user: preparation.user,
                    error,
                };
            }
        };
        if let Some(migrated) = migrated {
            preparation.user.encrypted_private_key = Some(migrated);
        }
        let curve_type = match CurveType::from_str(&preparation.user.curve_type) {
            Ok(curve) => curve,
            Err(error) => {
                return LoginVerification::Rejected {
                    user: preparation.user,
                    error: AuthError::CryptoError(format!(
                        "Invalid stored curve type: {error}"
                    )),
                };
            }
        };
        preparation.user.record_successful_login();
        LoginVerification::Authenticated {
            normalized_email: preparation.normalized_email,
            user: preparation.user,
            private_key,
            curve_type,
        }
    }

    pub fn finish_login(
        &mut self,
        verification: LoginVerification,
        session_timeout: Option<Duration>,
    ) -> AuthResult<Session> {
        match verification {
            LoginVerification::Rejected { user, error } => {
                self.user_store.update_user(&user)?;
                Err(error)
            }
            LoginVerification::Authenticated {
                normalized_email,
                user,
                private_key,
                curve_type,
            } => {
                let wallet_address = user.wallet_address.clone();
                self.user_store.update_user(&user)?;
                Ok(self.session_manager.create_session(
                    normalized_email,
                    wallet_address,
                    Some(private_key),
                    curve_type,
                    session_timeout,
                ))
            }
        }
    }

"""
    if marker not in s:
        raise RuntimeError("login method insertion marker missing")
    s = s.replace(marker, methods + marker, 1)
# Replace original login implementation body with staged path. Locate exact function.
start = s.index("    pub fn login(\n")
body_start = s.index("    {", start) if False else -1
# Use known full signature/body delimiters.
old_start = """    pub fn login(
        &mut self,
        email: &str,
        password: &str,
        session_timeout: Option<Duration>,
    ) -> AuthResult<Session> {"""
if old_start in s and "self.finish_login(verification, session_timeout)" not in s[s.index(old_start):s.index(old_start)+500]:
    start = s.index(old_start)
    end = s.index("\n    /// Logout and invalidate a session", start)
    new_login = """    pub fn login(
        &mut self,
        email: &str,
        password: &str,
        session_timeout: Option<Duration>,
    ) -> AuthResult<Session> {
        let preparation = self.prepare_login(email)?;
        let verification = Self::verify_login_preparation(preparation, password);
        self.finish_login(verification, session_timeout)
    }
"""
    s = s[:start] + new_login + s[end:]
# Core-level authorization for encrypted key retrieval.
old = """    pub fn get_user_encrypted_key(
        &self,
        email: &str,
    ) -> AuthResult<(String, String, String, Option<String>)> {
        let user = self
            .user_store
            .get_user(email)?"""
new = """    pub fn get_user_encrypted_key(
        &mut self,
        session_id: &str,
        email: &str,
    ) -> AuthResult<(String, String, String, Option<String>)> {
        let normalized_email = email_validator::normalize_email(email);
        let session_email = self.validate_session(session_id)?.email.clone();
        if session_email != normalized_email {
            return Err(AuthError::AuthenticationFailed);
        }
        let user = self
            .user_store
            .get_user(&normalized_email)?"""
s = replace_required(s, old, new, "core encrypted-key authorization")
# Avoid cloning Zeroizing private key into a Wallet String.
s = s.replace(
    "            private_key.clone(),",
    "            private_key.as_str().to_string(),",
)
save(path, s)

# ---------------------------------------------------------------------------
# run-auth login: release Tokio mutex during password/KDF and enforce core API.
# ---------------------------------------------------------------------------
path = "crates/run-auth/src/handlers.rs"
s = load(path)
old = """    let normalized_email = kanari_auth::email_validator::normalize_email(&payload.email);
    let mut auth = state.auth_manager.lock().await;

    match auth.login(&payload.email, &payload.password, session_timeout) {"""
new = """    let normalized_email = kanari_auth::email_validator::normalize_email(&payload.email);
    let preparation = {
        let auth = state.auth_manager.lock().await;
        match auth.prepare_login(&payload.email) {
            Ok(preparation) => preparation,
            Err(error) => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(ApiResponse::<serde_json::Value>::error(&error.to_string())),
                )
                    .into_response();
            }
        }
    };
    let password = payload.password.clone();
    let verification = match tokio::task::spawn_blocking(move || {
        kanari_auth::AuthManager::verify_login_preparation(preparation, &password)
    })
    .await
    {
        Ok(verification) => verification,
        Err(error) => {
            error!("Login verification worker failed: {}", error);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::<serde_json::Value>::error("Login failed")),
            )
                .into_response();
        }
    };
    let mut auth = state.auth_manager.lock().await;

    match auth.finish_login(verification, session_timeout) {"""
s = replace_required(s, old, new, "login KDF outside mutex")
s = s.replace(
    "auth.get_user_encrypted_key(&payload.email)",
    "auth.get_user_encrypted_key(&session.session_id, &payload.email)",
    1,
)
s = s.replace(
    "match auth.get_user_encrypted_key(&payload.email) {",
    "match auth.get_user_encrypted_key(&payload.session_id, &payload.email) {",
)
save(path, s)

print("auth session custody, authorization, and KDF concurrency hardening applied")
