from __future__ import annotations

from pathlib import Path


def replace_test(name: str, replacement: str) -> None:
    path = Path("crates/kanari-core/tests/unit/engine_tests.rs")
    text = path.read_text()
    marker = f"fn {name}("
    fn_pos = text.find(marker)
    if fn_pos < 0:
        raise RuntimeError(f"test {name} not found")
    start = text.rfind("#[test]", 0, fn_pos)
    brace = text.find("{", fn_pos)
    depth = 0
    end = None
    for index in range(brace, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                end = index + 1
                break
    if start < 0 or end is None:
        raise RuntimeError(f"cannot parse test {name}")
    path.write_text(text[:start] + replacement.strip() + "\n" + text[end:])


replace_test(
    "legacy_native_transfer_is_rejected_before_execution",
    r'''
#[test]
fn legacy_native_transfer_fails_without_moving_funds() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let sender_address = AccountAddress::from_hex_literal(&sender.address).unwrap();
    let tx = Transaction::new_transfer(sender.tagged_address(), recipient.address, 1, 0);
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();

    let (_, changeset) = engine.execute_transaction_immediate(signed_tx).unwrap();
    assert!(!changeset.success);
    assert!(
        changeset
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("NUMBER_OF_ARGUMENTS_MISMATCH"))
    );
    let sender_change = changeset.account_changes.get(&sender_address).unwrap();
    assert_eq!(sender_change.balance_delta, 0);
    assert_eq!(sender_change.sequence_increment, 1);
    assert!(changeset.created_objects.is_empty());
}
''',
)

replace_test(
    "rejected_legacy_transfer_does_not_change_state",
    r'''
#[test]
fn failed_legacy_transfer_does_not_change_balances_or_supply() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender(&engine, &sender.address, 1_000_000);
    let sender_address = AccountAddress::from_hex_literal(&sender.address).unwrap();
    let recipient_address = AccountAddress::from_hex_literal(&recipient.address).unwrap();
    let supply_before = engine.state_read().total_supply;

    let tx = Transaction::new_transfer(sender.tagged_address(), recipient.address, 10, 0);
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let (_, changeset) = engine.execute_transaction_immediate(signed_tx).unwrap();
    assert!(!changeset.success);

    let mut state = engine.state_write();
    state.apply_changeset(&changeset).unwrap();
    assert_eq!(state.get_account(&sender_address).unwrap().native_balance(), 1_000_000);
    assert_eq!(
        state
            .get_account(&recipient_address)
            .map(|account| account.native_balance())
            .unwrap_or(0),
        0
    );
    assert_eq!(state.total_supply, supply_before);
    assert_eq!(state.get_account(&sender_address).unwrap().sequence_number, 1);
    state.validate_supply_invariants().unwrap();
}
''',
)

replace_test(
    "legacy_self_transfer_is_rejected_and_supply_remains_valid",
    r'''
#[test]
fn failed_legacy_self_transfer_only_advances_sequence() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender(&engine, &sender.address, 1_000_000);
    let sender_address = AccountAddress::from_hex_literal(&sender.address).unwrap();
    let supply_before = engine.state_read().total_supply;

    let tx = Transaction::new_transfer(sender.tagged_address(), sender.address.clone(), 1, 0);
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let (_, changeset) = engine.execute_transaction_immediate(signed_tx).unwrap();
    assert!(!changeset.success);

    let mut state = engine.state_write();
    state.apply_changeset(&changeset).unwrap();
    let account = state.get_account(&sender_address).unwrap();
    assert_eq!(account.native_balance(), 1_000_000);
    assert_eq!(account.sequence_number, 1);
    assert_eq!(state.total_supply, supply_before);
    state.validate_supply_invariants().unwrap();
}
''',
)

print("security test migration round 2 applied")
