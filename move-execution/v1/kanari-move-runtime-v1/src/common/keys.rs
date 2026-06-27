// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use move_core_types::account_address::AccountAddress;

pub(crate) fn metadata_key(prefix: &[u8], suffix: &str) -> Vec<u8> {
    let mut key = prefix.to_vec();
    key.extend_from_slice(suffix.as_bytes());
    key
}

pub(crate) fn object_key(id: &str) -> Vec<u8> {
    let mut key = b"object:".to_vec();
    key.extend_from_slice(id.as_bytes());
    key
}

pub(crate) fn owned_objects_key(owner: &AccountAddress) -> Vec<u8> {
    let mut key = b"owned_objects:".to_vec();
    key.extend_from_slice(owner.as_ref());
    key
}
