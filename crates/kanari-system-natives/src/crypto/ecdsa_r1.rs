// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! ECDSA P-256 (R1) native functions

use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::NativeFunction;
use move_vm_types::natives::function::{NativeResult, PartialVMResult};
use move_vm_types::{
    pop_arg,
    values::{Value, VectorRef},
};
use smallvec::smallvec;

use kanari_crypto::cryptos::{NativeCryptoError, verify_p256_sha256_native};

use move_core_types::gas_algebra::InternalGas;

use crate::crypto::make_native;
use crate::helpers::expect_native_signature;

// Error codes for ECDSA R1 native functions
pub const E_INVALID_SIGNATURE: u64 = 2;
pub const E_INVALID_PUBKEY: u64 = 3;
pub const E_UNSUPPORTED_HASH_FOR_P256: u64 = 4;
pub const E_INVALID_MESSAGE: u64 = 6;

// Maximum message length accepted by natives (prevent large-memory DoS)
const MAX_MSG_BYTES: usize = 1_000_000; // 1 MB

/// Creates the ecdsa_r1 native functions iterator
pub fn make_ecdsa_r1_natives(
    gas_cost: InternalGas,
) -> impl Iterator<Item = (String, NativeFunction)> {
    let natives = vec![("native_verify".to_string(), make_verify_r1_native(gas_cost))];

    crate::helpers::make_module_natives(natives)
}

/// Creates the ecdsa_r1::native_verify native function
fn make_verify_r1_native(gas_cost: InternalGas) -> NativeFunction {
    make_native(
        move |context, ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            use move_vm_types::natives::function::NativeResult as NR;
            native_charge_gas_early_exit!(context, gas_cost);
            expect_native_signature(arguments.len(), 4, ty_args.len(), 0)?;

            let hash_type: u8 = pop_arg!(arguments, u8);
            let msg_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let public_key_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let signature_ref: VectorRef = pop_arg!(arguments, VectorRef);

            let msg: Vec<u8> = msg_ref.as_bytes_ref().to_vec();
            let public_key: Vec<u8> = public_key_ref.as_bytes_ref().to_vec();
            let signature: Vec<u8> = signature_ref.as_bytes_ref().to_vec();

            // Validate inputs
            if signature.is_empty() {
                return Ok(NR::err(context.gas_used(), E_INVALID_SIGNATURE));
            }

            if msg.len() > MAX_MSG_BYTES {
                return Ok(NR::err(context.gas_used(), E_INVALID_MESSAGE));
            }

            // Disallow Keccak for P-256 (non-standard)
            if hash_type == 0u8 {
                return Ok(NR::err(context.gas_used(), E_UNSUPPORTED_HASH_FOR_P256));
            }

            match verify_p256_sha256_native(&public_key, &signature, &msg) {
                Ok(verified) => Ok(NR::ok(context.gas_used(), smallvec![Value::bool(verified)])),
                Err(NativeCryptoError::InvalidPublicKey) => {
                    Ok(NR::err(context.gas_used(), E_INVALID_PUBKEY))
                }
                Err(NativeCryptoError::InvalidSignature) => {
                    Ok(NR::ok(context.gas_used(), smallvec![Value::bool(false)]))
                }
                Err(_) => Ok(NR::err(context.gas_used(), E_INVALID_SIGNATURE)),
            }
        },
    )
}
