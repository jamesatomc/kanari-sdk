// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! ECDSA secp256k1 (K1) native functions

use move_core_types::account_address::AccountAddress;
use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::{NativeContext, NativeFunction, make_table_from_iter};
use move_vm_types::natives::function::{NativeResult, PartialVMResult};
use move_vm_types::{
    loaded_data::runtime_types::Type,
    pop_arg,
    values::{Value, VectorRef},
};
use smallvec::smallvec;

use k256::PublicKey as K256PublicKey;
use k256::ecdsa::{
    Signature as K256Signature, VerifyingKey as K256VerifyingKey,
    signature::hazmat::PrehashVerifier as K256PrehashVerifier,
};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use secp256k1::{
    Message as SecpMessage, Secp256k1, ecdsa::RecoverableSignature as SecpRecoverableSignature,
    ecdsa::RecoveryId as SecpRecoveryId, XOnlyPublicKey, schnorr::Signature as SchnorrSig,
};
use sha2::Sha256;
use sha3::Keccak256;

use std::{collections::VecDeque, convert::TryInto, sync::Arc};

use crate::helpers::make_module_natives;
use super::types::GasParameters;

// Maximum message length accepted by natives (prevent large-memory DoS)
pub(crate) const MAX_MSG_BYTES: usize = 1_000_000; // 1 MB

// Error codes
const E_INVALID_RECOVERY: u64 = 1;
const E_INVALID_SIGNATURE: u64 = 2;
const E_INVALID_PUBKEY: u64 = 3;
const E_INVALID_XONLY_PUBKEY: u64 = 5;
const E_INVALID_MESSAGE: u64 = 6;
const E_INVALID_SCHNORR_SIGNATURE: u64 = 7;

fn make_native<F>(f: F) -> NativeFunction
where
    F: Fn(&mut NativeContext, Vec<Type>, VecDeque<Value>) -> PartialVMResult<NativeResult>
        + Send
        + Sync
        + 'static,
{
    Arc::new(f)
}

pub fn make_ecdsa_k1(gas_params: GasParameters) -> impl Iterator<Item = (String, NativeFunction)> {
    make_module_natives(
        all_natives_with_gas(AccountAddress::ZERO, gas_params)
            .into_iter()
            .filter(|(_, module_name, _, _)| module_name.as_str() == "ecdsa_k1")
            .map(|(_, _, func_name, func)| (func_name.to_string(), func)),
    )
}

fn all_natives_with_gas(
    move_addr: AccountAddress,
    gas_params: GasParameters,
) -> Vec<(String, String, NativeFunction)> {
    let mut natives = vec![];

    let ecrecover_cost = gas_params.ecrecover;
    let decompress_pubkey_cost = gas_params.decompress_pubkey;
    let verify_k1_cost = gas_params.verify_k1;

    // ecdsa_k1::ecrecover(signature: vector<u8>, msg: vector<u8>, hash: u8): vector<u8>
    let ecrecover_native = make_native(
        move |context, _ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            native_charge_gas_early_exit!(context, ecrecover_cost);

            let hash_type: u8 = pop_arg!(arguments, u8);
            let msg_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let signature_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let msg: Vec<u8> = msg_ref.as_bytes_ref().to_vec();
            let signature: Vec<u8> = signature_ref.as_bytes_ref().to_vec();

            if signature.len() != 65 {
                return Ok(NativeResult::err(context.gas_used(), E_INVALID_SIGNATURE));
            }

            if msg.len() > MAX_MSG_BYTES {
                return Ok(NativeResult::err(context.gas_used(), E_INVALID_MESSAGE));
            }

            let msg_hash = if hash_type == 0u8 {
                Keccak256::digest(&msg).to_vec()
            } else {
                Sha256::digest(&msg).to_vec()
            };

            let mut sig64 = [0u8; 64];
            sig64.copy_from_slice(&signature[0..64]);
            let v = signature[64];
            let rec_id = if v <= 3 {
                SecpRecoveryId::try_from(v as i32)
            } else if v == 27 || v == 28 {
                SecpRecoveryId::try_from((v - 27) as i32)
            } else {
                Err(secp256k1::Error::InvalidSignature)
            };
            let rec_id = match rec_id {
                Ok(r) => r,
                Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_RECOVERY)),
            };
            let secp_sig = match SecpRecoverableSignature::from_compact(&sig64, rec_id) {
                Ok(s) => s,
                Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_RECOVERY)),
            };
            let secp = Secp256k1::new();
            if msg_hash.len() != 32 {
                return Ok(NativeResult::err(context.gas_used(), E_INVALID_MESSAGE));
            }
            let msg32: [u8; 32] = match msg_hash.try_into() {
                Ok(arr) => arr,
                Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_MESSAGE)),
            };
            let message = SecpMessage::from_digest(msg32);
            let pubkey = match secp.recover_ecdsa(message, &secp_sig) {
                Ok(pk) => pk,
                Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_RECOVERY)),
            };
            let out = pubkey.serialize().to_vec();
            Ok(NativeResult::ok(context.gas_used(), smallvec![Value::vector_u8(out)]))
        },
    );

    // ecdsa_k1::decompress_pubkey(pubkey: vector<u8>): vector<u8>
    let decompress_native = make_native(
        move |context, _ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            native_charge_gas_early_exit!(context, decompress_pubkey_cost);
            let pubkey_ref: VectorRef = pop_arg!(arguments, VectorRef);
            let mut pubkey: Vec<u8> = pubkey_ref.as_bytes_ref().to_vec();

            if pubkey.len() == 64 {
                let mut prefixed = Vec::with_capacity(65);
                prefixed.push(0x04);
                prefixed.extend_from_slice(&pubkey);
                pubkey = prefixed;
            }

            let pk = match K256PublicKey::from_sec1_bytes(&pubkey) {
                Ok(p) => p,
                Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_PUBKEY)),
            };
            let ep = pk.to_encoded_point(false);
            let out = ep.as_bytes().to_vec();
            Ok(NativeResult::ok(context.gas_used(), smallvec![Value::vector_u8(out)]))
        },
    );

    // ecdsa_k1::verify(signature, public_key, msg, hash) -> bool
    let verify_k1 = make_native(
        move |context, _ty_args, mut arguments| -> PartialVMResult<NativeResult> {
            native_charge_gas_early_exit!(context, verify_k1_cost);
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

            // Check for Schnorr signature (64 bytes with 32-byte x-only pubkey)
            if signature.len() == 64 {
                if public_key.len() == 32 {
                    if msg.len() != 32 {
                        return Ok(NativeResult::err(context.gas_used(), E_INVALID_MESSAGE));
                    }

                    let msg32: [u8; 32] = match msg.as_slice().try_into() {
                        Ok(a) => a,
                        Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_MESSAGE)),
                    };

                    let pub_array: [u8; 32] = match public_key.try_into() {
                        Ok(arr) => arr,
                        Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_XONLY_PUBKEY)),
                    };
                    let xpk = match XOnlyPublicKey::from_byte_array(pub_array) {
                        Ok(x) => x,
                        Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_XONLY_PUBKEY)),
                    };

                    let sig_array: [u8; 64] = match signature.try_into() {
                        Ok(arr) => arr,
                        Err(_) => {
                            return Ok(NativeResult::err(context.gas_used(), E_INVALID_SCHNORR_SIGNATURE));
                        }
                    };
                    let sch_sig = SchnorrSig::from_byte_array(sig_array);

                    let secp = Secp256k1::new();
                    let verified = secp.verify_schnorr(&sch_sig, &msg32, &xpk).is_ok();
                    return NativeResult::map_partial_vm_result_one(context.gas_used(), Ok(Value::bool(verified)));
                }

                if public_key.len() < 33 {
                    return Ok(NativeResult::err(context.gas_used(), E_INVALID_XONLY_PUBKEY));
                }
            } else {
                if public_key.len() == 32 {
                    return Ok(NativeResult::err(context.gas_used(), E_INVALID_SCHNORR_SIGNATURE));
                }
            }

            // Normalize pubkey encoding
            if public_key.len() == 64 {
                let mut prefixed = Vec::with_capacity(65);
                prefixed.push(0x04);
                prefixed.extend_from_slice(&public_key);
                public_key = prefixed;
            }

            let vk = match K256VerifyingKey::from_sec1_bytes(&public_key) {
                Ok(v) => v,
                Err(_) => return Ok(NativeResult::err(context.gas_used(), E_INVALID_PUBKEY)),
            };

            let sig_bytes = if signature.len() == 65 {
                &signature[..64]
            } else {
                signature.as_slice()
            };

            let sig = if let Ok(s) = K256Signature::from_der(&signature) {
                s
            } else if sig_bytes.len() == 64 {
                match K256Signature::try_from(sig_bytes) {
                    Ok(s) => s,
                    Err(_) => {
                        log::debug!("Failed to parse signature as raw 64 bytes");
                        return Ok(NativeResult::ok(context.gas_used(), smallvec![Value::bool(false)]));
                    }
                }
            } else {
                log::debug!("Invalid signature length: {}", signature.len());
                return Ok(NativeResult::ok(context.gas_used(), smallvec![Value::bool(false)]));
            };

            let verified = if hash_type == 0u8 {
                let msg_hash = Keccak256::digest(&msg);
                vk.verify_prehash(msg_hash.as_slice(), &sig).is_ok()
            } else {
                let msg_hash = Sha256::digest(&msg);
                vk.verify_prehash(msg_hash.as_slice(), &sig).is_ok()
            };

            NativeResult::map_partial_vm_result_one(
                context.gas_used(),
                Ok(Value::bool(verified)),
            )
        },
    );

    natives.push(("ecdsa_k1".to_string(), "ecrecover".to_string(), ecrecover_native));
    natives.push(("ecdsa_k1".to_string(), "decompress_pubkey".to_string(), decompress_native));
    natives.push(("ecdsa_k1".to_string(), "verify".to_string(), verify_k1));

    natives
}
