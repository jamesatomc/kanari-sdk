// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use move_core_types::account_address::AccountAddress;
use move_vm_runtime::native_functions::{NativeFunctionTable, make_table_from_iter};

pub mod address;
pub mod base64;
pub mod crypto;
pub mod dynamic_field;
pub mod event;
mod helpers;
pub mod math_calculate;
mod native_ext;
pub mod object;
pub mod transfer_natives;
pub mod tx_context;

#[derive(Debug, Clone)]
pub struct GasParameters {
    pub address: address::GasParameters,
    pub base64: base64::GasParameters,
    pub crypto: crypto::GasParameters,
    pub event: event::GasParameters,
    pub math_calculate: math_calculate::GasParameters,
    pub object: object::GasParameters,
    pub transfer: transfer_natives::GasParameters,
    pub tx_context: tx_context::GasParameters,
    pub dynamic_field: dynamic_field::GasParameters,
}

impl GasParameters {
    pub fn zeros() -> Self {
        Self {
            address: address::GasParameters::zeros(),
            base64: base64::GasParameters::zeros(),
            crypto: crypto::GasParameters::zeros(),
            event: event::GasParameters::zeros(),
            math_calculate: math_calculate::GasParameters::zeros(),
            object: object::GasParameters::zeros(),
            transfer: transfer_natives::GasParameters::zeros(),
            tx_context: tx_context::GasParameters::zeros(),
            dynamic_field: dynamic_field::GasParameters::zeros(),
        }
    }
}

pub fn all_natives(move_addr: AccountAddress, gas_params: GasParameters) -> NativeFunctionTable {
    let mut natives = vec![];

    macro_rules! add_module_natives {
        ($module_name:expr, $module_natives:expr) => {
            natives.extend(
                $module_natives
                    .map(|(func_name, func)| ($module_name.to_string(), func_name, func)),
            );
        };
    }

    add_module_natives!("base64", base64::make_base64_natives(gas_params.base64));
    add_module_natives!("ecdsa_k1", crypto::make_ecdsa_k1(gas_params.crypto.clone()));
    add_module_natives!("ecdsa_r1", crypto::make_ecdsa_r1(gas_params.crypto.clone()));
    add_module_natives!("ed25519", crypto::make_ed25519(gas_params.crypto.clone()));
    add_module_natives!(
        "dilithium2",
        crypto::make_dilithium2(gas_params.crypto.clone())
    );
    add_module_natives!(
        "dilithium3",
        crypto::make_dilithium3(gas_params.crypto.clone())
    );
    add_module_natives!(
        "dilithium5",
        crypto::make_dilithium5(gas_params.crypto.clone())
    );
    add_module_natives!(
        "ed25519_dilithium3",
        crypto::make_ed25519_dilithium3(gas_params.crypto.clone())
    );
    add_module_natives!(
        "k256_dilithium3",
        crypto::make_k256_dilithium3(gas_params.crypto.clone())
    );
    add_module_natives!("rs256", crypto::make_rs256(gas_params.crypto));

    add_module_natives!("event", event::make_all(gas_params.event));
    add_module_natives!("math", math_calculate::make_all(gas_params.math_calculate));
    add_module_natives!("object", object::make_all(gas_params.object));
    add_module_natives!("transfer", transfer_natives::make_all(gas_params.transfer));
    add_module_natives!("tx_context", tx_context::make_all(gas_params.tx_context));
    add_module_natives!(
        "dynamic_field",
        dynamic_field::make_all(gas_params.dynamic_field.clone())
    );
    add_module_natives!(
        "dynamic_object_field",
        dynamic_field::make_all(gas_params.dynamic_field)
    );

    add_module_natives!("address", address::make_address_natives(gas_params.address));

    make_table_from_iter(move_addr, natives)
}
