// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use better_any::{Tid, TidAble};
use move_core_types::account_address::AccountAddress;
use move_core_types::gas_algebra::{InternalGas, InternalGasPerByte, NumBytes};
use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::NativeContext;
use move_vm_runtime::native_functions::NativeFunction;
use move_vm_types::loaded_data::runtime_types::Type;
use move_vm_types::natives::function::{NativeResult, PartialVMError, PartialVMResult};
use move_vm_types::pop_arg;
use move_vm_types::values::{Struct, Value};
use sha3::{Digest, Sha3_256};
use smallvec::smallvec;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::helpers::{expect_native_signature, make_module_natives};

/// Per-execution transaction info shared with `tx_context` natives.
///
/// Populated by the executing runtime for real transactions. Hosts that do
/// not set it (unit-test runner, view calls) get `Default`: no gas price,
/// no sponsor.
#[derive(Tid, Default, Debug, Clone)]
pub struct TxInfoExt {
    /// Gas price submitted with the transaction, if the host provides one.
    pub gas_price: Option<u64>,
    /// Sponsor address. The Kanari protocol has no sponsored transactions,
    /// so executing hosts always leave this as `None`.
    pub sponsor: Option<AccountAddress>,
}

#[derive(Debug, Clone)]
pub struct GasParameters {
    pub derive_id: DeriveIdGasParameters,
    pub gas_price: GasPriceGasParameters,
    pub sponsor: SponsorGasParameters,
}

#[derive(Debug, Clone)]
pub struct DeriveIdGasParameters {
    pub base: InternalGas,
    pub per_byte: InternalGasPerByte,
}

#[derive(Debug, Clone)]
pub struct GasPriceGasParameters {
    pub base: InternalGas,
}

#[derive(Debug, Clone)]
pub struct SponsorGasParameters {
    pub base: InternalGas,
}

impl GasParameters {
    pub fn zeros() -> Self {
        Self {
            derive_id: DeriveIdGasParameters {
                base: 0.into(),
                per_byte: 0.into(),
            },
            gas_price: GasPriceGasParameters { base: 0.into() },
            sponsor: SponsorGasParameters { base: 0.into() },
        }
    }
}

pub fn make_all(gas_params: GasParameters) -> impl Iterator<Item = (String, NativeFunction)> {
    let derive_params = gas_params.derive_id;
    let derive_id: NativeFunction = Arc::new(move |context, ty_args, args| {
        native_derive_id(&derive_params, context, ty_args, args)
    });
    let gas_price_params = gas_params.gas_price;
    let gas_price: NativeFunction = Arc::new(move |context, ty_args, args| {
        native_gas_price(&gas_price_params, context, ty_args, args)
    });
    let sponsor_params = gas_params.sponsor;
    let sponsor: NativeFunction = Arc::new(move |context, ty_args, args| {
        native_sponsor(&sponsor_params, context, ty_args, args)
    });
    make_module_natives([
        ("derive_id", derive_id),
        ("gas_price", gas_price),
        ("sponsor", sponsor),
    ])
}

fn native_derive_id(
    gas_params: &DeriveIdGasParameters,
    context: &mut NativeContext,
    _ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;

    expect_native_signature(arguments.len(), 2, _ty_args.len(), 0)?;

    let ids_created = pop_arg!(arguments, u64);
    let tx_hash = pop_arg!(arguments, Vec<u8>);

    native_charge_gas_early_exit!(context, gas_params.base);
    native_charge_gas_early_exit!(
        context,
        gas_params.per_byte * NumBytes::new(tx_hash.len() as u64)
    );

    // Hash(tx_hash || ids_created)
    let mut hasher = Sha3_256::new();
    hasher.update(&tx_hash);
    hasher.update(ids_created.to_le_bytes());
    let hash = hasher.finalize();

    // Convert hash to address (take first 32 bytes)
    // AccountAddress::LENGTH is 32 (typically).
    // We safeguard against length mismatch if AccountAddress length changes.
    let addr_bytes = if hash.len() >= AccountAddress::LENGTH {
        &hash[..AccountAddress::LENGTH]
    } else {
        // Should not happen with Sha3_256 (32 bytes) and AccountAddress (32 bytes or less)
        return Err(PartialVMError::new(
            move_core_types::vm_status::StatusCode::INTERNAL_TYPE_ERROR,
        )
        .with_message("Hash length insufficient for address".to_string()));
    };

    let addr = AccountAddress::from_bytes(addr_bytes).map_err(|e| {
        PartialVMError::new(move_core_types::vm_status::StatusCode::INTERNAL_TYPE_ERROR)
            .with_message(format!("Failed to create address from hash: {}", e))
    })?;

    Ok(NR::ok(context.gas_used(), smallvec![Value::address(addr)]))
}

/// Native function: gas_price(&TxContext): u64
/// Returns the gas price submitted with the current transaction.
/// Returns 0 when the executing host provides none (unit tests, view calls).
fn native_gas_price(
    gas_params: &GasPriceGasParameters,
    context: &mut NativeContext,
    _ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;
    use move_vm_types::values::values_impl::Reference;

    native_charge_gas_early_exit!(context, gas_params.base);
    expect_native_signature(arguments.len(), 1, _ty_args.len(), 0)?;

    let _ctx_ref = pop_arg!(arguments, Reference);

    let price = crate::native_ext::with_ext_mut_or_default::<TxInfoExt, _>(context, |ext| {
        ext.gas_price
    })
    .flatten()
    .unwrap_or(0);
    Ok(NR::ok(context.gas_used(), smallvec![Value::u64(price)]))
}

/// Native function: sponsor(&TxContext): Option<address>
/// Returns the transaction sponsor, or `none`. The Kanari protocol has no
/// sponsored transactions, so this is always `none`; the extension field
/// exists so a future protocol upgrade can populate it without changing
/// the Move API.
fn native_sponsor(
    gas_params: &SponsorGasParameters,
    context: &mut NativeContext,
    _ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;
    use move_vm_types::values::values_impl::Reference;

    native_charge_gas_early_exit!(context, gas_params.base);
    expect_native_signature(arguments.len(), 1, _ty_args.len(), 0)?;

    let _ctx_ref = pop_arg!(arguments, Reference);

    let sponsor: Option<AccountAddress> =
        crate::native_ext::with_ext_mut_or_default::<TxInfoExt, _>(context, |ext| ext.sponsor)
            .unwrap_or(None);
    let inner = match sponsor {
        Some(addr) => Value::vector_address(vec![addr]),
        None => Value::vector_address(Vec::<AccountAddress>::new()),
    };
    let opt = Value::struct_(Struct::pack(vec![inner]));
    Ok(NR::ok(context.gas_used(), smallvec![opt]))
}
