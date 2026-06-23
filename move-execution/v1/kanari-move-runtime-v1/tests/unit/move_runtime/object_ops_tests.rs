use super::*;
use crate::move_runtime::MoveRuntime;
#[path = "../test_support.rs"]
mod test_support;

use test_support::test_addr_unwrap;

#[test]
fn speculative_transferred_objects_do_not_mutate_object_storage() {
    let runtime = MoveRuntime::new_with_natives_in_memory(vec![]).unwrap();
    let owner = test_addr_unwrap("0x1234");
    let object_id = "0xabcd".to_string();
    let object_type = "0x2::test::Object".to_string();
    let object = TransferredObject {
        object_id: object_id.clone(),
        object_type: object_type.clone(),
        recipient: owner,
        data: vec![1, 2, 3],
        should_persist: true,
        is_frozen: false,
    };

    let mut speculative = ChangeSet::new();
    runtime.add_transferred_objects_to_changeset(&mut speculative, vec![object.clone()], false);

    assert_eq!(runtime.object_storage.count(), 0);
    assert_eq!(speculative.created_objects.len(), 1);

    let mut canonical = ChangeSet::new();
    runtime.add_transferred_objects_to_changeset(&mut canonical, vec![object], true);

    assert_eq!(runtime.object_storage.count(), 1);
    let canonical_id = MoveRuntime::canonical_object_id_str(&object_id).unwrap();
    assert!(runtime.object_storage.get_object(&canonical_id).is_some());
}

#[test]
fn frozen_objects_use_distinct_immutable_owner_marker() {
    let runtime = MoveRuntime::new_with_natives_in_memory(vec![]).unwrap();
    let object = TransferredObject {
        object_id: "0xf00d".to_string(),
        object_type: "0x2::test::Frozen".to_string(),
        recipient: AccountAddress::ZERO,
        data: vec![1],
        should_persist: true,
        is_frozen: true,
    };
    let mut changeset = ChangeSet::new();
    runtime.add_transferred_objects_to_changeset(&mut changeset, vec![object], false);
    assert_eq!(changeset.created_objects.len(), 1);
    assert_eq!(
        changeset.created_objects[0].1.owner,
        MoveRuntime::immutable_object_owner()
    );
    assert_ne!(MoveRuntime::immutable_object_owner(), AccountAddress::ZERO);
}
