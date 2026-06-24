#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_once(relative: str, old: str, new: str) -> None:
    path = ROOT / relative
    text = path.read_text()
    if new in text and old not in text:
        return
    count = text.count(old)
    if count != 1:
        raise RuntimeError(
            f"{relative}: expected one match, found {count}\n--- needle ---\n{old}"
        )
    path.write_text(text.replace(old, new, 1))


engine = "crates/kanari-core/src/engine.rs"
replace_once(
    engine,
    """        gas_meter.consume(gas_op.gas_units())?;
        let gas_cost = gas_meter.total_cost();
        let total_required = required_amount.saturating_add(gas_cost);
""",
    """        gas_meter.consume(gas_op.gas_units())?;
        let gas_cost = gas_meter.total_cost();
        // Reserve the sender's maximum signed gas liability before execution.
        // Runtime usage can increase above the static admission cost, but can
        // never exceed gas_limit; reserving the limit prevents an otherwise
        // valid checkpoint from failing later while applying the final debit.
        let max_gas_cost = tx
            .gas_limit()
            .checked_mul(tx.gas_price())
            .ok_or_else(|| anyhow::anyhow!("Gas cost overflow"))?;
        let total_required = required_amount
            .checked_add(max_gas_cost)
            .ok_or_else(|| anyhow::anyhow!("Required balance overflow"))?;
""",
)
replace_once(
    engine,
    """                            "Insufficient balance: need {} (amount: {}, gas: {}) but have {}",
                            total_required, required_amount, gas_cost, balance
""",
    """                            "Insufficient balance: need {} (amount: {}, max gas: {}) but have {}",
                            total_required, required_amount, max_gas_cost, balance
""",
)
replace_once(
    engine,
    """                            "Insufficient balance for gas: need {}, have {}",
                            gas_cost, balance
""",
    """                            "Insufficient balance for maximum gas liability: need {}, have {}",
                            max_gas_cost, balance
""",
)
replace_once(
    engine,
    """        if required_amount > (i64::MAX as u64).saturating_sub(gas_cost) {
""",
    """        if required_amount > (i64::MAX as u64).saturating_sub(max_gas_cost) {
""",
)
replace_once(
    engine,
    """                    Err(e) => {
                        changeset.mark_failed(format!("Publish failed: {}", e));
                    }
""",
    """                    Err(e) => {
                        // MoveVM currently returns an error without its consumed meter.
                        // Charge the signed limit fail-closed so an attacker cannot run
                        // to out-of-gas repeatedly while paying only admission gas.
                        changeset.set_gas_used(tx.gas_limit());
                        changeset.mark_failed(format!("Publish failed: {}", e));
                    }
""",
)
replace_once(
    engine,
    """                    Err(e) => {
                        changeset.mark_failed(format!("Execution failed: {}", e));
                    }
""",
    """                    Err(e) => {
                        // Preserve deterministic anti-DoS accounting even though
                        // the runtime error path cannot return its internal meter.
                        changeset.set_gas_used(tx.gas_limit());
                        changeset.mark_failed(format!("Execution failed: {}", e));
                    }
""",
)

apply_checkpoint = "crates/kanari-core/src/engine/apply_checkpoint.rs"
replace_once(
    apply_checkpoint,
    """    pub(crate) fn prepare_checkpoint_state(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<PreparedCheckpointState> {
        let state_snapshot = self.state_read().clone();
""",
    """    fn validate_checkpoint_execution_shape(
        transactions: &[SignedTransaction],
    ) -> Result<()> {
        let opaque_count = transactions
            .iter()
            .filter(|tx| !tx.transaction.has_complete_conflict_set())
            .count();
        anyhow::ensure!(
            opaque_count == 0 || (opaque_count == 1 && transactions.len() == 1),
            "Checkpoint contains an opaque Move transaction mixed with other transactions"
        );
        Ok(())
    }

    pub(crate) fn prepare_checkpoint_state(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<PreparedCheckpointState> {
        Self::validate_checkpoint_execution_shape(&checkpoint.transactions)?;
        let state_snapshot = self.state_read().clone();
""",
)
replace_once(
    apply_checkpoint,
    """    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {
        info!(
            "[ENGINE] Applying checkpoint {} with {} txs",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        let (computed_root, verified_state, receipts) =
            self.prepare_checkpoint_state(&checkpoint)?;
        self.ensure_checkpoint_root_matches(&checkpoint, &computed_root)?;
        self.apply_prepared_checkpoint(checkpoint, verified_state, receipts, true)
    }
}
""",
    """    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {
        info!(
            "[ENGINE] Applying checkpoint {} with {} txs",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        let (computed_root, verified_state, receipts) =
            self.prepare_checkpoint_state(&checkpoint)?;
        self.ensure_checkpoint_root_matches(&checkpoint, &computed_root)?;
        self.apply_prepared_checkpoint(checkpoint, verified_state, receipts, true)
    }
}

#[cfg(test)]
mod checkpoint_shape_tests {
    use super::*;
    use kanari_types::transaction::Transaction;

    fn native_tx(sender: &str) -> SignedTransaction {
        SignedTransaction::new(Transaction::ExecuteFunction {
            sender: sender.to_string(),
            module: Transaction::KANARI_MODULE.to_string(),
            function: Transaction::BURN_AMOUNT_FUNCTION.to_string(),
            type_args: vec![],
            args: vec![bcs::to_bytes(&0u64).unwrap()],
            gas_limit: 1_000,
            gas_price: 0,
            sequence_number: 0,
        })
    }

    fn opaque_tx(sender: &str) -> SignedTransaction {
        SignedTransaction::new(Transaction::ExecuteFunction {
            sender: sender.to_string(),
            module: "0x42::opaque".to_string(),
            function: "touch_global".to_string(),
            type_args: vec![],
            args: vec![],
            gas_limit: 1_000,
            gas_price: 0,
            sequence_number: 0,
        })
    }

    #[test]
    fn accepts_native_only_checkpoint() {
        BlockchainEngine::validate_checkpoint_execution_shape(&[
            native_tx("0x1"),
            native_tx("0x2"),
        ])
        .unwrap();
    }

    #[test]
    fn accepts_single_opaque_transaction() {
        BlockchainEngine::validate_checkpoint_execution_shape(&[opaque_tx("0x1")]).unwrap();
    }

    #[test]
    fn rejects_opaque_transaction_mixed_with_other_work() {
        let error = BlockchainEngine::validate_checkpoint_execution_shape(&[
            opaque_tx("0x1"),
            native_tx("0x2"),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("opaque Move transaction"));
    }
}
""",
)

produce = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
replace_once(
    produce,
    """            // Publishing mutates the resolver/module cache. Keep it in a
            // checkpoint by itself until same-checkpoint overlay resolution is
            // explicitly supported. This is deterministic and fail-closed.
            if matches!(tx.transaction, Transaction::PublishModule { .. }) {
""",
    """            // Arbitrary Move execution has an opaque global/resource access
            // set and the runtime resolver reads committed state rather than the
            // speculative checkpoint overlay. Isolate it until overlay-aware
            // resolution is supported. This is deterministic and fail-closed.
            if !tx.transaction.has_complete_conflict_set() {
""",
)
replace_once(
    produce,
    """    fn tx(sender: &str, sequence: u64, gas_limit: u64) -> SignedTransaction {
        SignedTransaction::new(Transaction::ExecuteFunction {
            sender: sender.to_string(),
            module: Transaction::KANARI_MODULE.to_string(),
            function: Transaction::BURN_AMOUNT_FUNCTION.to_string(),
            type_args: vec![],
            args: vec![bcs::to_bytes(&0u64).unwrap()],
            gas_limit,
            gas_price: GasConfig::default().min_gas_price,
            sequence_number: sequence,
        })
    }

""",
    """    fn tx(sender: &str, sequence: u64, gas_limit: u64) -> SignedTransaction {
        SignedTransaction::new(Transaction::ExecuteFunction {
            sender: sender.to_string(),
            module: Transaction::KANARI_MODULE.to_string(),
            function: Transaction::BURN_AMOUNT_FUNCTION.to_string(),
            type_args: vec![],
            args: vec![bcs::to_bytes(&0u64).unwrap()],
            gas_limit,
            gas_price: GasConfig::default().min_gas_price,
            sequence_number: sequence,
        })
    }

    fn opaque_tx(sender: &str, gas_limit: u64) -> SignedTransaction {
        SignedTransaction::new(Transaction::ExecuteFunction {
            sender: sender.to_string(),
            module: "0x42::opaque".to_string(),
            function: "touch_global".to_string(),
            type_args: vec![],
            args: vec![],
            gas_limit,
            gas_price: GasConfig::default().min_gas_price,
            sequence_number: 0,
        })
    }

""",
)
replace_once(
    produce,
    """    #[test]
    fn checkpoint_budget_preserves_sender_sequence_prefix() {
        let selected = DagEngine::select_checkpoint_transactions_with_budget(
            vec![tx("0x1", 0, 800), tx("0x2", 0, 300), tx("0x2", 1, 100)],
            1_000,
        )
        .unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].transaction.sender_address(), "0x1");
    }
}
""",
    """    #[test]
    fn checkpoint_budget_preserves_sender_sequence_prefix() {
        let selected = DagEngine::select_checkpoint_transactions_with_budget(
            vec![tx("0x1", 0, 800), tx("0x2", 0, 300), tx("0x2", 1, 100)],
            1_000,
        )
        .unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].transaction.sender_address(), "0x1");
    }

    #[test]
    fn opaque_move_transaction_isolated_in_its_checkpoint() {
        let selected = DagEngine::select_checkpoint_transactions_with_budget(
            vec![opaque_tx("0x1", 500), tx("0x2", 0, 100)],
            1_000,
        )
        .unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].transaction.sender_address(), "0x1");
        assert!(!selected[0].transaction.has_complete_conflict_set());
    }
}
""",
)

print("patched gas accounting and opaque Move checkpoint isolation")
