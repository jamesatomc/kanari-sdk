// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Serde adapters for zeroizing wallet secrets.

use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S>(value: &zeroize::Zeroizing<String>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(value.as_str())
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<zeroize::Zeroizing<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(zeroize::Zeroizing::new(s))
}
