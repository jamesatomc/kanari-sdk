from __future__ import annotations

from pathlib import Path


def replace_test(path: str, name: str, replacement: str) -> None:
    file = Path(path)
    text = file.read_text()
    marker = f"fn {name}("
    fn_pos = text.find(marker)
    if fn_pos < 0:
        raise RuntimeError(f"test function {name} not found in {path}")
    start = text.rfind("#[test]", 0, fn_pos)
    if start < 0:
        raise RuntimeError(f"#[test] for {name} not found")
    brace = text.find("{", fn_pos)
    depth = 0
    end = None
    for index in range(brace, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                end = index + 1
                break
    if end is None:
        raise RuntimeError(f"unterminated function {name}")
    file.write_text(text[:start] + replacement.strip() + "\n" + text[end:])


ENGINE_TESTS = "crates/kanari-core/tests/unit/engine_tests.rs"

replace_test(
    ENGINE_TESTS,
    "configured_dag_engine_rejects_empty_checkpoint",
    r'''
#[test]
fn configured_dag_engine_progresses_without_finalizing_empty_checkpoint() {
    let mut engine = BlockchainEngine::new_in_memory().unwrap();
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
    engine.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    engine
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();

    match engine.produce_checkpoint() {
        Ok(info) => {
            assert_eq!(info.tx_count, 0);
            assert!(info.checkpoint.is_none());
        }
        Err(error) => assert!(error.to_string().contains("DAG_WAITING")),
    }
    assert_eq!(engine.get_stats().height, 0);
}
''',
)

replace_test(
    ENGINE_TESTS,
    "restarted_engine_does_not_create_empty_dag_progress",
    r'''
#[test]
fn restarted_engine_does_not_finalize_empty_dag_progress() {
    let temp_dir = tempfile::tempdir().unwrap();
    let data_dir = temp_dir.path().to_str().unwrap();
    let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];

    {
        let mut engine = BlockchainEngine::new_dir(data_dir).unwrap();
        if engine.persistent_store.is_none() {
            return;
        }
        engine.set_authorities("0x1".to_string(), authorities.clone());
        let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
        engine
            .set_consensus_signing_key(local_key, public_keys)
            .unwrap();
        let _ = engine.produce_checkpoint();
        assert_eq!(engine.get_stats().height, 0);
    }

    let mut restarted = BlockchainEngine::new_dir(data_dir).unwrap();
    if restarted.persistent_store.is_none() {
        return;
    }
    restarted.set_authorities("0x1".to_string(), authorities.clone());
    let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
    restarted
        .set_consensus_signing_key(local_key, public_keys)
        .unwrap();

    assert_eq!(restarted.get_stats().pending_transactions, 0);
    assert_eq!(restarted.get_stats().height, 0);
    let _ = restarted.produce_checkpoint();
    assert_eq!(restarted.get_stats().height, 0);
}
''',
)

replace_test(
    ENGINE_TESTS,
    "gas_validation_rejects_overflowing_gas_cost",
    r'''
#[test]
fn gas_validation_rejects_nonzero_protocol_price() {
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = Transaction::ExecuteFunction {
        sender: sender.tagged_address(),
        module: "0x2::test_support".to_string(),
        function: "noop".to_string(),
        type_args: vec![],
        args: vec![],
        gas_limit: kanari_types::gas::GasConfig::default().default_transaction_gas_limit(),
        gas_price: u64::MAX,
        sequence_number: 0,
    };
    let error = BlockchainEngine::validate_transaction_gas(&tx).unwrap_err();
    assert!(error.to_string().contains("Invalid gas price"));
}
''',
)

replace_test(
    ENGINE_TESTS,
    "batch_submit_rejects_gas_price_below_minimum",
    r'''
#[test]
fn batch_submit_accepts_protocol_zero_gas_price() {
    let engine = BlockchainEngine::new().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = Transaction::ExecuteFunction {
        sender: sender.tagged_address(),
        module: "0x2::test_support".to_string(),
        function: "noop".to_string(),
        type_args: vec![],
        args: vec![],
        gas_limit: 100_000,
        gas_price: 0,
        sequence_number: 0,
    };
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    assert!(engine.submit_transactions_batch(vec![signed_tx]).is_ok());
}
''',
)

replace_test(
    ENGINE_TESTS,
    "batch_submit_rejects_gas_limit_below_operation_cost",
    r'''
#[test]
fn batch_submit_rejects_gas_limit_below_operation_cost() {
    let engine = BlockchainEngine::new().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = Transaction::ExecuteFunction {
        sender: sender.tagged_address(),
        module: "0x2::test_support".to_string(),
        function: "noop".to_string(),
        type_args: vec![],
        args: vec![],
        gas_limit: 1,
        gas_price: 0,
        sequence_number: 0,
    };
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let error = engine
        .submit_transactions_batch(vec![signed_tx])
        .unwrap_err();
    assert!(error.to_string().contains("below required operation cost"));
}
''',
)

replace_test(
    ENGINE_TESTS,
    "deterministic_parallel_execution_matches_strict_serial_root",
    r'''
#[test]
fn deterministic_parallel_execution_matches_strict_serial_root() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let txs = (0..16)
        .map(|_| {
            let sender = generate_keypair(CurveType::Ed25519).unwrap();
            signed_transfer_from(&sender, 0)
        })
        .collect::<Vec<_>>();

    let base_state = engine.state_read().clone();
    let strict_state = Arc::new(RwLock::new(base_state.clone()));
    let parallel_state = Arc::new(RwLock::new(base_state));

    let strict_counts = engine
        .execute_tx_waves_parallel(txs.clone(), &strict_state, Some(123), false, true)
        .unwrap();
    let parallel_counts = engine
        .execute_tx_waves_deterministic_parallel(txs, &parallel_state, Some(123), false)
        .unwrap();

    let strict_root = strict_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();
    let parallel_root = parallel_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .compute_state_root();

    assert_eq!(strict_counts, parallel_counts);
    assert_eq!(strict_root, parallel_root);
}
''',
)

replace_test(
    ENGINE_TESTS,
    "native_transfer_charges_gas_from_gas_module",
    r'''
#[test]
fn legacy_native_transfer_is_rejected_before_execution() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    let tx = Transaction::new_transfer(sender.tagged_address(), recipient.address, 1, 0);
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let error = engine.execute_transaction_immediate(signed_tx).unwrap_err();
    assert!(error.to_string().contains("legacy account-ledger KANARI"));
}
''',
)

replace_test(
    ENGINE_TESTS,
    "applied_native_transfer_debits_sender_fee_and_credits_dao",
    r'''
#[test]
fn rejected_legacy_transfer_does_not_change_state() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let recipient = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender(&engine, &sender.address, 1_000_000);
    let before = engine.state_read().compute_state_root();
    let tx = Transaction::new_transfer(sender.tagged_address(), recipient.address, 10, 0);
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let error = engine.execute_transaction_immediate(signed_tx).unwrap_err();
    assert!(error.to_string().contains("legacy account-ledger KANARI"));
    assert_eq!(engine.state_read().compute_state_root(), before);
}
''',
)

replace_test(
    ENGINE_TESTS,
    "self_native_transfer_only_charges_gas_and_keeps_supply_valid",
    r'''
#[test]
fn legacy_self_transfer_is_rejected_and_supply_remains_valid() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    fund_sender(&engine, &sender.address, 1_000_000);
    let tx = Transaction::new_transfer(sender.tagged_address(), sender.address.clone(), 1, 0);
    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .unwrap();
    let error = engine.execute_transaction_immediate(signed_tx).unwrap_err();
    assert!(error.to_string().contains("legacy account-ledger KANARI"));
    engine.state_read().validate_supply_invariants().unwrap();
}
''',
)

replace_test(
    ENGINE_TESTS,
    "failed_execution_produces_and_persists_receipt",
    r'''
#[test]
fn failed_execution_produces_and_persists_receipt() {
    let engine = BlockchainEngine::new_in_memory().unwrap();
    let sender = generate_keypair(CurveType::Ed25519).unwrap();
    let signed_tx = signed_transfer_from(&sender, 0);
    let tx_hash = signed_tx.transaction_hash().to_vec();
    let state = Arc::new(RwLock::new(engine.state_read().clone()));

    let execution = engine
        .execute_tx_waves_strict_serial_with_receipts(vec![signed_tx], &state, Some(123), false)
        .unwrap();

    assert_eq!(execution.executed, 0);
    assert_eq!(execution.failed, 1);
    assert_eq!(execution.receipts.len(), 1);
    assert!(!execution.receipts[0].success);
    assert!(execution.receipts[0].error_message.is_some());

    engine
        .persist_transaction_receipts(&execution.receipts)
        .unwrap();
    assert_eq!(
        engine.get_transaction_execution_receipt(&tx_hash),
        Some(execution.receipts[0].clone())
    );
}
''',
)

print("security test migration applied")
