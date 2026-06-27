// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use move_core_types::account_address::AccountAddress;

pub(crate) fn canonical_object_id(id: &str) -> Option<String> {
    AccountAddress::from_hex_literal(id)
        .ok()
        .map(|addr| addr.to_hex_literal())
}

pub(crate) fn object_id_from_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() < AccountAddress::LENGTH {
        return None;
    }

    let mut arr = [0u8; AccountAddress::LENGTH];
    arr.copy_from_slice(&bytes[..AccountAddress::LENGTH]);
    Some(AccountAddress::new(arr).to_hex_literal())
}
