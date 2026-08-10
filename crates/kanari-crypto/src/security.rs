// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Security policy helpers shared across wallet, keystore, backup, and tests.

use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

/// Maximum accepted password length for expensive password-based operations.
pub const MAX_PASSWORD_LEN: usize = 1024;

/// Recommended minimum password length for Kanari wallets.
pub const MIN_RECOMMENDED_PASSWORD_LENGTH: usize = 16;

/// Security level used by this library.
pub const SECURITY_LEVEL: &str =
    "Post-Quantum Ready - hybrid-first, fail-closed, benchmarked verification";

/// Common weak passwords to reject.
const COMMON_WEAK_PASSWORDS: &[&str] = &[
    "password",
    "password123",
    "password1234",
    "12345678",
    "123456789",
    "qwerty",
    "abc123",
    "letmein",
    "welcome",
    "admin",
    "root",
    "Password123!",
    "Password1234!",
    "Passw0rd!",
];

/// Get current Unix timestamp in seconds.
///
/// Returns current timestamp or 1 on system time error.
/// Note: Return value of 1 indicates an error condition (system clock before epoch).
/// Callers should treat timestamps near epoch (< 1000000000 = year 2001) as suspicious.
#[must_use]
pub fn get_current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_else(|_| {
            // System time is before UNIX epoch - this should never happen in practice.
            // Return 1 to avoid 0 edge cases while signaling error.
            1
        })
}

/// Check if a password meets strong security requirements.
///
/// Returns true if password is at least 16 characters and contains:
/// - At least one uppercase letter
/// - At least one lowercase letter
/// - At least one digit
/// - At least one special character from safe set
/// - Not in common weak passwords list
/// - No control characters or null bytes
#[must_use]
pub fn is_password_strong(password: &str) -> bool {
    if password.len() < MIN_RECOMMENDED_PASSWORD_LENGTH {
        return false;
    }

    if password.chars().any(|c| c.is_control() || c == '\0') {
        return false;
    }

    let password_lower = password.to_lowercase();
    if COMMON_WEAK_PASSWORDS
        .iter()
        .any(|weak| password_lower == weak.to_lowercase())
    {
        return false;
    }

    if has_repetitive_pattern(password) {
        return false;
    }

    let has_uppercase = password.chars().any(|c| c.is_uppercase());
    let has_lowercase = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_numeric());
    const SPECIAL_CHARS: &str = "!@#$%^&*()_+-=[]{}|;:',.<>?/~`\"";
    let has_special = password.chars().any(|c| SPECIAL_CHARS.contains(c));

    has_uppercase && has_lowercase && has_digit && has_special
}

fn has_repetitive_pattern(password: &str) -> bool {
    const MAX_PATTERN_CHECK_LEN: usize = 128;

    let mut pw = password;
    while pw.chars().count() > MAX_PATTERN_CHECK_LEN {
        let truncate_pos = pw
            .char_indices()
            .nth(MAX_PATTERN_CHECK_LEN)
            .map(|(idx, _)| idx)
            .unwrap_or(pw.len());
        pw = &pw[..truncate_pos];
    }

    let chars: Vec<char> = pw.chars().collect();

    for index in 0..chars.len().saturating_sub(2) {
        if chars[index] == chars[index + 1] && chars[index] == chars[index + 2] {
            return true;
        }
    }

    let max_seq_len = (chars.len() / 2).min(32);
    for seq_len in 2..=max_seq_len {
        if chars.len() >= seq_len * 2 {
            let first_half: Vec<char> = chars.iter().take(seq_len).copied().collect();
            let second_half: Vec<char> =
                chars.iter().skip(seq_len).take(seq_len).copied().collect();
            if first_half == second_half {
                return true;
            }
        }
    }

    false
}

/// Rate limiter for security-sensitive operations.
///
/// Tracks failed attempts and enforces exponential backoff.
pub struct RateLimiter {
    attempts: HashMap<String, (u32, u64)>,
    max_attempts: u32,
    lockout_duration_secs: u64,
}

const MAX_RATE_LIMITER_ENTRIES: usize = 1000;

impl RateLimiter {
    /// Create a new rate limiter.
    #[must_use]
    pub fn new(max_attempts: u32, lockout_duration_secs: u64) -> Self {
        Self {
            attempts: HashMap::new(),
            max_attempts,
            lockout_duration_secs,
        }
    }

    /// Check if an operation is allowed for the given identifier.
    pub fn check_allowed(&mut self, identifier: &str) -> bool {
        let now = get_current_timestamp();

        if self.attempts.len() > MAX_RATE_LIMITER_ENTRIES {
            self.attempts
                .retain(|_, (_, locked_until)| now < *locked_until);
        }

        if let Some((count, locked_until)) = self.attempts.get(identifier) {
            if now < *locked_until {
                return false;
            }
            if *count >= self.max_attempts {
                self.attempts.remove(identifier);
            }
        }

        true
    }

    /// Record a failed attempt.
    pub fn record_failure(&mut self, identifier: &str) {
        let now = get_current_timestamp();
        let (count, _) = self.attempts.get(identifier).unwrap_or(&(0, 0));
        let new_count = count + 1;
        let lockout = std::cmp::min(2u64.pow(new_count), self.lockout_duration_secs);

        self.attempts
            .insert(identifier.to_string(), (new_count, now + lockout));
    }

    /// Record a successful attempt (resets the counter).
    pub fn record_success(&mut self, identifier: &str) {
        self.attempts.remove(identifier);
    }

    /// Get remaining lockout time in seconds.
    #[must_use]
    pub fn get_lockout_remaining(&self, identifier: &str) -> Option<u64> {
        let now = get_current_timestamp();

        if let Some((_, locked_until)) = self.attempts.get(identifier)
            && now < *locked_until
        {
            return Some(*locked_until - now);
        }

        None
    }
}

/// Version information for the crypto library.
#[must_use]
pub const fn version() -> &'static str {
    "3.1.0-pqc-audited"
}

/// Returns security information about the library.
#[must_use]
pub const fn security_info() -> &'static str {
    "Kanari Crypto v3.1 - Post-Quantum Ready / Audited
    
    Classical Algorithms:
    - AES-256-GCM encryption
    - Ed25519, K256, P256 signatures
    - Argon2id password hashing
    - SHA2-256/512, SHA3-256/512, BLAKE3, SHAKE256 hashing
    
    Post-Quantum Algorithms:
    - ML-DSA / Dilithium2/3/5-compatible signatures
    - Falcon512/1024 compact lattice signatures
    - SPHINCS+ / SLH-DSA hash-based signatures
    
    Hybrid Schemes:
    - Ed25519+Dilithium3 signatures
    - K256+Dilithium3 signatures

    Production Safety:
    - Tagged-address verification is fail-closed
    - Ed25519 true batch verification
    - K256/P256 parallel batch verification without unsafe ECDSA aggregation
    - Oversized message/signature/key/batch inputs are rejected before heavy parsing
    - Wallet, keystore, backup, compatibility, attack-simulation, and fuzz-style tests
    
    Guidance:
    - Use Ed25519 for hot-path speed when long-term quantum resistance is not required
    - Use Ed25519+Dilithium3 for general long-term account security
    - Use SPHINCS+/SLH-DSA or stronger PQC profiles for cold storage
    - RS256 is compatibility-only public verification and is not used for wallet signing"
}
