use super::*;
use kanari_types::transaction::Transaction;

fn create_dummy_tx(sender: &str, module: &str, object: Option<&str>) -> SignedTransaction {
    let mut args = Vec::new();
    if let Some(obj) = object {
        // Mock object ID as 32 bytes
        let mut id = vec![0u8; 32];
        // Fill with object string bytes for uniqueness (simplified)
        let bytes = obj.as_bytes();
        for (i, b) in bytes.iter().enumerate().take(32) {
            id[i] = *b;
        }
        args.push(id);
    }

    let tx = Transaction::ExecuteFunction {
        sender: sender.to_string(),
        module: module.to_string(),
        function: "test".to_string(),
        type_args: vec![],
        args,
        gas_limit: 1000,
        gas_price: 1,
        sequence_number: 0,
    };
    SignedTransaction::new(tx)
}

#[test]
fn test_schedule_parallel() {
    // Tx1: A -> uses Obj1
    // Tx2: B -> uses Obj2
    // Tx3: C -> uses Obj1
    // Tx4: D -> uses Obj2

    // Expected:
    // Wave 0: Tx1, Tx2 (independent)
    // Wave 1: Tx3 (conflicts with Tx1), Tx4 (conflicts with Tx2)

    // We use different modules to avoid module-level conflicts
    let tx1 = create_dummy_tx("A", "M1", Some("Obj1"));
    let tx2 = create_dummy_tx("B", "M2", Some("Obj2"));
    let tx3 = create_dummy_tx("C", "M3", Some("Obj1"));
    let tx4 = create_dummy_tx("D", "M4", Some("Obj2"));

    let txs = vec![tx1, tx2, tx3, tx4];
    let waves = TransactionScheduler::schedule(txs);

    assert_eq!(waves.len(), 2);
    assert_eq!(waves[0].len(), 2); // Tx1, Tx2
    assert_eq!(waves[1].len(), 2); // Tx3, Tx4
}

#[test]
fn test_schedule_chain() {
    // Tx1: A
    // Tx2: A (depends on Tx1)
    // Tx3: A (depends on Tx2)

    let tx1 = create_dummy_tx("A", "M1", None);
    let tx2 = create_dummy_tx("A", "M2", None);
    let tx3 = create_dummy_tx("A", "M3", None);

    let txs = vec![tx1, tx2, tx3];
    let waves = TransactionScheduler::schedule(txs);

    assert_eq!(waves.len(), 3);
    assert_eq!(waves[0].len(), 1);
    assert_eq!(waves[1].len(), 1);
    assert_eq!(waves[2].len(), 1);
}

#[test]
fn test_schedule_complex() {
    // Tx1: A (Obj1)
    // Tx2: B (Obj1) -> Conflicts with Tx1
    // Tx3: C (Obj2) -> Independent
    // Tx4: D (Obj1) -> Conflicts with Tx2
    // Tx5: E (Obj2) -> Conflicts with Tx3

    // Expected:
    // Wave 0: Tx1, Tx3
    // Wave 1: Tx2, Tx5
    // Wave 2: Tx4

    let tx1 = create_dummy_tx("A", "M1", Some("Obj1"));
    let tx2 = create_dummy_tx("B", "M2", Some("Obj1"));
    let tx3 = create_dummy_tx("C", "M3", Some("Obj2"));
    let tx4 = create_dummy_tx("D", "M4", Some("Obj1"));
    let tx5 = create_dummy_tx("E", "M5", Some("Obj2"));

    let txs = vec![tx1, tx2, tx3, tx4, tx5];
    let waves = TransactionScheduler::schedule(txs);

    assert_eq!(waves.len(), 3);
    assert_eq!(waves[0].len(), 2); // Tx1, Tx3
    assert_eq!(waves[1].len(), 2); // Tx2, Tx5
    assert_eq!(waves[2].len(), 1); // Tx4
}
