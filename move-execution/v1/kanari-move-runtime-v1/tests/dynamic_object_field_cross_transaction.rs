use anyhow::{Context, Result};
use kanari_move_runtime_v1::move_runtime::EntryFunctionObjectContext;
use kanari_move_runtime_v1::move_runtime::MoveRuntime;
use kanari_move_runtime_v1::state::StateManager;
use kanari_move_runtime_v1::storage::persistent_store::PersistentStore;
use kanari_types::transaction::{ObjectInput, ObjectOwnerKind, ObjectRef};
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::ModuleId;
use move_package::BuildConfig;
use serde_json::Value as JsonValue;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;

#[path = "support/move_manifest.rs"]
mod move_manifest;

const TESTER_ADDR: &str = "0x43";
const MODULE_NAME: &str = "dynamic_object_field_e2e";

#[test]
fn dynamic_object_field_persists_across_runtime_instances() -> Result<()> {
    let package_dir = create_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;
    let store = Arc::new(PersistentStore::open_in_memory()?);

    let mut state = StateManager::new(store.clone());
    let runtime = MoveRuntime::new_with_kanari_natives_and_store(store.clone())?;
    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish dynamic_object_field_e2e module")?;

    let create_host = runtime
        .execute_entry_function(
            &module_id,
            "create_host",
            vec![],
            vec![],
            Some(*module_id.address()),
            None,
            None,
        )
        .context("create host object")?;

    let host_id = create_host
        .created_objects
        .iter()
        .find(|(_, created)| created.type_.ends_with("::dynamic_object_field_e2e::Host"))
        .map(|(id, _)| id.clone())
        .context("host object should be created")?;

    let add_changes = runtime
        .execute_entry_function_with_object_context_and_persistence(
            &module_id,
            "add_child",
            vec![],
            vec![
                object_arg(&host_id)?,
                bcs::to_bytes(&3u64)?,
                bcs::to_bytes(&500u64)?,
            ],
            EntryFunctionObjectContext {
                object_inputs: vec![host_object_input(&host_id, *module_id.address(), true)],
                sender: Some(*module_id.address()),
                gas_info: None,
                timestamp: None,
                tx_hash: None,
                persist_runtime_state: true,
                state_overlay: None,
            },
        )
        .context("add dynamic object field child")?;
    assert_eq!(add_changes.added_dynamic_fields.len(), 1);
    assert!(add_changes.removed_dynamic_fields.is_empty());
    state.apply_changeset(&add_changes)?;
    state.commit()?;

    let second_runtime = MoveRuntime::new_with_kanari_natives_and_store(store.clone())?;
    let read_before_update = second_runtime
        .execute_view_function(
            TESTER_ADDR,
            MODULE_NAME,
            "read_child_value",
            &[],
            &[object_arg(&host_id)?, bcs::to_bytes(&3u64)?],
            &[host_object_input(&host_id, *module_id.address(), false)],
        )
        .context("read persisted dynamic object field value")?;
    assert_eq!(read_before_update, JsonValue::from(500u64));

    let update_changes = second_runtime
        .execute_entry_function_with_object_context_and_persistence(
            &module_id,
            "write_child_value",
            vec![],
            vec![
                object_arg(&host_id)?,
                bcs::to_bytes(&3u64)?,
                bcs::to_bytes(&777u64)?,
            ],
            EntryFunctionObjectContext {
                object_inputs: vec![host_object_input(&host_id, *module_id.address(), true)],
                sender: Some(*module_id.address()),
                gas_info: None,
                timestamp: None,
                tx_hash: None,
                persist_runtime_state: true,
                state_overlay: None,
            },
        )
        .context("mutate persisted dynamic object field value")?;
    assert_eq!(update_changes.added_dynamic_fields.len(), 1);
    assert!(update_changes.removed_dynamic_fields.is_empty());
    state.apply_changeset(&update_changes)?;
    state.commit()?;

    let third_runtime = MoveRuntime::new_with_kanari_natives_and_store(store.clone())?;
    let read_after_update = third_runtime
        .execute_view_function(
            TESTER_ADDR,
            MODULE_NAME,
            "read_child_value",
            &[],
            &[object_arg(&host_id)?, bcs::to_bytes(&3u64)?],
            &[host_object_input(&host_id, *module_id.address(), false)],
        )
        .context("read updated dynamic object field value")?;
    assert_eq!(read_after_update, JsonValue::from(777u64));

    let remove_changes = third_runtime
        .execute_entry_function_with_object_context_and_persistence(
            &module_id,
            "remove_child",
            vec![],
            vec![object_arg(&host_id)?, bcs::to_bytes(&3u64)?],
            EntryFunctionObjectContext {
                object_inputs: vec![host_object_input(&host_id, *module_id.address(), true)],
                sender: Some(*module_id.address()),
                gas_info: None,
                timestamp: None,
                tx_hash: None,
                persist_runtime_state: true,
                state_overlay: None,
            },
        )
        .context("remove persisted dynamic object field value")?;
    assert!(remove_changes.added_dynamic_fields.is_empty());
    assert_eq!(remove_changes.removed_dynamic_fields.len(), 1);
    state.apply_changeset(&remove_changes)?;
    state.commit()?;

    let fourth_runtime = MoveRuntime::new_with_kanari_natives_and_store(store)?;
    let exists_after_remove = fourth_runtime
        .execute_view_function(
            TESTER_ADDR,
            MODULE_NAME,
            "has_child",
            &[],
            &[object_arg(&host_id)?, bcs::to_bytes(&3u64)?],
            &[host_object_input(&host_id, *module_id.address(), false)],
        )
        .context("check dynamic object field absence after remove")?;
    assert_eq!(exists_after_remove, JsonValue::from(0u64));

    Ok(())
}

fn create_test_package() -> Result<PathBuf> {
    let dir = tempdir()?;
    let package_dir = dir.keep();
    fs::create_dir_all(package_dir.join("sources"))?;

    let dependency_path = move_manifest::kanari_system_package_path()?;
    let manifest = format!(
        "[package]\nname = \"DynamicObjectFieldE2E\"\n\n[dependencies]\nKanariSystem = {{ local = \"{}\" }}\n\n[addresses]\ntester = \"{}\"\n",
        dependency_path, TESTER_ADDR,
    );
    fs::write(package_dir.join("Move.toml"), manifest)?;

    let source = format!(
        "module tester::{module_name} {{\n    use kanari_system::dynamic_object_field;\n    use kanari_system::object::{{Self, UID}};\n    use kanari_system::transfer;\n    use kanari_system::tx_context::{{Self, TxContext}};\n\n    struct Host has key, store {{\n        id: UID,\n    }}\n\n    struct Child has key, store {{\n        id: UID,\n        value: u64,\n    }}\n\n    fun new_child(ctx: &mut TxContext, value: u64): Child {{\n        Child {{ id: object::new(ctx), value }}\n    }}\n\n    public entry fun create_host(ctx: &mut TxContext) {{\n        let host = Host {{ id: object::new(ctx) }};\n        transfer::public_transfer(host, tx_context::sender(ctx));\n    }}\n\n    public entry fun add_child(host: &mut Host, key: u64, value: u64, ctx: &mut TxContext) {{\n        let child = new_child(ctx, value);\n        dynamic_object_field::add<u64, Child>(&mut host.id, key, child);\n        object::save_object(host);\n    }}\n\n    public entry fun write_child_value(host: &mut Host, key: u64, value: u64) {{\n        dynamic_object_field::borrow_mut<u64, Child>(&mut host.id, key).value = value;\n        object::save_object(host);\n    }}\n\n    public entry fun remove_child(host: &mut Host, key: u64) {{\n        let child = dynamic_object_field::remove<u64, Child>(&mut host.id, key);\n        let Child {{ id, value: _ }} = child;\n        object::delete(id);\n        object::save_object(host);\n    }}\n\n    public fun read_child_value(host: &Host, key: u64): u64 {{\n        dynamic_object_field::borrow<u64, Child>(&host.id, key).value\n    }}\n\n    public fun has_child(host: &Host, key: u64): bool {{\n        dynamic_object_field::exists_<u64>(&host.id, key)\n    }}\n}}\n",
        module_name = MODULE_NAME,
    );
    fs::write(
        package_dir
            .join("sources")
            .join(format!("{MODULE_NAME}.move")),
        source,
    )?;

    Ok(package_dir)
}

fn compile_test_module(package_dir: &Path) -> Result<(ModuleId, Vec<u8>)> {
    let package = BuildConfig::default().compile_package(package_dir, &mut Vec::new())?;
    let unit = package
        .root_modules()
        .next()
        .context("compiled package should contain a root module")?;
    let module = &unit.unit;
    let module_id = ModuleId::new(
        AccountAddress::new(module.address.into_bytes()),
        Identifier::new(module.name.to_string())?,
    );
    Ok((module_id, module.serialize(None)))
}

fn object_arg(object_id: &str) -> Result<Vec<u8>> {
    let clean = object_id.strip_prefix("0x").unwrap_or(object_id);
    hex::decode(clean).context("decode object id argument")
}

fn host_object_input(host_id: &str, owner: AccountAddress, mutable: bool) -> ObjectInput {
    ObjectInput {
        object_ref: ObjectRef::new(host_id.to_string(), None, None),
        owner: Some(ObjectOwnerKind::AddressOwner(owner.to_hex_literal())),
        mutable,
    }
}
