// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Secure cryptographic primitives for the Kanari blockchain platform
//!
//! This crate provides comprehensive cryptographic operations supporting:
//! - **Classical Elliptic Curve Cryptography (ECC)**: K256, P256, Ed25519
//! - **Post-Quantum Cryptography (PQC)**: ML-DSA/Dilithium-compatible, Falcon/FN-DSA, SPHINCS+/SLH-DSA
//! - **Hybrid Cryptography**: Combined classical + PQC schemes for quantum-safe transition
//! - Key generation and management
//! - Digital signatures (RFC-8032 compliant Ed25519, ECC, PQC)
//! - Encryption and decryption
//! - Wallet operations
//! - Key rotation and lifecycle management
//! - Security audit logging
//! - Backup and restore functionality
//!
//! # 🌐 Multi-Algorithm Support Overview
//!
//! ## Classical ECC Algorithms
//! - **K256 (secp256k1)**: Bitcoin/Ethereum compatible curve with SHA3-256 pre-hashing
//! - **P256 (secp256r1)**: NIST P-256 curve with SHA3-256 pre-hashing  
//! - **Ed25519**: Modern EdDSA scheme with RFC-8032 compliance (direct signing, no pre-hashing)
//!
//! ## Post-Quantum Cryptography (PQC) - NIST Standards
//! - **Dilithium2**: Fast lattice-based signatures (~2.5KB), NIST Level 2 security
//! - **Dilithium3**: Balanced lattice-based signatures (~4KB), NIST Level 3 security (Recommended)
//! - **Dilithium5**: Maximum security lattice-based signatures (~5KB), NIST Level 5 security
//! - **Falcon512/1024**: Compact lattice-based signatures for PQC verification paths
//! - **SPHINCS+/SLH-DSA**: Hash-based signatures (~50KB), conservative long-term storage option
//!
//! ## Hybrid Schemes (Classical + PQC)
//! - **Ed25519 + Dilithium3**: Best of both worlds - fast classical + quantum-safe PQC
//! - **K256 + Dilithium3**: Bitcoin/Ethereum compatible + quantum-safe PQC
//! - Format: `[2-byte classical_len] || classical_sig || pqc_sig`
//!
//! # 🔒 Security Features
//!
//! ## Quantum-Resistant Design
//! - SHA2/SHA3/SHAKE/BLAKE3 hash helpers with explicit algorithm selection
//! - PQC algorithms follow NIST-oriented profiles where supported
//! - Hybrid schemes provide backward compatibility during quantum transition
//!
//! ## Secure Implementation
//! - Automatic memory zeroization for sensitive data
//! - Constant-time operations to prevent timing attacks
//! - Tagged addresses for unambiguous algorithm identification
//! - Comprehensive input validation and error handling
//!
//! # 📋 Usage Examples
//!
//! ## Generate Keys
//! ```rust
//! use kanari_crypto::{generate_keypair, CurveType};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Classical ECC
//! let k256_key = generate_keypair(CurveType::K256)?;
//! let ed25519_key = generate_keypair(CurveType::Ed25519)?;
//!
//! // Post-Quantum
//! let dilithium3_key = generate_keypair(CurveType::Dilithium3)?;
//!
//! // Hybrid
//! let hybrid_key = generate_keypair(CurveType::Ed25519Dilithium3)?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Sign Messages
//! ```rust
//! use kanari_crypto::{generate_keypair, CurveType};
//! use kanari_crypto::signatures::sign_message;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let keypair = generate_keypair(CurveType::Dilithium3)?;
//! let message = b"Hello, quantum world!";
//! let signature = sign_message(&keypair.private_key, message, CurveType::Dilithium3)?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Verify Signatures (Recommended: Use Tagged Addresses)
//! ```rust
//! use kanari_crypto::{generate_keypair, verify_signature, CurveType};
//! use kanari_crypto::signatures::sign_message;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let keypair = generate_keypair(CurveType::Dilithium3)?;
//! let message = b"Hello, quantum world!";
//! let signature = sign_message(&keypair.private_key, message, CurveType::Dilithium3)?;
//! // ✅ Secure: Use tagged address to avoid timing attacks
//! let valid = verify_signature(&keypair.tagged_address(), message, &signature)?;
//! assert!(valid);
//!
//! // ⚠️ Less secure: Fallback to trying all curves (timing attack risk)
//! assert!(verify_signature(&keypair.address, message, &signature).is_err());
//! # Ok(())
//! # }
//! ```
//!
//! # Compatibility & Standards
//!
//! ## Ed25519 Signature Interoperability ✅
//!
//! **Kanari Ed25519 is 100% RFC-8032 COMPLIANT:**
//! - Signatures created by Kanari can be verified by ANY standard Ed25519 implementation
//! - Kanari can verify Ed25519 signatures from libsodium, NaCl, cryptonote, etc.
//! - No format conversion or special handling needed for interoperability
//! - Uses direct signing/verification (no pre-hashing)
//!
//! ### Supported Interoperability Chains:
//! - ✅ libsodium (crypto_sign_*)
//! - ✅ NaCl / TweetNaCl
//! - ✅ Python PyNaCl
//! - ✅ Go ed25619 package
//! - ✅ Node.js TweetNaCl.js / tweetnacl-js
//! - ✅ Any RFC-8032 compliant library
//!
//! ## Curve-Specific Signing Strategies
//!
//! | Curve Type | Signing Strategy | Standard Compliance | Interop |
//! |-----------|-----------------|-------------------|---------|
//! | **Ed25519** | Direct (no hash) | ✅ RFC-8032 | Full interop |
//! | **K256** | SHA3-256 hash | Kanari-specific | Kanari-only |
//! | **P256** | SHA3-256 hash | Kanari-specific | Kanari-only |
//! | **Dilithium** | Direct (no hash) | ✅ NIST standard | Kanari/NIST-compatible |
//! | **SPHINCS+** | Direct (no hash) | ✅ NIST standard | Kanari/NIST-compatible |
//! | **Hybrid** | Applies per-curve | Mixed | Partial |
//!

pub mod audit;
pub mod backup;
pub mod compression;
pub mod cryptos;
pub mod encryption;
pub mod hashs;
pub mod hd_wallet;
pub mod key_rotation;
pub mod keys;
pub mod keystore;
pub mod security;
pub mod signatures;
pub mod traits;
pub mod wallet;

// Re-export signature functionality
pub use signatures::{
    BatchVerificationItem, SignatureError, verify_batch_tagged, verify_batch_with_curve,
    verify_signature,
};

// Re-export encryption functionality - now using actual functions from the module
pub use encryption::{
    DEFAULT_STREAM_CHUNK_SIZE, EncryptedData, EncryptionError, StreamDecryptingReader,
    StreamEncryptingWriter, StreamEncryptionHeader, decrypt_data, decrypt_stream, decrypt_string,
    encrypt_data, encrypt_stream, encrypt_string, secure_erase, stream_encrypting_writer,
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

// Re-export key generation functions
pub use keys::{
    AlgorithmFamily, AlgorithmMetadata, CurveType, KeyError, KeyPair, UsageProfile,
    generate_hybrid_ed25519_dilithium3_keypair, generate_hybrid_k256_dilithium3_keypair,
    generate_keypair, keypair_from_mnemonic, keypair_from_private_key,
};

// Re-export primitive trait layer
pub use traits::{
    CryptoSigner, CryptoVerifier, PublicKeyVerifier, TaggedAddressVerifier, all_algorithm_metadata,
    recommended_curves_for_usage,
};

// Re-export security policy helpers
pub use security::{
    MAX_PASSWORD_LEN, MIN_RECOMMENDED_PASSWORD_LENGTH, RateLimiter, SECURITY_LEVEL,
    get_current_timestamp, is_password_strong, security_info, version,
};

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

// Re-export hash functionality
pub use hashs as hash;
pub use hashs::{
    HashAlgorithm, hash_data, hash_data_blake3, hash_data_blake3_array, hash_data_blake3_chunks,
    hash_data_blake3_chunks_array, hash_data_sha2_256, hash_data_sha2_256_chunks,
    hash_data_sha2_512, hash_data_sha2_512_chunks, hash_data_sha3_256_chunks, hash_data_sha3_512,
    hash_data_sha3_512_chunks, hash_data_shake256, hash_data_shake256_chunks,
    hash_data_shake256_custom, hash_data_shake256_custom_chunks, hash_data_with_algorithm,
    hash_data_with_algorithm_chunks,
};
