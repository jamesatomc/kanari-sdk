// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Hierarchical Deterministic (HD) Wallet utilities (BIP-32 / BIP-44 helpers)
//!
//! Small helpers to derive child private keys and produce KeyPairs compatible
//! with the rest of the crate.

use crate::keys::{CurveType, KANARI_KEY_PREFIX, KeyPair, keypair_from_private_key};
use bip32::{DerivationPath, XPrv};
use bip39::{Language, Mnemonic};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;
use thiserror::Error;

// Maximum entries in rate limiter before cleanup (prevent memory leak)
const MAX_RATE_LIMITER_ENTRIES: usize = 1000;

// Rate limiter for derivation operations
lazy_static::lazy_static! {
    static ref DERIVE_RATE_LIMITER: Mutex<HashMap<String, (usize, u64)>> = Mutex::new(HashMap::new());
}

/// Errors returned from HD wallet operations
#[derive(Error, Debug)]
pub enum HdError {
    #[error("Invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    #[error("Invalid derivation path: {0}")]
    InvalidDerivationPath(String),

    #[error("Key derivation failed: {0}")]
    DerivationFailed(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),
}

/// Derive a child private key from the mnemonic at the given derivation path
/// and return a `KeyPair` for the requested curve.
pub fn derive_keypair_from_path(
    mnemonic_phrase: &str,
    password: &str,
    derivation_path: &str,
    curve: CurveType,
) -> Result<KeyPair, HdError> {
    // Early reject for post-quantum / hybrid curves: HD derivation is not supported
    if curve.is_post_quantum() {
        return Err(HdError::DerivationFailed(
            "Post-quantum curves do not support HD derivation".to_string(),
        ));
    }
    let mnemonic = Mnemonic::parse_in(Language::English, mnemonic_phrase)
        .map_err(|e| HdError::InvalidMnemonic(e.to_string()))?;

    let seed = mnemonic.to_seed(password);

    // Create master extended private key
    let xprv = XPrv::new(seed.as_ref()).map_err(|e| HdError::DerivationFailed(e.to_string()))?;

    // Parse the requested derivation path
    let path = DerivationPath::from_str(derivation_path)
        .map_err(|e| HdError::InvalidDerivationPath(e.to_string()))?;

    // Iteratively derive along the path (derive_child accepts a ChildNumber)
    let mut derived = xprv;
    for cn in path.into_iter() {
        derived = derived
            .derive_child(cn)
            .map_err(|e| HdError::DerivationFailed(e.to_string()))?;
    }

    // Extract private key bytes (32 bytes) and format as hex
    let priv_bytes = derived.private_key().to_bytes();

    // Prepend kanari prefix (keys module expects this format)
    let mut formatted = format!("{}{}", KANARI_KEY_PREFIX, hex::encode(priv_bytes));

    // Zeroize sensitive data immediately
    use zeroize::Zeroize;
    let mut priv_bytes_mut = priv_bytes.to_vec();
    priv_bytes_mut.zeroize();

    // Build KeyPair using existing helper
    let result = keypair_from_private_key(&formatted, curve)
        .map_err(|e| HdError::DerivationFailed(e.to_string()));

    // Zeroize the formatted string before returning
    formatted.zeroize();

    result
}

#[allow(clippy::items_after_test_module)]
/// Derive multiple addresses using a path template that contains `{index}`.
/// Includes rate limiting to prevent DoS attacks via repeated calls.
pub fn derive_multiple_addresses(
    mnemonic_phrase: &str,
    password: &str,
    path_template: &str,
    curve: CurveType,
    count: usize,
) -> Result<Vec<KeyPair>, HdError> {
    // Rate limiting: max 1000 derivations per 60 seconds per mnemonic
    const MAX_DERIVATIONS_PER_MINUTE: usize = 1000;
    const RATE_LIMIT_WINDOW_SECS: u64 = 60;

    let mnemonic_hash = {
        use sha3::{Digest, Sha3_256};
        let mut hasher = Sha3_256::new();
        hasher.update(mnemonic_phrase.as_bytes());
        hex::encode(&hasher.finalize()[..8]) // Use first 8 bytes as key
    };

    {
        // Handle mutex poisoning by recovering from poisoned state
        let mut limiter = DERIVE_RATE_LIMITER.lock().unwrap_or_else(|poisoned| {
            // Recover from poisoned mutex - data is still valid
            poisoned.into_inner()
        });

        let now = crate::get_current_timestamp();

        // Cleanup expired entries if too many accumulated (prevent memory leak)
        if limiter.len() > MAX_RATE_LIMITER_ENTRIES {
            limiter.retain(|_, (_, window_start)| {
                now.saturating_sub(*window_start) < RATE_LIMIT_WINDOW_SECS * 2
            });
        }

        let (count_in_window, window_start) =
            limiter.entry(mnemonic_hash.clone()).or_insert((0, now));

        // Reset window if expired
        if now.saturating_sub(*window_start) > RATE_LIMIT_WINDOW_SECS {
            *count_in_window = 0;
            *window_start = now;
        }

        // Check rate limit
        if *count_in_window + count > MAX_DERIVATIONS_PER_MINUTE {
            return Err(HdError::RateLimitExceeded(format!(
                "Maximum {} derivations per {} seconds exceeded",
                MAX_DERIVATIONS_PER_MINUTE, RATE_LIMIT_WINDOW_SECS
            )));
        }

        // Don't increment counter yet - wait until after validation and successful derivation
    }

    // Validate inputs
    if mnemonic_phrase.trim().is_empty() {
        return Err(HdError::InvalidMnemonic(
            "Empty mnemonic phrase".to_string(),
        ));
    }

    // Password can be empty but validate it's valid UTF-8 by checking length
    if password.len() > 1024 {
        return Err(HdError::DerivationFailed("Password too long".to_string()));
    }

    // Validate maximum count to prevent DoS via unbounded allocation
    const MAX_DERIVE_COUNT: usize = 10_000;
    if count > MAX_DERIVE_COUNT {
        return Err(HdError::DerivationFailed(format!(
            "Count exceeds maximum allowed ({})",
            MAX_DERIVE_COUNT
        )));
    }

    if !path_template.contains("{index}") {
        return Err(HdError::InvalidDerivationPath(
            "path_template must include {index}".to_string(),
        ));
    }

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let path = path_template.replace("{index}", &i.to_string());
        let kp = derive_keypair_from_path(mnemonic_phrase, password, &path, curve)?;
        out.push(kp);
    }

    // Increment rate limiter counter only after successful completion
    {
        let mut limiter = DERIVE_RATE_LIMITER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((count_in_window, _)) = limiter.get_mut(&mnemonic_hash) {
            *count_in_window += count;
        }
    }

    Ok(out)
}

#[cfg(test)]
#[path = "../tests/unit/hd_wallet_test.rs"]
mod tests;
