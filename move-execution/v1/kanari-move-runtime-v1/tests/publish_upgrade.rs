use kanari_move_runtime_v1::move_runtime::MoveRuntime;
use kanari_types::error::KanariUnwrapExt;
use move_binary_format::file_format::{CompiledModule, IdentifierIndex, StructFieldInformation};
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::Identifier;
use move_core_types::metadata::Metadata;
use move_core_types::runtime_value::MoveValue;

#[test]
fn module_upgrade_preserves_storage_and_publish_rejects_existing_module() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let modules = runtime.list_modules();
    assert!(!modules.is_empty(), "expected at least one system module");

    // Find a module which has persisted bytes in the runtime state
    let maybe = modules
        .into_iter()
        .find_map(|m| runtime.get_module_bytes(&m).map(|b| (m, b)));
    let (module_id, module_bytes) = match maybe {
        Some(pair) => pair,
        None => panic!("no preloaded module had persisted bytes in state"),
    };

    let publish_err = runtime
        .publish_module(module_bytes.clone(), *module_id.address(), None, None)
        .expect_err("publish should reject an existing module");
    assert!(
        publish_err.to_string().contains("already exists"),
        "unexpected error: {publish_err}"
    );

    runtime
        .upgrade_module(module_bytes.clone(), *module_id.address(), None, None)
        .invariant("upgrade should succeed");

    // Ensure storage contains the module bytes after publish
    let stored = runtime
        .get_module_bytes(&module_id)
        .invariant("module bytes present after publish");
    assert_eq!(stored, module_bytes);

    // Confirm module_id still present in listed modules
    let listed = runtime.list_modules();
    assert!(listed.iter().any(|m| m == &module_id));

    runtime
        .upgrade_module(module_bytes.clone(), *module_id.address(), None, None)
        .invariant("second upgrade should succeed");

    let stored2 = runtime
        .get_module_bytes(&module_id)
        .invariant("module bytes present after second publish");
    assert_eq!(stored2, module_bytes);
}

#[test]
fn incompatible_module_upgrade_is_rejected_before_storage_changes() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");

    let (module_id, module_bytes, mut upgraded) = runtime
        .list_modules()
        .into_iter()
        .filter_map(|m| {
            let bytes = runtime.get_module_bytes(&m)?;
            let compiled = CompiledModule::deserialize_with_defaults(&bytes).ok()?;
            Some((m, bytes, compiled))
        })
        .find(|(_, _, compiled)| {
            compiled.struct_defs.iter().any(|s| {
                matches!(
                    &s.field_information,
                    StructFieldInformation::Declared(fields) if !fields.is_empty()
                )
            })
        })
        .invariant("expected a module with declared struct fields");

    break_first_declared_struct_field_name(&mut upgraded);

    let upgraded_bytes = serialize_module(&upgraded);

    let err = runtime
        .upgrade_module(upgraded_bytes, *module_id.address(), None, None)
        .expect_err("incompatible upgrade should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("Incompatible module upgrade"),
        "unexpected error: {msg}"
    );

    let stored = runtime
        .get_module_bytes(&module_id)
        .invariant("module bytes should remain present");
    assert_eq!(stored, module_bytes);
}

#[test]
fn compatible_module_upgrade_can_change_constant_value() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");

    let (module_id, module_bytes, mut upgraded) = runtime
        .list_modules()
        .into_iter()
        .filter_map(|m| {
            let bytes = runtime.get_module_bytes(&m)?;
            let compiled = CompiledModule::deserialize_with_defaults(&bytes).ok()?;
            Some((m, bytes, compiled))
        })
        .find(|(_, _, compiled)| {
            let mut clone = compiled.clone();
            mutate_first_constant_value(&mut clone)
        })
        .invariant("expected a module with a mutatable constant value");

    assert!(
        mutate_first_constant_value(&mut upgraded),
        "constant mutation should remain available for selected module"
    );

    let upgraded_bytes = serialize_module(&upgraded);
    assert_ne!(
        upgraded_bytes, module_bytes,
        "constant upgrade should change the persisted module bytes"
    );

    runtime
        .upgrade_module(upgraded_bytes.clone(), *module_id.address(), None, None)
        .invariant("constant-only compatible upgrade should succeed");

    let stored = runtime
        .get_module_bytes(&module_id)
        .invariant("upgraded module bytes should remain present");
    assert_eq!(stored, upgraded_bytes);
}

#[test]
fn compatible_module_upgrade_can_change_metadata() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes, mut upgraded) = first_published_module(&runtime);

    upgraded.metadata.push(Metadata {
        key: b"kanari:test:upgrade-metadata".to_vec(),
        value: b"v2".to_vec(),
    });

    let upgraded_bytes = serialize_module(&upgraded);
    assert_ne!(
        upgraded_bytes, module_bytes,
        "metadata upgrade should change the persisted module bytes"
    );

    runtime
        .upgrade_module(upgraded_bytes.clone(), *module_id.address(), None, None)
        .invariant("metadata-only compatible upgrade should succeed");

    let stored = runtime
        .get_module_bytes(&module_id)
        .invariant("upgraded module bytes should remain present");
    assert_eq!(stored, upgraded_bytes);
}

#[test]
fn invalid_module_upgrade_bytes_are_rejected_before_storage_changes() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes, _) = first_published_module(&runtime);

    let mut invalid_bytes = module_bytes.clone();
    invalid_bytes.truncate(invalid_bytes.len().saturating_sub(1));

    runtime
        .upgrade_module(invalid_bytes, *module_id.address(), None, None)
        .expect_err("truncated bytecode should be rejected");
    assert_module_storage_unchanged(&runtime, &module_id, &module_bytes);
}

#[test]
fn module_upgrade_rejects_missing_module_before_storage_changes() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (missing_sender, missing_module_id, missing_module_bytes) =
        missing_module_bytecode(&runtime, "0x42");
    assert!(
        runtime.get_module_bytes(&missing_module_id).is_none(),
        "test setup must target a missing module id"
    );

    let err = runtime
        .upgrade_module(missing_module_bytes, missing_sender, None, None)
        .expect_err("upgrade must reject a missing module");
    let msg = err.to_string();
    assert!(
        msg.contains("Module upgrade rejected") && msg.contains("does not exist"),
        "unexpected error: {msg}"
    );
}

#[test]
fn package_upgrade_rejects_missing_module_before_storage_changes() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (missing_sender, missing_module_id, missing_module_bytes) =
        missing_module_bytecode(&runtime, "0x43");

    let err = runtime
        .upgrade_package_with_context_and_persistence(
            vec![(
                missing_module_id.name().to_string(),
                missing_module_bytes.clone(),
            )],
            missing_sender,
            None,
            None,
            None,
            true,
        )
        .expect_err("package upgrade must reject a missing module");
    let msg = err.to_string();
    assert!(
        msg.contains("Module upgrade rejected") && msg.contains("does not exist"),
        "unexpected error: {msg}"
    );
    assert!(
        runtime.get_module_bytes(&missing_module_id).is_none(),
        "missing package upgrade must not create module storage"
    );
}

#[test]
fn package_publish_rejects_existing_module_before_storage_changes() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes, _) = first_published_module(&runtime);

    let err = runtime
        .publish_package_with_context_and_persistence(
            vec![(module_id.name().to_string(), module_bytes.clone())],
            *module_id.address(),
            None,
            None,
            None,
            true,
        )
        .expect_err("package publish must reject an existing module");
    let msg = err.to_string();
    assert!(
        msg.contains("Module publish rejected") && msg.contains("already exists"),
        "unexpected error: {msg}"
    );
    assert_module_storage_unchanged(&runtime, &module_id, &module_bytes);
}

#[test]
fn package_upgrade_existing_module_preserves_storage() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes, _) = first_published_module(&runtime);

    runtime
        .upgrade_package_with_context_and_persistence(
            vec![(module_id.name().to_string(), module_bytes.clone())],
            *module_id.address(),
            None,
            None,
            None,
            true,
        )
        .invariant("package upgrade should accept an existing compatible module");
    assert_module_storage_unchanged(&runtime, &module_id, &module_bytes);
}

#[test]
fn package_upgrade_can_change_metadata() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes, mut upgraded) = first_published_module(&runtime);
    upgraded.metadata.push(Metadata {
        key: b"kanari:test:package-upgrade-metadata".to_vec(),
        value: b"v2".to_vec(),
    });
    let upgraded_bytes = serialize_module(&upgraded);
    assert_ne!(
        upgraded_bytes, module_bytes,
        "package metadata upgrade should change module bytes"
    );

    runtime
        .upgrade_package_with_context_and_persistence(
            vec![(module_id.name().to_string(), upgraded_bytes.clone())],
            *module_id.address(),
            None,
            None,
            None,
            true,
        )
        .invariant("package metadata-only upgrade should succeed");
    assert_module_storage_unchanged(&runtime, &module_id, &upgraded_bytes);
}

#[test]
fn bootstrap_module_allows_existing_framework_module_idempotently() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes, _) = first_published_module(&runtime);

    runtime
        .bootstrap_module_with_context_and_persistence(
            module_bytes.clone(),
            *module_id.address(),
            None,
            None,
            None,
            true,
        )
        .invariant("bootstrap publish should be idempotent for framework modules");
    assert_module_storage_unchanged(&runtime, &module_id, &module_bytes);
}

#[test]
fn bootstrap_module_rejects_incompatible_existing_framework_module() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes, mut upgraded) = runtime
        .list_modules()
        .into_iter()
        .filter_map(|m| {
            let bytes = runtime.get_module_bytes(&m)?;
            let compiled = CompiledModule::deserialize_with_defaults(&bytes).ok()?;
            Some((m, bytes, compiled))
        })
        .find(|(_, _, compiled)| {
            compiled.struct_defs.iter().any(|s| {
                matches!(
                    &s.field_information,
                    StructFieldInformation::Declared(fields) if !fields.is_empty()
                )
            })
        })
        .invariant("expected a module with declared struct fields");
    break_first_declared_struct_field_name(&mut upgraded);
    let upgraded_bytes = serialize_module(&upgraded);

    let err = runtime
        .bootstrap_module_with_context_and_persistence(
            upgraded_bytes,
            *module_id.address(),
            None,
            None,
            None,
            true,
        )
        .expect_err("bootstrap must reject incompatible replacement bytes");
    let msg = err.to_string();
    assert!(
        msg.contains("Incompatible module upgrade"),
        "unexpected error: {msg}"
    );
    assert_module_storage_unchanged(&runtime, &module_id, &module_bytes);
}

#[test]
fn package_publish_rejects_declared_name_mismatch_before_storage_changes() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes, _) = first_published_module(&runtime);

    let err = runtime
        .publish_package_with_context_and_persistence(
            vec![("wrong_declared_name".to_string(), module_bytes.clone())],
            *module_id.address(),
            None,
            None,
            None,
            true,
        )
        .expect_err("package publish should reject declared-name mismatch");
    let msg = err.to_string();
    assert!(
        msg.contains("declared module name") && msg.contains("does not match bytecode self id"),
        "unexpected error: {msg}"
    );
    assert_module_storage_unchanged(&runtime, &module_id, &module_bytes);
}

#[test]
fn module_publish_requires_sender_to_match_module_address() {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory().invariant("init runtime");
    let (module_id, module_bytes) = runtime
        .list_modules()
        .into_iter()
        .find_map(|m| runtime.get_module_bytes(&m).map(|b| (m, b)))
        .invariant("expected a published module");

    let wrong_sender =
        AccountAddress::from_hex_literal("0x42").invariant("valid account address literal");
    assert_ne!(&wrong_sender, module_id.address());

    let err = runtime
        .publish_module(module_bytes, wrong_sender, None, None)
        .expect_err("sender/module address mismatch should be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("sender") && msg.contains("Module address must match"),
        "unexpected error: {msg}"
    );
}

fn first_published_module(
    runtime: &MoveRuntime,
) -> (
    move_core_types::language_storage::ModuleId,
    Vec<u8>,
    CompiledModule,
) {
    runtime
        .list_modules()
        .into_iter()
        .find_map(|m| {
            let bytes = runtime.get_module_bytes(&m)?;
            let compiled = CompiledModule::deserialize_with_defaults(&bytes).ok()?;
            Some((m, bytes, compiled))
        })
        .invariant("expected a published module")
}

fn assert_module_storage_unchanged(
    runtime: &MoveRuntime,
    module_id: &move_core_types::language_storage::ModuleId,
    expected_bytes: &[u8],
) {
    let stored = runtime
        .get_module_bytes(module_id)
        .invariant("module bytes should remain present");
    assert_eq!(stored, expected_bytes);
}

fn missing_module_bytecode(
    runtime: &MoveRuntime,
    address: &str,
) -> (
    AccountAddress,
    move_core_types::language_storage::ModuleId,
    Vec<u8>,
) {
    let (_, _, mut compiled) = first_published_module(runtime);
    let missing_sender = AccountAddress::from_hex_literal(address).invariant("valid account");
    let self_handle_idx = compiled.self_module_handle_idx.0 as usize;
    let address_idx = compiled.module_handles[self_handle_idx].address.0 as usize;
    compiled.address_identifiers[address_idx] = missing_sender;
    let missing_module_id = compiled.self_id();
    let missing_module_bytes = serialize_module(&compiled);
    (missing_sender, missing_module_id, missing_module_bytes)
}

fn serialize_module(module: &CompiledModule) -> Vec<u8> {
    let mut bytes = Vec::new();
    module.serialize(&mut bytes).invariant("serialize module");
    bytes
}

fn break_first_declared_struct_field_name(module: &mut CompiledModule) {
    let replacement_name =
        Identifier::new("compatibility_breaking_field_name").invariant("valid identifier");
    let replacement_idx = IdentifierIndex(module.identifiers.len() as u16);
    module.identifiers.push(replacement_name);
    for struct_def in &mut module.struct_defs {
        if let StructFieldInformation::Declared(fields) = &mut struct_def.field_information
            && let Some(field) = fields.first_mut()
        {
            field.name = replacement_idx;
            return;
        }
    }
    panic!("test setup expected a declared struct field");
}

fn mutate_first_constant_value(module: &mut CompiledModule) -> bool {
    for index in 0..module.constant_pool.len() {
        let Some(value) = module.constant_pool[index].deserialize_constant() else {
            continue;
        };
        let Some(mutated) = mutate_move_value(value) else {
            continue;
        };
        let Some(data) = mutated.simple_serialize() else {
            continue;
        };
        if data != module.constant_pool[index].data {
            let old_data = std::mem::replace(&mut module.constant_pool[index].data, data);
            if constant_pool_has_duplicates(module) {
                module.constant_pool[index].data = old_data;
                continue;
            }
            return true;
        }
    }
    false
}

fn constant_pool_has_duplicates(module: &CompiledModule) -> bool {
    for (index, constant) in module.constant_pool.iter().enumerate() {
        if module.constant_pool[(index + 1)..]
            .iter()
            .any(|other| other == constant)
        {
            return true;
        }
    }
    false
}

fn mutate_move_value(value: MoveValue) -> Option<MoveValue> {
    match value {
        MoveValue::Bool(value) => Some(MoveValue::Bool(!value)),
        MoveValue::U8(value) => Some(MoveValue::U8(value.wrapping_add(1))),
        MoveValue::U16(value) => Some(MoveValue::U16(value.wrapping_add(1))),
        MoveValue::U32(value) => Some(MoveValue::U32(value.wrapping_add(1))),
        MoveValue::U64(value) => Some(MoveValue::U64(value.wrapping_add(1))),
        MoveValue::U128(value) => Some(MoveValue::U128(value.wrapping_add(1))),
        MoveValue::Vector(values) => mutate_vector_value(values),
        MoveValue::Address(_)
        | MoveValue::Signer(_)
        | MoveValue::Struct(_)
        | MoveValue::U256(_) => None,
    }
}

fn mutate_vector_value(mut values: Vec<MoveValue>) -> Option<MoveValue> {
    for value in &mut values {
        let Some(mutated) = mutate_move_value(value.clone()) else {
            continue;
        };
        *value = mutated;
        return Some(MoveValue::Vector(values));
    }
    None
}
