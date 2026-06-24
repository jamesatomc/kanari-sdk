#!/usr/bin/env python3
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]


def replace_once(relative: str, old: str, new: str) -> None:
    path = ROOT / relative
    text = path.read_text()
    if new in text and old not in text:
        return
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{relative}: expected one source match, found {count}\n{old}")
    path.write_text(text.replace(old, new, 1))


def replace_all(relative: str, old: str, new: str, minimum: int = 1) -> None:
    path = ROOT / relative
    text = path.read_text()
    if old not in text and new in text:
        return
    count = text.count(old)
    if count < minimum:
        raise RuntimeError(f"{relative}: expected at least {minimum} matches, found {count}: {old}")
    path.write_text(text.replace(old, new))


def append_once(relative: str, marker: str, content: str) -> None:
    path = ROOT / relative
    text = path.read_text()
    if marker in text:
        return
    path.write_text(text.rstrip() + "\n\n" + content.strip() + "\n")


shutil.copyfile(
    ROOT / "scripts/fix-gas-templates/apply_checkpoint.rs",
    ROOT / "crates/kanari-core/src/engine/apply_checkpoint.rs",
)

replace_all(
    "crates/kanari-core/src/engine.rs",
    "runtime.persist_created_objects(&changeset);",
    "runtime.persist_created_objects(&changeset)?;",
    minimum=1,
)
replace_all(
    "crates/kanari-core/src/engine.rs",
    "runtime.persist_deleted_objects(&changeset);",
    "runtime.persist_deleted_objects(&changeset)?;",
    minimum=1,
)
replace_all(
    "crates/kanari-core/src/engine.rs",
    "runtime.persist_created_objects(&cs);",
    "runtime.persist_created_objects(&cs)?;",
    minimum=1,
)
replace_all(
    "crates/kanari-core/src/engine.rs",
    "runtime.persist_deleted_objects(&cs);",
    "runtime.persist_deleted_objects(&cs)?;",
    minimum=1,
)

replace_once(
    "crates/kanari-core/src/engine.rs",
    """                    KanariAddress::parse_to_account_address(sender)?,\n                    None,\n                    timestamp,\n""",
    """                    KanariAddress::parse_to_account_address(sender)?,\n                    Some((tx.gas_limit(), tx.gas_price())),\n                    timestamp,\n""",
)
replace_once(
    "crates/kanari-core/src/engine.rs",
    """                    args.clone(),\n                    Some(sender_addr),\n                    None,\n                    timestamp,\n""",
    """                    args.clone(),\n                    Some(sender_addr),\n                    Some((tx.gas_limit(), tx.gas_price())),\n                    timestamp,\n""",
)
replace_once(
    "crates/kanari-core/src/engine.rs",
    """        Self::apply_gas_and_sequence(&mut changeset, sender_addr, gas_cost, gas_meter.gas_used)?;\n        Ok(changeset)\n""",
    """        let final_gas_used = changeset.gas_used.max(gas_meter.gas_used);\n        let final_gas_cost = final_gas_used\n            .checked_mul(tx.gas_price())\n            .ok_or_else(|| anyhow::anyhow!(\"Gas cost overflow\"))?;\n        Self::apply_gas_and_sequence(\n            &mut changeset,\n            sender_addr,\n            final_gas_cost,\n            final_gas_used,\n        )?;\n        Ok(changeset)\n""",
)
replace_once(
    "crates/kanari-core/src/engine.rs",
    """        changeset.collect_gas(dao_addr, gas_cost);\n        changeset.set_gas_used(gas_used);\n""",
    """        changeset.collect_gas(dao_addr, gas_cost);\n        changeset.set_gas_used(changeset.gas_used.max(gas_used));\n""",
)

# Genesis is an explicit persistence path and must not discard storage errors.
replace_all(
    "move-execution/v1/kanari-move-runtime-v1/src/genesis.rs",
    "runtime.persist_created_objects(&changeset);",
    "runtime.persist_created_objects(&changeset)?;",
    minimum=1,
)
replace_all(
    "move-execution/v1/kanari-move-runtime-v1/src/genesis.rs",
    "runtime.persist_deleted_objects(&changeset);",
    "runtime.persist_deleted_objects(&changeset)?;",
    minimum=1,
)

produce = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
replace_once(
    produce,
    """struct StagedCheckpoint {\n    checkpoint: Checkpoint,\n    verified_state: StateManager,\n    to_execute: Vec<SignedTransaction>,\n    receipts: Vec<TransactionExecutionReceipt>,\n""",
    """struct StagedCheckpoint {\n    checkpoint: Checkpoint,\n    verified_state: StateManager,\n    receipts: Vec<TransactionExecutionReceipt>,\n""",
)
replace_once(
    produce,
    """    pub fn produce_vertex(&self) -> Result<CheckpointProductionInfo> {\n""",
    """    fn select_checkpoint_transactions_with_budget(\n        mut transactions: Vec<SignedTransaction>,\n        block_gas_budget: u64,\n    ) -> Result<Vec<SignedTransaction>> {\n        transactions.sort_by(|a, b| {\n            a.transaction\n                .sender_address()\n                .cmp(b.transaction.sender_address())\n                .then_with(|| {\n                    a.transaction\n                        .sequence_number()\n                        .cmp(&b.transaction.sequence_number())\n                })\n                .then_with(|| a.transaction_hash().cmp(b.transaction_hash()))\n        });\n\n        let mut remaining = block_gas_budget;\n        let mut blocked_senders = HashSet::new();\n        let mut selected = Vec::new();\n\n        for tx in transactions {\n            let sender = tx.transaction.sender_address().to_string();\n            if blocked_senders.contains(&sender) {\n                continue;\n            }\n            BlockchainEngine::validate_transaction_gas(&tx.transaction)?;\n\n            // Publishing mutates the resolver/module cache. Keep it in a\n            // checkpoint by itself until same-checkpoint overlay resolution is\n            // explicitly supported. This is deterministic and fail-closed.\n            if matches!(tx.transaction, Transaction::PublishModule { .. }) {\n                if selected.is_empty() {\n                    selected.push(tx);\n                }\n                break;\n            }\n\n            let reserved = tx.transaction.gas_limit();\n            if reserved > remaining {\n                // Preserve this sender's sequence prefix, but continue looking\n                // for independent senders that fit the remaining budget.\n                blocked_senders.insert(sender);\n                continue;\n            }\n            remaining -= reserved;\n            selected.push(tx);\n        }\n\n        anyhow::ensure!(\n            !selected.is_empty(),\n            \"No pending transaction fits the checkpoint gas budget\"\n        );\n        Ok(selected)\n    }\n\n    fn select_checkpoint_transactions(\n        transactions: Vec<SignedTransaction>,\n    ) -> Result<Vec<SignedTransaction>> {\n        Self::select_checkpoint_transactions_with_budget(\n            transactions,\n            GasConfig::default().max_gas_per_block,\n        )\n    }\n\n    pub fn produce_vertex(&self) -> Result<CheckpointProductionInfo> {\n""",
)
replace_once(
    produce,
    """        let mut transactions = self.engine.pending_transactions_snapshot();\n        transactions.sort_by(|a, b| {\n            a.transaction\n                .sender_address()\n                .cmp(b.transaction.sender_address())\n                .then_with(|| {\n                    a.transaction\n                        .sequence_number()\n                        .cmp(&b.transaction.sequence_number())\n                })\n                .then_with(|| a.transaction_hash().cmp(b.transaction_hash()))\n        });\n        let tx_count = transactions.len();\n        if tx_count == 0 {\n            anyhow::bail!(\"No new transactions to checkpoint\");\n        }\n""",
    """        let pending = self.engine.pending_transactions_snapshot();\n        if pending.is_empty() {\n            anyhow::bail!(\"No new transactions to checkpoint\");\n        }\n        let transactions = Self::select_checkpoint_transactions(pending)?;\n        let tx_count = transactions.len();\n""",
)
replace_once(
    produce,
    """        let (state_root, executed, failed, verified_state, to_execute, receipts, validate_supply) = {\n""",
    """        let (state_root, executed, failed, verified_state, receipts, validate_supply) = {\n""",
)
replace_once(
    produce,
    """                verified_state,\n                if validate_supply {\n                    transactions.clone()\n                } else {\n                    Vec::new()\n                },\n                execution.receipts,\n""",
    """                verified_state,\n                execution.receipts,\n""",
)
replace_once(
    produce,
    """            verified_state,\n            to_execute,\n            receipts,\n""",
    """            verified_state,\n            receipts,\n""",
)
replace_once(
    produce,
    """        verified_state: StateManager,\n        to_execute: Vec<SignedTransaction>,\n        receipts: Vec<TransactionExecutionReceipt>,\n""",
    """        verified_state: StateManager,\n        receipts: Vec<TransactionExecutionReceipt>,\n""",
)
replace_once(
    produce,
    """                checkpoint: checkpoint.clone(),\n                verified_state,\n                to_execute,\n                receipts,\n""",
    """                checkpoint: checkpoint.clone(),\n                verified_state,\n                receipts,\n""",
)
replace_once(
    produce,
    """            staged.checkpoint.clone(),\n            staged.verified_state,\n            staged.to_execute,\n            staged.receipts,\n""",
    """            staged.checkpoint.clone(),\n            staged.verified_state,\n            staged.receipts,\n""",
)

append_once(
    produce,
    "mod checkpoint_gas_tests",
    r'''
#[cfg(test)]
mod checkpoint_gas_tests {
    use super::*;

    fn tx(sender: &str, sequence: u64, gas_limit: u64) -> SignedTransaction {
        SignedTransaction::new(Transaction::ExecuteFunction {
            sender: sender.to_string(),
            module: Transaction::KANARI_MODULE.to_string(),
            function: Transaction::BURN_AMOUNT_FUNCTION.to_string(),
            type_args: vec![],
            args: vec![bcs::to_bytes(&0u64).unwrap()],
            gas_limit,
            gas_price: 0,
            sequence_number: sequence,
        })
    }

    #[test]
    fn checkpoint_budget_does_not_head_of_line_block_other_senders() {
        let selected = DagEngine::select_checkpoint_transactions_with_budget(
            vec![
                tx("0x1", 0, 900),
                tx("0x1", 1, 200),
                tx("0x2", 0, 100),
            ],
            1_000,
        )
        .unwrap();

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].transaction.sender_address(), "0x1");
        assert_eq!(selected[1].transaction.sender_address(), "0x2");
    }

    #[test]
    fn checkpoint_budget_preserves_sender_sequence_prefix() {
        let selected = DagEngine::select_checkpoint_transactions_with_budget(
            vec![
                tx("0x1", 0, 800),
                tx("0x2", 0, 300),
                tx("0x2", 1, 100),
            ],
            1_000,
        )
        .unwrap();

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].transaction.sender_address(), "0x1");
    }
}
''',
)

print("patched core gas flow and atomic checkpoint finalization")
