use super::*;
use anyhow::Context;
use kanari_types::gas_coin::GasModule;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::{StructTag, TypeTag};
use move_package::BuildConfig;
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tempfile::tempdir;

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct SerializedTxContext {
    sender: AccountAddress,
    tx_hash: Vec<u8>,
    ids_created: u64,
    epoch_timestamp_ms: u64,
    sponsor: u64,
}

#[test]
fn refresh_committed_modules_rebuilds_runtime_module_index() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let committed: HashSet<_> = runtime.state.get_all_module_ids()?.into_iter().collect();
    assert!(!committed.is_empty());

    runtime
        .published_modules
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clear();
    runtime.refresh_committed_modules()?;

    assert_eq!(
        *runtime
            .published_modules
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
        committed
    );
    Ok(())
}

#[test]
fn tx_context_without_timestamp_is_deterministic() -> Result<()> {
    let runtime = MoveRuntime::new_with_natives_in_memory(vec![])?;
    let sender = AccountAddress::from_hex_literal("0x1111")?;

    let first = runtime.build_tx_context_bytes(Some(sender), None, None)?;
    let second = runtime.build_tx_context_bytes(Some(sender), None, None)?;
    let ctx: SerializedTxContext = bcs::from_bytes(&first)?;

    assert_eq!(first, second);
    assert_eq!(ctx.sender, sender);
    assert_eq!(ctx.epoch_timestamp_ms, 0);
    assert_eq!(ctx.ids_created, 0);
    assert_eq!(ctx.sponsor, 0);

    Ok(())
}

#[test]
fn tx_context_uses_canonical_timestamp_and_hash() -> Result<()> {
    let runtime = MoveRuntime::new_with_natives_in_memory(vec![])?;
    let sender = AccountAddress::from_hex_literal("0x1111")?;
    let tx_hash = vec![7u8; 32];

    let bytes = runtime.build_tx_context_bytes(Some(sender), Some(42), Some(&tx_hash))?;
    let ctx: SerializedTxContext = bcs::from_bytes(&bytes)?;

    assert_eq!(ctx.sender, sender);
    assert_eq!(ctx.tx_hash, tx_hash);
    assert_eq!(ctx.epoch_timestamp_ms, 42);

    Ok(())
}

#[test]
fn native_hash_can_trigger_out_of_gas_end_to_end() -> Result<()> {
    let runtime = new_runtime_with_metered_sha2_native()?;
    let package_dir = create_hash_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime.load_system_modules()?;
    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish hash meter test module")?;

    let err = runtime
        .execute_entry_function(
            &module_id,
            "hash_once",
            vec![],
            vec![bcs::to_bytes(&vec![42u8; 64])?],
            Some(*module_id.address()),
            Some((20, 1)),
            None,
        )
        .expect_err("metered sha2 native should exceed the provided gas limit");

    assert_out_of_gas(err);
    Ok(())
}

#[test]
fn native_event_emit_can_trigger_out_of_gas_end_to_end() -> Result<()> {
    let runtime = new_runtime_with_metered_event_native()?;
    let package_dir = create_event_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime.load_system_modules()?;
    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish event meter test module")?;

    let err = runtime
        .execute_entry_function(
            &module_id,
            "emit_blob",
            vec![],
            vec![bcs::to_bytes(&vec![7u8; 48])?],
            Some(*module_id.address()),
            Some((20, 1)),
            None,
        )
        .expect_err("metered event native should exceed the provided gas limit");

    assert_out_of_gas(err);
    Ok(())
}

#[test]
fn native_save_object_can_trigger_out_of_gas_end_to_end() -> Result<()> {
    let runtime = new_runtime_with_metered_save_object_native()?;
    let package_dir = create_save_object_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime.load_system_modules()?;
    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish save_object meter test module")?;

    let err = runtime
        .execute_entry_function(
            &module_id,
            "save_blob",
            vec![],
            vec![bcs::to_bytes(&vec![9u8; 48])?],
            Some(*module_id.address()),
            Some((20, 1)),
            None,
        )
        .expect_err("metered save_object native should exceed the provided gas limit");

    assert_out_of_gas(err);
    Ok(())
}

#[test]
fn native_public_transfer_can_trigger_out_of_gas_end_to_end() -> Result<()> {
    let runtime = new_runtime_with_metered_transfer_native()?;
    let package_dir = create_transfer_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime.load_system_modules()?;
    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish transfer meter test module")?;

    let err = runtime
        .execute_entry_function(
            &module_id,
            "transfer_blob",
            vec![],
            vec![bcs::to_bytes(&vec![11u8; 48])?],
            Some(*module_id.address()),
            Some((20, 1)),
            None,
        )
        .expect_err("metered transfer native should exceed the provided gas limit");

    assert_out_of_gas(err);
    Ok(())
}

#[test]
fn production_runtime_hash_native_can_trigger_out_of_gas() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let package_dir = create_hash_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish production hash meter test module")?;

    let err = runtime
        .execute_entry_function(
            &module_id,
            "hash_once",
            vec![],
            vec![bcs::to_bytes(&vec![42u8; 64])?],
            Some(*module_id.address()),
            Some((20, 1)),
            None,
        )
        .expect_err("production runtime hash native should exceed the provided gas limit");

    assert_out_of_gas(err);
    Ok(())
}

#[test]
fn production_runtime_event_native_can_trigger_out_of_gas() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let package_dir = create_event_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish production event meter test module")?;

    let err = runtime
        .execute_entry_function(
            &module_id,
            "emit_blob",
            vec![],
            vec![bcs::to_bytes(&vec![7u8; 48])?],
            Some(*module_id.address()),
            Some((20, 1)),
            None,
        )
        .expect_err("production runtime event native should exceed the provided gas limit");

    assert_out_of_gas(err);
    Ok(())
}

#[test]
fn production_runtime_save_object_native_can_trigger_out_of_gas() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let package_dir = create_save_object_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish production save_object meter test module")?;

    let err = runtime
        .execute_entry_function(
            &module_id,
            "save_blob",
            vec![],
            vec![bcs::to_bytes(&vec![9u8; 48])?],
            Some(*module_id.address()),
            Some((20, 1)),
            None,
        )
        .expect_err("production runtime save_object native should exceed the provided gas limit");

    assert_out_of_gas(err);
    Ok(())
}
#[test]
fn production_runtime_transfer_native_can_trigger_out_of_gas() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let package_dir = create_transfer_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish production transfer meter test module")?;

    let err = runtime
        .execute_entry_function(
            &module_id,
            "transfer_blob",
            vec![],
            vec![bcs::to_bytes(&vec![11u8; 48])?],
            Some(*module_id.address()),
            Some((20, 1)),
            None,
        )
        .expect_err("production runtime transfer native should exceed the provided gas limit");

    assert_out_of_gas(err);
    Ok(())
}

#[test]
fn kanari_transfer_keeps_distinct_sender_gas_coin_objects() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let sender = kanari_types::address::Address::dev_account_address();
    let recipient = AccountAddress::from_hex_literal(
        "0x3141a487d7a5382bb435c0ad39a6060067765e60e45b50953a0050bcf24b03a3",
    )?;
    let coin_type = format!("0x2::coin::Coin<{}>", kanari_types::gas_coin::GAS_COIN);
    let transfer_coin_id = "0x15fd69ee1ac3b3e43b0348b5c59202059404f529a23d4c054d56841193fdb45a";
    let gas_coin_id = "0xdfa10cddb4fb9b10fdb5083fe6bc1e03970cbdd891b1e31e894070c33462e672";
    let owner_kind = ObjectOwnerKind::AddressOwner(sender.to_hex_literal());
    let transfer_coin = StoredObject {
        id: transfer_coin_id.to_string(),
        owner: sender,
        owner_kind: owner_kind.clone(),
        type_name: coin_type.clone(),
        data: coin_data(transfer_coin_id, 1_000_000_000)?,
        version: 1,
    };
    let gas_coin = StoredObject {
        id: gas_coin_id.to_string(),
        owner: sender,
        owner_kind: owner_kind.clone(),
        type_name: coin_type,
        data: coin_data(gas_coin_id, 10_000_000_000)?,
        version: 1,
    };
    runtime.object_storage.store_object(transfer_coin.clone())?;
    runtime.object_storage.store_object(gas_coin.clone())?;

    let transfer_amount = 100_000_000u64;
    let transfer_coin_before = coin_balance(&transfer_coin.data);

    let changeset = runtime.execute_entry_function_with_object_context_and_persistence(
        &GasModule::get_module_id()?,
        GasModule::function_names().transfer,
        vec![],
        vec![
            vec![],
            bcs::to_bytes(&transfer_amount)?,
            bcs::to_bytes(&recipient)?,
        ],
        EntryFunctionObjectContext {
            object_inputs: vec![ObjectInput {
                object_ref: ObjectRef::new(
                    transfer_coin.id.clone(),
                    Some(transfer_coin.version),
                    None,
                ),
                owner: Some(ObjectOwnerKind::AddressOwner(sender.to_hex_literal())),
                mutable: true,
            }],
            sender: Some(sender),
            gas_info: Some((100_000, 1)),
            timestamp: None,
            tx_hash: Some(vec![9; 32]),
            persist_runtime_state: false,
            state_overlay: None,
        },
    )?;

    let module_key = format!(
        "module:{}:{}",
        GasModule::get_module_id()?.address().to_hex_literal(),
        GasModule::get_module_id()?.name()
    )
    .into_bytes();
    assert!(
        changeset.resolver_reads.contains(&module_key),
        "entry module must be traced even when it was already present in the VM cache"
    );

    assert!(
        !changeset.deleted_objects.contains(&gas_coin.id),
        "runtime must not auto-merge and delete the separate gas coin"
    );

    let (_, updated_transfer_coin) = changeset
        .created_objects
        .iter()
        .find(|(id, _)| id == &transfer_coin.id)
        .context("transfer coin should be written back as a mutated object")?;
    assert_eq!(
        coin_balance(&updated_transfer_coin.data),
        transfer_coin_before - transfer_amount,
        "mutated transfer coin must only lose the requested split amount"
    );

    Ok(())
}

#[test]
fn entry_can_borrow_coin_id_after_string_address_and_u64_args() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let package_dir = create_escrow_like_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish escrow-like object input test module")?;

    let sender = *module_id.address();
    let seller = AccountAddress::from_hex_literal(
        "0x3141a487d7a5382bb435c0ad39a6060067765e60e45b50953a0050bcf24b03a3",
    )?;
    let coin_id = "0x15fd69ee1ac3b3e43b0348b5c59202059404f529a23d4c054d56841193fdb45a";
    let coin_type = format!("0x2::coin::Coin<{}>", kanari_types::gas_coin::GAS_COIN);
    let coin = StoredObject {
        id: coin_id.to_string(),
        owner: sender,
        owner_kind: ObjectOwnerKind::AddressOwner(sender.to_hex_literal()),
        type_name: coin_type,
        data: coin_data(coin_id, 1_000_000_000)?,
        version: 1,
    };
    runtime.object_storage.store_object(coin.clone())?;

    runtime.execute_entry_function_with_object_context_and_persistence(
        &module_id,
        "create_deal",
        vec![TypeTag::Struct(Box::new(StructTag::from_str(
            kanari_types::gas_coin::GAS_COIN,
        )?))],
        vec![
            bcs::to_bytes("deal-1")?,
            bcs::to_bytes(&seller)?,
            bcs::to_bytes(&100u64)?,
            bcs::to_bytes("escrow description")?,
            bcs::to_bytes(&AccountAddress::from_hex_literal(coin.id.as_str())?)?,
        ],
        EntryFunctionObjectContext {
            object_inputs: vec![],
            sender: Some(sender),
            gas_info: Some((100_000, 1)),
            timestamp: Some(1_785_475_231_485),
            tx_hash: Some(vec![3; 32]),
            persist_runtime_state: false,
            state_overlay: None,
        },
    )?;

    Ok(())
}

#[test]
fn entry_cannot_mutably_borrow_address_owned_coin_from_other_owner() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let package_dir = create_escrow_like_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish escrow-like unauthorized borrow test module")?;

    let sender = *module_id.address();
    let other_owner = AccountAddress::from_hex_literal(
        "0x8e8bbedac8598c9cb45e92f48c61c9671aa474c199e5d113a5e76cd9e154aa74",
    )?;
    let seller = AccountAddress::from_hex_literal(
        "0x3141a487d7a5382bb435c0ad39a6060067765e60e45b50953a0050bcf24b03a3",
    )?;
    let coin_id = "0x25fd69ee1ac3b3e43b0348b5c59202059404f529a23d4c054d56841193fdb45a";
    let coin_type = format!("0x2::coin::Coin<{}>", kanari_types::gas_coin::GAS_COIN);
    runtime.object_storage.store_object(StoredObject {
        id: coin_id.to_string(),
        owner: other_owner,
        owner_kind: ObjectOwnerKind::AddressOwner(other_owner.to_hex_literal()),
        type_name: coin_type,
        data: coin_data(coin_id, 1_000_000_000)?,
        version: 1,
    })?;

    let err = runtime
        .execute_entry_function_with_object_context_and_persistence(
            &module_id,
            "create_deal",
            vec![TypeTag::Struct(Box::new(StructTag::from_str(
                kanari_types::gas_coin::GAS_COIN,
            )?))],
            vec![
                bcs::to_bytes("deal-1")?,
                bcs::to_bytes(&seller)?,
                bcs::to_bytes(&100u64)?,
                bcs::to_bytes("escrow description")?,
                bcs::to_bytes(&AccountAddress::from_hex_literal(coin_id)?)?,
            ],
            EntryFunctionObjectContext {
                object_inputs: vec![],
                sender: Some(sender),
                gas_info: Some((100_000, 1)),
                timestamp: Some(1_785_475_231_485),
                tx_hash: Some(vec![4; 32]),
                persist_runtime_state: false,
                state_overlay: None,
            },
        )
        .expect_err("sender must not mutably borrow another owner's owned coin");

    let message = format!("{err:?}");
    assert!(
        message.contains("9005") || message.contains("E_OBJECT_NOT_MUTABLY_BORROWABLE"),
        "expected mutable borrow authorization failure, got: {message}"
    );

    Ok(())
}

#[test]
fn raw_address_arg_cannot_mutably_borrow_non_coin_object_from_other_owner() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let package_dir = create_escrow_like_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish escrow-like cross-owner object test module")?;

    let sender = *module_id.address();
    let other_owner = AccountAddress::from_hex_literal(
        "0x8e8bbedac8598c9cb45e92f48c61c9671aa474c199e5d113a5e76cd9e154aa74",
    )?;
    let marker_id = "0x35fd69ee1ac3b3e43b0348b5c59202059404f529a23d4c054d56841193fdb45a";
    runtime.object_storage.store_object(StoredObject {
        id: marker_id.to_string(),
        owner: other_owner,
        owner_kind: ObjectOwnerKind::AddressOwner(other_owner.to_hex_literal()),
        type_name: format!(
            "{}::escrow_like_object_input::Marker",
            module_id.address().to_hex_literal()
        ),
        data: AccountAddress::from_hex_literal(marker_id)?.to_vec(),
        version: 1,
    })?;

    let err = runtime
        .execute_entry_function_with_object_context_and_persistence(
            &module_id,
            "touch_marker",
            vec![],
            vec![bcs::to_bytes(&AccountAddress::from_hex_literal(
                marker_id,
            )?)?],
            EntryFunctionObjectContext {
                object_inputs: vec![],
                sender: Some(sender),
                gas_info: Some((100_000, 1)),
                timestamp: Some(1_785_475_231_485),
                tx_hash: Some(vec![5; 32]),
                persist_runtime_state: false,
                state_overlay: None,
            },
        )
        .expect_err("raw address args must not mutably borrow another owner's object");

    let message = format!("{err:?}");
    assert!(
        message.contains("9005") || message.contains("E_OBJECT_NOT_MUTABLY_BORROWABLE"),
        "expected explicit object input requirement failure, got: {message}"
    );

    Ok(())
}

#[test]
fn explicit_object_input_can_mutably_borrow_non_coin_defi_object_from_other_owner() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;
    let package_dir = create_escrow_like_test_package()?;
    let (module_id, module_bytes) = compile_test_module(&package_dir)?;

    runtime
        .publish_module(module_bytes, *module_id.address(), None, None)
        .context("publish escrow-like explicit cross-owner object test module")?;

    let sender = *module_id.address();
    let other_owner = AccountAddress::from_hex_literal(
        "0x8e8bbedac8598c9cb45e92f48c61c9671aa474c199e5d113a5e76cd9e154aa74",
    )?;
    let marker_id = "0x45fd69ee1ac3b3e43b0348b5c59202059404f529a23d4c054d56841193fdb45a";
    runtime.object_storage.store_object(StoredObject {
        id: marker_id.to_string(),
        owner: other_owner,
        owner_kind: ObjectOwnerKind::AddressOwner(other_owner.to_hex_literal()),
        type_name: format!(
            "{}::escrow_like_object_input::Marker",
            module_id.address().to_hex_literal()
        ),
        data: AccountAddress::from_hex_literal(marker_id)?.to_vec(),
        version: 1,
    })?;

    runtime.execute_entry_function_with_object_context_and_persistence(
        &module_id,
        "touch_marker",
        vec![],
        vec![bcs::to_bytes(&AccountAddress::from_hex_literal(
            marker_id,
        )?)?],
        EntryFunctionObjectContext {
            object_inputs: vec![ObjectInput {
                object_ref: ObjectRef::new(marker_id.to_string(), Some(1), None),
                owner: Some(ObjectOwnerKind::AddressOwner(other_owner.to_hex_literal())),
                mutable: true,
            }],
            sender: Some(sender),
            gas_info: Some((100_000, 1)),
            timestamp: Some(1_785_475_231_485),
            tx_hash: Some(vec![6; 32]),
            persist_runtime_state: false,
            state_overlay: None,
        },
    )?;

    Ok(())
}

fn coin_balance(data: &[u8]) -> u64 {
    if data.len() < 40 {
        return 0;
    }
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&data[32..40]);
    u64::from_le_bytes(bytes)
}

fn coin_data(object_id: &str, balance: u64) -> Result<Vec<u8>> {
    let mut data = AccountAddress::from_hex_literal(object_id)?.to_vec();
    data.extend_from_slice(&balance.to_le_bytes());
    Ok(data)
}

fn assert_out_of_gas(err: anyhow::Error) {
    let message = format!("{err:?}");
    assert!(
        message.contains("OUT_OF_GAS"),
        "expected out-of-gas error, got: {message}"
    );
}

fn new_runtime_with_metered_sha2_native() -> Result<MoveRuntime> {
    let mut std_gas = move_stdlib_natives::GasParameters::zeros();
    std_gas.hash.sha2_256.base = 10.into();
    std_gas.hash.sha2_256.per_byte = 1.into();
    std_gas.hash.sha2_256.legacy_min_input_len = 0.into();

    let natives = vec![
        move_stdlib_natives::all_natives(KanariAddress::std_account_address(), std_gas),
        kanari_system_natives::all_natives(
            KanariAddress::kanari_system_account_address(),
            kanari_system_natives::GasParameters::zeros(),
        ),
    ];

    MoveRuntime::new_with_natives_in_memory(natives)
}

fn new_runtime_with_metered_event_native() -> Result<MoveRuntime> {
    let mut system_gas = kanari_system_natives::GasParameters::zeros();
    system_gas.event.base = 8.into();
    system_gas.event.per_byte = 1.into();

    let natives = vec![
        move_stdlib_natives::all_natives(
            KanariAddress::std_account_address(),
            move_stdlib_natives::GasParameters::zeros(),
        ),
        kanari_system_natives::all_natives(
            KanariAddress::kanari_system_account_address(),
            system_gas,
        ),
    ];

    MoveRuntime::new_with_natives_in_memory(natives)
}

fn new_runtime_with_metered_save_object_native() -> Result<MoveRuntime> {
    let mut system_gas = kanari_system_natives::GasParameters::zeros();
    system_gas.object.save_object.base = 8.into();
    system_gas.object.save_object.per_byte_serialized = 1.into();

    let natives = vec![
        move_stdlib_natives::all_natives(
            KanariAddress::std_account_address(),
            move_stdlib_natives::GasParameters::zeros(),
        ),
        kanari_system_natives::all_natives(
            KanariAddress::kanari_system_account_address(),
            system_gas,
        ),
    ];

    MoveRuntime::new_with_natives_in_memory(natives)
}

fn new_runtime_with_metered_transfer_native() -> Result<MoveRuntime> {
    let mut system_gas = kanari_system_natives::GasParameters::zeros();
    system_gas.transfer.transfer_with_uid.base = 8.into();
    system_gas.transfer.transfer_with_uid.per_byte = 1.into();

    let natives = vec![
        move_stdlib_natives::all_natives(
            KanariAddress::std_account_address(),
            move_stdlib_natives::GasParameters::zeros(),
        ),
        kanari_system_natives::all_natives(
            KanariAddress::kanari_system_account_address(),
            system_gas,
        ),
    ];

    MoveRuntime::new_with_natives_in_memory(natives)
}

fn create_hash_test_package() -> Result<PathBuf> {
    create_test_package(
        "GasMeterHashE2E",
        "0x44",
        "gas_meter_hash_e2e.move",
        "module tester::gas_meter_hash_e2e {\n    use std::hash;\n\n    public entry fun hash_once(data: vector<u8>) {\n        let _digest = hash::sha2_256(data);\n    }\n}\n",
    )
}

fn create_event_test_package() -> Result<PathBuf> {
    create_test_package(
        "GasMeterEventE2E",
        "0x45",
        "gas_meter_event_e2e.move",
        "module tester::gas_meter_event_e2e {\n    use kanari_system::event;\n\n    public entry fun emit_blob(data: vector<u8>) {\n        event::emit<vector<u8>>(data);\n    }\n}\n",
    )
}

fn create_save_object_test_package() -> Result<PathBuf> {
    create_test_package(
        "GasMeterSaveObjectE2E",
        "0x46",
        "gas_meter_save_object_e2e.move",
        "module tester::gas_meter_save_object_e2e {\n    use kanari_system::object::{Self, UID};\n    use kanari_system::tx_context::TxContext;\n\n    struct Blob has key, store {\n        id: UID,\n        data: vector<u8>,\n    }\n\n    public entry fun save_blob(data: vector<u8>, ctx: &mut TxContext) {\n        let blob = Blob { id: object::new(ctx), data };\n        object::save_object(&blob);\n        let Blob { id, data: _ } = blob;\n        object::delete(id);\n    }\n}\n",
    )
}

fn create_transfer_test_package() -> Result<PathBuf> {
    create_test_package(
        "GasMeterTransferE2E",
        "0x47",
        "gas_meter_transfer_e2e.move",
        "module tester::gas_meter_transfer_e2e {\n    use kanari_system::object::{Self, UID};\n    use kanari_system::transfer;\n    use kanari_system::tx_context::{Self, TxContext};\n\n    struct Blob has key, store {\n        id: UID,\n        data: vector<u8>,\n    }\n\n    public entry fun transfer_blob(data: vector<u8>, ctx: &mut TxContext) {\n        let blob = Blob { id: object::new(ctx), data };\n        transfer::public_transfer(blob, tx_context::sender(ctx));\n    }\n}\n",
    )
}

fn create_escrow_like_test_package() -> Result<PathBuf> {
    create_test_package(
        "EscrowLikeObjectInput",
        "0x48",
        "escrow_like_object_input.move",
        "#[allow(unused_field)]\nmodule tester::escrow_like_object_input {\n    use std::string::String;\n    use kanari_system::coin::{Self, Coin};\n    use kanari_system::object::{Self, UID};\n    use kanari_system::tx_context::{Self, TxContext};\n\n    const E_NOT_ENOUGH_BALANCE: u64 = 7;\n\n    struct Marker has key, store {\n        id: UID,\n    }\n\n    public entry fun create_deal<CoinType>(\n        deal_id: String,\n        seller: address,\n        amount: u64,\n        description: String,\n        buyer_coin_id: address,\n        ctx: &mut TxContext,\n    ) {\n        let _buyer = tx_context::sender(ctx);\n        let _deal_id = deal_id;\n        let _seller = seller;\n        let _description = description;\n        let buyer_coin: &mut Coin<CoinType> = object::borrow_global_mut<Coin<CoinType>>(buyer_coin_id);\n        assert!(coin::value(buyer_coin) >= amount, E_NOT_ENOUGH_BALANCE);\n    }\n\n    public entry fun touch_marker(marker_id: address, ctx: &mut TxContext) {\n        let _sender = tx_context::sender(ctx);\n        let _marker: &mut Marker = object::borrow_global_mut<Marker>(marker_id);\n    }\n}\n",
    )
}

fn create_test_package(
    package_name: &str,
    tester_addr: &str,
    module_filename: &str,
    source: &str,
) -> Result<PathBuf> {
    let dir = tempdir()?;
    let package_dir = dir.keep();
    fs::create_dir_all(package_dir.join("sources"))?;

    let dependency_path = kanari_system_package_path()?;
    let manifest = format!(
        "[package]\nname = \"{package_name}\"\n\n[dependencies]\nKanariSystem = {{ local = \"{}\" }}\n\n[addresses]\ntester = \"{tester_addr}\"\n",
        dependency_path,
    );
    fs::write(package_dir.join("Move.toml"), manifest)?;
    fs::write(package_dir.join("sources").join(module_filename), source)?;

    Ok(package_dir)
}

fn kanari_system_package_path() -> Result<String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let package = manifest_dir.join("../../../crates/kanari-frameworks/packages/kanari-system");
    let canonical = package.canonicalize().with_context(|| {
        format!(
            "resolve KanariSystem package path from {}",
            package.display()
        )
    })?;
    Ok(move_manifest_local_path(&canonical))
}

fn move_manifest_local_path(path: &Path) -> String {
    let normalized = path.display().to_string().replace('\\', "/");
    normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .to_string()
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
