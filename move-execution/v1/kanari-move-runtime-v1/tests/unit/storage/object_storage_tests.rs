use super::*;
#[path = "../test_support.rs"]
mod test_support;

use test_support::test_addr;

#[test]
fn persistent_owner_lookup_prefers_canonical_owned_objects_index() -> Result<()> {
    let store = Arc::new(PersistentStore::open_in_memory()?);
    let owner = test_addr("0x1")?;
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
    store.save(&ObjectStorage::owner_key(&owner), &vec![stale_id.clone()])?;
    store.save(
        &ObjectStorage::canonical_owned_objects_key(&owner),
        &vec![canonical_id.clone()],
    )?;

    let storage = ObjectStorage::new_with_store(store)?;
    let objects = storage.get_objects_by_owner(&owner);

    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0].id, canonical_id);

    Ok(())
}
