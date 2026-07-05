// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use better_any::{Tid, TidAble};
use move_core_types::gas_algebra::InternalGas;
use move_core_types::runtime_value::MoveTypeLayout;
use move_core_types::vm_status::StatusCode;
use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::{NativeContext, NativeFunction};
use move_vm_types::loaded_data::runtime_types::Type;
use move_vm_types::natives::function::{NativeResult, PartialVMError, PartialVMResult};
use move_vm_types::pop_arg;
use move_vm_types::values::values_impl::{GlobalValue, Reference};
use move_vm_types::values::{Struct, Value};
use smallvec::smallvec;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use crate::helpers::{expect_native_signature, make_module_natives};

const E_FIELD_ALREADY_EXISTS: u64 = 1;
const E_FIELD_DOES_NOT_EXIST: u64 = 2;

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

pub trait DynamicFieldResolver: Send + Sync {
    fn get_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Option<Vec<u8>>;
}

#[derive(Tid, Clone, Default)]
pub struct DynamicFieldStorageExt {
    resolver: Option<Arc<dyn DynamicFieldResolver>>,
}

impl DynamicFieldStorageExt {
    pub fn new(resolver: Arc<dyn DynamicFieldResolver>) -> Self {
        Self {
            resolver: Some(resolver),
        }
    }

    fn get_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Option<Vec<u8>> {
        self.resolver
            .as_ref()
            .and_then(|resolver| resolver.get_dynamic_field(object_id, name_bytes))
    }
}

#[derive(Clone, Debug)]
struct FieldLocation {
    object_id: String,
    name_bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
struct ValueSpec {
    layout: MoveTypeLayout,
    type_name: String,
}

struct BorrowRequest {
    location: FieldLocation,
    value_spec: ValueSpec,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DynamicFieldState {
    Clean,
    Dirty,
    Deleted,
}

struct CachedDynamicField {
    location: FieldLocation,
    value_spec: ValueSpec,
    wrapper: GlobalValue,
    last_value_bytes: Vec<u8>,
    existed_before_tx: bool,
    state: DynamicFieldState,
}

impl CachedDynamicField {
    fn new(
        location: FieldLocation,
        value_spec: ValueSpec,
        value: Value,
        value_bytes: Vec<u8>,
        existed_before_tx: bool,
        state: DynamicFieldState,
    ) -> Self {
        Self {
            location,
            value_spec,
            wrapper: GlobalValue::cached(Value::struct_(Struct::pack([value])))
                .expect("wrapped dynamic field value must be cacheable"),
            last_value_bytes: value_bytes,
            existed_before_tx,
            state,
        }
    }

    fn borrow_ref(&self) -> PartialVMResult<Value> {
        self.wrapper
            .borrow_global()?
            .value_as::<move_vm_types::values::StructRef>()?
            .borrow_field(0)
    }

    fn read_value(&self) -> PartialVMResult<Value> {
        self.borrow_ref()?.value_as::<Reference>()?.read_ref()
    }

    fn serialize_current_value(&self) -> Option<Vec<u8>> {
        self.read_value()
            .ok()?
            .simple_serialize(&self.value_spec.layout)
            .or_else(|| Some(self.last_value_bytes.clone()))
    }
}

#[derive(Tid, Default)]
pub struct DynamicFieldsExt {
    fields: BTreeMap<String, CachedDynamicField>,
}

impl DynamicFieldsExt {
    fn field_key(location: &FieldLocation) -> String {
        format!(
            "{}:{}",
            location.object_id,
            hex::encode(&location.name_bytes)
        )
    }

    fn cached_exists(&self, location: &FieldLocation) -> Option<bool> {
        self.fields
            .get(&Self::field_key(location))
            .map(|entry| entry.state != DynamicFieldState::Deleted)
    }

    fn insert(&mut self, entry: CachedDynamicField) {
        self.fields.insert(Self::field_key(&entry.location), entry);
    }

    fn get_mut(&mut self, location: &FieldLocation) -> Option<&mut CachedDynamicField> {
        self.fields.get_mut(&Self::field_key(location))
    }

    pub fn take_all(&mut self) -> Vec<DynamicFieldOp> {
        let mut ops = Vec::new();

        for (_, entry) in std::mem::take(&mut self.fields) {
            match entry.state {
                DynamicFieldState::Deleted => {
                    if entry.existed_before_tx {
                        ops.push(DynamicFieldOp::Remove {
                            object_id: entry.location.object_id,
                            name_bytes: entry.location.name_bytes,
                        });
                    }
                }
                DynamicFieldState::Dirty => {
                    if let Some(value_bytes) = entry.serialize_current_value() {
                        ops.push(DynamicFieldOp::Add {
                            object_id: entry.location.object_id,
                            name_bytes: entry.location.name_bytes,
                            value_bytes,
                        });
                    }
                }
                DynamicFieldState::Clean => {}
            }
        }

        ops
    }
}

fn object_id_from_uid_ref(uid_ref: Reference) -> PartialVMResult<String> {
    let uid_val = uid_ref.read_ref()?;
    let uid_bytes = crate::object::uid_address_bytes(&uid_val).ok_or_else(|| {
        PartialVMError::new(StatusCode::INTERNAL_TYPE_ERROR)
            .with_message("Failed to serialize UID".to_string())
    })?;
    Ok(format!("0x{}", hex::encode(uid_bytes)))
}

fn type_layout(context: &mut NativeContext, ty: &Type) -> PartialVMResult<MoveTypeLayout> {
    match context.type_to_type_layout(ty) {
        Ok(Some(layout)) => Ok(layout),
        _ => Err(PartialVMError::new(StatusCode::TYPE_RESOLUTION_FAILURE)),
    }
}

fn type_string(context: &mut NativeContext, ty: &Type) -> PartialVMResult<String> {
    Ok(format!("{}", context.type_to_type_tag(ty)?))
}

fn serialize_arg_with_layout(value: &Value, layout: &MoveTypeLayout) -> PartialVMResult<Vec<u8>> {
    value
        .simple_serialize(layout)
        .ok_or_else(|| PartialVMError::new(StatusCode::VALUE_SERIALIZATION_ERROR))
}

fn pop_name_arg(arguments: &mut VecDeque<Value>) -> PartialVMResult<Value> {
    arguments.pop_back().ok_or_else(|| {
        PartialVMError::new(StatusCode::NUMBER_OF_ARGUMENTS_MISMATCH)
            .with_message("Missing dynamic field name argument".to_string())
    })
}

fn parse_field_location(
    context: &mut NativeContext,
    name_ty: &Type,
    uid_ref: Reference,
    name: Value,
) -> PartialVMResult<FieldLocation> {
    let object_id = object_id_from_uid_ref(uid_ref)?;
    let name_layout = type_layout(context, name_ty)?;
    let name_bytes = serialize_arg_with_layout(&name, &name_layout)?;
    Ok(FieldLocation {
        object_id,
        name_bytes,
    })
}

fn parse_value_spec(context: &mut NativeContext, value_ty: &Type) -> PartialVMResult<ValueSpec> {
    Ok(ValueSpec {
        layout: type_layout(context, value_ty)?,
        type_name: type_string(context, value_ty)?,
    })
}

fn parse_borrow_request(
    context: &mut NativeContext,
    ty_args: &[Type],
    arguments: &mut VecDeque<Value>,
) -> PartialVMResult<BorrowRequest> {
    let name = pop_name_arg(arguments)?;
    let uid_ref = pop_arg!(arguments, Reference);

    Ok(BorrowRequest {
        location: parse_field_location(context, &ty_args[0], uid_ref, name)?,
        value_spec: parse_value_spec(context, &ty_args[1])?,
    })
}

fn parse_exists_location(
    context: &mut NativeContext,
    name_ty: &Type,
    arguments: &mut VecDeque<Value>,
) -> PartialVMResult<FieldLocation> {
    let name = pop_name_arg(arguments)?;
    let uid_ref = pop_arg!(arguments, Reference);
    parse_field_location(context, name_ty, uid_ref, name)
}

fn load_from_resolver(context: &mut NativeContext, location: &FieldLocation) -> Option<Vec<u8>> {
    crate::native_ext::with_ext_mut_or_default::<DynamicFieldStorageExt, _>(context, |ext| {
        ext.get_dynamic_field(&location.object_id, &location.name_bytes)
    })
    .flatten()
}

fn field_exists(context: &mut NativeContext, location: &FieldLocation) -> bool {
    let mut cached = None;
    crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
        cached = ext.cached_exists(location);
    });

    cached.unwrap_or_else(|| load_from_resolver(context, location).is_some())
}

fn ensure_field_loaded(
    context: &mut NativeContext,
    location: &FieldLocation,
    value_spec: &ValueSpec,
) -> PartialVMResult<bool> {
    let mut cached = None;
    crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
        cached = ext.cached_exists(location);
    });
    if let Some(exists) = cached {
        return Ok(exists);
    }

    let Some(value_bytes) = load_from_resolver(context, location) else {
        return Ok(false);
    };
    let Some(value) = Value::simple_deserialize(&value_bytes, &value_spec.layout) else {
        return Ok(false);
    };

    crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
        ext.insert(CachedDynamicField::new(
            location.clone(),
            value_spec.clone(),
            value,
            value_bytes,
            true,
            DynamicFieldState::Clean,
        ));
    });
    Ok(true)
}

fn with_live_field<R>(
    context: &mut NativeContext,
    location: &FieldLocation,
    value_spec: &ValueSpec,
    on_hit: impl FnOnce(&mut CachedDynamicField) -> PartialVMResult<R>,
) -> PartialVMResult<Option<R>> {
    if !ensure_field_loaded(context, location, value_spec)? {
        return Ok(None);
    }

    let mut result = None;
    let mut error = None;
    crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
        if let Some(entry) = ext.get_mut(location)
            && entry.state != DynamicFieldState::Deleted
            && entry.value_spec.type_name == value_spec.type_name
        {
            match on_hit(entry) {
                Ok(value) => result = Some(value),
                Err(err) => error = Some(err),
            }
        }
    });

    if let Some(err) = error {
        return Err(err);
    }

    Ok(result)
}

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
    let name = pop_name_arg(&mut arguments)?;
    let uid_ref = pop_arg!(arguments, Reference);

    let location = parse_field_location(context, &ty_args[0], uid_ref, name)?;
    let value_spec = parse_value_spec(context, &ty_args[1])?;
    let value_bytes = serialize_arg_with_layout(&value, &value_spec.layout)?;

    if field_exists(context, &location) {
        return Ok(NR::err(context.gas_used(), E_FIELD_ALREADY_EXISTS));
    }

    crate::native_ext::with_ext_mut_or_default::<DynamicFieldsExt, _>(context, |ext| {
        ext.insert(CachedDynamicField::new(
            location,
            value_spec,
            value,
            value_bytes,
            false,
            DynamicFieldState::Dirty,
        ));
    });

    Ok(NR::ok(context.gas_used(), smallvec![]))
}

fn native_borrow_mut(
    gas_base: InternalGas,
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;

    native_charge_gas_early_exit!(context, gas_base);
    expect_native_signature(arguments.len(), 2, ty_args.len(), 2)?;

    let request = parse_borrow_request(context, &ty_args, &mut arguments)?;
    let borrowed = with_live_field(context, &request.location, &request.value_spec, |entry| {
        entry.state = DynamicFieldState::Dirty;
        entry.borrow_ref()
    })?;

    match borrowed {
        Some(value) => Ok(NR::ok(context.gas_used(), smallvec![value])),
        None => Ok(NR::err(context.gas_used(), E_FIELD_DOES_NOT_EXIST)),
    }
}

fn native_borrow(
    gas_base: InternalGas,
    context: &mut NativeContext,
    ty_args: Vec<Type>,
    mut arguments: VecDeque<Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;

    native_charge_gas_early_exit!(context, gas_base);
    expect_native_signature(arguments.len(), 2, ty_args.len(), 2)?;

    let request = parse_borrow_request(context, &ty_args, &mut arguments)?;
    let borrowed = with_live_field(context, &request.location, &request.value_spec, |entry| {
        entry.borrow_ref()
    })?;

    match borrowed {
        Some(value) => Ok(NR::ok(context.gas_used(), smallvec![value])),
        None => Ok(NR::err(context.gas_used(), E_FIELD_DOES_NOT_EXIST)),
    }
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

    let request = parse_borrow_request(context, &ty_args, &mut arguments)?;
    let removed = with_live_field(context, &request.location, &request.value_spec, |entry| {
        let value = entry.read_value()?;
        entry.state = DynamicFieldState::Deleted;
        Ok(value)
    })?;

    match removed {
        Some(value) => Ok(NR::ok(context.gas_used(), smallvec![value])),
        None => Ok(NR::err(context.gas_used(), E_FIELD_DOES_NOT_EXIST)),
    }
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

    let location = parse_exists_location(context, &ty_args[0], &mut arguments)?;
    Ok(NR::ok(
        context.gas_used(),
        smallvec![Value::bool(field_exists(context, &location))],
    ))
}
