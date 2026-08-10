// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! ECDSA secp256k1 (K1) native functions

use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::{NativeContext, NativeFunction};
use move_vm_types::natives::function::{NativeResult, PartialVMResult};
use move_vm_types::{
    pop_arg,
    values::{Value, VectorRef},
};
use smallvec::smallvec;

use kanari_crypto::cryptos::{
    NativeCryptoError, NativeEcdsaHash, decompress_secp256k1_pubkey, native_ecdsa_message_hash,
    recover_secp256k1_public_key, verify_secp256k1_ecdsa_native, verify_secp256k1_schnorr_native,
};

use move_core_types::gas_algebra::InternalGas;

use crate::crypto::make_native;
use crate::helpers::expect_native_signature;

// Error codes for ECDSA K1 native functions
pub const E_INVALID_RECOVERY: u64 = 1;
pub const E_INVALID_SIGNATURE: u64 = 2;
pub const E_INVALID_PUBKEY: u64 = 3;
pub const E_INVALID_XONLY_PUBKEY: u64 = 5;
pub const E_INVALID_MESSAGE: u64 = 6;
pub const E_INVALID_SCHNORR_SIGNATURE: u64 = 7;

// Maximum message length accepted by natives (prevent large-memory DoS)
pub const MAX_MSG_BYTES: usize = 1_000_000; // 1 MB

/// Creates the ecdsa_k1::ecrecover native function
pub fn make_ecrecover_native(gas_cost: InternalGas) -> NativeFunction {
    make_native(
        move |context, ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            use move_vm_types::natives::function::NativeResult as NR;

            native_charge_gas_early_exit!(context, gas_cost);
            expect_native_signature(arguments.len(), 3, ty_args.len(), 0)?;

            // pop in reverse order: hash, msg, signature
            let hash_type: u8 = pop_arg!(arguments, u8);
            let msg_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let signature_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let msg: Vec<u8> = msg_ref.as_bytes_ref().to_vec();
            let signature: Vec<u8> = signature_ref.as_bytes_ref().to_vec();

            // Validate signature length
            if signature.len() != 65 {
                return Ok(NR::err(context.gas_used(), E_INVALID_SIGNATURE));
            }

            // Prevent overly large messages
            if msg.len() > MAX_MSG_BYTES {
                return Ok(NR::err(context.gas_used(), E_INVALID_MESSAGE));
            }

            let msg_hash =
                native_ecdsa_message_hash(&msg, NativeEcdsaHash::from_move_selector(hash_type));

            // Recover public key from signature
            match recover_secp256k1_public_key(&signature, &msg_hash) {
                Ok(pubkey_bytes) => Ok(NR::ok(
                    context.gas_used(),
                    smallvec![Value::vector_u8(pubkey_bytes)],
                )),
                Err(error) => Ok(NR::err(context.gas_used(), native_error_code(error))),
            }
        },
    )
}

/// Creates the ecdsa_k1::decompress_pubkey native function
pub fn make_decompress_pubkey_native(gas_cost: InternalGas) -> NativeFunction {
    make_native(
        move |context, ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            use move_vm_types::natives::function::NativeResult as NR;
            native_charge_gas_early_exit!(context, gas_cost);
            expect_native_signature(arguments.len(), 1, ty_args.len(), 0)?;

            let pubkey_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let pubkey: Vec<u8> = pubkey_ref.as_bytes_ref().to_vec();

            match decompress_secp256k1_pubkey(&pubkey) {
                Ok(out) => Ok(NR::ok(context.gas_used(), smallvec![Value::vector_u8(out)])),
                Err(error) => Ok(NR::err(context.gas_used(), native_error_code(error))),
            }
        },
    )
}

/// Creates the ecdsa_k1::verify native function
pub fn make_verify_k1_native(gas_cost: InternalGas) -> NativeFunction {
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

            // Try Schnorr verification first (64-byte signature with 32-byte x-only pubkey)
            if signature.len() == 64 && public_key.len() == 32 {
                return verify_schnorr_signature(context, &msg, &public_key, &signature);
            }

            // Handle invalid x-only pubkey scenarios
            if signature.len() == 64 && public_key.len() < 33 {
                return Ok(NR::err(context.gas_used(), E_INVALID_XONLY_PUBKEY));
            }

            if signature.len() != 64 && public_key.len() == 32 {
                return Ok(NR::err(context.gas_used(), E_INVALID_SCHNORR_SIGNATURE));
            }

            // Perform ECDSA verification with proper error handling
            match verify_secp256k1_ecdsa_native(
                &public_key,
                &signature,
                &msg,
                NativeEcdsaHash::from_move_selector(hash_type),
            ) {
                Ok(verified) => Ok(NR::ok(context.gas_used(), smallvec![Value::bool(verified)])),
                Err(error) => Ok(NR::err(context.gas_used(), native_error_code(error))),
            }
        },
    )
}

/// Verifies a Schnorr signature
fn verify_schnorr_signature(
    context: &mut NativeContext,
    msg: &[u8],
    public_key: &[u8],
    signature: &[u8],
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;

    match verify_secp256k1_schnorr_native(public_key, signature, msg) {
        Ok(verified) => {
            NR::map_partial_vm_result_one(context.gas_used(), Ok(Value::bool(verified)))
        }
        Err(error) => Ok(NR::err(context.gas_used(), native_error_code(error))),
    }
}

fn native_error_code(error: NativeCryptoError) -> u64 {
    match error {
        NativeCryptoError::InvalidRecovery => E_INVALID_RECOVERY,
        NativeCryptoError::InvalidSignature => E_INVALID_SIGNATURE,
        NativeCryptoError::InvalidPublicKey => E_INVALID_PUBKEY,
        NativeCryptoError::InvalidXOnlyPublicKey => E_INVALID_XONLY_PUBKEY,
        NativeCryptoError::InvalidMessage => E_INVALID_MESSAGE,
        NativeCryptoError::InvalidSchnorrSignature => E_INVALID_SCHNORR_SIGNATURE,
    }
}
