// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use better_any::{Tid, TidAble};
use kanari_crypto::hash_data_blake3;
use move_core_types::gas_algebra::InternalGas;
use move_core_types::runtime_value::{MoveStructLayout, MoveTypeLayout};
use move_core_types::vm_status::StatusCode;
use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::{NativeContext, NativeFunction};
use move_vm_types::loaded_data::runtime_types::Type;
use move_vm_types::natives::function::{NativeResult, PartialVMError, PartialVMResult};
use move_vm_types::pop_arg;
use move_vm_types::values::values_impl::Reference;
use move_vm_types::values::{Locals, Value};
use smallvec::smallvec;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::helpers::{expect_native_signature, make_module_natives};

// ==============================================================================
// Error Codes (must match declarations in dynamic_field.move)
// ==============================================================================
const E_FIELD_ALREADY_EXISTS: u64 = 1;
const E_FIELD_DOES_NOT_EXIST: u64 = 2;

// ==============================================================================
// Gas Parameters
// ==============================================================================

#[derive(Debug, Clone)]
pub struct GasParameters {
    pub add: InternalGas,
    pub borrow: InternalGas,
    pub borrow_mut: InternalGas,
    pub remove: InternalGas,
    pub exists_: InternalGas,
}

impl GasParameters {
    pub fn zeros() -> Self {
        Self {
            add: 0.into(),
            borrow: 0.into(),
            borrow_mut: 0.into(),
            remove: 0.into(),
            exists_: 0.into(),
        }
    }
}

// ==============================================================================
// State Extensions (Tid) for ObjectRuntime
// ==============================================================================

#[derive(Clone, Debug)]
pub enum DynamicFieldOp {
    Add {
        object_id: String,
        name_bytes: Vec<u8>,
        value_bytes: Vec<u8>,
    },
    Remove {
        object_id: String,
        name_bytes: Vec<u8>,
    },
}

#[derive(Tid, Default)]
pub struct DynamicFieldsExt {
    pub ops: Vec<DynamicFieldOp>,
}

impl DynamicFieldsExt {
    pub fn record(&mut self, op: DynamicFieldOp) {
        self.ops.push(op);
    }
    pub fn take_all(&mut self) -> Vec<DynamicFieldOp> {
        std::mem::take(&mut self.ops)
    }
}

pub trait DynamicFieldResolver: Send + Sync {
    fn get_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Option<Vec<u8>>;
}

#[derive(Tid, Default, Clone)]
pub struct DynamicFieldResolverExt {
    pub resolver: Option<Arc<dyn DynamicFieldResolver>>,
}

struct DynamicFieldEntry {
    object_id: String,
    name_bytes: Vec<u8>,
    layout: MoveTypeLayout,
    locals: Locals,
    mutable: bool,
}

#[derive(Tid, Default)]
pub struct DynamicFieldReferencesExt {
    entries: BTreeMap<String, DynamicFieldEntry>,
}

impl DynamicFieldReferencesExt {
    fn field_key(object_id: &str, name_bytes: &[u8]) -> String {
        let hash = hash_data_blake3(name_bytes);
        format!("{object_id}:{}", hex::encode(&hash[..16]))
    }

    fn snapshot_bytes(&self, object_id: &str, name_bytes: &[u8]) -> Option<Vec<u8>> {
        let key = Self::field_key(object_id, name_bytes);
        let entry = self.entries.get(&key)?;
        let value = entry.locals.copy_loc(0).ok()?;
        value.simple_serialize(&entry.layout)
    }

    fn borrow_existing(&self, object_id: &str, name_bytes: &[u8]) -> Option<Value> {
        let key = Self::field_key(object_id, name_bytes);
        self.entries.get(&key)?.locals.borrow_loc(0).ok()
    }

    fn insert(
        &mut self,
        object_id: String,
        name_bytes: Vec<u8>,
        layout: MoveTypeLayout,
        value: Value,
        mutable: bool,
    ) -> PartialVMResult<Value> {
        let key = Self::field_key(&object_id, &name_bytes);
        if let Some(entry) = self.entries.get_mut(&key) {
            if mutable {
                entry.mutable = true;
            }
            return entry.locals.borrow_loc(0);
        }

        let mut locals = Locals::new(1);
        locals.store_loc(0, value, false)?;
        let borrowed = locals.borrow_loc(0)?;
        self.entries.insert(
            key,
            DynamicFieldEntry {
                object_id,
                name_bytes,
                layout,
                locals,
                mutable,
            },
        );
        Ok(borrowed)
    }

    fn remove_value(
        &mut self,
        object_id: &str,
        name_bytes: &[u8],
    ) -> Option<(Value, MoveTypeLayout)> {
        let key = Self::field_key(object_id, name_bytes);
        let mut entry = self.entries.remove(&key)?;
        let value = entry.locals.move_loc(0, false).ok()?;
        Some((value, entry.layout))
    }

    pub fn take_mutated(&mut self) -> Vec<(String, Vec<u8>, Vec<u8>)> {
        let mut out = Vec::new();
        for entry in self.entries.values() {
            if !entry.mutable {
                continue;
            }
            let Some(value) = entry.locals.copy_loc(0).ok() else {
                continue;
            };
            let Some(bytes) = value.simple_serialize(&entry.layout) else {
                continue;
            };
            out.push((entry.object_id.clone(), entry.name_bytes.clone(), bytes));
        }
        self.entries.clear();
        out
    }
}

enum DynamicFieldState<'a> {
    Added(&'a [u8]),
    Removed,
    Missing,
}

enum DynamicFieldBytesState {
    Added(Vec<u8>),
    Removed,
    Missing,
}

fn uid_object_id_from_ref(uid_ref: Reference) -> PartialVMResult<String> {
    let uid_val = uid_ref.read_ref()?;
    let layout = MoveTypeLayout::Struct(MoveStructLayout::new(vec![MoveTypeLayout::Address]));
    let uid_bytes = uid_val
        .simple_serialize(&layout)
        .ok_or_else(|| PartialVMError::new(StatusCode::VALUE_SERIALIZATION_ERROR))?;
    Ok(format!("0x{}", hex::encode(uid_bytes)))
}

fn latest_dynamic_field_state<'a>(
    ops: &'a [DynamicFieldOp],
    object_id: &str,
    name_bytes: &[u8],
) -> DynamicFieldState<'a> {
    for op in ops.iter().rev() {
        match op {
            DynamicFieldOp::Add {
                object_id: existing_object_id,
                name_bytes: existing_name,
                value_bytes,
            } if existing_object_id == object_id && existing_name == name_bytes => {
                return DynamicFieldState::Added(value_bytes);
            }
            DynamicFieldOp::Remove {
                object_id: existing_object_id,
                name_bytes: existing_name,
            } if existing_object_id == object_id && existing_name == name_bytes => {
                return DynamicFieldState::Removed;
            }
            _ => {}
        }
    }
    DynamicFieldState::Missing
}

fn load_dynamic_field_bytes(
    context: &mut NativeContext,
    object_id: &str,
    name_bytes: &[u8],
) -> DynamicFieldBytesState {
    if let Some(bytes) =
        crate::native_ext::with_ext_mut_or_default::<DynamicFieldReferencesExt, _>(context, |ext| {
            ext.snapshot_bytes(object_id, name_bytes)
        })
        .flatten()
    {
        return DynamicFieldBytesState::Added(bytes);
    }

    if let Some(state) =
        crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
            match latest_dynamic_field_state(&ext.ops, object_id, name_bytes) {
                DynamicFieldState::Added(bytes) => {
                    Some(DynamicFieldBytesState::Added(bytes.to_vec()))
                }
                DynamicFieldState::Removed => Some(DynamicFieldBytesState::Removed),
                DynamicFieldState::Missing => None,
            }
        })
        .flatten()
    {
        return state;
    }

    if let Some(bytes) =
        crate::native_ext::with_ext_mut_or_default::<DynamicFieldResolverExt, _>(context, |ext| {
            ext.resolver
                .as_ref()
                .and_then(|resolver| resolver.get_dynamic_field(object_id, name_bytes))
        })
        .flatten()
    {
        return DynamicFieldBytesState::Added(bytes);
    }

    DynamicFieldBytesState::Missing
}

// ==============================================================================
// Native Function Registrations
// ==============================================================================

pub fn make_all(gas_params: GasParameters) -> impl Iterator<Item = (String, NativeFunction)> {
    let add_gas = gas_params.add;
    let borrow_gas = gas_params.borrow;
    let borrow_mut_gas = gas_params.borrow_mut;
    let remove_gas = gas_params.remove;
    let exists_gas = gas_params.exists_;

    let add: NativeFunction =
        Arc::new(move |context, ty_args, args| native_add(add_gas, context, ty_args, args));
    let borrow_mut: NativeFunction = Arc::new(move |context, ty_args, args| {
        native_borrow_mut(borrow_mut_gas, context, ty_args, args)
    });
    let borrow: NativeFunction =
        Arc::new(move |context, ty_args, args| native_borrow(borrow_gas, context, ty_args, args));
    let remove: NativeFunction =
        Arc::new(move |context, ty_args, args| native_remove(remove_gas, context, ty_args, args));
    let exists_: NativeFunction =
        Arc::new(move |context, ty_args, args| native_exists_(exists_gas, context, ty_args, args));

    make_module_natives([
        ("add", add),
        ("borrow_mut", borrow_mut),
        ("borrow", borrow),
        ("remove", remove),
        ("exists_", exists_),
    ])
}

// ==============================================================================
// Native Implementations (Safe Mode)
// ==============================================================================

fn native_add(
    gas_base: InternalGas,
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;

    native_charge_gas_early_exit!(context, gas_base);

    expect_native_signature(arguments.len(), 3, ty_args.len(), 2)?;

    let value = arguments.pop_back().ok_or_else(|| {
        PartialVMError::new(StatusCode::NUMBER_OF_ARGUMENTS_MISMATCH)
            .with_message("Missing dynamic field value argument".to_string())
    })?;
    let name = arguments.pop_back().ok_or_else(|| {
        PartialVMError::new(StatusCode::NUMBER_OF_ARGUMENTS_MISMATCH)
            .with_message("Missing dynamic field name argument".to_string())
    })?;
    let uid_ref = pop_arg!(arguments, Reference);

    // 1. Serialize Name safely (avoid unwrap)
    let name_layout = match context.type_to_type_layout(&ty_args[0]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };
    let name_bytes = match name.simple_serialize(&name_layout) {
        Some(bytes) => bytes,
        None => return Err(PartialVMError::new(StatusCode::VALUE_SERIALIZATION_ERROR)),
    };

    // 2. Serialize Value safely
    let value_layout = match context.type_to_type_layout(&ty_args[1]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };
    let value_bytes = match value.simple_serialize(&value_layout) {
        Some(bytes) => bytes,
        None => return Err(PartialVMError::new(StatusCode::VALUE_SERIALIZATION_ERROR)),
    };

    let object_id_str = uid_object_id_from_ref(uid_ref)?;

    let already_exists = matches!(
        load_dynamic_field_bytes(context, &object_id_str, &name_bytes),
        DynamicFieldBytesState::Added(_)
    );

    if !already_exists {
        crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
            ext.record(DynamicFieldOp::Add {
                object_id: object_id_str.clone(),
                name_bytes,
                value_bytes,
            });
        });
    }

    // If duplicate Key is added, return Error gracefully to Move VM (Abort but Node does not crash)
    if already_exists {
        Ok(NR::err(context.gas_used(), E_FIELD_ALREADY_EXISTS))
    } else {
        Ok(NR::ok(context.gas_used(), smallvec![]))
    }
}

fn native_borrow_mut(
    gas_base: InternalGas,
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    native_charge_gas_early_exit!(context, gas_base);

    expect_native_signature(arguments.len(), 2, ty_args.len(), 2)?;
    let name = arguments.pop_back().ok_or_else(|| {
        PartialVMError::new(StatusCode::NUMBER_OF_ARGUMENTS_MISMATCH)
            .with_message("Missing dynamic field name argument".to_string())
    })?;
    let uid_ref = pop_arg!(arguments, Reference);

    let name_layout = match context.type_to_type_layout(&ty_args[0]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };
    let name_bytes = match name.simple_serialize(&name_layout) {
        Some(bytes) => bytes,
        None => return Err(PartialVMError::new(StatusCode::VALUE_SERIALIZATION_ERROR)),
    };
    let object_id = uid_object_id_from_ref(uid_ref)?;
    let value_layout = match context.type_to_type_layout(&ty_args[1]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };

    if let Some(existing_ref) = crate::native_ext::with_ext_mut_or_default::<
        DynamicFieldReferencesExt,
        _,
    >(context, |ext| ext.borrow_existing(&object_id, &name_bytes))
    .flatten()
    {
        return Ok(NativeResult::ok(
            context.gas_used(),
            smallvec![existing_ref],
        ));
    }

    let bytes = match load_dynamic_field_bytes(context, &object_id, &name_bytes) {
        DynamicFieldBytesState::Added(bytes) => bytes,
        DynamicFieldBytesState::Removed | DynamicFieldBytesState::Missing => {
            return Ok(NativeResult::err(
                context.gas_used(),
                E_FIELD_DOES_NOT_EXIST,
            ));
        }
    };
    let value = Value::simple_deserialize(&bytes, &value_layout)
        .ok_or_else(|| PartialVMError::new(StatusCode::VALUE_DESERIALIZATION_ERROR))?;
    let borrowed = crate::native_ext::with_ext_mut_or_default::<DynamicFieldReferencesExt, _>(
        context,
        |ext| {
            ext.insert(
                object_id.clone(),
                name_bytes.clone(),
                value_layout.clone(),
                value,
                true,
            )
        },
    )
    .ok_or_else(|| PartialVMError::new(StatusCode::UNKNOWN_INVARIANT_VIOLATION_ERROR))??;

    Ok(NativeResult::ok(context.gas_used(), smallvec![borrowed]))
}

fn native_borrow(
    gas_base: InternalGas,
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    native_charge_gas_early_exit!(context, gas_base);

    expect_native_signature(arguments.len(), 2, ty_args.len(), 2)?;
    let name = arguments.pop_back().ok_or_else(|| {
        PartialVMError::new(StatusCode::NUMBER_OF_ARGUMENTS_MISMATCH)
            .with_message("Missing dynamic field name argument".to_string())
    })?;
    let uid_ref = pop_arg!(arguments, Reference);

    let name_layout = match context.type_to_type_layout(&ty_args[0]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };
    let name_bytes = match name.simple_serialize(&name_layout) {
        Some(bytes) => bytes,
        None => return Err(PartialVMError::new(StatusCode::VALUE_SERIALIZATION_ERROR)),
    };
    let object_id = uid_object_id_from_ref(uid_ref)?;
    let value_layout = match context.type_to_type_layout(&ty_args[1]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };

    if let Some(existing_ref) = crate::native_ext::with_ext_mut_or_default::<
        DynamicFieldReferencesExt,
        _,
    >(context, |ext| ext.borrow_existing(&object_id, &name_bytes))
    .flatten()
    {
        return Ok(NativeResult::ok(
            context.gas_used(),
            smallvec![existing_ref],
        ));
    }

    let bytes = match load_dynamic_field_bytes(context, &object_id, &name_bytes) {
        DynamicFieldBytesState::Added(bytes) => bytes,
        DynamicFieldBytesState::Removed | DynamicFieldBytesState::Missing => {
            return Ok(NativeResult::err(
                context.gas_used(),
                E_FIELD_DOES_NOT_EXIST,
            ));
        }
    };
    let value = Value::simple_deserialize(&bytes, &value_layout)
        .ok_or_else(|| PartialVMError::new(StatusCode::VALUE_DESERIALIZATION_ERROR))?;
    let borrowed = crate::native_ext::with_ext_mut_or_default::<DynamicFieldReferencesExt, _>(
        context,
        |ext| {
            ext.insert(
                object_id.clone(),
                name_bytes.clone(),
                value_layout.clone(),
                value,
                false,
            )
        },
    )
    .ok_or_else(|| PartialVMError::new(StatusCode::UNKNOWN_INVARIANT_VIOLATION_ERROR))??;

    Ok(NativeResult::ok(context.gas_used(), smallvec![borrowed]))
}

fn native_remove(
    gas_base: InternalGas,
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;

    native_charge_gas_early_exit!(context, gas_base);

    expect_native_signature(arguments.len(), 2, ty_args.len(), 2)?;
    let name = arguments.pop_back().ok_or_else(|| {
        PartialVMError::new(StatusCode::NUMBER_OF_ARGUMENTS_MISMATCH)
            .with_message("Missing dynamic field name argument".to_string())
    })?;
    let uid_ref = pop_arg!(arguments, Reference);

    let name_layout = match context.type_to_type_layout(&ty_args[0]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };
    let name_bytes = match name.simple_serialize(&name_layout) {
        Some(bytes) => bytes,
        None => return Err(PartialVMError::new(StatusCode::VALUE_SERIALIZATION_ERROR)),
    };
    let object_id = uid_object_id_from_ref(uid_ref)?;

    let value_layout = match context.type_to_type_layout(&ty_args[1]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };

    if let Some((value, _layout)) = crate::native_ext::with_ext_mut_or_default::<
        DynamicFieldReferencesExt,
        _,
    >(context, |ext| ext.remove_value(&object_id, &name_bytes))
    .flatten()
    {
        crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
            ext.record(DynamicFieldOp::Remove {
                object_id: object_id.clone(),
                name_bytes: name_bytes.clone(),
            });
        });
        return Ok(NR::ok(context.gas_used(), smallvec![value]));
    }

    let value_bytes = match load_dynamic_field_bytes(context, &object_id, &name_bytes) {
        DynamicFieldBytesState::Added(bytes) => bytes,
        DynamicFieldBytesState::Removed | DynamicFieldBytesState::Missing => {
            return Ok(NR::err(context.gas_used(), E_FIELD_DOES_NOT_EXIST));
        }
    };
    let value = Value::simple_deserialize(&value_bytes, &value_layout)
        .ok_or_else(|| PartialVMError::new(StatusCode::VALUE_DESERIALIZATION_ERROR))?;

    crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
        ext.record(DynamicFieldOp::Remove {
            object_id: object_id.clone(),
            name_bytes: name_bytes.clone(),
        });
    });

    Ok(NR::ok(context.gas_used(), smallvec![value]))
}

fn native_exists_(
    gas_base: InternalGas,
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;

    native_charge_gas_early_exit!(context, gas_base);

    expect_native_signature(arguments.len(), 2, ty_args.len(), 1)?;

    let name = arguments.pop_back().ok_or_else(|| {
        PartialVMError::new(StatusCode::NUMBER_OF_ARGUMENTS_MISMATCH)
            .with_message("Missing dynamic field name argument".to_string())
    })?;
    let uid_ref = pop_arg!(arguments, Reference);

    // Serialize Name safely
    let name_layout = match context.type_to_type_layout(&ty_args[0]) {
        Ok(Some(layout)) => layout,
        _ => return Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    };
    let name_bytes = match name.simple_serialize(&name_layout) {
        Some(bytes) => bytes,
        None => return Err(PartialVMError::new(StatusCode::VALUE_SERIALIZATION_ERROR)),
    };

    let object_id = uid_object_id_from_ref(uid_ref)?;
    let is_exist = matches!(
        load_dynamic_field_bytes(context, &object_id, &name_bytes),
        DynamicFieldBytesState::Added(_)
    );

    Ok(NR::ok(context.gas_used(), smallvec![Value::bool(is_exist)]))
}
