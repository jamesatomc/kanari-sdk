// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Primitive-level signing and verification traits.
//!
//! This layer gives wallet, RPC, benchmark, and application code a stable API
//! without knowing each algorithm-specific module. It deliberately delegates to
//! the existing Kanari signing/verification functions so wire formats and
//! compatibility behavior stay unchanged.

use crate::{
    keys::{AlgorithmMetadata, CurveType, KeyPair, UsageProfile},
    signatures::{SignatureError, sign_message, verify_signature, verify_signature_with_curve},
};

/// Common signer interface for Kanari-supported signature algorithms.
pub trait CryptoSigner {
    /// Algorithm used by this signer.
    fn curve_type(&self) -> CurveType;

    /// Sign message bytes using the algorithm's Kanari-defined semantics.
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignatureError>;

    /// Stable metadata for policy/UI/benchmark consumers.
    fn metadata(&self) -> AlgorithmMetadata {
        self.curve_type().metadata()
    }
}

/// Common verifier interface for Kanari-supported signature algorithms.
pub trait CryptoVerifier {
    /// Algorithm used by this verifier.
    fn curve_type(&self) -> CurveType;

    /// Verify message bytes using the algorithm's Kanari-defined semantics.
    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool, SignatureError>;

    /// Stable metadata for policy/UI/benchmark consumers.
    fn metadata(&self) -> AlgorithmMetadata {
        self.curve_type().metadata()
    }
}

impl CryptoSigner for KeyPair {
    fn curve_type(&self) -> CurveType {
        self.curve_type
    }

    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, SignatureError> {
        sign_message(&self.private_key, message, self.curve_type)
    }
}

impl CryptoVerifier for KeyPair {
    fn curve_type(&self) -> CurveType {
        self.curve_type
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool, SignatureError> {
        crate::signatures::verify_signature_with_keypair(self, message, signature)
    }
}

/// Verifier backed by an explicit public key/address and curve.
#[derive(Debug, Clone, Copy)]
pub struct PublicKeyVerifier<'a> {
    public_key_or_address: &'a str,
    curve_type: CurveType,
}

impl<'a> PublicKeyVerifier<'a> {
    /// Create a verifier from an untagged public key/address plus explicit curve.
    pub const fn new(public_key_or_address: &'a str, curve_type: CurveType) -> Self {
        Self {
            public_key_or_address,
            curve_type,
        }
    }
}

impl CryptoVerifier for PublicKeyVerifier<'_> {
    fn curve_type(&self) -> CurveType {
        self.curve_type
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool, SignatureError> {
        verify_signature_with_curve(
            self.public_key_or_address,
            message,
            signature,
            self.curve_type,
        )
    }
}

/// Verifier backed by a tagged address such as `Ed25519:...`.
#[derive(Debug, Clone, Copy)]
pub struct TaggedAddressVerifier<'a> {
    tagged_address: &'a str,
}

impl<'a> TaggedAddressVerifier<'a> {
    /// Create a verifier from a tagged address.
    pub const fn new(tagged_address: &'a str) -> Self {
        Self { tagged_address }
    }
}

impl CryptoVerifier for TaggedAddressVerifier<'_> {
    fn curve_type(&self) -> CurveType {
        KeyPair::parse_tagged_address(self.tagged_address)
            .map(|(curve_type, _)| curve_type)
            .unwrap_or_default()
    }

    fn verify(&self, message: &[u8], signature: &[u8]) -> Result<bool, SignatureError> {
        verify_signature(self.tagged_address, message, signature)
    }
}

/// Return all algorithm metadata in stable `CurveType::ALL` order.
#[must_use]
pub fn all_algorithm_metadata() -> Vec<AlgorithmMetadata> {
    CurveType::ALL.iter().map(CurveType::metadata).collect()
}

/// Return recommended algorithms for a usage profile.
#[must_use]
pub fn recommended_curves_for_usage(profile: UsageProfile) -> Vec<CurveType> {
    CurveType::ALL
        .iter()
        .copied()
        .filter(|curve| curve.usage_profile() == profile && curve.is_recommended_for_new_accounts())
        .collect()
}
