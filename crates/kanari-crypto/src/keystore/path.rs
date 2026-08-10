// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Keystore path discovery helpers.

use kanari_common::{get_kanari_config_path, load_kanari_config};
use std::path::PathBuf;

/// Get path to the keystore file.
pub fn get_keystore_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KANARI_KEYSTORE_PATH")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
    {
        return path;
    }

    if let Ok(config) = load_kanari_config()
        && let Some(path) = config
            .get("keystore_path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|path| !path.is_empty())
    {
        return PathBuf::from(path);
    }

    default_keystore_path()
}

fn default_keystore_path() -> PathBuf {
    let mut path = get_kanari_config_path();
    path.pop();
    path.push("kanari.keystore");
    path
}

/// Check if keystore file exists.
pub fn keystore_exists() -> bool {
    get_keystore_path().exists()
}
