use kanari_move_runtime_v1::move_runtime::MoveRuntime;
use kanari_types::error::KanariUnwrapExt;
use move_binary_format::file_format::{CompiledModule, IdentifierIndex, StructFieldInformation};
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::Identifier;

#[test]
fn publish_module_upgrade_preserves_storage() {
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

    runtime
        .publish_module(module_bytes.clone(), *module_id.address(), None, None)
        .invariant("publish (upgrade) should succeed");

    // Ensure storage contains the module bytes after publish
    let stored = runtime
        .get_module_bytes(&module_id)
        .invariant("module bytes present after publish");
    assert_eq!(stored, module_bytes);

    // Confirm module_id still present in listed modules
    let listed = runtime.list_modules();
    assert!(listed.iter().any(|m| m == &module_id));

    runtime
        .publish_module(module_bytes.clone(), *module_id.address(), None, None)
        .invariant("second publish should succeed");

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

    let replacement_name =
        Identifier::new("compatibility_breaking_field_name").invariant("valid identifier");
    let replacement_idx = IdentifierIndex(upgraded.identifiers.len() as u16);
    upgraded.identifiers.push(replacement_name);
    for struct_def in &mut upgraded.struct_defs {
        if let StructFieldInformation::Declared(fields) = &mut struct_def.field_information
            && let Some(field) = fields.first_mut()
        {
            field.name = replacement_idx;
            break;
        }
    }

    let mut upgraded_bytes = Vec::new();
    upgraded
        .serialize(&mut upgraded_bytes)
        .invariant("serialize upgraded module");

    let err = runtime
        .publish_module(upgraded_bytes, *module_id.address(), None, None)
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
