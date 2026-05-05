// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use move_core_types::gas_algebra::InternalGas;

/// Gas parameters for crypto native functions
#[derive(Debug, Clone)]
pub struct GasParameters {
    pub ecrecover: InternalGas,
    pub decompress_pubkey: InternalGas,
    pub verify_k1: InternalGas,
    pub verify_r1: InternalGas,
    pub ed25519_verify: InternalGas,
}

impl GasParameters {
    pub fn zeros() -> Self {
        Self {
            ecrecover: 0.into(),
            decompress_pubkey: 0.into(),
            verify_k1: 0.into(),
            verify_r1: 0.into(),
            ed25519_verify: 0.into(),
        }
    }
}
