// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Wallet selection and listing helpers backed by Kanari config.

use crate::Keystore;
use kanari_common::{get_active_address, set_active_address};
use std::io;

/// Check if any wallets exist
#[must_use]
pub fn check_wallet_exists() -> bool {
    Keystore::load().is_ok_and(|keystore| !keystore.list_wallets().is_empty())
}

/// List all available wallets with selection status
pub fn list_wallet_files() -> Result<Vec<(String, bool)>, io::Error> {
    let selected = get_selected_wallet().unwrap_or_default();
    let mut wallets = Vec::new();

    match Keystore::load() {
        Ok(keystore) => {
            for address in keystore.list_wallets() {
                let is_selected = address == selected;
                wallets.push((address, is_selected));
            }

            wallets.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(wallets)
        }
        Err(e) => Err(io::Error::other(format!("Failed to load keystore: {e}"))),
    }
}

/// Set the currently selected wallet address in configuration
pub fn set_selected_wallet(wallet_address: &str) -> io::Result<()> {
    set_active_address(wallet_address)
}

/// Get the currently selected wallet from configuration
#[must_use]
pub fn get_selected_wallet() -> Option<String> {
    get_active_address()
}
