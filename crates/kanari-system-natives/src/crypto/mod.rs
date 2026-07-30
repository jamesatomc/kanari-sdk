// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

mod ecdsa_k1;
mod ecdsa_r1;
mod ed25519;
mod pqc;
mod rs256;

#[cfg(test)]
mod tests;

use std::{collections::VecDeque, sync::Arc};

use move_core_types::gas_algebra::InternalGas;

// Re-export error codes and constants from submodules for backward compatibility
pub use ecdsa_k1::{
    E_INVALID_MESSAGE, E_INVALID_PUBKEY, E_INVALID_RECOVERY, E_INVALID_SCHNORR_SIGNATURE,
    E_INVALID_SIGNATURE, E_INVALID_XONLY_PUBKEY, MAX_MSG_BYTES,
};
pub use ecdsa_r1::E_UNSUPPORTED_HASH_FOR_P256;
use move_vm_runtime::native_functions::{NativeContext, NativeFunction};
use move_vm_types::{
    loaded_data::runtime_types::Type,
    natives::function::{NativeResult, PartialVMResult},
    values::Value,
};
pub use rs256::{E_INVALID_HASH_TYPE as RS256_E_INVALID_HASH_TYPE, E_INVALID_MESSAGE_LENGTH};

/// Helper function to create native functions
pub fn make_native<F>(f: F) -> NativeFunction
where
    F: Fn(&mut NativeContext, Vec<Type>, VecDeque<Value>) -> PartialVMResult<NativeResult>
        + Send
        + Sync
        + 'static,
{
    Arc::new(f)
}

#[derive(Debug, Clone)]
pub struct GasParameters {
    pub ecrecover: InternalGas,
    pub decompress_pubkey: InternalGas,
    pub verify_k1: InternalGas,
    pub verify_r1: InternalGas,
    pub ed25519_verify: InternalGas,
    pub dilithium2_verify: InternalGas,
    pub dilithium3_verify: InternalGas,
    pub dilithium5_verify: InternalGas,
    pub ed25519_dilithium3_verify: InternalGas,
    pub k256_dilithium3_verify: InternalGas,
    pub rs256_verify: InternalGas,
}

impl GasParameters {
    pub fn zeros() -> Self {
        Self {
            ecrecover: 0.into(),
            decompress_pubkey: 0.into(),
            verify_k1: 0.into(),
            verify_r1: 0.into(),
            ed25519_verify: 0.into(),
            dilithium2_verify: 0.into(),
            dilithium3_verify: 0.into(),
            dilithium5_verify: 0.into(),
            ed25519_dilithium3_verify: 0.into(),
            k256_dilithium3_verify: 0.into(),
            rs256_verify: 0.into(),
        }
    }
}

pub fn make_ecdsa_k1(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, move_vm_runtime::native_functions::NativeFunction)> {
    let natives = vec![
        (
            "ecrecover".to_string(),
            ecdsa_k1::make_ecrecover_native(gas_params.ecrecover),
        ),
        (
            "decompress_pubkey".to_string(),
            ecdsa_k1::make_decompress_pubkey_native(gas_params.decompress_pubkey),
        ),
        (
            "verify".to_string(),
            ecdsa_k1::make_verify_k1_native(gas_params.verify_k1),
        ),
    ];

    crate::helpers::make_module_natives(natives)
}

pub fn make_ecdsa_r1(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, move_vm_runtime::native_functions::NativeFunction)> {
    ecdsa_r1::make_ecdsa_r1_natives(gas_params.verify_r1)
}

pub fn make_ed25519(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, move_vm_runtime::native_functions::NativeFunction)> {
    ed25519::make_ed25519_natives(gas_params.ed25519_verify)
}

pub fn make_dilithium2(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, NativeFunction)> {
    pqc::make_dilithium2(gas_params.dilithium2_verify)
}

pub fn make_dilithium3(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, NativeFunction)> {
    pqc::make_dilithium3(gas_params.dilithium3_verify)
}

pub fn make_dilithium5(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, NativeFunction)> {
    pqc::make_dilithium5(gas_params.dilithium5_verify)
}

pub fn make_ed25519_dilithium3(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, NativeFunction)> {
    pqc::make_ed25519_dilithium3(gas_params.ed25519_dilithium3_verify)
}

pub fn make_k256_dilithium3(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, NativeFunction)> {
    pqc::make_k256_dilithium3(gas_params.k256_dilithium3_verify)
}

pub fn make_rs256(
    gas_params: GasParameters,
) -> impl Iterator<Item = (String, move_vm_runtime::native_functions::NativeFunction)> {
    rs256::make_rs256_natives(gas_params.rs256_verify)
}
