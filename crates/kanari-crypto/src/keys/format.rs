// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Kanari private-key prefixes and formatting helpers.

use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Prefix used for Kanari private keys
pub const KANARI_KEY_PREFIX: &str = "kanari";

/// Additional known prefixes
pub const KANAPQC_PREFIX: &str = "kanapqc";
pub const KANAMLDSA_PREFIX: &str = "kanamldsa";
pub const KANASLHDSA_PREFIX: &str = "kanaslh";
pub const KANAFALCON_PREFIX: &str = "kanafalcon";
pub const KANAHYBRID_PREFIX: &str = "kanahybrid";

pub(crate) const MAX_FORMATTED_PRIVATE_KEY_LEN: usize = 128 * 1024;

/// Securely encode bytes to hex string using a zeroizing buffer.
///
/// This prevents intermediate allocations from leaking sensitive data in memory dumps.
pub(crate) fn secure_hex_encode(bytes: &[u8]) -> Zeroizing<String> {
    let mut result = String::with_capacity(bytes.len() * 2);
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";

    for &byte in bytes {
        result.push(HEX_CHARS[(byte >> 4) as usize] as char);
        result.push(HEX_CHARS[(byte & 0x0F) as usize] as char);
    }

    Zeroizing::new(result)
}

/// Constant-time check if a string starts with a given prefix.
///
/// Prevents timing attacks that could leak information about key formats.
pub(crate) fn constant_time_starts_with(s: &str, prefix: &str) -> bool {
    let s_bytes = s.as_bytes();
    let prefix_bytes = prefix.as_bytes();

    if prefix_bytes.len() > s_bytes.len() {
        return false;
    }

    let s_prefix = &s_bytes[..prefix_bytes.len()];
    s_prefix.ct_eq(prefix_bytes).into()
}

/// Format a raw hex private key with the Kanari prefix.
#[must_use]
pub fn format_private_key(raw_key: &str) -> String {
    format!("{}{}", KANARI_KEY_PREFIX, raw_key)
}

/// Extract the raw hex key from a formatted private key using constant-time comparison.
#[must_use]
pub fn extract_raw_key(formatted_key: &str) -> &str {
    if constant_time_starts_with(formatted_key, KANAHYBRID_PREFIX) {
        &formatted_key[KANAHYBRID_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANAMLDSA_PREFIX) {
        &formatted_key[KANAMLDSA_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANASLHDSA_PREFIX) {
        &formatted_key[KANASLHDSA_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANAFALCON_PREFIX) {
        &formatted_key[KANAFALCON_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANAPQC_PREFIX) {
        &formatted_key[KANAPQC_PREFIX.len()..]
    } else if constant_time_starts_with(formatted_key, KANARI_KEY_PREFIX) {
        &formatted_key[KANARI_KEY_PREFIX.len()..]
    } else {
        formatted_key
    }
}

/// Skip the uncompressed EC point prefix (0x04) safely.
pub(crate) fn skip_uncompressed_point_prefix(bytes: &[u8]) -> &[u8] {
    if bytes.is_empty() {
        return bytes;
    }

    if bytes[0] == 0x04 && bytes.len() > 1 {
        &bytes[1..]
    } else {
        bytes
    }
}
