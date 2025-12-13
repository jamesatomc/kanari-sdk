//! Hierarchical Deterministic (HD) Wallet utilities (BIP-32 / BIP-44 helpers)
//!
//! Small helpers to derive child private keys and produce KeyPairs compatible
//! with the rest of the crate.

use crate::keys::{CurveType, KANARI_KEY_PREFIX, KeyPair, keypair_from_private_key};
use bip32::{DerivationPath, XPrv};
use bip39::{Language, Mnemonic};
use std::str::FromStr;
use thiserror::Error;

/// Errors returned from HD wallet operations
#[derive(Error, Debug)]
pub enum HdError {
    #[error("Invalid mnemonic: {0}")]
    InvalidMnemonic(String),

    #[error("Invalid derivation path: {0}")]
    InvalidDerivationPath(String),

    #[error("Key derivation failed: {0}")]
    DerivationFailed(String),
}

/// Derive a child private key from the mnemonic at the given derivation path
/// and return a `KeyPair` for the requested curve.
pub fn derive_keypair_from_path(
    mnemonic_phrase: &str,
    password: &str,
    derivation_path: &str,
    curve: CurveType,
) -> Result<KeyPair, HdError> {
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
    let raw_hex = hex::encode(priv_bytes);

    // Zeroize sensitive data immediately
    use zeroize::Zeroize;
    let mut priv_bytes_mut = priv_bytes.to_vec();
    priv_bytes_mut.zeroize();

    // Prepend kanari prefix (keys module expects this format)
    let formatted = format!("{}{}", KANARI_KEY_PREFIX, raw_hex);

    // Build KeyPair using existing helper
    keypair_from_private_key(&formatted, curve)
        .map_err(|e| HdError::DerivationFailed(e.to_string()))
}

/// Derive multiple addresses using a path template that contains `{index}`.
pub fn derive_multiple_addresses(
    mnemonic_phrase: &str,
    password: &str,
    path_template: &str,
    curve: CurveType,
    count: usize,
) -> Result<Vec<KeyPair>, HdError> {
    // Validate inputs
    if mnemonic_phrase.trim().is_empty() {
        return Err(HdError::InvalidMnemonic("Empty mnemonic phrase".to_string()));
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

    Ok(out)
}
