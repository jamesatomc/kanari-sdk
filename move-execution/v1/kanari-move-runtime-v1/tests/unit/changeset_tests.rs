use kanari_types::address::Address as KanariAddress;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::transaction::{
    GasPayment, ObjectChangeKind, ObjectGraphEdgeKind, ObjectInput, ObjectOwnerKind, ObjectRef,
};

use super::*;

#[test]
fn test_changeset_transfer() {
    let mut cs = ChangeSet::new();
    let from = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)
        .invariant("valid standard address");
    let to = AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS)
        .invariant("valid system address");

    cs.transfer(from, to, 100);

    assert_eq!(cs.owner_deltas.len(), 2);
    assert_eq!(
        cs.owner_deltas
            .get(&from)
            .invariant("sender change missing")
            .balance_delta,
        -100
    );
    assert_eq!(
        cs.owner_deltas
            .get(&to)
            .invariant("recipient change missing")
            .balance_delta,
        100
    );
}

#[test]
fn test_changeset_mint() {
    let mut cs = ChangeSet::new();
    let to = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)
        .invariant("valid standard address");

    cs.mint(to, 1000);

    assert_eq!(cs.owner_deltas.len(), 1);
    assert_eq!(
        cs.owner_deltas
            .get(&to)
            .invariant("recipient change missing")
            .balance_delta,
        1000
    );
}

#[test]
fn test_changeset_burn() {
    let mut cs = ChangeSet::new();
    let from = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)
        .invariant("valid standard address");

    cs.burn(from, 500);

    assert_eq!(cs.owner_deltas.len(), 1);
    assert_eq!(
        cs.owner_deltas
            .get(&from)
            .invariant("sender change missing")
            .balance_delta,
        -500
    );
}

#[test]
fn test_changeset_module_publish() {
    let mut cs = ChangeSet::new();
    let publisher = AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS)
        .invariant("valid system address");

    cs.publish_module(publisher, "kanari".to_string());

    let change = cs
        .owner_deltas
        .get(&publisher)
        .invariant("publisher change missing");
    assert_eq!(change.modules_added.len(), 1);
    assert!(change.modules_added.contains("kanari"));
}

#[test]
fn effects_bucket_object_changes_and_preserve_input_refs() {
    let owner = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)
        .invariant("valid standard address");
    let mut cs = ChangeSet::new();
    cs.set_transaction_context(
        vec![
            ObjectInput {
                object_ref: ObjectRef::new("0x1", Some(7), Some("0xaaa".to_string())),
                owner: Some(ObjectOwnerKind::AddressOwner(owner.to_hex_literal())),
                mutable: true,
            },
            ObjectInput {
                object_ref: ObjectRef::new("0x2", Some(3), Some("0xbbb".to_string())),
                owner: Some(ObjectOwnerKind::Shared),
                mutable: false,
            },
            ObjectInput {
                object_ref: ObjectRef::new("0x3", Some(9), Some("0xccc".to_string())),
                owner: Some(ObjectOwnerKind::Immutable),
                mutable: false,
            },
        ],
        Some(GasPayment {
            payment_objects: vec![ObjectRef::new("0xgas", Some(1), Some("0xddd".to_string()))],
            owner: owner.to_hex_literal(),
            budget: 100,
            price: 1,
        }),
    );
    cs.set_explicit_object_changes(vec![
        ObjectChange {
            change_type: ObjectChangeKind::Created,
            object_ref: ObjectRef::new("0xc", Some(1), Some("0x1".to_string())),
            previous_object_ref: None,
            type_: Some("0x2::test::Created".to_string()),
            owner: Some(ObjectOwnerKind::AddressOwner(owner.to_hex_literal())),
            previous_owner: None,
            previous_version: None,
        },
        ObjectChange {
            change_type: ObjectChangeKind::Transferred,
            object_ref: ObjectRef::new("0xt", Some(2), Some("0x2".to_string())),
            previous_object_ref: Some(ObjectRef::new("0xt", Some(1), Some("0xold".to_string()))),
            type_: Some("0x2::test::Transferred".to_string()),
            owner: Some(ObjectOwnerKind::AddressOwner(owner.to_hex_literal())),
            previous_owner: Some(ObjectOwnerKind::Shared),
            previous_version: Some(1),
        },
    ]);

    let effects = cs.effects(None);
    assert_eq!(effects.input_objects.len(), 3);
    assert_eq!(effects.shared_inputs.len(), 1);
    assert_eq!(effects.immutable_inputs.len(), 1);
    assert_eq!(effects.gas_object_refs.len(), 1);
    assert_eq!(effects.created.len(), 1);
    assert_eq!(effects.transferred.len(), 1);
    assert!(
        effects
            .causal_edges
            .iter()
            .any(|edge| matches!(edge.relation, ObjectGraphEdgeKind::VersionSuccessor))
    );
    assert!(
        effects
            .causal_edges
            .iter()
            .any(|edge| matches!(edge.relation, ObjectGraphEdgeKind::GasCreate))
    );
    assert!(
        effects
            .causal_edges
            .iter()
            .any(|edge| matches!(edge.relation, ObjectGraphEdgeKind::OwnershipTransfer))
    );
    assert_eq!(
        effects.transferred[0]
            .previous_object_ref
            .as_ref()
            .invariant("previous ref should exist")
            .version,
        Some(1)
    );
}

#[test]
fn deterministic_access_sets_detect_read_write_but_allow_read_read() {
    let key = b"resource:0x2::test::Shared".to_vec();
    let mut reader_a = ChangeSet::new();
    reader_a.record_resolver_reads([key.clone()]);
    let mut reader_b = ChangeSet::new();
    reader_b.record_resolver_reads([key.clone()]);
    let mut writer = ChangeSet::new();
    writer.record_move_write(key, Some(vec![1]));

    let reader_a = reader_a.deterministic_access_set();
    let reader_b = reader_b.deterministic_access_set();
    let writer = writer.deterministic_access_set();
    assert!(!reader_a.conflicts_with(&reader_b));
    assert!(reader_a.conflicts_with(&writer));
    assert!(writer.conflicts_with(&reader_b));
}

#[test]
fn merge_preserves_resolver_reads_for_conflict_validation() {
    let key = b"module:0x2:test".to_vec();
    let mut outer = ChangeSet::new();
    let mut inner = ChangeSet::new();
    inner.record_resolver_reads([key.clone()]);
    outer.merge(inner);

    assert!(outer.resolver_reads.contains(&key));
    assert!(outer.deterministic_access_set().reads.contains(&key));
}

#[test]
fn dynamic_field_write_conflicts_with_conservative_read_fence() {
    let reader = ChangeSet::new().deterministic_access_set();
    let mut writer = ChangeSet::new();
    writer
        .added_dynamic_fields
        .push(("0x1".to_string(), vec![7], vec![9]));

    assert!(reader.conflicts_with(&writer.deterministic_access_set()));
}

#[test]
fn object_backed_native_gas_debits_do_not_lock_entire_owner() {
    let owner = AccountAddress::from_hex_literal("0x42").unwrap();
    let mut left = ChangeSet::new();
    left.burn(owner, 10);
    left.gas_payment = Some(GasPayment {
        payment_objects: vec![ObjectRef::new("0x100".to_string(), Some(1), None)],
        owner: owner.to_hex_literal(),
        budget: 10,
        price: 1,
    });
    left.gas_object_refs
        .push(ObjectRef::new("0x100".to_string(), Some(1), None));

    let mut right = ChangeSet::new();
    right.burn(owner, 10);
    right.gas_payment = Some(GasPayment {
        payment_objects: vec![ObjectRef::new("0x101".to_string(), Some(1), None)],
        owner: owner.to_hex_literal(),
        budget: 10,
        price: 1,
    });
    right
        .gas_object_refs
        .push(ObjectRef::new("0x101".to_string(), Some(1), None));

    let left_access = left.deterministic_access_set();
    let right_access = right.deterministic_access_set();
    assert!(
        !left_access
            .writes
            .contains(&format!("owner:{}", owner.to_hex_literal()).into_bytes())
    );
    assert!(!left_access.conflicts_with(&right_access));
}

#[test]
fn non_gas_owner_debits_still_lock_owner() {
    let owner = AccountAddress::from_hex_literal("0x42").unwrap();
    let mut left = ChangeSet::new();
    left.burn(owner, 10);
    let mut right = ChangeSet::new();
    right.burn(owner, 10);

    assert!(
        left.deterministic_access_set()
            .conflicts_with(&right.deterministic_access_set())
    );
}

#[test]
fn access_conflict_detection_matches_reference_model_for_generated_sets() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        seed
    };

    for _ in 0..512 {
        let mut left = StateAccessSet::default();
        let mut right = StateAccessSet::default();
        for key_index in 0..16u8 {
            let key = vec![key_index];
            let mask = next();
            if mask & 1 != 0 {
                left.reads.insert(key.clone());
            }
            if mask & 2 != 0 {
                left.writes.insert(key.clone());
            }
            if mask & 4 != 0 {
                right.reads.insert(key.clone());
            }
            if mask & 8 != 0 {
                right.writes.insert(key);
            }
        }

        let expected = left
            .writes
            .iter()
            .any(|key| right.reads.contains(key) || right.writes.contains(key))
            || right.writes.iter().any(|key| left.reads.contains(key));
        assert_eq!(left.conflicts_with(&right), expected);
        assert_eq!(left.conflicts_with(&right), right.conflicts_with(&left));
    }
}

#[test]
fn failed_effects_use_canonical_failed_status() {
    let mut changeset = ChangeSet::new();
    changeset.mark_failed("expected test failure".to_string());

    let effects = changeset.effects(None);
    assert_eq!(effects.status, "failed");
    assert_eq!(
        effects.error_message.as_deref(),
        Some("expected test failure")
    );
}
