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
    """        gas_meter.consume(gas_op.gas_units())?;\n        let gas_cost = gas_meter.total_cost();\n        let total_required = required_amount.saturating_add(gas_cost);\n""",
    """        gas_meter.consume(gas_op.gas_units())?;\n        let gas_cost = gas_meter.total_cost();\n        // Reserve the sender's maximum signed gas liability before execution.\n        // Runtime usage can increase above the static admission cost, but can\n        // never exceed gas_limit; reserving the limit prevents an otherwise\n        // valid checkpoint from failing later while applying the final debit.\n        let max_gas_cost = tx\n            .gas_limit()\n            .checked_mul(tx.gas_price())\n            .ok_or_else(|| anyhow::anyhow!(\"Gas cost overflow\"))?;\n        let total_required = required_amount\n            .checked_add(max_gas_cost)\n            .ok_or_else(|| anyhow::anyhow!(\"Required balance overflow\"))?;\n""",
)
replace_once(
    """                            \"Insufficient balance: need {} (amount: {}, gas: {}) but have {}\",\n                            total_required, required_amount, gas_cost, balance\n""",
    """                            \"Insufficient balance: need {} (amount: {}, max gas: {}) but have {}\",\n                            total_required, required_amount, max_gas_cost, balance\n""",
)
replace_once(
    """                            \"Insufficient balance for gas: need {}, have {}\",\n                            gas_cost, balance\n""",
    """                            \"Insufficient balance for maximum gas liability: need {}, have {}\",\n                            max_gas_cost, balance\n""",
)
replace_once(
    """        if required_amount > (i64::MAX as u64).saturating_sub(gas_cost) {\n""",
    """        if required_amount > (i64::MAX as u64).saturating_sub(max_gas_cost) {\n""",
)
replace_once(
    """                    Err(e) => {\n                        changeset.mark_failed(format!(\"Publish failed: {}\", e));\n                    }\n""",
    """                    Err(e) => {\n                        // MoveVM currently returns an error without its consumed meter.\n                        // Charge the signed limit fail-closed so an attacker cannot run\n                        // to out-of-gas repeatedly while paying only admission gas.\n                        changeset.set_gas_used(tx.gas_limit());\n                        changeset.mark_failed(format!(\"Publish failed: {}\", e));\n                    }\n""",
)
replace_once(
    """                    Err(e) => {\n                        changeset.mark_failed(format!(\"Execution failed: {}\", e));\n                    }\n""",
    """                    Err(e) => {\n                        // Preserve deterministic anti-DoS accounting even though\n                        // the runtime error path cannot return its internal meter.\n                        changeset.set_gas_used(tx.gas_limit());\n                        changeset.mark_failed(format!(\"Execution failed: {}\", e));\n                    }\n""",
)

PATH.write_text(text)
print("patched maximum gas reservation and fail-closed runtime error charging")
