use kanari_types::address::Address as KanariAddress;
use kanari_types::error::KanariUnwrapExt;

use super::*;

#[test]
fn test_changeset_transfer() {
    let mut cs = ChangeSet::new();
    let from = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)
        .invariant("valid standard address");
    let to = AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS)
        .invariant("valid system address");

    cs.transfer(from, to, 100);

    assert_eq!(cs.account_changes.len(), 2);
    assert_eq!(
        cs.account_changes
            .get(&from)
            .invariant("sender change missing")
            .balance_delta,
        -100
    );
    assert_eq!(
        cs.account_changes
            .get(&to)
            .invariant("recipient change missing")
            .balance_delta,
        100
    );
    assert_eq!(
        cs.account_changes
            .get(&from)
            .invariant("sender change missing")
            .sequence_increment,
        1
    );
}

#[test]
fn test_changeset_mint() {
    let mut cs = ChangeSet::new();
    let to = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)
        .invariant("valid standard address");

    cs.mint(to, 1000);

    assert_eq!(cs.account_changes.len(), 1);
    assert_eq!(
        cs.account_changes
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

    assert_eq!(cs.account_changes.len(), 1);
    assert_eq!(
        cs.account_changes
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
        .account_changes
        .get(&publisher)
        .invariant("publisher change missing");
    assert_eq!(change.modules_added.len(), 1);
    assert!(change.modules_added.contains("kanari"));
    // Note: sequence_increment is NOT set by publish_module - it's handled by engine
    assert_eq!(change.sequence_increment, 0);
}
