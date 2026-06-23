use anyhow::Result;
use kanari_move_runtime_v1::move_runtime::MoveRuntime;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use move_core_types::account_address::AccountAddress;
#[path = "test_support.rs"]
mod test_support;

const SECURITY_TEST_MODULE_SOURCE: &str = r#"
module attacker::borrow_global_security {
    use kanari_system::coin;
    use kanari_system::kanari::KANARI;
    use kanari_system::object;
    use kanari_system::transfer;
    use kanari_system::tx_context;

    public fun read_value(addr: address): u64 {
        let coin_ref = object::borrow_global<coin::Coin<KANARI>>(addr);
        coin::value<KANARI>(coin_ref)
    }

    public entry fun read_only(addr: address) {
        let coin_ref = object::borrow_global<coin::Coin<KANARI>>(addr);
        let _ = coin::value<KANARI>(coin_ref);
    }

    public entry fun try_mut(addr: address) {
        let coin_ref = object::borrow_global_mut<coin::Coin<KANARI>>(addr);
        let _ = coin::value<KANARI>(coin_ref);
    }

    public entry fun owner_split_to_self(addr: address, ctx: &mut tx_context::TxContext) {
        let coin_ref = object::borrow_global_mut<coin::Coin<KANARI>>(addr);
        let split_coin = coin::split<KANARI>(coin_ref, 1, ctx);
        transfer::public_transfer(split_coin, tx_context::sender(ctx));
    }
}
"#;

#[test]
fn address_param_borrow_global_mut_rejects_foreign_object_mutation() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;

    let victim = AccountAddress::from_hex_literal("0x1111").expect("valid victim account address");
    let attacker =
        AccountAddress::from_hex_literal("0x2222").expect("valid attacker account address");
    let object_addr = AccountAddress::from_hex_literal(
        "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    )
    .expect("valid object address");
    let object_id = object_addr.to_hex_literal();

    runtime.preload_object_snapshot(
        &object_id,
        victim,
        &format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
        test_support::coin_object_bytes(object_addr, 100),
        1,
    )?;

    let module_id = test_support::publish_temp_module(
        &runtime,
        "BorrowGlobalSecurity",
        "attacker",
        attacker,
        SECURITY_TEST_MODULE_SOURCE,
    )?;

    let read_result = runtime.execute_entry_function(
        &module_id,
        "read_only",
        vec![],
        vec![bcs::to_bytes(&object_addr)?],
        Some(attacker),
        None,
        None,
    );
    assert!(read_result.is_ok(), "immutable borrow should succeed");

    let err = runtime
        .execute_entry_function(
            &module_id,
            "try_mut",
            vec![],
            vec![bcs::to_bytes(&object_addr)?],
            Some(attacker),
            None,
            None,
        )
        .expect_err("foreign mutable borrow should fail");
    let msg = err.to_string();
    assert!(
        msg.contains("9005")
            || msg.contains("E_OBJECT_MUTATION_NOT_ALLOWED")
            || msg.contains("MUTATION_NOT_ALLOWED"),
        "unexpected error: {msg}"
    );

    Ok(())
}

#[test]
fn address_param_borrow_global_mut_allows_owner_mutation() -> Result<()> {
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()?;

    let owner = AccountAddress::from_hex_literal("0x1111").expect("valid owner account address");
    let publisher =
        AccountAddress::from_hex_literal("0x2222").expect("valid publisher account address");
    let object_addr = AccountAddress::from_hex_literal(
        "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("valid object address");
    let object_id = object_addr.to_hex_literal();

    runtime.preload_object_snapshot(
        &object_id,
        owner,
        &format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE),
        test_support::coin_object_bytes(object_addr, 100),
        1,
    )?;

    let module_id = test_support::publish_temp_module(
        &runtime,
        "BorrowGlobalSecurityOwner",
        "attacker",
        publisher,
        SECURITY_TEST_MODULE_SOURCE,
    )?;

    let before = runtime.execute_view_function(
        &publisher.to_hex_literal(),
        module_id.name().as_str(),
        "read_value",
        &[],
        &[bcs::to_bytes(&object_addr)?],
    )?;
    assert_eq!(before, serde_json::json!(100));

    let cs = runtime.execute_entry_function(
        &module_id,
        "owner_split_to_self",
        vec![],
        vec![bcs::to_bytes(&object_addr)?],
        Some(owner),
        None,
        None,
    )?;

    let updated_original = cs
        .created_objects
        .iter()
        .find(|(id, _)| id == &object_id)
        .map(|(_, created)| created)
        .expect("original coin should be updated");
    assert_eq!(updated_original.owner, owner);
    assert_eq!(
        u64::from_le_bytes(
            updated_original.data[32..40]
                .try_into()
                .expect("coin balance bytes")
        ),
        99
    );

    let after = runtime.execute_view_function(
        &publisher.to_hex_literal(),
        module_id.name().as_str(),
        "read_value",
        &[],
        &[bcs::to_bytes(&object_addr)?],
    )?;
    assert_eq!(after, serde_json::json!(99));

    assert!(
        cs.created_objects
            .iter()
            .any(|(id, created)| id != &object_id
                && created.owner == owner
                && created.type_ == format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE)),
        "split should create a new owner-held coin object"
    );

    Ok(())
}
