// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Ed25519 native functions

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

use ed25519_dalek::{Signature as EdSignature, VerifyingKey as EdPublicKey};

use std::{collections::VecDeque, sync::Arc};

use crate::helpers::make_module_natives;
use super::types::GasParameters;
use super::MAX_MSG_BYTES;

fn make_native<F>(f: F) -> NativeFunction
where
    F: Fn(&mut NativeContext, Vec<Type>, VecDeque<Value>) -> PartialVMResult<NativeResult>
        + Send
        + Sync
        + 'static,
{
    Arc::new(f)
}

pub fn make_ed25519(gas_params: GasParameters) -> impl Iterator<Item = (String, NativeFunction)> {
    make_module_natives(
        all_natives_with_gas(AccountAddress::ZERO, gas_params)
            .into_iter()
            .filter(|(_, module_name, _, _)| module_name.as_str() == "ed25519")
            .map(|(_, _, func_name, func)| (func_name.to_string(), func)),
    )
}

fn all_natives_with_gas(
    move_addr: AccountAddress,
    gas_params: GasParameters,
) -> Vec<(String, String, NativeFunction)> {
    let mut natives = vec![];

    let ed25519_verify_cost = gas_params.ed25519_verify;

    // ed25519::verify(signature, public_key, msg) -> bool
    let ed25519_verify = make_native(
        move |context, _ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            native_charge_gas_early_exit!(context, ed25519_verify_cost);
            let msg_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let public_key_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let signature_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let msg: Vec<u8> = msg_ref.as_bytes_ref().to_vec();
            let public_key: Vec<u8> = public_key_ref.as_bytes_ref().to_vec();
            let signature: Vec<u8> = signature_ref.as_bytes_ref().to_vec();

            if msg.len() > MAX_MSG_BYTES {
                return Ok(NativeResult::ok(context.gas_used(), smallvec![Value::bool(false)]));
            }

            // Wrap verification in a panic catcher
            let result = std::panic::catch_unwind(|| {
                if public_key.len() != 32 || signature.len() != 64 {
                    return false;
                }

                let pk_arr: [u8; 32] = match public_key.as_slice().try_into() {
                    Ok(a) => a,
                    Err(_) => return false,
                };
                let pk = match EdPublicKey::from_bytes(&pk_arr) {
                    Ok(p) => p,
                    Err(_) => return false,
                };

                let sig_arr: [u8; 64] = match signature.as_slice().try_into() {
                    Ok(a) => a,
                    Err(_) => return false,
                };
                let sig = EdSignature::from_bytes(&sig_arr);

                pk.verify(&msg, &sig).is_ok()
            });

            let verified: bool = result.unwrap_or_default();

            Ok(NativeResult::ok(context.gas_used(), smallvec![Value::bool(verified)]))
        },
    );

    natives.push(("ed25519".to_string(), "verify".to_string(), ed25519_verify));

    natives
}
