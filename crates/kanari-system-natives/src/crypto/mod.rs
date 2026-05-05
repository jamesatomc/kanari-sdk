// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Crypto native functions module
//! 
//! This module provides cryptographic operations including:
//! - ECDSA secp256k1 (K1) operations: ecrecover, decompress_pubkey, verify
//! - ECDSA P-256 (R1) operations: verify
//! - Ed25519 operations: verify

pub mod types;
mod ecdsa_k1;
mod ecdsa_r1;
mod ed25519;
#[cfg(test)]
mod tests;

pub use types::GasParameters;
pub use ecdsa_k1::make_ecdsa_k1;
pub use ecdsa_r1::make_ecdsa_r1;
pub use ed25519::make_ed25519;

// Re-export MAX_MSG_BYTES for use in other modules
pub use ecdsa_r1::MAX_MSG_BYTES;
