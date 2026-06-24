use super::*;
use kanari_types::transaction::Transaction;

fn native_transfer(sender: &str, recipient: &str, sequence_number: u64) -> SignedTransaction {
    SignedTransaction::new(Transaction::ExecuteFunction {
        sender: sender.to_string(),
        module: Transaction::KANARI_MODULE.to_string(),
        function: Transaction::TRANSFER_AMOUNT_FUNCTION.to_string(),
        type_args: vec![],
        args: vec![
            bcs::to_bytes(&1u64).unwrap(),
            bcs::to_bytes(&recipient.to_string()).unwrap(),
        ],
        gas_limit: 1_000,
        gas_price: 0,
        sequence_number,
    })
}

fn generic_move_call(sender: &str, sequence_number: u64) -> SignedTransaction {
    SignedTransaction::new(Transaction::ExecuteFunction {
        sender: sender.to_string(),
        module: "0x42::arbitrary".to_string(),
        function: "mutate_hidden_global".to_string(),
        type_args: vec![],
        args: vec![],
        gas_limit: 1_000,
        gas_price: 0,
        sequence_number,
    })
}

#[test]
fn independent_native_transfers_share_waves() {
    let waves = TransactionScheduler::schedule(vec![
        native_transfer("0x1", "0x11", 0),
        native_transfer("0x2", "0x22", 0),
        native_transfer("0x3", "0x1", 0),
        native_transfer("0x4", "0x2", 0),
    ]);

    assert_eq!(waves.len(), 2);
    assert_eq!(waves[0].len(), 2);
    assert_eq!(waves[1].len(), 2);
}

#[test]
fn sender_sequence_is_always_serialized() {
    let waves = TransactionScheduler::schedule(vec![
        native_transfer("0x1", "0x11", 0),
        native_transfer("0x1", "0x12", 1),
        native_transfer("0x1", "0x13", 2),
    ]);

    assert_eq!(waves.len(), 3);
    assert!(waves.iter().all(|wave| wave.len() == 1));
}

#[test]
fn unknown_move_access_set_is_a_global_serial_barrier() {
    let waves = TransactionScheduler::schedule(vec![
        native_transfer("0x1", "0x11", 0),
        native_transfer("0x2", "0x22", 0),
        generic_move_call("0x3", 0),
        native_transfer("0x4", "0x44", 0),
        native_transfer("0x5", "0x55", 0),
        generic_move_call("0x6", 0),
    ]);

    assert_eq!(waves.len(), 4);
    assert_eq!(waves[0].len(), 2);
    assert_eq!(waves[1].len(), 1);
    assert_eq!(waves[2].len(), 2);
    assert_eq!(waves[3].len(), 1);
}

#[test]
fn module_publish_is_a_serial_barrier() {
    let publish = SignedTransaction::new(Transaction::PublishModule {
        sender: "0x3".to_string(),
        module_bytes: vec![1, 2, 3],
        module_name: "m".to_string(),
        gas_limit: 1_000,
        gas_price: 0,
        sequence_number: 0,
    });
    let waves = TransactionScheduler::schedule(vec![
        native_transfer("0x1", "0x11", 0),
        native_transfer("0x2", "0x22", 0),
        publish,
        native_transfer("0x4", "0x44", 0),
    ]);

    assert_eq!(waves.len(), 3);
    assert_eq!(waves[0].len(), 2);
    assert_eq!(waves[1].len(), 1);
    assert_eq!(waves[2].len(), 1);
}
