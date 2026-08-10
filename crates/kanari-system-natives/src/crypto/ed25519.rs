// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Ed25519 native functions

use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::NativeFunction;
use move_vm_types::natives::function::{NativeResult, PartialVMResult};
use move_vm_types::{
    pop_arg,
    values::{Value, VectorRef},
};
use smallvec::smallvec;

use kanari_crypto::cryptos::verify_ed25519_native;

use move_core_types::gas_algebra::InternalGas;

use crate::crypto::make_native;
use crate::helpers::expect_native_signature;

// Maximum message length accepted by natives (prevent large-memory DoS)
const MAX_MSG_BYTES: usize = 1_000_000; // 1 MB

/// Creates the ed25519 native functions iterator
pub fn make_ed25519_natives(
    gas_cost: InternalGas,
) -> impl Iterator<Item = (String, NativeFunction)> {
    let natives = vec![("verify".to_string(), make_ed25519_verify_native(gas_cost))];

    crate::helpers::make_module_natives(natives)
}

/// Creates the ed25519::verify native function
fn make_ed25519_verify_native(gas_cost: InternalGas) -> NativeFunction {
    make_native(
        move |context, ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            use move_vm_types::natives::function::NativeResult as NR;
            native_charge_gas_early_exit!(context, gas_cost);
            expect_native_signature(arguments.len(), 3, ty_args.len(), 0)?;

            // Pop arguments
            let msg_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let public_key_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let signature_ref: VectorRef = pop_arg!(arguments, VectorRef);

            let msg: Vec<u8> = msg_ref.as_bytes_ref().to_vec();
            let public_key: Vec<u8> = public_key_ref.as_bytes_ref().to_vec();
            let signature: Vec<u8> = signature_ref.as_bytes_ref().to_vec();

            // Prevent overly large messages
            if msg.len() > MAX_MSG_BYTES {
                return Ok(NR::ok(context.gas_used(), smallvec![Value::bool(false)]));
            }

            // Wrap verification in panic catcher
            let result =
                std::panic::catch_unwind(|| verify_ed25519_native(&public_key, &signature, &msg));

            let verified: bool = result.unwrap_or_default();
            Ok(NR::ok(context.gas_used(), smallvec![Value::bool(verified)]))
        },
    )
}
