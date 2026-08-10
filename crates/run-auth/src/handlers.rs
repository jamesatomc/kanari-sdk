use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use axum_client_ip::ClientIp;
use kanari_crypto::keys::CurveType;
use serde::Serialize;
use tracing::{error, info, warn};

use crate::{
    AppState,
    models::{
        ApiResponse, ChangePasswordRequest, DeleteAccountRequest, EncryptedKeyResponse,
        LoginRequest, LoginResponse, LogoutAllRequest, LogoutRequest, RegisterRequest,
        RegisterResponse, UserInfoResponse, ValidateSessionResponse,
    },
};

const SECONDS_PER_HOUR: u64 = 3_600;
const MAX_SESSION_TIMEOUT_HOURS: u64 = 24 * 7;

fn parse_session_timeout_hours(hours: Option<u64>) -> Result<Option<std::time::Duration>, ()> {
    let Some(hours) = hours else {
        return Ok(None);
    };
    if hours == 0 || hours > MAX_SESSION_TIMEOUT_HOURS {
        return Err(());
    }
    hours
        .checked_mul(SECONDS_PER_HOUR)
        .map(std::time::Duration::from_secs)
        .map(Some)
        .ok_or(())
}

fn internal_error<T: Serialize>(message: &'static str) -> Json<ApiResponse<T>> {
    Json(ApiResponse::error(message))
}

fn bearer_session_id(headers: &HeaderMap) -> Result<&str, StatusCode> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let session_id = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(session_id)
}

fn validate_session_owner(
    auth: &mut kanari_auth::AuthManager,
    session_id: &str,
    email: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<serde_json::Value>>)> {
    let normalized_email = kanari_auth::email_validator::normalize_email(email);

    match auth.validate_session(session_id) {
        Ok(session) if session.email == normalized_email => Ok(()),
        Ok(_) => Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse::error(
                "Unauthorized: Session does not match email",
            )),
        )),
        Err(_) => Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::error("Invalid or expired session")),
        )),
    }
}

fn validate_session_owner_for<T: Serialize>(
    auth: &mut kanari_auth::AuthManager,
    session_id: &str,
    email: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<T>>)> {
    validate_session_owner(auth, session_id, email).map_err(|(status, json)| {
        (
            status,
            Json(ApiResponse::error(
                json.0.error.unwrap_or_else(|| "Request failed".to_string()),
            )),
        )
    })
}

fn build_rate_limit_response(
    retry_after_secs: u64,
) -> (StatusCode, HeaderMap, Json<ApiResponse<serde_json::Value>>) {
    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&retry_after_secs.to_string()) {
        headers.insert("Retry-After", value);
    }

    (
        StatusCode::TOO_MANY_REQUESTS,
        headers,
        Json(ApiResponse::error("Rate limit exceeded")),
    )
}

/// Register a new user
///
/// Supported curve types:
/// - **Classical ECC**: `ed25519`, `k256` (or `secp256k1`), `p256` (or `secp256r1`, `nist`)
/// - **Hybrid**: `ed25519dilithium3` (or `ed25519+dilithium3`), `k256dilithium3` (or `k256+dilithium3`)
///
/// Default: `ed25519` (if curve_type not provided)
pub async fn register(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    Json(payload): Json<RegisterRequest>,
) -> axum::response::Response {
    info!("Registration attempt for email: {}", payload.email);
    info!("Requested curve type: {:?}", payload.curve_type);

    if let Err(rate_limit) = state.rate_limiter.check_rate_limit(client_ip).await {
        state
            .audit_logger
            .log_failure(
                crate::audit_logger::AuditEventType::RateLimitExceeded,
                crate::audit_logger::AuditSeverity::Warning,
                Some(payload.email.clone()),
                Some(client_ip.to_string()),
                None,
                serde_json::json!({"endpoint": "register"}),
                "Rate limit exceeded".to_string(),
            )
            .await;
        return build_rate_limit_response(rate_limit.retry_after_secs).into_response();
    }

    // SECURITY FIX #2: Validate password strength before processing
    if let Err(e) = kanari_auth::UserRecord::validate_password(&payload.password) {
        warn!("Weak password rejected for email: {}", payload.email);
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::<serde_json::Value>::error(format!(
                "Password validation failed: {}",
                e
            ))),
        )
            .into_response();
    }

    // Parse curve type if provided
    let curve_type = if let Some(curve_str) = &payload.curve_type {
        info!("Parsing curve type string: '{}'", curve_str);
        match curve_str.as_str() {
            // Classical ECC
            "ed25519" => Some(CurveType::Ed25519),
            "k256" | "secp256k1" => Some(CurveType::K256),
            "p256" | "secp256r1" | "nist" => Some(CurveType::P256),

            // Hybrid Schemes
            "ed25519dilithium3" => Some(CurveType::Ed25519Dilithium3),
            "k256dilithium3" => Some(CurveType::K256Dilithium3),

            _ => {
                warn!("Invalid curve type: {}", curve_str);
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiResponse::<serde_json::Value>::error(
                        "Invalid curve type. Supported: ed25519, k256, p256, dilithium2, dilithium3, dilithium5, sphincsplus, ed25519dilithium3, k256dilithium3",
                    )),
                )
                    .into_response();
            }
        }
    } else {
        info!("No curve type specified, will use default (Ed25519)");
        None
    };

    info!("Final curve type to use: {:?}", curve_type);

    let mut auth = state.auth_manager.lock().await;

    match auth.register_user(&payload.email, &payload.password, curve_type) {
        Ok(wallet) => {
            info!("User registered successfully: {}", payload.email);
            let response = RegisterResponse {
                success: true,
                wallet_address: wallet.address.to_hex_literal(),
                message: "User registered successfully".to_string(),
            };
            state
                .audit_logger
                .log_success(
                    crate::audit_logger::AuditEventType::Registration,
                    Some(payload.email.clone()),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"curve_type": curve_type.map(|c| c.to_string())}),
                )
                .await;
            // SECURITY FIX #3: Remove sensitive data from logs - only log success/failure
            info!("Registration successful for email: {}", payload.email);
            (StatusCode::CREATED, Json(ApiResponse::success(response))).into_response()
        }
        Err(e) => {
            error!("Registration failed: {:?}", e);
            state
                .audit_logger
                .log_failure(
                    crate::audit_logger::AuditEventType::Registration,
                    crate::audit_logger::AuditSeverity::Warning,
                    Some(payload.email.clone()),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"curve_type": payload.curve_type.clone()}),
                    format!("{:?}", e),
                )
                .await;
            let status_code = match e {
                kanari_auth::AuthError::UserAlreadyExists(_) => StatusCode::CONFLICT,
                kanari_auth::AuthError::InvalidEmail(_)
                | kanari_auth::AuthError::InvalidPassword(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            let message = match status_code {
                StatusCode::CONFLICT => "User already exists",
                StatusCode::BAD_REQUEST => "Invalid registration data",
                _ => "Registration failed",
            };
            (
                status_code,
                Json(ApiResponse::<serde_json::Value>::error(message)),
            )
                .into_response()
        }
    }
}

/// Login user
pub async fn login(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    Json(payload): Json<LoginRequest>,
) -> axum::response::Response {
    info!("Login attempt for email: {}", payload.email);

    if let Err(rate_limit) = state.rate_limiter.check_rate_limit(client_ip).await {
        state
            .audit_logger
            .log_failure(
                crate::audit_logger::AuditEventType::RateLimitExceeded,
                crate::audit_logger::AuditSeverity::Warning,
                Some(payload.email.clone()),
                Some(client_ip.to_string()),
                None,
                serde_json::json!({"endpoint": "login"}),
                "Rate limit exceeded".to_string(),
            )
            .await;
        return build_rate_limit_response(rate_limit.retry_after_secs).into_response();
    }

    let session_timeout = match parse_session_timeout_hours(payload.session_timeout_hours) {
        Ok(timeout) => timeout,
        Err(()) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::<serde_json::Value>::error(
                    "sessionTimeoutHours must be between 1 and 168",
                )),
            )
                .into_response();
        }
    };

    let normalized_email = kanari_auth::email_validator::normalize_email(&payload.email);
    let mut auth = state.auth_manager.lock().await;

    match auth.login(&payload.email, &payload.password, session_timeout) {
        Ok(session) => {
            let two_factor_status =
                match auth.get_two_factor_status(&normalized_email, &payload.password) {
                    Ok(status) => status,
                    Err(e) => {
                        error!("Failed to read 2FA status: {:?}", e);
                        let _ = auth.logout(&session.session_id);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse::<serde_json::Value>::error(
                                "Failed to read 2FA status",
                            )),
                        )
                            .into_response();
                    }
                };
            let two_factor_enabled = two_factor_status
                .as_ref()
                .map(|status| status.enabled)
                .unwrap_or(false);
            if let Some(two_factor_status) = two_factor_status.filter(|status| status.enabled) {
                let verification = if let Some(code) = payload.totp_code.as_deref() {
                    if state
                        .totp_manager
                        .verify_code(&two_factor_status.secret, code)
                    {
                        Ok(crate::two_factor::VerificationMethod::Totp)
                    } else {
                        Err("Invalid TOTP code")
                    }
                } else if let Some(backup_code) = payload.backup_code.as_deref() {
                    match auth.consume_backup_code(&normalized_email, backup_code) {
                        Ok(true) => Ok(crate::two_factor::VerificationMethod::BackupCode),
                        Ok(false) => Err("Invalid backup code"),
                        Err(_) => Err("Failed to validate backup code"),
                    }
                } else {
                    Err("Two-factor authentication required")
                };

                match verification {
                    Ok(method) => {
                        state
                            .audit_logger
                            .log_success(
                                crate::audit_logger::AuditEventType::TwoFactorVerification,
                                Some(payload.email.clone()),
                                Some(client_ip.to_string()),
                                None,
                                serde_json::json!({"method": format!("{:?}", method)}),
                            )
                            .await;
                    }
                    Err(err) => {
                        let _ = auth.logout(&session.session_id);
                        state
                            .audit_logger
                            .log_failure(
                                crate::audit_logger::AuditEventType::TwoFactorVerification,
                                crate::audit_logger::AuditSeverity::Warning,
                                Some(payload.email.clone()),
                                Some(client_ip.to_string()),
                                None,
                                serde_json::json!({"endpoint": "login"}),
                                err.to_string(),
                            )
                            .await;
                        return (
                            StatusCode::UNAUTHORIZED,
                            Json(ApiResponse::<serde_json::Value>::error(
                                "Two-factor authentication required or invalid verification code",
                            )),
                        )
                            .into_response();
                    }
                }
            }

            info!("Login successful: {}", payload.email);
            let (_, _, curve_type, encrypted_private_key) =
                match auth.get_user_encrypted_key(&payload.email) {
                    Ok(data) => data,
                    Err(e) => {
                        error!("Failed to fetch encrypted key after login: {:?}", e);
                        let _ = auth.logout(&session.session_id);
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ApiResponse::<serde_json::Value>::error(
                                "Failed to fetch encrypted key after login",
                            )),
                        )
                            .into_response();
                    }
                };
            state
                .audit_logger
                .log_success(
                    crate::audit_logger::AuditEventType::LoginSuccess,
                    Some(payload.email.clone()),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"two_factor_enabled": two_factor_enabled}),
                )
                .await;
            (
                StatusCode::OK,
                Json(ApiResponse::success(LoginResponse {
                    success: true,
                    two_factor_enabled,
                    session_id: session.session_id.clone(),
                    user_email: session.email.clone(),
                    wallet_address: session.wallet_address.clone(),
                    curve_type,
                    encrypted_private_key,
                    expires_at: session.expires_at.to_rfc3339(),
                })),
            )
                .into_response()
        }
        Err(e) => {
            error!("Login failed: {:?}", e);
            state
                .audit_logger
                .log_failure(
                    crate::audit_logger::AuditEventType::LoginFailure,
                    crate::audit_logger::AuditSeverity::Warning,
                    Some(payload.email.clone()),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "login_failed"}),
                    format!("{:?}", e),
                )
                .await;
            let status_code = match e {
                kanari_auth::AuthError::AuthenticationFailed
                | kanari_auth::AuthError::UserNotFound(_) => StatusCode::UNAUTHORIZED,
                kanari_auth::AuthError::AccountLocked => StatusCode::FORBIDDEN,
                kanari_auth::AuthError::InvalidEmail(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            let message = match status_code {
                StatusCode::UNAUTHORIZED => "Invalid credentials",
                StatusCode::FORBIDDEN => "Account locked",
                StatusCode::BAD_REQUEST => "Invalid login request",
                _ => "Login failed",
            };
            (
                status_code,
                Json(ApiResponse::<serde_json::Value>::error(message)),
            )
                .into_response()
        }
    }
}

/// Logout user
pub async fn logout(
    State(state): State<AppState>,
    Json(payload): Json<LogoutRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    info!("Logout attempt");

    let mut auth = state.auth_manager.lock().await;

    match auth.logout(&payload.session_id) {
        Ok(_) => {
            info!("Logout successful");
            (
                StatusCode::OK,
                Json(ApiResponse::success(serde_json::json!({
                    "message": "Logged out successfully"
                }))),
            )
        }
        Err(e) => {
            error!("Logout failed: {:?}", e);
            let status_code = match e {
                kanari_auth::AuthError::InvalidSession | kanari_auth::AuthError::SessionExpired => {
                    StatusCode::UNAUTHORIZED
                }
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            let message = if status_code == StatusCode::UNAUTHORIZED {
                "Invalid or expired session"
            } else {
                "Logout failed"
            };
            (status_code, Json(ApiResponse::error(message)))
        }
    }
}

/// Logout all sessions for a user
pub async fn logout_all(
    State(state): State<AppState>,
    Json(payload): Json<LogoutAllRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    info!("Logout all sessions for email: {}", payload.email);

    let mut auth = state.auth_manager.lock().await;
    if let Err(response) =
        validate_session_owner_for(&mut auth, &payload.session_id, &payload.email)
    {
        return response;
    }

    match auth.logout_all(&payload.email) {
        Ok(_) => {
            info!("All sessions logged out for: {}", payload.email);
            (
                StatusCode::OK,
                Json(ApiResponse::success(serde_json::json!({
                    "message": "All sessions logged out successfully"
                }))),
            )
        }
        Err(e) => {
            error!("Logout all failed: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error("Logout all failed")),
            )
        }
    }
}

/// Change password
pub async fn change_password(
    State(state): State<AppState>,
    Json(payload): Json<ChangePasswordRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    info!("Password change attempt for email: {}", payload.email);

    let mut auth = state.auth_manager.lock().await;
    if let Err(response) =
        validate_session_owner_for(&mut auth, &payload.session_id, &payload.email)
    {
        return response;
    }

    match auth.change_password(&payload.email, &payload.old_password, &payload.new_password) {
        Ok(_) => {
            info!("Password changed successfully: {}", payload.email);
            (
                StatusCode::OK,
                Json(ApiResponse::success(serde_json::json!({
                    "message": "Password changed successfully. All sessions invalidated."
                }))),
            )
        }
        Err(e) => {
            error!("Password change failed: {:?}", e);
            let status_code = match e {
                kanari_auth::AuthError::AuthenticationFailed => StatusCode::UNAUTHORIZED,
                kanari_auth::AuthError::UserNotFound(_) => StatusCode::NOT_FOUND,
                kanari_auth::AuthError::InvalidPassword(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            let message = match status_code {
                StatusCode::UNAUTHORIZED => "Authentication failed",
                StatusCode::NOT_FOUND => "User not found",
                StatusCode::BAD_REQUEST => "Invalid new password",
                _ => "Password change failed",
            };
            (status_code, Json(ApiResponse::error(message)))
        }
    }
}

/// Delete account
pub async fn delete_account(
    State(state): State<AppState>,
    Json(payload): Json<DeleteAccountRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    info!("Account deletion attempt for email: {}", payload.email);

    let mut auth = state.auth_manager.lock().await;
    if let Err(response) =
        validate_session_owner_for(&mut auth, &payload.session_id, &payload.email)
    {
        return response;
    }

    match auth.delete_account(&payload.email, &payload.password) {
        Ok(_) => {
            info!("Account deleted successfully: {}", payload.email);
            (
                StatusCode::OK,
                Json(ApiResponse::success(serde_json::json!({
                    "message": "Account deleted successfully"
                }))),
            )
        }
        Err(e) => {
            error!("Account deletion failed: {:?}", e);
            let status_code = match e {
                kanari_auth::AuthError::AuthenticationFailed => StatusCode::UNAUTHORIZED,
                kanari_auth::AuthError::UserNotFound(_) => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            let message = match status_code {
                StatusCode::UNAUTHORIZED => "Authentication failed",
                StatusCode::NOT_FOUND => "User not found",
                _ => "Account deletion failed",
            };
            (status_code, Json(ApiResponse::error(message)))
        }
    }
}

/// Get user info from session
pub async fn get_user_info(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<UserInfoResponse>>) {
    let session_id = match bearer_session_id(&headers) {
        Ok(session_id) => session_id,
        Err(status) => {
            return (
                status,
                Json(ApiResponse::error(
                    "Missing or invalid Authorization header",
                )),
            );
        }
    };

    info!("Getting user info endpoint called");

    let mut auth = state.auth_manager.lock().await;

    let session = match auth.validate_session(session_id) {
        Ok(session) => session.clone(),
        Err(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse::error("Invalid or expired session")),
            );
        }
    };

    match auth.get_user_info(&session) {
        Ok((email, wallet_address)) => (
            StatusCode::OK,
            Json(ApiResponse::success(UserInfoResponse {
                success: true,
                email,
                wallet_address,
            })),
        ),
        Err(e) => {
            error!("Failed to load user info: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error("Failed to load user info")),
            )
        }
    }
}

/// Get user's encrypted private key for wallet restoration
/// SECURITY FIX #5: Now requires session validation to prevent unauthorized access
pub async fn get_user_encrypted_key(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    Json(payload): Json<crate::models::GetEncryptedKeyRequest>,
) -> (StatusCode, Json<ApiResponse<EncryptedKeyResponse>>) {
    info!(
        "Encrypted key retrieval attempt for email: {}",
        payload.email
    );

    let mut auth = state.auth_manager.lock().await;

    // SECURITY FIX #5: Validate session before returning encrypted key
    if let Err(response) =
        validate_session_owner_for(&mut auth, &payload.session_id, &payload.email)
    {
        state
            .audit_logger
            .log_failure(
                crate::audit_logger::AuditEventType::EncryptedKeyAccess,
                crate::audit_logger::AuditSeverity::Warning,
                Some(payload.email.clone()),
                Some(client_ip.to_string()),
                None,
                serde_json::json!({"action": "encrypted_key_access_denied"}),
                "Invalid or unauthorized session".to_string(),
            )
            .await;
        return response;
    }

    match auth.get_user_encrypted_key(&payload.email) {
        Ok((email, wallet_address, curve_type, encrypted_key)) => {
            info!("Encrypted key retrieved successfully for: {}", email);
            state
                .audit_logger
                .log_success(
                    crate::audit_logger::AuditEventType::EncryptedKeyAccess,
                    Some(email.clone()),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "encrypted_key_access_granted"}),
                )
                .await;
            (
                StatusCode::OK,
                Json(ApiResponse::success(EncryptedKeyResponse {
                    success: true,
                    email,
                    wallet_address,
                    curve_type,
                    encrypted_private_key: encrypted_key,
                })),
            )
        }
        Err(e) => {
            error!("Encrypted key retrieval failed: {:?}", e);
            state
                .audit_logger
                .log_failure(
                    crate::audit_logger::AuditEventType::EncryptedKeyAccess,
                    crate::audit_logger::AuditSeverity::Warning,
                    Some(payload.email.clone()),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "encrypted_key_access_failed"}),
                    "Encrypted key retrieval failed".to_string(),
                )
                .await;
            let status_code = match e {
                kanari_auth::AuthError::UserNotFound(_) => StatusCode::NOT_FOUND,
                kanari_auth::AuthError::AccountLocked => StatusCode::FORBIDDEN,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            let message = match status_code {
                StatusCode::NOT_FOUND => "User not found",
                StatusCode::FORBIDDEN => "Account locked",
                _ => "Encrypted key retrieval failed",
            };
            (status_code, Json(ApiResponse::error(message)))
        }
    }
}

/// Validate session
pub async fn validate_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<ValidateSessionResponse>>) {
    let session_id = match bearer_session_id(&headers) {
        Ok(session_id) => session_id,
        Err(status) => {
            return (
                status,
                Json(ApiResponse::error(
                    "Missing or invalid Authorization header",
                )),
            );
        }
    };

    info!("Validating session");

    let mut auth = state.auth_manager.lock().await;

    match auth.validate_session(session_id) {
        Ok(_session) => (
            StatusCode::OK,
            Json(ApiResponse::success(ValidateSessionResponse {
                valid: true,
                session_id: session_id.to_string(),
            })),
        ),
        Err(e) => {
            let is_expired = matches!(e, kanari_auth::AuthError::SessionExpired);
            let is_invalid = matches!(e, kanari_auth::AuthError::InvalidSession);

            if is_expired || is_invalid {
                (
                    StatusCode::OK,
                    Json(ApiResponse::success(ValidateSessionResponse {
                        valid: false,
                        session_id: session_id.to_string(),
                    })),
                )
            } else {
                error!("Session validation failed: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    internal_error("Session validation failed"),
                )
            }
        }
    }
}

/// Setup 2FA for a user (generate QR code and secret)
pub async fn setup_2fa(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    headers: HeaderMap,
    Json(payload): Json<crate::models::TwoFactorSetupRequest>,
) -> (
    StatusCode,
    Json<ApiResponse<crate::models::TwoFactorSetupResponse>>,
) {
    info!("2FA setup requested for email: {}", payload.email);

    if let Err(limit) = state.rate_limiter.check_rate_limit(client_ip).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::error(format!(
                "Rate limit exceeded; retry in {} seconds",
                limit.retry_after_secs
            ))),
        );
    }

    let mut auth = state.auth_manager.lock().await;

    let session_id = match bearer_session_id(&headers) {
        Ok(session_id) => session_id,
        Err(status) => {
            return (
                status,
                Json(ApiResponse::error(
                    "Missing or invalid Authorization header",
                )),
            );
        }
    };
    if let Err((status, response)) = validate_session_owner(&mut auth, session_id, &payload.email) {
        return (
            status,
            Json(ApiResponse::error(
                response
                    .0
                    .error
                    .unwrap_or_else(|| "Unauthorized".to_string()),
            )),
        );
    }
    if matches!(auth.get_two_factor_status(&payload.email, &payload.password), Ok(Some(status)) if status.enabled)
    {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error(
                "Disable existing 2FA before creating a new setup",
            )),
        );
    }

    // Verify user exists and password is correct
    match auth.login(&payload.email, &payload.password, None) {
        Ok(session) => {
            // Generate 2FA setup
            let normalized_email = kanari_auth::email_validator::normalize_email(&payload.email);
            let setup = state.totp_manager.generate_setup(&normalized_email);
            if let Err(e) = auth.save_two_factor_setup(
                &normalized_email,
                setup.secret.clone(),
                setup.backup_codes.clone(),
                &payload.password,
            ) {
                error!("Failed to persist 2FA setup: {:?}", e);
                let _ = auth.logout(&session.session_id);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse::error("Failed to persist 2FA setup")),
                );
            }
            let qr_code_svg = state
                .totp_manager
                .generate_qr_svg(&setup.otpauth_url)
                .unwrap_or_default();

            // Log the event
            state
                .audit_logger
                .log_success(
                    crate::audit_logger::AuditEventType::TwoFactorSetup,
                    Some(payload.email.clone()),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "2fa_setup_initiated"}),
                )
                .await;

            // Invalidate the temporary session
            let _ = auth.logout(&session.session_id);

            (
                StatusCode::OK,
                Json(ApiResponse::success(
                    crate::models::TwoFactorSetupResponse {
                        success: true,
                        secret: setup.secret,
                        otpauth_url: setup.otpauth_url,
                        qr_code_svg,
                        backup_codes: setup.backup_codes,
                        message: "Scan the QR code with your authenticator app".to_string(),
                    },
                )),
            )
        }
        Err(e) => {
            error!("2FA setup failed - authentication error: {:?}", e);
            state
                .audit_logger
                .log_failure(
                    crate::audit_logger::AuditEventType::LoginFailure,
                    crate::audit_logger::AuditSeverity::Warning,
                    Some(payload.email),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "2fa_setup_failed"}),
                    format!("{:?}", e),
                )
                .await;

            (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse::error("Authentication failed")),
            )
        }
    }
}

/// Enable 2FA for a user (verify TOTP code and save)
pub async fn enable_2fa(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    headers: HeaderMap,
    Json(payload): Json<crate::two_factor::Enable2faRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if let Err(limit) = state.rate_limiter.check_rate_limit(client_ip).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::error(format!(
                "Rate limit exceeded; retry in {} seconds",
                limit.retry_after_secs
            ))),
        );
    }
    let normalized_email = kanari_auth::email_validator::normalize_email(&payload.email);
    let mut auth = state.auth_manager.lock().await;
    let session_id = match bearer_session_id(&headers) {
        Ok(session_id) => session_id,
        Err(status) => {
            return (
                status,
                Json(ApiResponse::error(
                    "Missing or invalid Authorization header",
                )),
            );
        }
    };
    if let Err((status, response)) = validate_session_owner(&mut auth, session_id, &payload.email) {
        return (
            status,
            Json(ApiResponse::error(
                response
                    .0
                    .error
                    .unwrap_or_else(|| "Unauthorized".to_string()),
            )),
        );
    }

    let temp_session = match auth.login(&payload.email, &payload.password, None) {
        Ok(session) => session,
        Err(_) => {
            state
                .audit_logger
                .log_failure(
                    crate::audit_logger::AuditEventType::TwoFactorEnabled,
                    crate::audit_logger::AuditSeverity::Warning,
                    Some(payload.email),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "2fa_enable_failed"}),
                    "Authentication failed".to_string(),
                )
                .await;
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse::error("Authentication failed")),
            );
        }
    };
    let _ = auth.logout(&temp_session.session_id);

    let status = match auth.get_two_factor_status(&normalized_email, &payload.password) {
        Ok(Some(status)) => status,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiResponse::error("No pending 2FA setup found")),
            );
        }
        Err(e) => {
            error!("Failed to load 2FA state: {:?}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error("Failed to load 2FA state")),
            );
        }
    };

    if status.enabled {
        return (
            StatusCode::CONFLICT,
            Json(ApiResponse::error("2FA is already enabled")),
        );
    }

    if !state
        .totp_manager
        .verify_code(&status.secret, &payload.code)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error("Invalid TOTP code")),
        );
    }

    match auth.enable_two_factor(&normalized_email) {
        Ok(()) => {
            state
                .audit_logger
                .log_success(
                    crate::audit_logger::AuditEventType::TwoFactorEnabled,
                    Some(normalized_email),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "2fa_enabled"}),
                )
                .await;
            (
                StatusCode::OK,
                Json(ApiResponse::success(serde_json::json!({
                    "message": "2FA enabled successfully"
                }))),
            )
        }
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse::error(err.to_string())),
        ),
    }
}

/// Disable 2FA for a user
pub async fn disable_2fa(
    State(state): State<AppState>,
    ClientIp(client_ip): ClientIp,
    headers: HeaderMap,
    Json(payload): Json<crate::two_factor::Disable2faRequest>,
) -> (StatusCode, Json<ApiResponse<serde_json::Value>>) {
    if let Err(limit) = state.rate_limiter.check_rate_limit(client_ip).await {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ApiResponse::error(format!(
                "Rate limit exceeded; retry in {} seconds",
                limit.retry_after_secs
            ))),
        );
    }
    let mut auth = state.auth_manager.lock().await;
    let session_id = match bearer_session_id(&headers) {
        Ok(session_id) => session_id,
        Err(status) => {
            return (
                status,
                Json(ApiResponse::error(
                    "Missing or invalid Authorization header",
                )),
            );
        }
    };
    if let Err((status, response)) = validate_session_owner(&mut auth, session_id, &payload.email) {
        return (
            status,
            Json(ApiResponse::error(
                response
                    .0
                    .error
                    .unwrap_or_else(|| "Unauthorized".to_string()),
            )),
        );
    }
    let temp_session = match auth.login(&payload.email, &payload.password, None) {
        Ok(session) => session,
        Err(_) => {
            state
                .audit_logger
                .log_failure(
                    crate::audit_logger::AuditEventType::TwoFactorDisabled,
                    crate::audit_logger::AuditSeverity::Warning,
                    Some(payload.email),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "2fa_disable_failed"}),
                    "Authentication failed".to_string(),
                )
                .await;
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiResponse::error("Authentication failed")),
            );
        }
    };
    let _ = auth.logout(&temp_session.session_id);

    let normalized_email = kanari_auth::email_validator::normalize_email(&payload.email);
    let status = match auth.get_two_factor_status(&normalized_email, &payload.password) {
        Ok(Some(status)) if status.enabled => status,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiResponse::error("2FA is not enabled for this user")),
            );
        }
    };
    if !state
        .totp_manager
        .verify_code(&status.secret, &payload.code)
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiResponse::error("Invalid TOTP code")),
        );
    }
    match auth.disable_two_factor(&normalized_email) {
        Ok(true) => {
            state
                .audit_logger
                .log_success(
                    crate::audit_logger::AuditEventType::TwoFactorDisabled,
                    Some(normalized_email),
                    Some(client_ip.to_string()),
                    None,
                    serde_json::json!({"action": "2fa_disabled"}),
                )
                .await;
            (
                StatusCode::OK,
                Json(ApiResponse::success(serde_json::json!({
                    "message": "2FA disabled successfully"
                }))),
            )
        }
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::error("2FA is not enabled for this user")),
        ),
        Err(e) => {
            error!("Failed to disable 2FA: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse::error("Failed to disable 2FA")),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_PASSWORD: &str = "SecurePass123!";

    #[test]
    fn session_timeout_rejects_zero_and_over_cap() {
        assert!(parse_session_timeout_hours(Some(0)).is_err());
        assert!(parse_session_timeout_hours(Some(MAX_SESSION_TIMEOUT_HOURS + 1)).is_err());
    }

    #[test]
    fn session_timeout_rejects_overflow() {
        assert!(parse_session_timeout_hours(Some(u64::MAX)).is_err());
    }

    #[test]
    fn session_timeout_accepts_valid_bounds() {
        assert_eq!(parse_session_timeout_hours(None).unwrap(), None);
        assert_eq!(
            parse_session_timeout_hours(Some(1)).unwrap(),
            Some(std::time::Duration::from_secs(SECONDS_PER_HOUR))
        );
        assert_eq!(
            parse_session_timeout_hours(Some(MAX_SESSION_TIMEOUT_HOURS)).unwrap(),
            Some(std::time::Duration::from_secs(
                MAX_SESSION_TIMEOUT_HOURS * SECONDS_PER_HOUR
            ))
        );
    }

    #[test]
    fn session_owner_rejects_cross_account_reuse() {
        let mut auth = kanari_auth::AuthManager::new();
        auth.register_user("alice@example.com", TEST_PASSWORD, None)
            .unwrap();
        auth.register_user("bob@example.com", TEST_PASSWORD, None)
            .unwrap();
        let session = auth
            .login("alice@example.com", TEST_PASSWORD, None)
            .unwrap();

        assert!(
            validate_session_owner(&mut auth, &session.session_id, "alice@example.com").is_ok()
        );
        let err =
            validate_session_owner(&mut auth, &session.session_id, "bob@example.com").unwrap_err();
        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn logged_out_session_cannot_be_replayed() {
        let mut auth = kanari_auth::AuthManager::new();
        auth.register_user("alice@example.com", TEST_PASSWORD, None)
            .unwrap();
        let session_id = auth
            .login("alice@example.com", TEST_PASSWORD, None)
            .unwrap()
            .session_id
            .clone();

        validate_session_owner(&mut auth, &session_id, "alice@example.com").unwrap();
        auth.logout(&session_id).unwrap();

        let err = validate_session_owner(&mut auth, &session_id, "alice@example.com").unwrap_err();
        assert_eq!(err.0, StatusCode::UNAUTHORIZED);
    }
}
