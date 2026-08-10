# Kanari Auth API Server

A RESTful HTTP API server for the Kanari Auth authentication system, built with Axum.

## Overview

This crate provides a production-ready HTTP API wrapper around the `kanari-auth` library, enabling email-based authentication and transaction signing through standard HTTP endpoints. It supports persistent SQLite storage, secure session management, and integration with the Kanari blockchain wallet infrastructure.

## Features

✅ **RESTful API** - Standard HTTP endpoints for all auth operations  
✅ **Persistent Storage** - SQLite backend with configurable path  
✅ **Multi-Curve Support** - Ed25519, K256, P256, Dilithium2/3/5  
✅ **Session Management** - UUID-based sessions with configurable timeouts  
✅ **CORS Support** - Cross-origin resource sharing enabled by default  
✅ **Structured Logging** - Tracing-based logging with env-filter support  
✅ **Health Check** - Built-in health monitoring endpoint  
✅ **Error Handling** - Consistent JSON error responses with HTTP status codes  

## Quick Start

### Running the Server

```bash
# Local development loads crates/run-auth/.env automatically.
cp crates/run-auth/.env.example crates/run-auth/.env
cargo run -p run-auth

# LAN development, reachable as http://192.168.1.101:3000 from another device.
# Keep this behind a trusted firewall and use HTTPS/reverse proxy for production.
export AUTH_ALLOW_INSECURE_HTTP=true
export AUTH_BIND_ADDRESS=0.0.0.0
export AUTH_ALLOWED_ORIGIN=http://192.168.1.101:3000
export AUTH_API_PORT=3000
cargo run -p run-auth

# Production behind a local HTTPS reverse proxy.
export AUTH_DB_PATH=data/auth.db
export AUTH_ALLOWED_ORIGIN=https://auth.example.com
export AUTH_BIND_ADDRESS=127.0.0.1
cargo run -p run-auth
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `AUTH_DB_PATH` | Path to SQLite database file | `data/auth.db` |
| `AUDIT_LOG_DIR` | Protected audit log directory | `logs` |
| `AUTH_API_PORT` | HTTP server port | `3000` |
| `AUTH_BIND_ADDRESS` | Bind address; use `0.0.0.0` only for trusted LAN/reverse-proxy exposure | `127.0.0.1` |
| `AUTH_HOST` | Alias for `AUTH_BIND_ADDRESS` when `AUTH_BIND_ADDRESS` is unset | None |
| `AUTH_ALLOWED_ORIGIN` | Exact browser origin; required in secure mode and cannot be `*` | None |
| `AUTH_TRUSTED_PROXY_IPS` | Comma-separated reverse-proxy IPs allowed to assert HTTPS | Loopback only |
| `AUTH_ALLOW_INSECURE_HTTP` | Explicit local-development override | `false` |
| `AUTH_ALLOW_LEGACY_TOTP_MIGRATION` | Temporary migration mode for old plaintext TOTP records | `false` |
| `RUST_LOG` | Log level filter | `run_auth=debug,tower_http=info,axum=info` |

## API Endpoints

### Health Check

```http
GET /health
```

**Response:**

```json
{
  "status": "healthy",
  "service": "kanari-auth-api",
  "timestamp": "2024-01-15T10:30:00Z"
}
```

---

### Authentication

#### Register User

```http
POST /api/v1/register
Content-Type: application/json

{
  "email": "user@example.com",
  "password": "SecurePass123!",
  "curve_type": "ed25519"  // optional: ed25519, k256, p256, dilithium2, dilithium3, dilithium5
}
```

**Response (201 Created):**

```json
{
  "success": true,
  "data": {
    "success": true,
    "wallet_address": "0x1234...",
    "message": "User registered successfully"
  },
  "error": null
}
```

**Supported Curve Types:**

**Classical Elliptic Curve Cryptography (ECC):**

- `ed25519` - Ed25519 curve (default, fast & secure)
- `k256` or `secp256k1` - Secp256k1 curve (Bitcoin/Ethereum compatible)
- `p256`, `secp256r1`, or `nist` - NIST P-256 curve (Enterprise standard)

**Post-Quantum Cryptography (PQC):**

- `dilithium2` - Dilithium2, Level 2 security (~2.5KB signatures)
- `dilithium3` - Dilithium3, Level 3 security (~4KB signatures) **← Recommended**
- `dilithium5` - Dilithium5, Level 5 security (~5KB signatures)
- `sphincsplus`, `sphincs+sha256`, or `sphincs` - SPHINCS+ hash-based (~50KB signatures)

**Hybrid Schemes (Classical + PQC):**

- `ed25519dilithium3` or `ed25519+dilithium3` - Ed25519 + Dilithium3 hybrid
- `k256dilithium3` or `k256+dilithium3` - K256 + Dilithium3 hybrid (Bitcoin-compatible + quantum-safe)

#### Login

```http
POST /api/v1/login
Content-Type: application/json

{
  "email": "user@example.com",
  "password": "SecurePass123!",
  "session_timeout_hours": 24  // optional
}
```

**Response (200 OK):**

```json
{
  "success": true,
  "data": {
    "success": true,
    "session_id": "550e8400-e29b-41d4-a716-446655440000",
    "user_email": "user@example.com",
    "wallet_address": "0x1234...",
    "expires_at": "2024-01-16T10:30:00Z"
  },
  "error": null
}
```

#### Logout

```http
POST /api/v1/logout
Content-Type: application/json

{
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Response (200 OK):**

```json
{
  "success": true,
  "data": {
    "message": "Logged out successfully"
  },
  "error": null
}
```

#### Logout All Sessions

```http
POST /api/v1/logout-all
Content-Type: application/json

{
  "email": "user@example.com"
}
```

**Response (200 OK):**

```json
{
  "success": true,
  "data": {
    "message": "All sessions logged out successfully"
  },
  "error": null
}
```

#### Change Password

```http
POST /api/v1/change-password
Content-Type: application/json

{
  "email": "user@example.com",
  "old_password": "OldPass123!",
  "new_password": "NewSecurePass456!"
}
```

**Response (200 OK):**

```json
{
  "success": true,
  "data": {
    "message": "Password changed successfully. All sessions invalidated."
  },
  "error": null
}
```

#### Delete Account

```http
POST /api/v1/delete-account
Content-Type: application/json

{
  "email": "user@example.com",
  "password": "SecurePass123!"
}
```

**Response (200 OK):**

```json
{
  "success": true,
  "data": {
    "message": "Account deleted successfully"
  },
  "error": null
}
```

---

### User Management

#### List Users

```http
GET /api/v1/users
```

**Response (200 OK):**

```json
{
  "success": true,
  "data": {
    "users": ["user1@example.com", "user2@example.com"],
    "count": 2
  },
  "error": null
}
```

#### Get User Count

```http
GET /api/v1/users/count
```

**Response (200 OK):**

```json
{
  "success": true,
  "data": {
    "count": 2
  },
  "error": null
}
```

---

### Session Validation

#### Validate Session

```http
GET /api/v1/session/validate/:session_id
```

**Response (200 OK):**

```json
{
  "success": true,
  "data": {
    "valid": true,
    "session_id": "550e8400-e29b-41d4-a716-446655440000"
  },
  "error": null
}
```

---

## Error Responses

All errors follow a consistent format:

```json
{
  "success": false,
  "data": null,
  "error": "Error description message"
}
```

### HTTP Status Codes

| Code | Meaning               | Example                       |
|------|-----------------------|-------------------------------|
|  200 | Success               | Successful operation          |
|  201 | Created               | User registered               |
|  400 | Bad Request           | Invalid email/password format |
|  401 | Unauthorized          | Invalid credentials           |
|  403 | Forbidden             | Account locked                |
|  404 | Not Found             | User not found                |
|  409 | Conflict              | User already exists           |
|  500 | Internal Server Error | Database/system error         |
|  501 | Not Implemented       | Endpoint not yet implemented  |

---

## Security Considerations

### Password Requirements

- Minimum 8 characters
- At least one uppercase letter
- At least one lowercase letter
- At least one digit
- At least one special character (!@#$%^&* etc.)

### Rate Limiting

⚠️ **Note**: This API does not currently implement rate limiting at the HTTP layer. For production deployments, consider adding:

- IP-based rate limiting using middleware
- Per-email login attempt tracking
- Integration with tools like `governor` or `tower-governor`

### Session Security

- Sessions use UUID v4 (cryptographically random)
- Default timeout: 24 hours (configurable)
- Sessions are automatically invalidated on password change
- Expired sessions are rejected with appropriate error codes

### Database Security

- SQLite database file should have restricted permissions
- Consider encrypting the database file at rest
- Private keys are stored encrypted in the database (via kanari-auth)

---

## Development

### Building

```bash
# Debug build
cargo build -p run-auth

# Release build
cargo build --release -p run-auth
```

### Testing

Currently, the API relies on the underlying `kanari-auth` tests. Future enhancements should include:

- Integration tests for HTTP endpoints
- Load testing with tools like `wrk` or `hey`
- Security scanning with OWASP ZAP

### Logging

The API uses structured logging via `tracing`. Configure log levels with `RUST_LOG`:

```bash
# Verbose logging
RUST_LOG=debug cargo run -p run-auth

# Only show errors
RUST_LOG=error cargo run -p run-auth

# Custom module filtering
RUST_LOG=run_auth=trace,axum=info cargo run -p run-auth
```

Logs include:

- Request/response details
- Authentication attempts (success/failure)
- Database operations
- Error traces with context

---

## Architecture

```rs
┌─────────────┐      ┌──────────────┐     ┌─────────────┐
│   Client    │────▶│  Axum Router │────▶│  Handlers   │
│  (HTTP)     │      │  + CORS      │     │             │
└─────────────┘      └──────────────┘     └──────┬──────┘
                                                 │
                                           ┌─────▼──────┐
                                           │ AppState   │
                                           │ (Mutex)    │
                                           └─────┬──────┘
                                                 │
                                           ┌─────▼──────┐
                                           │AuthManager │
                                           │ (SQLite)   │
                                           └────────────┘
```

### Thread Safety

- `AuthManager` is wrapped in `Arc<Mutex<>>` for thread-safe access
- SQLite connections are not `Send + Sync`, so Mutex is used instead of RwLock
- Concurrent requests are serialized at the database level

---

## Production Deployment

### Docker Example

```er
FROM rust:latest as builder
WORKDIR /app
COPY . .
RUN cargo build --release -p run-auth

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y libssl-dev
COPY --from=builder /app/target/release/run-auth /usr/local/bin/
EXPOSE 3000
ENV AUTH_DB_PATH=/data/auth.db
ENV AUTH_API_PORT=3000
ENV AUTH_BIND_ADDRESS=0.0.0.0
# Set AUTH_ALLOWED_ORIGIN and AUTH_TRUSTED_PROXY_IPS at deployment time.
CMD ["run-auth"]
```

### Reverse Proxy (Nginx)

```nginx
server {
    listen 443 ssl;
    server_name auth.example.com;

    location / {
        proxy_pass http://localhost:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_hide_header Server;
    }
}
```

Keep port 3000 private. If Nginx is not on loopback, add only its exact IP to
`AUTH_TRUSTED_PROXY_IPS`. The service rejects spoofed `X-Forwarded-Proto` headers
from all other peers.

### Monitoring

Consider adding:

- Prometheus metrics endpoint (`/metrics`)
- Health check intervals (every 30s)
- Log aggregation (ELK stack, Datadog)
- Alerting on error rates (>5% of requests)

---

## License

Apache-2.0

## Contributing

Contributions welcome! Please read our contributing guidelines and submit pull requests.
