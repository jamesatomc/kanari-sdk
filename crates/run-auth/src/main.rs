use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Request},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use axum_client_ip::ClientIpSource;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use kanari_auth::AuthManager;

mod audit_logger;
mod handlers;
mod models;
mod rate_limiter;
mod security;
mod two_factor;

const MAX_JSON_BODY_BYTES: usize = 64 * 1024;

fn client_ip_source_for_config(
    allow_insecure: bool,
    trusted_proxy_ips: Option<&str>,
) -> ClientIpSource {
    if !allow_insecure && trusted_proxy_ips.is_some_and(|value| !value.trim().is_empty()) {
        ClientIpSource::XRealIp
    } else {
        ClientIpSource::ConnectInfo
    }
}

fn build_router(state: AppState, cors: CorsLayer, client_ip_source: ClientIpSource) -> Router {
    Router::new()
        // Health check
        .route("/health", get(health_check))
        // Authentication routes
        .route("/api/v1/register", post(handlers::register))
        .route("/api/v1/login", post(handlers::login))
        .route("/api/v1/logout", post(handlers::logout))
        .route("/api/v1/logout-all", post(handlers::logout_all))
        .route("/api/v1/change-password", post(handlers::change_password))
        .route("/api/v1/delete-account", post(handlers::delete_account))
        // User management
        .route("/api/v1/user/info", get(handlers::get_user_info))
        // SECURITY FIX #5: Changed from GET to POST for encrypted key retrieval (requires session validation)
        .route(
            "/api/v1/user/encrypted-key",
            post(handlers::get_user_encrypted_key),
        )
        // Session validation
        .route("/api/v1/session/validate", get(handlers::validate_session))
        // Two-Factor Authentication routes
        .route("/api/v1/2fa/setup", post(handlers::setup_2fa))
        .route("/api/v1/2fa/enable", post(handlers::enable_2fa))
        .route("/api/v1/2fa/disable", post(handlers::disable_2fa))
        // Apply middleware and state
        .layer(DefaultBodyLimit::max(MAX_JSON_BODY_BYTES))
        .layer(client_ip_source.into_extension())
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(enforce_https))
        .with_state(state)
}

/// Application state shared across all handlers
#[derive(Clone)]
pub struct AppState {
    pub auth_manager: Arc<Mutex<AuthManager>>,
    pub audit_logger: audit_logger::AuditLogger,
    pub rate_limiter: rate_limiter::RateLimiter,
    pub totp_manager: two_factor::TotpManager,
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn is_trusted_https_request(
    peer_ip: std::net::IpAddr,
    forwarded_proto: Option<&str>,
    trusted_proxy_ips: Option<&str>,
) -> bool {
    let peer_is_trusted = peer_ip.is_loopback()
        || trusted_proxy_ips.is_some_and(|trusted| {
            trusted.split(',').any(|candidate| {
                candidate
                    .trim()
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip == peer_ip)
            })
        });
    let forwarded_https = forwarded_proto
        .and_then(|value| value.split(',').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"));

    peer_is_trusted && forwarded_https
}
async fn enforce_https(request: Request<Body>, next: Next) -> Response {
    if env_flag("AUTH_ALLOW_INSECURE_HTTP") {
        return next.run(request).await;
    }

    let trusted_proxy_ips = std::env::var("AUTH_TRUSTED_PROXY_IPS").ok();
    let is_secure = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|peer| {
            is_trusted_https_request(
                peer.0.ip(),
                request
                    .headers()
                    .get("x-forwarded-proto")
                    .and_then(|value| value.to_str().ok()),
                trusted_proxy_ips.as_deref(),
            )
        })
        .unwrap_or(false);

    if !is_secure {
        return (
            StatusCode::UPGRADE_REQUIRED,
            "HTTPS is required; connect through the trusted local reverse proxy",
        )
            .into_response();
    }

    next.run(request).await
}

/// Health check endpoint
async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "healthy",
        "service": "kanari-auth-api",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load crate-local configuration when launched from the workspace root.
    if dotenvy::from_filename("crates/run-auth/.env").is_err() {
        dotenvy::dotenv().ok();
    }
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "run_auth=debug,tower_http=info,axum=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Kanari Auth API server...");

    // Create auth manager with persistent storage.
    let db_path =
        PathBuf::from(std::env::var("AUTH_DB_PATH").unwrap_or_else(|_| "data/auth.db".to_string()));
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
        security::secure_path(parent, true)?;
    }

    let auth_manager = AuthManager::with_persistence(db_path.clone())?;
    security::secure_path(&db_path, false)?;

    if auth_manager.has_legacy_two_factor_secrets()?
        && !env_flag("AUTH_ALLOW_LEGACY_TOTP_MIGRATION")
    {
        anyhow::bail!(
            "legacy plaintext TOTP secrets detected; run temporarily with AUTH_ALLOW_LEGACY_TOTP_MIGRATION=true and require affected users to log in"
        );
    }

    // Initialize audit logger. Failure to secure or open the log is fatal.
    let audit_log_dir = std::env::var("AUDIT_LOG_DIR").ok().map(PathBuf::from);
    let audit_logger = audit_logger::AuditLogger::new(audit_log_dir)?;
    tracing::info!("Audit logging enabled: {:?}", audit_logger.log_path());

    // Initialize rate limiter (strict for auth endpoints)
    let rate_limiter = rate_limiter::RateLimiter::new(rate_limiter::RateLimitConfig::strict());

    // Initialize TOTP manager
    let totp_manager = two_factor::TotpManager::new(None);

    let state = AppState {
        auth_manager: Arc::new(Mutex::new(auth_manager)),
        audit_logger,
        rate_limiter,
        totp_manager,
    };

    // Configure a single explicit browser origin. Native clients do not need CORS.
    let allow_insecure = env_flag("AUTH_ALLOW_INSECURE_HTTP");
    let trusted_proxy_ips = std::env::var("AUTH_TRUSTED_PROXY_IPS").ok();
    let allowed_origin = std::env::var("AUTH_ALLOWED_ORIGIN").ok();
    if !allow_insecure && allowed_origin.is_none() {
        anyhow::bail!("AUTH_ALLOWED_ORIGIN must be set in secure mode");
    }
    let cors = if let Some(origin) = allowed_origin {
        if origin == "*" {
            anyhow::bail!("AUTH_ALLOWED_ORIGIN cannot be '*'");
        }
        if !allow_insecure && !origin.starts_with("https://") {
            anyhow::bail!("AUTH_ALLOWED_ORIGIN must use https:// in secure mode");
        }
        CorsLayer::new()
            .allow_origin(HeaderValue::from_str(&origin)?)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION])
    } else {
        CorsLayer::new()
    };

    let client_ip_source =
        client_ip_source_for_config(allow_insecure, trusted_proxy_ips.as_deref());
    tracing::info!("Client IP source: {}", client_ip_source);
    let app = build_router(state, cors, client_ip_source);

    // Get port from environment or default to 3000
    let port = std::env::var("AUTH_API_PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse::<u16>()?;

    let bind_address = std::env::var("AUTH_BIND_ADDRESS")
        .or_else(|_| std::env::var("AUTH_HOST"))
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let addr = format!("{}:{}", bind_address, port);
    if bind_address == "0.0.0.0" || bind_address == "::" {
        tracing::warn!(
            "Kanari Auth API is binding to all interfaces. Use this only on a trusted LAN or behind a firewall/reverse proxy."
        );
    }
    tracing::info!("Kanari Auth API listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
#[cfg(test)]
mod security_tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::task::JoinHandle;
    use tower::ServiceExt;

    const TEST_PASSWORD: &str = "SecurePass123!";

    fn unique_test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("kanari-run-auth-{name}-{unique}"))
    }

    fn test_app() -> Router {
        test_app_with_ip_source(ClientIpSource::ConnectInfo)
    }

    fn test_app_with_ip_source(client_ip_source: ClientIpSource) -> Router {
        test_app_with_config(client_ip_source, rate_limiter::RateLimitConfig::strict())
    }

    fn test_app_with_config(
        client_ip_source: ClientIpSource,
        rate_limit_config: rate_limiter::RateLimitConfig,
    ) -> Router {
        let audit_logger = audit_logger::AuditLogger::new(Some(unique_test_dir("audit"))).unwrap();
        let state = AppState {
            auth_manager: Arc::new(Mutex::new(AuthManager::new())),
            audit_logger,
            rate_limiter: rate_limiter::RateLimiter::new(rate_limit_config),
            totp_manager: two_factor::TotpManager::new(None),
        };
        build_router(state, CorsLayer::new(), client_ip_source)
    }

    async fn spawn_live_test_server() -> (String, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = test_app();
        let handle = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    fn unique_email(label: &str) -> String {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{label}-{unique}@example.com")
    }

    fn secure_json_request(method: &str, uri: &str, body: impl Into<Body>) -> Request<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-forwarded-proto", "https")
            .body(body.into())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 49152))));
        request
    }

    fn secure_auth_request(method: &str, uri: &str, session_id: &str) -> Request<Body> {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {session_id}"))
            .header("x-forwarded-proto", "https")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 49152))));
        request
    }

    async fn json_response(response: Response) -> (StatusCode, Value) {
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or_else(
            |_| serde_json::json!({"raw": String::from_utf8_lossy(&bytes).to_string()}),
        );
        (status, json)
    }

    #[test]
    fn direct_http_is_rejected() {
        assert!(!is_trusted_https_request(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            None,
            None,
        ));
    }

    #[test]
    fn loopback_https_proxy_is_trusted() {
        assert!(is_trusted_https_request(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("https"),
            None,
        ));
    }

    #[test]
    fn unlisted_proxy_cannot_spoof_https() {
        assert!(!is_trusted_https_request(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8)),
            Some("https"),
            Some("10.0.0.7"),
        ));
    }

    #[test]
    fn explicitly_listed_proxy_is_trusted() {
        assert!(is_trusted_https_request(
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8)),
            Some("https"),
            Some("10.0.0.7,10.0.0.8"),
        ));
    }

    #[test]
    fn spoofed_forwarded_proto_chain_is_rejected() {
        assert!(!is_trusted_https_request(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some("http, https"),
            None,
        ));
    }

    #[test]
    fn body_limit_is_explicitly_bounded_for_auth_api() {
        assert_eq!(MAX_JSON_BODY_BYTES, 64 * 1024);
    }

    #[test]
    fn trusted_proxy_config_uses_forwarded_client_ip() {
        assert_eq!(
            client_ip_source_for_config(false, Some("127.0.0.1")).to_string(),
            ClientIpSource::XRealIp.to_string()
        );
        assert_eq!(
            client_ip_source_for_config(false, None).to_string(),
            ClientIpSource::ConnectInfo.to_string()
        );
        assert_eq!(
            client_ip_source_for_config(true, Some("127.0.0.1")).to_string(),
            ClientIpSource::ConnectInfo.to_string()
        );
    }

    #[tokio::test]
    async fn e2e_register_login_validate_logout_replay() {
        let app = test_app();
        let email = "router-e2e@example.com";

        let register_body = serde_json::json!({
            "email": email,
            "password": TEST_PASSWORD,
            "curveType": "ed25519"
        })
        .to_string();
        let (status, register_json) = json_response(
            app.clone()
                .oneshot(secure_json_request(
                    "POST",
                    "/api/v1/register",
                    register_body,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{register_json}");
        assert_eq!(register_json["success"], true);

        let login_body = serde_json::json!({
            "email": email,
            "password": TEST_PASSWORD,
            "sessionTimeoutHours": 1
        })
        .to_string();
        let (status, login_json) = json_response(
            app.clone()
                .oneshot(secure_json_request("POST", "/api/v1/login", login_body))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{login_json}");
        let session_id = login_json["data"]["sessionId"].as_str().unwrap();
        assert!(!session_id.is_empty());
        assert!(login_json["data"]["encryptedPrivateKey"].as_str().is_some());

        let (status, validate_json) = json_response(
            app.clone()
                .oneshot(secure_auth_request(
                    "GET",
                    "/api/v1/session/validate",
                    session_id,
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{validate_json}");
        assert_eq!(validate_json["data"]["valid"], true);

        let logout_body = serde_json::json!({"sessionId": session_id}).to_string();
        let (status, logout_json) = json_response(
            app.clone()
                .oneshot(secure_json_request("POST", "/api/v1/logout", logout_body))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{logout_json}");

        let (status, replay_json) = json_response(
            app.oneshot(secure_auth_request(
                "GET",
                "/api/v1/session/validate",
                session_id,
            ))
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{replay_json}");
        assert_eq!(replay_json["data"]["valid"], false);
    }

    #[tokio::test]
    async fn e2e_rejects_oversized_json_body() {
        let app = test_app();
        let oversized_password = "A".repeat(MAX_JSON_BODY_BYTES + 1);
        let body = serde_json::json!({
            "email": "oversized@example.com",
            "password": oversized_password
        })
        .to_string();

        let response = app
            .oneshot(secure_json_request("POST", "/api/v1/register", body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn e2e_rejects_direct_http_request() {
        let app = test_app();
        let mut request = Request::builder()
            .method("GET")
            .uri("/health")
            .body(Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 49152))));

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
    }

    #[tokio::test]
    async fn e2e_trusted_proxy_rate_limit_uses_forwarded_client_ip() {
        let app = test_app_with_config(
            ClientIpSource::XRealIp,
            rate_limiter::RateLimitConfig {
                max_requests: 2,
                interval_secs: 60,
            },
        );

        for i in 0..2 {
            let body = serde_json::json!({
                "email": format!("xff-a-{i}@example.com"),
                "password": TEST_PASSWORD,
                "curveType": "ed25519"
            })
            .to_string();
            let mut request = secure_json_request("POST", "/api/v1/register", body);
            request
                .headers_mut()
                .insert("x-real-ip", HeaderValue::from_static("203.0.113.10"));
            let response = app.clone().oneshot(request).await.unwrap();
            assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        }

        let body = serde_json::json!({
            "email": "xff-a-over-limit@example.com",
            "password": TEST_PASSWORD,
            "curveType": "ed25519"
        })
        .to_string();
        let mut request = secure_json_request("POST", "/api/v1/register", body);
        request
            .headers_mut()
            .insert("x-real-ip", HeaderValue::from_static("203.0.113.10"));
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );

        let body = serde_json::json!({
            "email": "xff-b-not-limited@example.com",
            "password": TEST_PASSWORD,
            "curveType": "ed25519"
        })
        .to_string();
        let mut request = secure_json_request("POST", "/api/v1/register", body);
        request
            .headers_mut()
            .insert("x-real-ip", HeaderValue::from_static("203.0.113.11"));
        assert_eq!(
            app.oneshot(request).await.unwrap().status(),
            StatusCode::CREATED
        );
    }

    #[tokio::test]
    async fn live_http_register_login_validate_logout_replay() {
        let (base_url, server) = spawn_live_test_server().await;
        let client = reqwest::Client::new();
        let email = unique_email("live-auth");

        let register = client
            .post(format!("{base_url}/api/v1/register"))
            .header("x-forwarded-proto", "https")
            .json(&serde_json::json!({
                "email": email,
                "password": TEST_PASSWORD,
                "curveType": "ed25519"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(register.status(), StatusCode::CREATED);

        let login = client
            .post(format!("{base_url}/api/v1/login"))
            .header("x-forwarded-proto", "https")
            .json(&serde_json::json!({
                "email": email,
                "password": TEST_PASSWORD,
                "sessionTimeoutHours": 1
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let login_json: Value = login.json().await.unwrap();
        let session_id = login_json["data"]["sessionId"].as_str().unwrap();
        assert!(!session_id.is_empty());

        let validate = client
            .get(format!("{base_url}/api/v1/session/validate"))
            .header("x-forwarded-proto", "https")
            .bearer_auth(session_id)
            .send()
            .await
            .unwrap();
        assert_eq!(validate.status(), StatusCode::OK);
        let validate_json: Value = validate.json().await.unwrap();
        assert_eq!(validate_json["data"]["valid"], true);

        let logout = client
            .post(format!("{base_url}/api/v1/logout"))
            .header("x-forwarded-proto", "https")
            .json(&serde_json::json!({"sessionId": session_id}))
            .send()
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::OK);

        let replay = client
            .get(format!("{base_url}/api/v1/session/validate"))
            .header("x-forwarded-proto", "https")
            .bearer_auth(session_id)
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), StatusCode::OK);
        let replay_json: Value = replay.json().await.unwrap();
        assert_eq!(replay_json["data"]["valid"], false);

        server.abort();
    }

    #[tokio::test]
    async fn live_http_rejects_direct_http_without_trusted_proxy_header() {
        let (base_url, server) = spawn_live_test_server().await;
        let response = reqwest::get(format!("{base_url}/health")).await.unwrap();
        assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
        server.abort();
    }

    #[tokio::test]
    async fn live_http_rejects_login_bruteforce_burst() {
        let (base_url, server) = spawn_live_test_server().await;
        let client = reqwest::Client::new();
        let email = unique_email("live-bruteforce");

        let register = client
            .post(format!("{base_url}/api/v1/register"))
            .header("x-forwarded-proto", "https")
            .json(&serde_json::json!({
                "email": email,
                "password": TEST_PASSWORD,
                "curveType": "ed25519"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(register.status(), StatusCode::CREATED);

        let mut saw_rate_limit = false;
        for _ in 0..12 {
            let response = client
                .post(format!("{base_url}/api/v1/login"))
                .header("x-forwarded-proto", "https")
                .json(&serde_json::json!({
                    "email": email,
                    "password": "WrongPass123!"
                }))
                .send()
                .await
                .unwrap();
            if response.status() == StatusCode::TOO_MANY_REQUESTS {
                saw_rate_limit = true;
                assert!(response.headers().contains_key(header::RETRY_AFTER));
                break;
            }
        }

        assert!(saw_rate_limit);
        server.abort();
    }
}
