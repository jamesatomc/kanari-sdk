// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! BIP39 mnemonic seed derivation for post-quantum and hybrid keys.
//!
//! Classical curves derive directly from the first 32 bytes of the BIP39 seed.
//! Post-quantum algorithms need different seed sizes and must never share raw
//! seed material across algorithms, so this module derives domain-separated
//! sub-seeds with SHAKE256 (a NIST-approved XOF):
//!
//! ```text
//! sub_seed = SHAKE256("Kanari-PQC-Mnemonic-v1" || 0x00 || label || 0x00 || bip39_seed)
//! ```
//!
//! A distinct `label` per algorithm (and per HD derivation path) guarantees
//! that every key type gets independent seed material from the same mnemonic.

use zeroize::Zeroizing;

use crate::hashs::hash_data_shake256_custom_chunks;

/// Domain separation tag for all mnemonic-to-PQC seed derivations.
///
/// Bump this string if the derivation construction ever changes so that old
/// and new derivations can never collide.
pub(super) const MNEMONIC_PQC_DOMAIN: &str = "Kanari-PQC-Mnemonic-v1";

/// Derive `out_len` bytes of deterministic seed material from a BIP39 seed.
///
/// `label` identifies the target algorithm (and optionally an HD derivation
/// path). The returned bytes are wrapped in [`Zeroizing`] and must be
/// zeroized by the caller after use where applicable.
pub(super) fn derive_mnemonic_seed(
    bip39_seed: &[u8],
    label: &str,
    out_len: usize,
) -> Zeroizing<Vec<u8>> {
    let out = hash_data_shake256_custom_chunks(
        &[
            MNEMONIC_PQC_DOMAIN.as_bytes(),
            &[0u8],
            label.as_bytes(),
            &[0u8],
            bip39_seed,
        ],
        out_len,
    );
    Zeroizing::new(out)
}
