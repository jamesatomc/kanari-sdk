#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PATH = ROOT / "crates/kanari-core/src/engine.rs"
text = PATH.read_text()


def replace_once(old: str, new: str) -> None:
    global text
    if new in text and old not in text:
        return
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"engine.rs: expected one match, found {count}\n--- needle ---\n{old}")
    text = text.replace(old, new, 1)


replace_once(
    """impl TransactionBatchExecution {
    fn counts(&self) -> (usize, usize) {
""",
    """impl TransactionBatchExecution {
    #[cfg(test)]
    fn counts(&self) -> (usize, usize) {
""",
)

replace_once(
    """    fn persist_transaction_receipts(&self, receipts: &[TransactionExecutionReceipt]) -> Result<()> {
        let store = self.state_read().store.clone();
        for receipt in receipts {
            store
                .save(
                    &Self::transaction_receipt_key(&receipt.transaction_hash),
                    receipt,
                )
                .with_context(|| {
                    format!(
                        "Failed to persist execution receipt for transaction {}",
                        hex::encode(&receipt.transaction_hash)
                    )
                })?;
        }
        Ok(())
    }

""",
    "",
)

replace_once(
    """    pub(crate) fn persist_blockchain_snapshot(&self, chain: &Blockchain) -> Result<()> {
        let Some(store) = &self.persistent_store else {
            return Ok(());
        };
        Self::persist_blockchain_snapshot_to_store(store, chain)
    }

""",
    "",
)

replace_once(
    """    pub(crate) fn execute_tx_waves_strict_serial(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
    ) -> Result<(usize, usize)> {
        Ok(self
            .execute_tx_waves_strict_serial_with_receipts(
                transactions,
                state_arc,
                timestamp,
                persist_objects,
            )?
            .counts())
    }

""",
    "",
)

PATH.write_text(text)
print("removed obsolete wrappers and test-gated receipt counters")
