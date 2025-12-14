//! Secure cryptographic primitives for the Kanari blockchain platform
//!
//! This crate provides cryptographic operations including:
//! - Key generation and management
//! - Digital signatures
//! - Encryption and decryption
//! - Wallet operations
//! - Key rotation and lifecycle management
//! - Security audit logging
//! - Backup and restore functionality

pub mod api;
pub mod audit;
pub mod backup;
pub mod compression;
pub mod encryption;
pub mod hd_wallet;
pub mod key_rotation;
pub mod keys;
pub mod keystore;
pub mod signatures;
pub mod wallet;

mod frb_generated; /* AUTO INJECTED BY flutter_rust_bridge. This line may not be accurate, and you can change it according to your needs. */

// Re-export signature functionality
pub use signatures::{
    SignatureError, secure_clear, sign_message, verify_signature, verify_signature_with_curve,
};

// Re-export encryption functionality - now using actual functions from the module
pub use encryption::{
    EncryptedData, EncryptionError, decrypt_data, decrypt_string, encrypt_data, encrypt_string,
    secure_erase,
};

// Re-export wallet functionality
pub use wallet::{
    Wallet, WalletError, check_mnemonic_exists, check_wallet_exists, get_mnemonic_addresses,
    get_selected_wallet, list_wallet_files, load_mnemonic, load_wallet, remove_mnemonic,
    save_mnemonic, save_wallet, set_selected_wallet,
};

// Re-export keystore functionality
pub use keystore::{Keystore, get_keystore_path, keystore_exists};

// Re-export compression functionality
pub use compression::{compress_data, decompress_data};

// Constants for security limits
pub const MAX_PASSWORD_LEN: usize = 1024;

// Timestamp utilities
use std::time::{SystemTime, UNIX_EPOCH};

/// Get current Unix timestamp in seconds
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
            // System time is before UNIX epoch - this should never happen in practice
            // Return 1 to avoid 0 edge cases while signaling error
            1
        })
}

// Re-export key rotation functionality
pub use key_rotation::{
    KeyMetadata, KeyRotationError, KeyRotationManager, KeyRotationPolicy, RotationStatistics,
};

// Re-export audit functionality
pub use audit::{
    AuditEntry, AuditError, AuditLogger, EventSeverity, SecurityEvent, create_default_logger,
    get_default_audit_log_path,
};

// Re-export backup functionality
pub use backup::{BackupError, BackupInfo, BackupManager, BackupMetadata, EncryptedBackup};

/// Hash algorithm options (including quantum-resistant)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HashAlgorithm {
    /// SHA3-256 algorithm (default, quantum-resistant)
    #[default]
    Sha3_256,
    /// SHA3-512 algorithm (higher security, quantum-resistant)
    Sha3_512,
    /// Blake3 algorithm (faster, equally secure)
    Blake3,
    /// SHAKE256 (extendable output, quantum-resistant)
    Shake256,
}

/// Cryptographic hash using SHA3-256 (default)
#[must_use]
pub fn hash_data(data: &[u8]) -> Vec<u8> {
    hash_data_with_algorithm(data, HashAlgorithm::Sha3_256)
}

/// Cryptographic hash using SHA3-512 (quantum-resistant, 512-bit)
#[must_use]
pub fn hash_data_sha3_512(data: &[u8]) -> Vec<u8> {
    hash_data_with_algorithm(data, HashAlgorithm::Sha3_512)
}

/// Cryptographic hash using Blake3 (faster alternative)
#[must_use]
pub fn hash_data_blake3(data: &[u8]) -> Vec<u8> {
    hash_data_with_algorithm(data, HashAlgorithm::Blake3)
}

/// Cryptographic hash using SHAKE256 with 256-bit output (quantum-resistant)
#[must_use]
pub fn hash_data_shake256(data: &[u8]) -> Vec<u8> {
    hash_data_with_algorithm(data, HashAlgorithm::Shake256)
}

/// Cryptographic hash using SHAKE256 with custom output length
#[must_use]
pub fn hash_data_shake256_custom(data: &[u8], output_len: usize) -> Vec<u8> {
    use sha3::{
        Shake256,
        digest::{ExtendableOutput, Update, XofReader},
    };
    let mut hasher = Shake256::default();
    hasher.update(data);
    let mut reader = hasher.finalize_xof();
    let mut output = vec![0u8; output_len];
    reader.read(&mut output);
    output
}

/// Cryptographic hash using the specified algorithm
#[must_use]
pub fn hash_data_with_algorithm(data: &[u8], algorithm: HashAlgorithm) -> Vec<u8> {
    match algorithm {
        HashAlgorithm::Sha3_256 => {
            use sha3::{Digest, Sha3_256};
            let mut hasher = Sha3_256::new();
            hasher.update(data);
            hasher.finalize().to_vec()
        }
        HashAlgorithm::Sha3_512 => {
            use sha3::{Digest, Sha3_512};
            let mut hasher = Sha3_512::new();
            hasher.update(data);
            hasher.finalize().to_vec()
        }
        HashAlgorithm::Blake3 => {
            let mut hasher = blake3::Hasher::new();
            hasher.update(data);
            hasher.finalize().as_bytes().to_vec()
        }
        HashAlgorithm::Shake256 => {
            use sha3::{
                Shake256,
                digest::{ExtendableOutput, Update, XofReader},
            };
            let mut hasher = Shake256::default();
            hasher.update(data);
            let mut reader = hasher.finalize_xof();
            let mut output = vec![0u8; 32]; // 256-bit default
            reader.read(&mut output);
            output
        }
    }
}

// Add constant for recommended password length
pub const MIN_RECOMMENDED_PASSWORD_LENGTH: usize = 16; // Increased for quantum era

/// Common weak passwords to reject
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

/// Check if a password meets strong security requirements
/// Returns true if password is at least 16 characters and contains:
/// - At least one uppercase letter
/// - At least one lowercase letter
/// - At least one digit
/// - At least one special character from safe set
/// - Not in common weak passwords list
/// - No control characters or null bytes
pub fn is_password_strong(password: &str) -> bool {
    if password.len() < MIN_RECOMMENDED_PASSWORD_LENGTH {
        return false;
    }

    // Reject passwords with control characters or null bytes
    if password.chars().any(|c| c.is_control() || c == '\0') {
        return false;
    }

    // Check for common weak passwords (exact match, case-insensitive)
    let password_lower = password.to_lowercase();
    if COMMON_WEAK_PASSWORDS
        .iter()
        .any(|weak| password_lower == weak.to_lowercase())
    {
        return false;
    }

    // Check for repetitive patterns
    if has_repetitive_pattern(password) {
        return false;
    }

    let has_uppercase = password.chars().any(|c| c.is_uppercase());
    let has_lowercase = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_numeric());

    // Define safe special characters explicitly
    const SPECIAL_CHARS: &str = "!@#$%^&*()_+-=[]{}|;:',.<>?/~`\"";
    let has_special = password.chars().any(|c| SPECIAL_CHARS.contains(c));

    has_uppercase && has_lowercase && has_digit && has_special
}

/// Check for repetitive patterns in password (e.g., "aaa", "111", "abcabc")
fn has_repetitive_pattern(password: &str) -> bool {
    // Prevent DoS: limit password length for pattern checking
    const MAX_PATTERN_CHECK_LEN: usize = 128;

    // Iteratively truncate password to MAX_PATTERN_CHECK_LEN chars if needed
    let mut pw = password;
    while pw.chars().count() > MAX_PATTERN_CHECK_LEN {
        // Use char_indices to ensure we don't split UTF-8 characters
        let truncate_pos = pw
            .char_indices()
            .nth(MAX_PATTERN_CHECK_LEN)
            .map(|(idx, _)| idx)
            .unwrap_or(pw.len());
        pw = &pw[..truncate_pos];
    }

    let chars: Vec<char> = pw.chars().collect();

    // Check for 3+ consecutive identical characters
    for i in 0..chars.len().saturating_sub(2) {
        if chars[i] == chars[i + 1] && chars[i] == chars[i + 2] {
            return true;
        }
    }

    // Check for repeating sequences (e.g., "abcabc") - limit check to reasonable size
    // Use char boundaries for string slicing to ensure UTF-8 safety
    let max_seq_len = (chars.len() / 2).min(32); // Cap at 32 characters (not bytes)
    for seq_len in 2..=max_seq_len {
        if chars.len() >= seq_len * 2 {
            // Compare character sequences instead of byte slices
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

/// Rate limiter for security-sensitive operations
/// Tracks failed attempts and enforces exponential backoff
pub struct RateLimiter {
    attempts: std::collections::HashMap<String, (u32, u64)>,
    max_attempts: u32,
    lockout_duration_secs: u64,
}

const MAX_RATE_LIMITER_ENTRIES: usize = 1000;

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(max_attempts: u32, lockout_duration_secs: u64) -> Self {
        Self {
            attempts: std::collections::HashMap::new(),
            max_attempts,
            lockout_duration_secs,
        }
    }

    /// Check if an operation is allowed for the given identifier
    pub fn check_allowed(&mut self, identifier: &str) -> bool {
        let now = get_current_timestamp();

        // Cleanup expired entries if too many accumulated (prevent memory leak)
        if self.attempts.len() > MAX_RATE_LIMITER_ENTRIES {
            self.attempts
                .retain(|_, (_, locked_until)| now < *locked_until);
        }

        if let Some((count, locked_until)) = self.attempts.get(identifier) {
            if now < *locked_until {
                return false; // Still locked out
            }
            if *count >= self.max_attempts {
                // Reset after lockout period
                self.attempts.remove(identifier);
            }
        }

        true
    }

    /// Record a failed attempt
    pub fn record_failure(&mut self, identifier: &str) {
        let now = get_current_timestamp();

        let (count, _) = self.attempts.get(identifier).unwrap_or(&(0, 0));
        let new_count = count + 1;

        // Exponential backoff: 2^(attempts) seconds, capped at lockout_duration
        let lockout = std::cmp::min(2u64.pow(new_count), self.lockout_duration_secs);

        self.attempts
            .insert(identifier.to_string(), (new_count, now + lockout));
    }

    /// Record a successful attempt (resets the counter)
    pub fn record_success(&mut self, identifier: &str) {
        self.attempts.remove(identifier);
    }

    /// Get remaining lockout time in seconds
    pub fn get_lockout_remaining(&self, identifier: &str) -> Option<u64> {
        let now = get_current_timestamp();

        if let Some((_, locked_until)) = self.attempts.get(identifier) {
            if now < *locked_until {
                return Some(*locked_until - now);
            }
        }

        None
    }
}

/// Security level used by this library
pub const SECURITY_LEVEL: &str = "Maximum - Post-Quantum Ready with Hybrid Cryptography";

/// Version information for the crypto library
#[must_use]
pub const fn version() -> &'static str {
    "3.0.0-pqc"
}

/// Returns security information about the library
#[must_use]
pub const fn security_info() -> &'static str {
    "🔒 Kanari Crypto v3.0 - Post-Quantum Ready
    
    Classical Algorithms:
    - AES-256-GCM encryption
    - Ed25519, K256, P256 signatures
    - Argon2id password hashing
    - SHA3-256/512, Blake3, SHAKE256 hashing
    
    Post-Quantum Algorithms (NIST Standard):
    - Dilithium2/3/5 signatures (ML-DSA)
    - SPHINCS+ hash-based signatures
    
    Hybrid Schemes:
    - Ed25519+Dilithium3 signatures
    - K256+Dilithium3 signatures
    
    Security: Resistant to quantum computer attacks (Shor's and Grover's algorithms)
    Always use post-quantum or hybrid schemes for long-term security!"
}
