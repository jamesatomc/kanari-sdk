// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Algorithm metadata and policy helpers for supported key/signature types.

use super::CurveType;

/// High-level algorithm family for policy, UI, and benchmarking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlgorithmFamily {
    /// Classical elliptic-curve signature algorithm.
    Classical,
    /// Pure post-quantum signature algorithm.
    PostQuantum,
    /// Hybrid classical + post-quantum signature algorithm.
    Hybrid,
}

/// Recommended usage profile for a signature algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageProfile {
    /// Fast hot-wallet/user transaction signing.
    HotWallet,
    /// General production wallet/account use.
    GeneralPurpose,
    /// Long-term treasury/cold-storage use.
    ColdStorage,
    /// Compatibility with external ecosystems such as Bitcoin/Ethereum/WebAuthn.
    Interoperability,
    /// Experimental or specialized use where operational cost is expected.
    Specialized,
}

/// Stable metadata for an algorithm supported by `kanari-crypto`.
///
/// Sizes are encoded-signature/public-key byte sizes for the Kanari provider
/// where the value is fixed or a conservative upper-bound for composite schemes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlgorithmMetadata {
    pub curve_type: CurveType,
    pub family: AlgorithmFamily,
    pub usage_profile: UsageProfile,
    pub nist_level: Option<u8>,
    pub quantum_safe: bool,
    pub hybrid: bool,
    pub hd_wallet_derivation: bool,
    pub signature_size_hint: Option<usize>,
    pub public_key_size_hint: Option<usize>,
    pub recommended: bool,
}

impl CurveType {
    /// All signature algorithms supported by Kanari.
    pub const ALL: &'static [CurveType] = &[
        CurveType::K256,
        CurveType::P256,
        CurveType::Ed25519,
        CurveType::Dilithium2,
        CurveType::Dilithium3,
        CurveType::Dilithium5,
        CurveType::SphincsPlusSha256Robust,
        CurveType::Falcon512,
        CurveType::Falcon1024,
        CurveType::Ed25519Dilithium3,
        CurveType::K256Dilithium3,
    ];

    /// Production-friendly default for new Kanari accounts.
    pub const DEFAULT_PRODUCTION: CurveType = CurveType::Ed25519Dilithium3;

    /// Recommended hot-path default when transaction size and latency matter most.
    pub const DEFAULT_HOT_WALLET: CurveType = CurveType::Ed25519;

    /// Recommended long-term/cold-storage default.
    pub const DEFAULT_COLD_STORAGE: CurveType = CurveType::SphincsPlusSha256Robust;

    /// Returns true if this is a classical-only algorithm.
    pub fn is_classical(&self) -> bool {
        !self.is_post_quantum()
    }

    /// Returns true if BIP39/BIP32-style deterministic derivation is supported.
    pub fn supports_hd_wallet_derivation(&self) -> bool {
        matches!(self, CurveType::K256 | CurveType::P256 | CurveType::Ed25519)
    }

    /// Algorithm family used for policy and UI grouping.
    pub fn family(&self) -> AlgorithmFamily {
        if self.is_hybrid() {
            AlgorithmFamily::Hybrid
        } else if self.is_post_quantum() {
            AlgorithmFamily::PostQuantum
        } else {
            AlgorithmFamily::Classical
        }
    }

    /// Approximate or fixed Kanari signature size in bytes.
    ///
    /// Hybrid values include the 2-byte classical length prefix.
    pub fn signature_size_hint(&self) -> Option<usize> {
        match self {
            CurveType::K256 | CurveType::P256 => Some(64),
            CurveType::Ed25519 => Some(64),
            CurveType::Dilithium2 => Some(2_420),
            CurveType::Dilithium3 => Some(3_309),
            CurveType::Dilithium5 => Some(4_627),
            CurveType::SphincsPlusSha256Robust => Some(49_856),
            CurveType::Falcon512 => Some(666),
            CurveType::Falcon1024 => Some(1_280),
            CurveType::Ed25519Dilithium3 | CurveType::K256Dilithium3 => Some(2 + 64 + 3_309),
        }
    }

    /// Approximate or fixed Kanari public-key size in bytes.
    pub fn public_key_size_hint(&self) -> Option<usize> {
        match self {
            CurveType::K256 | CurveType::P256 => Some(65),
            CurveType::Ed25519 => Some(32),
            CurveType::Dilithium2 => Some(1_312),
            CurveType::Dilithium3 => Some(1_952),
            CurveType::Dilithium5 => Some(2_592),
            CurveType::SphincsPlusSha256Robust => Some(64),
            CurveType::Falcon512 => Some(897),
            CurveType::Falcon1024 => Some(1_793),
            CurveType::Ed25519Dilithium3 => Some(32 + 1_952),
            CurveType::K256Dilithium3 => Some(65 + 1_952),
        }
    }

    /// NIST security level where the mapping is defined.
    pub fn nist_level(&self) -> Option<u8> {
        match self {
            CurveType::Falcon512 => Some(1),
            CurveType::Dilithium2 => Some(2),
            CurveType::K256
            | CurveType::P256
            | CurveType::Ed25519
            | CurveType::Dilithium3
            | CurveType::Ed25519Dilithium3
            | CurveType::K256Dilithium3 => Some(3),
            CurveType::Dilithium5 | CurveType::SphincsPlusSha256Robust | CurveType::Falcon1024 => {
                Some(5)
            }
        }
    }

    /// Primary recommended usage profile.
    pub fn usage_profile(&self) -> UsageProfile {
        match self {
            CurveType::K256 | CurveType::P256 => UsageProfile::Interoperability,
            CurveType::Ed25519 => UsageProfile::HotWallet,
            CurveType::Dilithium3
            | CurveType::Falcon1024
            | CurveType::Ed25519Dilithium3
            | CurveType::K256Dilithium3 => UsageProfile::GeneralPurpose,
            CurveType::Dilithium5 | CurveType::SphincsPlusSha256Robust => UsageProfile::ColdStorage,
            CurveType::Dilithium2 | CurveType::Falcon512 => UsageProfile::Specialized,
        }
    }

    /// Returns true for algorithms recommended as first-choice production options.
    pub fn is_recommended_for_new_accounts(&self) -> bool {
        matches!(
            self,
            CurveType::Ed25519
                | CurveType::Dilithium3
                | CurveType::Falcon1024
                | CurveType::Ed25519Dilithium3
                | CurveType::SphincsPlusSha256Robust
        )
    }

    /// Stable metadata bundle for this algorithm.
    pub fn metadata(&self) -> AlgorithmMetadata {
        AlgorithmMetadata {
            curve_type: *self,
            family: self.family(),
            usage_profile: self.usage_profile(),
            nist_level: self.nist_level(),
            quantum_safe: self.is_post_quantum(),
            hybrid: self.is_hybrid(),
            hd_wallet_derivation: self.supports_hd_wallet_derivation(),
            signature_size_hint: self.signature_size_hint(),
            public_key_size_hint: self.public_key_size_hint(),
            recommended: self.is_recommended_for_new_accounts(),
        }
    }
}
