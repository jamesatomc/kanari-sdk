use kanari_types::error::KanariUnwrapExt;
use move_core_types::account_address::AccountAddress;

use super::*;
use crate::move_runtime::MoveRuntime;

#[test]
fn speculative_transferred_objects_do_not_mutate_object_storage() {
    let runtime = MoveRuntime::new_with_natives_in_memory(vec![]).invariant("runtime init");
    let owner = AccountAddress::from_hex_literal("0x1234").invariant("valid owner address");
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

    let baseline_count = runtime.object_storage.count();

    let mut speculative = ChangeSet::new();
    runtime.add_transferred_objects_to_changeset(&mut speculative, vec![object.clone()], false);

    assert_eq!(runtime.object_storage.count(), baseline_count);
    assert_eq!(speculative.created_objects.len(), 1);

    let mut canonical = ChangeSet::new();
    runtime.add_transferred_objects_to_changeset(&mut canonical, vec![object], true);

    assert_eq!(runtime.object_storage.count(), baseline_count + 1);
    let canonical_id = canonical_object_id(&object_id).invariant("canonical object id");
    assert!(runtime.object_storage.get_object(&canonical_id).is_some());
}
