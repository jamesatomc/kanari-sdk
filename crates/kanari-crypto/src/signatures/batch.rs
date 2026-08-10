// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use rayon::prelude::*;

use crate::CurveType;

use super::{
    SignatureError, validation::validate_batch_items, verify_batch_ed25519_native,
    verify_signature, verify_signature_with_curve,
};

/// One signature verification item used by batch APIs.
#[derive(Debug, Clone, Copy)]
pub struct BatchVerificationItem<'a> {
    pub public_key_or_address: &'a str,
    pub message: &'a [u8],
    pub signature: &'a [u8],
}

impl<'a> BatchVerificationItem<'a> {
    #[must_use]
    pub const fn new(
        public_key_or_address: &'a str,
        message: &'a [u8],
        signature: &'a [u8],
    ) -> Self {
        Self {
            public_key_or_address,
            message,
            signature,
        }
    }
}

/// Deterministic fail-closed batch verification for one explicit curve.
///
/// Empty batches fail by design, matching consensus-safe batch semantics.
/// Ed25519 uses native cryptographic batch verification. Other algorithms use
/// parallel per-signature verification so callers get a stable batch API while
/// avoiding unsafe or non-standard ECDSA aggregation.
pub fn verify_batch_with_curve(
    items: &[BatchVerificationItem<'_>],
    curve_type: CurveType,
) -> Result<bool, SignatureError> {
    validate_batch_items(items)?;

    if matches!(curve_type, CurveType::Ed25519) {
        return verify_batch_ed25519_native(items);
    }

    items
        .par_iter()
        .map(|item| {
            verify_signature_with_curve(
                item.public_key_or_address,
                item.message,
                item.signature,
                curve_type,
            )
        })
        .try_reduce(
            || true,
            |all_verified, item_verified| Ok(all_verified && item_verified),
        )
}

/// Deterministic fail-closed batch verification for tagged addresses.
pub fn verify_batch_tagged(items: &[BatchVerificationItem<'_>]) -> Result<bool, SignatureError> {
    validate_batch_items(items)?;

    items
        .par_iter()
        .map(|item| verify_signature(item.public_key_or_address, item.message, item.signature))
        .try_reduce(
            || true,
            |all_verified, item_verified| Ok(all_verified && item_verified),
        )
}
