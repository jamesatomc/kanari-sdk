// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Keystore statistics type.

use serde::{Deserialize, Serialize};

/// Keystore statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeystoreStatistics {
    pub total_keys: usize,
    pub has_mnemonic: bool,
    pub mnemonic_addresses: usize,
    pub version: String,
    pub last_modified: Option<u64>,
}
