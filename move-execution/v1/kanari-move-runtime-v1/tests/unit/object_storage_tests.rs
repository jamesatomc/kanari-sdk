use super::*;

#[test]
fn persistent_owner_lookup_prefers_canonical_owned_objects_index() -> Result<()> {
    let store = Arc::new(PersistentStore::open_in_memory()?);
    let owner = AccountAddress::from_hex_literal("0x1")?;
    let stale_id = "0xaaaa".to_string();
    let canonical_id = "0xbbbb".to_string();

    store.save(
        format!("object:{}", stale_id).as_bytes(),
        &StoredObject {
            id: stale_id.clone(),
            owner,
            type_name: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
            data: vec![1],
            version: 1,
        },
    )?;
    store.save(
        format!("object:{}", canonical_id).as_bytes(),
        &StoredObject {
            id: canonical_id.clone(),
            owner,
            type_name: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
            data: vec![2],
            version: 1,
        },
    )?;
    store.save(
        &ObjectStorage::legacy_owner_key(&owner),
        &vec![stale_id.clone()],
    )?;
    store.save(&owned_objects_key(&owner), &vec![canonical_id.clone()])?;

    let storage = ObjectStorage::new_with_store(store.clone())?;
    let objects = storage.get_objects_by_owner(&owner);

    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, canonical_id);
    assert_eq!(
        ObjectStorage::load_id_index(&store, &owned_objects_key(&owner))?,
        vec![canonical_id]
    );

    Ok(())
}

#[test]
fn legacy_owner_index_is_migrated_to_owned_objects_index() -> Result<()> {
    let store = Arc::new(PersistentStore::open_in_memory()?);
    let owner = AccountAddress::from_hex_literal("0x2")?;
    let object_id = "0xcccc".to_string();

    store.save(
        format!("object:{}", object_id).as_bytes(),
        &StoredObject {
            id: object_id.clone(),
            owner,
            type_name: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
            data: vec![3],
            version: 1,
        },
    )?;
    store.save(
        &ObjectStorage::legacy_owner_key(&owner),
        &vec![object_id.clone()],
    )?;

    let storage = ObjectStorage::new_with_store(store.clone())?;
    let objects = storage.get_objects_by_owner(&owner);

    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, object_id);
    assert_eq!(
        ObjectStorage::load_id_index(&store, &owned_objects_key(&owner))?,
        vec!["0xcccc".to_string()]
    );
    assert!(
        store
            .load::<Vec<String>>(&ObjectStorage::legacy_owner_key(&owner))?
            .is_none()
    );

    Ok(())
}
