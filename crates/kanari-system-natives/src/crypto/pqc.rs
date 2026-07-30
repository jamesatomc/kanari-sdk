// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Post-quantum and hybrid signature verification natives.

use std::collections::VecDeque;

use kanari_crypto::signatures::{
    dilithium2::verify_signature_dilithium2,
    dilithium3::verify_signature_dilithium3,
    dilithium5::verify_signature_dilithium5,
    hybrid::{verify_ed25519dilithium3, verify_k256dilithium3},
};
use move_core_types::gas_algebra::InternalGas;
use move_vm_runtime::{native_charge_gas_early_exit, native_functions::NativeFunction};
use move_vm_types::{
    loaded_data::runtime_types::Type,
    natives::function::NativeResult,
    pop_arg,
    values::{Value, VectorRef},
};
use smallvec::smallvec;

use crate::{crypto::make_native, helpers::expect_native_signature};

const MAX_MESSAGE_BYTES: usize = 1_000_000;
const MAX_SIGNATURE_BYTES: usize = 16_384;
const MAX_PUBLIC_KEY_BYTES: usize = 4_096;

type Verifier = fn(&str, &[u8], &[u8]) -> Result<bool, kanari_crypto::signatures::SignatureError>;

fn make_pqc_verify(gas_cost: InternalGas, verifier: Verifier) -> NativeFunction {
    make_native(
        move |context, ty_args: Vec<Type>, mut arguments: VecDeque<Value>| {
            native_charge_gas_early_exit!(context, gas_cost);
            expect_native_signature(arguments.len(), 3, ty_args.len(), 0)?;
            let message: VectorRef = pop_arg!(arguments, VectorRef);
            let public_key: VectorRef = pop_arg!(arguments, VectorRef);
            let signature: VectorRef = pop_arg!(arguments, VectorRef);
            let message = message.as_bytes_ref().to_vec();
            let public_key = public_key.as_bytes_ref().to_vec();
            let signature = signature.as_bytes_ref().to_vec();
            let valid = message.len() <= MAX_MESSAGE_BYTES
                && public_key.len() <= MAX_PUBLIC_KEY_BYTES
                && signature.len() <= MAX_SIGNATURE_BYTES
                && verifier(&hex::encode(public_key), &message, &signature).unwrap_or(false);
            Ok(NativeResult::ok(
                context.gas_used(),
                smallvec![Value::bool(valid)],
            ))
        },
    )
}

fn make_hybrid_verify(gas_cost: InternalGas, verifier: Verifier) -> NativeFunction {
    make_native(
        move |context, ty_args: Vec<Type>, mut arguments: VecDeque<Value>| {
            native_charge_gas_early_exit!(context, gas_cost);
            expect_native_signature(arguments.len(), 4, ty_args.len(), 0)?;
            let message: VectorRef = pop_arg!(arguments, VectorRef);
            let pqc_public_key: VectorRef = pop_arg!(arguments, VectorRef);
            let classical_public_key: VectorRef = pop_arg!(arguments, VectorRef);
            let signature: VectorRef = pop_arg!(arguments, VectorRef);
            let message = message.as_bytes_ref().to_vec();
            let pqc_public_key = pqc_public_key.as_bytes_ref().to_vec();
            let classical_public_key = classical_public_key.as_bytes_ref().to_vec();
            let signature = signature.as_bytes_ref().to_vec();
            let key = format!(
                "{}:{}",
                hex::encode(classical_public_key),
                hex::encode(pqc_public_key)
            );
            let valid = message.len() <= MAX_MESSAGE_BYTES
                && signature.len() <= MAX_SIGNATURE_BYTES
                && key.len() <= (MAX_PUBLIC_KEY_BYTES * 2 + 1) * 2
                && verifier(&key, &message, &signature).unwrap_or(false);
            Ok(NativeResult::ok(
                context.gas_used(),
                smallvec![Value::bool(valid)],
            ))
        },
    )
}

pub fn make_dilithium2(gas_cost: InternalGas) -> impl Iterator<Item = (String, NativeFunction)> {
    crate::helpers::make_module_natives(vec![(
        "verify".to_string(),
        make_pqc_verify(gas_cost, verify_signature_dilithium2),
    )])
}

pub fn make_dilithium3(gas_cost: InternalGas) -> impl Iterator<Item = (String, NativeFunction)> {
    crate::helpers::make_module_natives(vec![(
        "verify".to_string(),
        make_pqc_verify(gas_cost, verify_signature_dilithium3),
    )])
}

pub fn make_dilithium5(gas_cost: InternalGas) -> impl Iterator<Item = (String, NativeFunction)> {
    crate::helpers::make_module_natives(vec![(
        "verify".to_string(),
        make_pqc_verify(gas_cost, verify_signature_dilithium5),
    )])
}

pub fn make_ed25519_dilithium3(
    gas_cost: InternalGas,
) -> impl Iterator<Item = (String, NativeFunction)> {
    crate::helpers::make_module_natives(vec![(
        "verify".to_string(),
        make_hybrid_verify(gas_cost, verify_ed25519dilithium3),
    )])
}

pub fn make_k256_dilithium3(
    gas_cost: InternalGas,
) -> impl Iterator<Item = (String, NativeFunction)> {
    crate::helpers::make_module_natives(vec![(
        "verify".to_string(),
        make_hybrid_verify(gas_cost, verify_k256dilithium3),
    )])
}
