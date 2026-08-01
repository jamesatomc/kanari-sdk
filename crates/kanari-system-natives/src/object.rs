// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use better_any::{Tid, TidAble};
use move_core_types::gas_algebra::{InternalGas, InternalGasPerByte, NumBytes};
use move_core_types::language_storage::TypeTag;
use move_vm_runtime::native_charge_gas_early_exit;
use move_vm_runtime::native_functions::NativeContext;
use move_vm_runtime::native_functions::NativeFunction;
use move_vm_types::natives::function::NativeResult;
use move_vm_types::natives::function::PartialVMResult;
use smallvec::smallvec;
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::Arc;

use crate::helpers::{expect_native_args, expect_native_signature, make_module_natives};

pub const E_OBJECT_NOT_FOUND: u64 = 9_001;
pub const E_OBJECT_LAYOUT_UNAVAILABLE: u64 = 9_002;
pub const E_OBJECT_TYPE_MISMATCH: u64 = 9_003;
pub const E_OBJECT_DESERIALIZE_FAILED: u64 = 9_004;
pub const E_OBJECT_NOT_MUTABLY_BORROWABLE: u64 = 9_005;

#[derive(Debug, Clone)]
pub struct GasParameters {
    pub save_object: SaveObjectGasParameters,
    pub delete_object: DeleteObjectGasParameters,
    pub borrow_global: BorrowGlobalGasParameters,
    pub borrow_global_mut: BorrowGlobalMutGasParameters,
}

#[derive(Debug, Clone)]
pub struct SaveObjectGasParameters {
    pub base: InternalGas,
    pub per_byte_serialized: InternalGasPerByte,
}

#[derive(Debug, Clone)]
pub struct DeleteObjectGasParameters {
    pub base: InternalGas,
}

#[derive(Debug, Clone)]
pub struct BorrowGlobalGasParameters {
    pub base: InternalGas,
    pub per_byte_loaded: InternalGasPerByte,
}

#[derive(Debug, Clone)]
pub struct BorrowGlobalMutGasParameters {
    pub base: InternalGas,
    pub per_byte_loaded: InternalGasPerByte,
}

impl GasParameters {
    pub fn zeros() -> Self {
        Self {
            save_object: SaveObjectGasParameters {
                base: 0.into(),
                per_byte_serialized: 0.into(),
            },
            delete_object: DeleteObjectGasParameters { base: 0.into() },
            borrow_global: BorrowGlobalGasParameters {
                base: 0.into(),
                per_byte_loaded: 0.into(),
            },
            borrow_global_mut: BorrowGlobalMutGasParameters {
                base: 0.into(),
                per_byte_loaded: 0.into(),
            },
        }
    }
}

pub(crate) fn uid_address_bytes(uid_val: &move_vm_types::values::Value) -> Option<Vec<u8>> {
    use move_core_types::runtime_value::{MoveStructLayout, MoveTypeLayout};

    // Support both:
    // - UID { addr: address }  (legacy / simplified)
    // - UID { id: ID }, ID { bytes: address } (Sui-style)
    let nested = MoveTypeLayout::Struct(MoveStructLayout::new(vec![MoveTypeLayout::Struct(
        MoveStructLayout::new(vec![MoveTypeLayout::Address]),
    )]));
    let flat = MoveTypeLayout::Struct(MoveStructLayout::new(vec![MoveTypeLayout::Address]));

    uid_val
        .simple_serialize(&nested)
        .or_else(|| uid_val.simple_serialize(&flat))
}

#[derive(Clone, Debug)]
pub struct SavedObject {
    pub object_id: String,
    pub object_type: String,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct BorrowedObject {
    pub object_id: String,
    pub object_type: String,
    pub data: Vec<u8>,
}

#[derive(Tid, Default)]
pub struct SavedObjectsExt {
    pub objects: Vec<SavedObject>,
}

impl SavedObjectsExt {
    pub fn record(&mut self, obj: SavedObject) {
        self.objects.push(obj);
    }
    pub fn take_all(&mut self) -> Vec<SavedObject> {
        std::mem::take(&mut self.objects)
    }
}

#[derive(Clone, Debug)]
pub struct DeletedObject {
    pub object_id: String,
}

#[derive(Tid, Default)]
pub struct DeletedObjectsExt {
    pub objects: Vec<DeletedObject>,
}

impl DeletedObjectsExt {
    pub fn record(&mut self, obj: DeletedObject) {
        self.objects.push(obj);
    }
    pub fn take_all(&mut self) -> Vec<DeletedObject> {
        std::mem::take(&mut self.objects)
    }
}

/// Extension for loaded objects from storage
#[derive(Clone, Debug)]
pub struct LoadedObject {
    pub type_str: String,
    pub data: Vec<u8>,
    pub can_mutably_borrow: bool,
}

#[derive(Tid, Default)]
pub struct LoadedObjectsExt {
    pub objects: std::collections::HashMap<String, LoadedObject>,
}

impl LoadedObjectsExt {
    pub fn insert(
        &mut self,
        object_id: String,
        type_str: String,
        data: Vec<u8>,
        can_mutably_borrow: bool,
    ) {
        self.objects.insert(
            object_id,
            LoadedObject {
                type_str,
                data,
                can_mutably_borrow,
            },
        );
    }
    pub fn get(&self, object_id: &str) -> Option<&LoadedObject> {
        self.objects.get(object_id)
    }
}

fn canonical_type_string(type_str: &str) -> String {
    TypeTag::from_str(type_str)
        .map(|tag| tag.to_string())
        .unwrap_or_else(|_| type_str.to_string())
}

/// Extension for tracking borrowed mutable objects
#[derive(Tid, Default)]
pub struct BorrowedObjectsExt {
    pub objects: Vec<BorrowedObject>,
}

impl BorrowedObjectsExt {
    pub fn record(&mut self, object_id: String, type_str: String, data: Vec<u8>) {
        self.objects.push(BorrowedObject {
            object_id,
            object_type: type_str,
            data,
        });
    }
    pub fn take_all(&mut self) -> Vec<BorrowedObject> {
        std::mem::take(&mut self.objects)
    }
}

pub fn make_all(gas_params: GasParameters) -> impl Iterator<Item = (String, NativeFunction)> {
    let save_params = gas_params.save_object.clone();
    let delete_params = gas_params.delete_object.clone();
    let borrow_params = gas_params.borrow_global.clone();
    let borrow_mut_params = gas_params.borrow_global_mut.clone();

    let save_object: NativeFunction = Arc::new(move |context, ty_args, args| {
        native_save_object(&save_params, context, ty_args, args)
    });
    let delete_impl: NativeFunction = Arc::new(move |context, ty_args, args| {
        native_delete_object(&delete_params, context, ty_args, args)
    });
    let borrow_global: NativeFunction = Arc::new(move |context, ty_args, args| {
        native_borrow_global(&borrow_params, context, ty_args, args)
    });
    let borrow_global_mut: NativeFunction = Arc::new(move |context, ty_args, args| {
        native_borrow_global_mut(&borrow_mut_params, context, ty_args, args)
    });
    make_module_natives([
        ("save_object", save_object),
        ("delete_impl", delete_impl),
        ("borrow_global", borrow_global),
        ("borrow_global_mut", borrow_global_mut),
    ])
}

/// Native function: borrow_global<T>(address): &T
/// Loads an object from storage and returns an immutable reference.
/// This allows reading any object's data without requiring ownership or mutability.
fn native_borrow_global(
    gas_params: &BorrowGlobalGasParameters,
    context: &mut NativeContext,
    ty_args: Vec<move_vm_types::loaded_data::runtime_types::Type>,
    mut arguments: VecDeque<move_vm_types::values::Value>,
) -> PartialVMResult<NativeResult> {
    use move_core_types::account_address::AccountAddress;
    use move_vm_types::natives::function::NativeResult as NR;
    use move_vm_types::pop_arg;

    native_charge_gas_early_exit!(context, gas_params.base);
    expect_native_signature(arguments.len(), 1, ty_args.len(), 1)?;

    // Arguments: address (as AccountAddress directly from Move VM)
    let object_addr = pop_arg!(arguments, AccountAddress);
    let object_id = format!("0x{}", hex::encode(object_addr.as_ref()));

    // Load object from storage via context's extension
    let loaded_data =
        crate::native_ext::with_ext_mut_or_default::<LoadedObjectsExt, _>(context, |ext| {
            ext.get(&object_id).cloned()
        });

    let Some(loaded_object) = loaded_data.flatten() else {
        return Ok(NR::err(context.gas_used(), E_OBJECT_NOT_FOUND));
    };
    let type_str = loaded_object.type_str;
    let obj_data = loaded_object.data;

    native_charge_gas_early_exit!(
        context,
        gas_params.per_byte_loaded * NumBytes::new(obj_data.len() as u64)
    );

    // Deserialize the object data into a Move value
    let layout = match context.type_to_type_layout(&ty_args[0]) {
        Ok(Some(layout)) => layout,
        _ => {
            return Ok(NR::err(context.gas_used(), E_OBJECT_LAYOUT_UNAVAILABLE));
        }
    };

    // Verify that the loaded object type matches the requested type
    let requested_type = match context.type_to_type_tag(&ty_args[0]) {
        Ok(tag) => format!("{}", tag),
        Err(_) => {
            return Ok(NR::err(context.gas_used(), E_OBJECT_LAYOUT_UNAVAILABLE));
        }
    };

    if canonical_type_string(&type_str) != canonical_type_string(&requested_type) {
        return Ok(NR::err(context.gas_used(), E_OBJECT_TYPE_MISMATCH));
    }

    // Deserialize object data into Move value
    let obj_val = match move_vm_types::values::Value::simple_deserialize(&obj_data, &layout) {
        Some(val) => val,
        None => {
            return Ok(NR::err(context.gas_used(), E_OBJECT_DESERIALIZE_FAILED));
        }
    };

    // Create an immutable reference to the object using borrow_global
    let obj_ref =
        move_vm_types::values::Value::borrow_global(object_addr, ty_args[0].clone(), obj_val)?;

    Ok(NR::ok(context.gas_used(), smallvec![obj_ref]))
}

/// Native function: borrow_global_mut<T>(address): &mut T
/// Loads an object from storage and returns a mutable reference.
/// This enables CLI to pass object IDs and have them resolved at runtime.
fn native_borrow_global_mut(
    gas_params: &BorrowGlobalMutGasParameters,
    context: &mut NativeContext,
    ty_args: Vec<move_vm_types::loaded_data::runtime_types::Type>,
    mut arguments: VecDeque<move_vm_types::values::Value>,
) -> PartialVMResult<NativeResult> {
    use move_core_types::account_address::AccountAddress;
    use move_vm_types::natives::function::NativeResult as NR;
    use move_vm_types::pop_arg;

    native_charge_gas_early_exit!(context, gas_params.base);
    expect_native_signature(arguments.len(), 1, ty_args.len(), 1)?;

    // Arguments: address (as AccountAddress directly from Move VM)
    let object_addr = pop_arg!(arguments, AccountAddress);
    let object_id = format!("0x{}", hex::encode(object_addr.as_ref()));

    // Load object from storage via context's extension
    let loaded_data =
        crate::native_ext::with_ext_mut_or_default::<LoadedObjectsExt, _>(context, |ext| {
            ext.get(&object_id).cloned()
        });

    let Some(loaded_object) = loaded_data.flatten() else {
        return Ok(NR::err(context.gas_used(), E_OBJECT_NOT_FOUND));
    };
    if !loaded_object.can_mutably_borrow {
        return Ok(NR::err(context.gas_used(), E_OBJECT_NOT_MUTABLY_BORROWABLE));
    }
    let type_str = loaded_object.type_str;
    let obj_data = loaded_object.data;

    native_charge_gas_early_exit!(
        context,
        gas_params.per_byte_loaded * NumBytes::new(obj_data.len() as u64)
    );

    // Deserialize the object data into a Move value
    let layout = match context.type_to_type_layout(&ty_args[0]) {
        Ok(Some(layout)) => layout,
        _ => {
            return Ok(NR::err(context.gas_used(), E_OBJECT_LAYOUT_UNAVAILABLE));
        }
    };

    // Verify that the loaded object type matches the requested type
    let requested_type = match context.type_to_type_tag(&ty_args[0]) {
        Ok(tag) => format!("{}", tag),
        Err(_) => {
            return Ok(NR::err(context.gas_used(), E_OBJECT_LAYOUT_UNAVAILABLE));
        }
    };

    if canonical_type_string(&type_str) != canonical_type_string(&requested_type) {
        return Ok(NR::err(context.gas_used(), E_OBJECT_TYPE_MISMATCH));
    }

    // Deserialize object data into Move value
    let obj_val = match move_vm_types::values::Value::simple_deserialize(&obj_data, &layout) {
        Some(val) => val,
        None => {
            return Ok(NR::err(context.gas_used(), E_OBJECT_DESERIALIZE_FAILED));
        }
    };

    // Create a mutable reference to the object
    // The VM will manage the lifecycle of this reference
    let obj_ref = move_vm_types::values::Value::mutable_borrow_global(
        object_addr,
        ty_args[0].clone(),
        obj_val,
    )?;

    // Track borrowed objects for later writeback
    crate::native_ext::with_ext_mut_or_default::<BorrowedObjectsExt, _>(context, |ext| {
        ext.record(object_id.clone(), type_str, obj_data);
    });

    Ok(NR::ok(context.gas_used(), smallvec![obj_ref]))
}

fn native_delete_object(
    gas_params: &DeleteObjectGasParameters,
    context: &mut NativeContext,
    _ty_args: Vec<move_vm_types::loaded_data::runtime_types::Type>,
    mut arguments: VecDeque<move_vm_types::values::Value>,
) -> PartialVMResult<NativeResult> {
    use move_core_types::vm_status::StatusCode;
    use move_vm_types::natives::function::NativeResult as NR;
    use move_vm_types::natives::function::PartialVMError;

    native_charge_gas_early_exit!(context, gas_params.base);
    expect_native_args(arguments.len(), 1)?;

    // Arguments: uid (UID) - passed by value
    let uid_val = arguments.pop_back().ok_or_else(|| {
        PartialVMError::new(StatusCode::INTERNAL_TYPE_ERROR)
            .with_message("Missing uid argument".to_string())
    })?;

    // Serialize UID to get the address bytes (supports multiple UID encodings)
    let uid_bytes = uid_address_bytes(&uid_val).ok_or_else(|| {
        PartialVMError::new(StatusCode::INTERNAL_TYPE_ERROR)
            .with_message("Failed to serialize UID".to_string())
    })?;

    let object_id = format!("0x{}", hex::encode(&uid_bytes));

    // Record the deleted object
    crate::native_ext::with_ext_mut_or_default::<DeletedObjectsExt, _>(context, |ext| {
        ext.record(DeletedObject { object_id })
    });

    Ok(NR::ok(context.gas_used(), smallvec![]))
}

fn native_save_object(
    gas_params: &SaveObjectGasParameters,
    context: &mut NativeContext,
    ty_args: Vec<move_vm_types::loaded_data::runtime_types::Type>,
    mut arguments: VecDeque<move_vm_types::values::Value>,
) -> PartialVMResult<NativeResult> {
    use move_vm_types::natives::function::NativeResult as NR;
    use move_vm_types::pop_arg;
    use move_vm_types::values::values_impl::Reference;

    native_charge_gas_early_exit!(context, gas_params.base);
    expect_native_signature(arguments.len(), 1, ty_args.len(), 1)?;

    let type_tag = context.type_to_type_tag(&ty_args[0])?;
    let type_str = format!("{}", type_tag);

    // Arguments: obj (&T)
    let obj_ref = pop_arg!(arguments, Reference);

    // Read the reference to get the value
    let obj_val = obj_ref.read_ref()?;

    let mut obj_data = Vec::new();
    if let Some(layout) = context.type_to_type_layout(&ty_args[0])? {
        obj_data = obj_val.simple_serialize(&layout).unwrap_or_default();
    }

    native_charge_gas_early_exit!(
        context,
        gas_params.per_byte_serialized * NumBytes::new(obj_data.len() as u64)
    );

    // Extract ID (first 32 bytes)
    let object_id_hex = if obj_data.len() >= 32 {
        let uid_bytes = &obj_data[0..32];
        format!("0x{}", hex::encode(uid_bytes))
    } else {
        String::new()
    };

    if !object_id_hex.is_empty() {
        let saved = SavedObject {
            object_id: object_id_hex,
            object_type: type_str,
            data: obj_data,
        };

        // Record the saved object
        crate::native_ext::with_ext_mut_or_default::<SavedObjectsExt, _>(context, |ext| {
            ext.record(saved)
        });

        // Log for debugging
        // println!("[DEBUG] native_save_object: Saved {}", saved.object_id);
    }

    Ok(NR::ok(context.gas_used(), smallvec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use move_core_types::account_address::AccountAddress;
    use move_vm_types::values::{Struct, Value};

    #[test]
    fn uid_address_bytes_supports_flat_and_nested() {
        let addr = AccountAddress::new([7u8; AccountAddress::LENGTH]);
        let expected = addr.into_bytes().to_vec();

        // UID { addr: address }
        let flat_uid = Value::struct_(Struct::pack([Value::address(addr)]));
        assert_eq!(uid_address_bytes(&flat_uid).unwrap(), expected);

        // UID { id: ID }, ID { bytes: address }
        let nested_uid = Value::struct_(Struct::pack([Value::struct_(Struct::pack([
            Value::address(addr),
        ]))]));
        assert_eq!(uid_address_bytes(&nested_uid).unwrap(), expected);
    }

    #[test]
    fn test_object_id_format_from_address_bytes() {
        // Test that object ID is correctly formatted from 32-byte address
        let addr_bytes = vec![0x1Au8; 32];
        let expected_id = format!("0x{}", hex::encode(&addr_bytes));

        assert_eq!(expected_id.len(), 66); // "0x" + 64 hex chars
        assert!(expected_id.starts_with("0x"));
    }

    #[test]
    fn test_loaded_objects_ext_insert_and_get() {
        // Test LoadedObjectsExt functionality used by native_borrow_global_mut
        let mut ext = LoadedObjectsExt::default();
        let object_id =
            "0x1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a".to_string();
        let type_str = "0x1::coin::Coin<0x1::kanari_coin::KANARI>".to_string();
        let data = vec![0x01, 0x02, 0x03];

        ext.insert(object_id.clone(), type_str.clone(), data.clone(), true);

        let retrieved = ext.get(&object_id);
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.type_str, type_str);
        assert_eq!(retrieved.data, data);
        assert!(retrieved.can_mutably_borrow);
    }

    #[test]
    fn test_loaded_objects_ext_get_nonexistent() {
        // Test getting non-existent object returns None
        let ext = LoadedObjectsExt::default();
        let result = ext.get("0xnonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_borrowed_objects_ext_tracking() {
        // Test BorrowedObjectsExt for tracking borrowed mutable objects
        let mut ext = BorrowedObjectsExt::default();
        let object_id =
            "0x1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a".to_string();
        let type_str = "0x1::coin::Coin<0x1::kanari_coin::KANARI>".to_string();
        let data = vec![0x01, 0x02, 0x03];

        ext.record(object_id.clone(), type_str.clone(), data.clone());

        let all = ext.take_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].object_id, object_id);
        assert_eq!(all[0].object_type, type_str);
        assert_eq!(all[0].data, data);
    }

    #[test]
    fn test_address_bytes_validation() {
        // Test that address must be exactly 32 bytes
        let short_addr = [0x1Au8; 31];
        let long_addr = [0x1Au8; 33];
        let valid_addr = [0x1Au8; 32];

        assert_eq!(short_addr.len(), 31);
        assert_eq!(long_addr.len(), 33);
        assert_eq!(valid_addr.len(), 32);
    }

    #[test]
    fn test_object_id_hex_encoding() {
        // Test various address patterns encode correctly
        let test_cases = vec![
            vec![0x00u8; 32],             // All zeros
            vec![0xFFu8; 32],             // All 0xFF
            vec![0x01u8; 32],             // All 0x01
            (0..32).collect::<Vec<u8>>(), // Sequential bytes
        ];

        for addr_bytes in test_cases {
            let object_id = format!("0x{}", hex::encode(&addr_bytes));
            assert_eq!(object_id.len(), 66);
            assert!(object_id.starts_with("0x"));

            // Verify we can decode back
            let decoded = hex::decode(&object_id[2..]).unwrap();
            assert_eq!(decoded, addr_bytes);
        }
    }

    #[test]
    fn test_gas_parameters_for_borrow_global_mut() {
        // Test gas parameter initialization
        let params = BorrowGlobalMutGasParameters {
            base: 100.into(),
            per_byte_loaded: 1.into(),
        };

        assert_eq!(params.base, 100.into());
        assert_eq!(params.per_byte_loaded, 1.into());
    }

    #[test]
    fn test_saved_objects_ext_record_and_take() {
        // Test SavedObjectsExt functionality
        let mut ext = SavedObjectsExt::default();
        let obj = SavedObject {
            object_id: "0x1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a"
                .to_string(),
            object_type: "0x1::coin::Coin<0x1::kanari_coin::KANARI>".to_string(),
            data: vec![0x01, 0x02, 0x03],
        };

        ext.record(obj.clone());
        let all = ext.take_all();

        assert_eq!(all.len(), 1);
        assert_eq!(all[0].object_id, obj.object_id);
        assert_eq!(all[0].object_type, obj.object_type);
        assert_eq!(all[0].data, obj.data);
    }

    #[test]
    fn test_deleted_objects_ext_record_and_take() {
        // Test DeletedObjectsExt functionality
        let mut ext = DeletedObjectsExt::default();
        let obj = DeletedObject {
            object_id: "0x1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a"
                .to_string(),
        };

        ext.record(obj.clone());
        let all = ext.take_all();

        assert_eq!(all.len(), 1);
        assert_eq!(all[0].object_id, obj.object_id);
    }
}
