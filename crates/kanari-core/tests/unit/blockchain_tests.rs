use super::*;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::transaction::Transaction;

fn test_tx(sequence_number: u64) -> SignedTransaction {
    SignedTransaction::new(Transaction::new_transfer(
        "0x1".to_string(),
        "0x2".to_string(),
        1,
        sequence_number,
    ))
}

#[test]
fn transaction_count_uses_queryable_index_not_stale_snapshot_counter() {
    let mut chain = Blockchain::new();
    let checkpoint = Checkpoint::new(
        1,
        vec![[1u8; 32]],
        vec![test_tx(0), test_tx(1)],
        vec![2u8; 32],
        1,
        chain
            .latest_checkpoint()
            .hash()
            .invariant("checkpoint hash"),
    );
    chain
        .add_checkpoint_with_validation(checkpoint, false)
        .invariant("test operation");
    chain.total_transaction_count = 99;

    assert_eq!(chain.get_transaction_count(), 2);
}
