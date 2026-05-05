// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! ECDSA P-256 (R1) native functions

use move_core_types::account_address::AccountAddress;
use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::{NativeContext, NativeFunction};
use move_vm_types::natives::function::{NativeResult, PartialVMResult};
use move_vm_types::{
    loaded_data::runtime_types::Type,
    pop_arg,
    values::{Value, VectorRef},
};
use smallvec::smallvec;

use p256::ecdsa::{Signature as P256Signature, VerifyingKey as P256VerifyingKey};
use sha2::Sha256;
use p256::ecdsa::signature::hazmat::PrehashVerifier as P256PrehashVerifier;

use std::{collections::VecDeque, sync::Arc};

use crate::helpers::make_module_natives;
use super::types::GasParameters;

// Maximum message length accepted by natives (prevent large-memory DoS)
pub(crate) const MAX_MSG_BYTES: usize = 1_000_000; // 1 MB

// Error codes
const E_INVALID_SIGNATURE: u64 = 2;
const E_INVALID_PUBKEY: u64 = 3;
const E_UNSUPPORTED_HASH_FOR_P256: u64 = 4;
const E_INVALID_MESSAGE: u64 = 6;

fn make_native<F>(f: F) -> NativeFunction
where
    F: Fn(&mut NativeContext, Vec<Type>, VecDeque<Value>) -> PartialVMResult<NativeResult>
        + Send
        + Sync
        + 'static,
{
    Arc::new(f)
}

pub fn make_ecdsa_r1(gas_params: GasParameters) -> impl Iterator<Item = (String, NativeFunction)> {
    make_module_natives(
        all_natives_with_gas(AccountAddress::ZERO, gas_params)
            .into_iter()
            .filter(|(_, module_name, _, _)| module_name.as_str() == "ecdsa_r1")
            .map(|(_, _, func_name, func)| (func_name.to_string(), func)),
    )
}

fn all_natives_with_gas(
    move_addr: AccountAddress,
    gas_params: GasParameters,
) -> Vec<(String, String, NativeFunction)> {
    let mut natives = vec![];

    let verify_r1_cost = gas_params.verify_r1;

    // ecdsa_r1 (P-256) verify(signature, public_key, msg, hash) -> bool
    let verify_r1 = make_native(
        move |context, _ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            native_charge_gas_early_exit!(context, verify_r1_cost);
            let hash_type: u8 = pop_arg!(arguments, u8);
            let msg_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let public_key_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let signature_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let msg: Vec<u8> = msg_ref.as_bytes_ref().to_vec();
            let mut public_key: Vec<u8> = public_key_ref.as_bytes_ref().to_vec();
            let signature: Vec<u8> = signature_ref.as_bytes_ref().to_vec();

            if signature.is_empty() {
                return Ok(NativeResult::err(context.gas_used(), E_INVALID_SIGNATURE));
            }

            if msg.len() > MAX_MSG_BYTES {
                return Ok(NativeResult::err(context.gas_used(), E_INVALID_MESSAGE));
            }

            // Normalize P-256 pubkey encodings
            if public_key.len() == 64 {
                let mut prefixed = Vec::with_capacity(65);
                prefixed.push(0x04);
                prefixed.extend_from_slice(&public_key);
                public_key = prefixed;
            }

            let vk = match P256VerifyingKey::from_sec1_bytes(&public_key) {
                Ok(v) => v,
                Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_PUBKEY)),
            };

            // Disallow Keccak for P-256
            if hash_type == 0u8 {
                return Ok(NativeResult::err(context.gas_used(), E_UNSUPPORTED_HASH_FOR_P256));
            }

            // Normalize signature encodings
            let sig_bytes = if signature.len() == 65 {
                &signature[..64]
            } else {
                signature.as_slice()
            };

            let sig = if let Ok(s) = P256Signature::from_der(&signature) {
                s
            } else if sig_bytes.len() == 64 {
                match P256Signature::try_from(sig_bytes) {
                    Ok(s) => s,
                    Err(_) => return Ok(NativeResult::ok(context.gas_used(), smallvec![Value::bool(false)])),
                }
            } else {
                return Ok(NativeResult::ok(context.gas_used(), smallvec![Value::bool(false)]));
            };

            // Hash then verify (SHA256 only for P-256)
            let msg_hash = Sha256::digest(&msg);
            let verified = vk.verify_prehash(msg_hash.as_slice(), &sig).is_ok();

            Ok(NativeResult::ok(context.gas_used(), smallvec![Value::bool(verified)]))
        },
    );

    natives.push(("ecdsa_r1".to_string(), "native_verify".to_string(), verify_r1));

    natives
}
