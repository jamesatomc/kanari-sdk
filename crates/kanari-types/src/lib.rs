use std::{collections::VecDeque, sync::Arc};

use move_vm_runtime::native_functions::NativeFunction;
use move_vm_types::natives::function::{NativeResult, PartialVMResult};

pub mod address;
pub mod balance;
pub mod coin;
pub mod kanari;
pub mod transfer;
pub mod transfer_natives;
pub mod tx_context;

// Move Standard Library bindings
pub mod stdlib;

pub mod crypto;
pub mod deny_list;
pub mod object;

// Build a NativeFunction easily
fn make_native<F>(f: F) -> NativeFunction
where
    F: Fn(
            &mut move_vm_runtime::native_functions::NativeContext,
            Vec<move_vm_types::loaded_data::runtime_types::Type>,
            VecDeque<move_vm_types::values::Value>,
        ) -> PartialVMResult<NativeResult>
        + Send
        + Sync
        + 'static,
{
    Arc::new(f)
}
