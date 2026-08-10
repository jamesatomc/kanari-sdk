// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! HD-wallet and mnemonic management helpers.

use super::{Wallet, WalletError, save_wallet, validation};
use crate::keys::CurveType;
use crate::{Keystore, compression, encryption, hd_wallet};
use move_core_types::account_address::AccountAddress;
use std::str::FromStr;

/// Create a child wallet derived from the stored mnemonic at the given path.
/// The created wallet is automatically saved to the keystore and set as active.
pub fn create_wallet_from_hd(
    password: &str,
    derivation_path: &str,
    curve: CurveType,
) -> Result<Wallet, WalletError> {
    let wallet = create_hd_wallet(password, derivation_path, curve)?;
    save_hd_wallet(&wallet, password)?;
    Ok(wallet)
}

/// Create (but do not persist) a child wallet derived from the stored mnemonic
/// at the given BIP32 derivation path. Returns the constructed Wallet.
pub fn create_hd_wallet(
    password: &str,
    derivation_path: &str,
    curve: CurveType,
) -> Result<Wallet, WalletError> {
    let mnemonic_phrase = load_mnemonic(password)?;

    let key_pair =
        hd_wallet::derive_keypair_from_path(&mnemonic_phrase, password, derivation_path, curve)?;

    let address = AccountAddress::from_str(&key_pair.address)
        .map_err(|e| WalletError::SerializationError(format!("Invalid derived address: {e}")))?;

    let priv_key = {
        let zk = key_pair.export_private_key_secure();
        zk.to_string()
    };

    Ok(Wallet::new(
        address,
        priv_key,
        String::new(),
        Some(derivation_path.to_string()),
        curve,
    ))
}

/// Persist a previously-created HD child wallet into the keystore using
/// the standard `save_wallet` path.
pub fn save_hd_wallet(wallet: &Wallet, password: &str) -> Result<(), WalletError> {
    save_wallet(
        &wallet.address,
        &wallet.private_key,
        &wallet.seed_phrase,
        wallet.derivation_path.as_deref(),
        password,
        wallet.curve_type,
    )
}

/// Save mnemonic phrase to keystore.
pub fn save_mnemonic(
    mnemonic: &str,
    password: &str,
    addresses: Vec<String>,
) -> Result<(), WalletError> {
    validation::validate_storage_password(password)?;

    if mnemonic.is_empty() {
        return Err(WalletError::EncryptionError(
            "Empty mnemonic not allowed".to_string(),
        ));
    }

    const MAX_MNEMONIC_SIZE: usize = 10240;
    if mnemonic.len() > MAX_MNEMONIC_SIZE {
        return Err(WalletError::SerializationError(format!(
            "Mnemonic data too large: {} bytes (max: {})",
            mnemonic.len(),
            MAX_MNEMONIC_SIZE
        )));
    }

    let compressed_data = compression::compress_data(mnemonic.as_bytes())
        .map_err(|e| WalletError::SerializationError(format!("Compression error: {e}")))?;

    let encrypted_data = encryption::encrypt_data(&compressed_data, password)
        .map_err(|e| WalletError::EncryptionError(e.to_string()))?;

    let mut keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore
        .set_mnemonic(encrypted_data, addresses)
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(())
}

/// Load mnemonic phrase from keystore.
pub fn load_mnemonic(password: &str) -> Result<String, WalletError> {
    if password.is_empty() {
        return Err(WalletError::InvalidPassword);
    }

    let keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    let encrypted_data = keystore
        .get_mnemonic()
        .ok_or_else(|| WalletError::NotFound("Mnemonic not found".to_string()))?;

    let decrypted = encryption::decrypt_data(encrypted_data, password)
        .map_err(|_| WalletError::InvalidPassword)?;

    let decompressed_data = compression::decompress_data(&decrypted)
        .map_err(|e| WalletError::DecryptionError(format!("Failed to decompress mnemonic: {e}")))?;

    String::from_utf8(decompressed_data)
        .map_err(|e| WalletError::DecryptionError(format!("Invalid UTF-8 in mnemonic: {e}")))
}

/// Get addresses derived from mnemonic.
pub fn get_mnemonic_addresses() -> Result<Vec<String>, WalletError> {
    let keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;
    Ok(keystore.get_mnemonic_addresses().clone())
}

/// Check if mnemonic exists in keystore.
#[must_use]
pub fn check_mnemonic_exists() -> bool {
    Keystore::load().is_ok_and(|keystore| keystore.has_mnemonic())
}

/// Remove mnemonic from keystore.
pub fn remove_mnemonic() -> Result<(), WalletError> {
    let mut keystore = Keystore::load().map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    keystore
        .remove_mnemonic()
        .map_err(|e| WalletError::KeystoreError(e.to_string()))?;

    Ok(())
}
