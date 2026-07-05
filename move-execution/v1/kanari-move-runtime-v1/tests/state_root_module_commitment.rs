use anyhow::Result;
use kanari_move_runtime_v1::state::StateManager;

#[test]
fn indexed_module_bytecode_changes_canonical_state_root() -> Result<()> {
    let state = StateManager::new_in_memory();
    let module_key = b"module:0x1:AuditModule";

    state
        .store
        .save(b"module_index", &vec!["module:0x1:AuditModule".to_string()])?;
    state.store.save(module_key, &vec![1u8, 2, 3])?;
    let first_root = state.compute_state_root();

    state.store.save(module_key, &vec![1u8, 2, 4])?;
    let second_root = state.compute_state_root();

    assert_ne!(
        first_root, second_root,
        "indexed Move module bytecode must be committed by the canonical state root"
    );
    Ok(())
}

#[test]
fn orphan_module_key_does_not_enter_canonical_state_root() -> Result<()> {
    let state = StateManager::new_in_memory();
    let before = state.compute_state_root();

    state
        .store
        .save(b"module:0x1:UnindexedLocalModule", &vec![9u8, 9, 9])?;

    assert_eq!(
        before,
        state.compute_state_root(),
        "unindexed runtime-local module blobs must not enter canonical state"
    );
    Ok(())
}

#[test]
fn move_resource_bytes_change_canonical_state_root() -> Result<()> {
    let state = StateManager::new_in_memory();
    let resource_key = b"resource:0x1:0x1::audit::Resource";

    state.store.save(resource_key, &vec![7u8, 8, 9])?;
    let first_root = state.compute_state_root();

    state.store.save(resource_key, &vec![7u8, 8, 10])?;
    let second_root = state.compute_state_root();

    assert_ne!(
        first_root, second_root,
        "Move resource bytes must be committed by the canonical state root"
    );
    Ok(())
}
