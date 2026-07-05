#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def save(path: str, text: str) -> None:
    (ROOT / path).write_text(text, encoding="utf-8")


def replace_required(text: str, old: str, new: str, label: str) -> str:
    if new in text:
        return text
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one source pattern, found {count}")
    return text.replace(old, new, 1)


# StateManager: i128-safe arithmetic, absolute token sets, and canonical Move writes.
path = "move-execution/v1/kanari-move-runtime-v1/src/state/apply.rs"
s = load(path)
s = replace_required(s, ".try_fold(0i64, |total, change|", ".try_fold(0i128, |total, change|", "supply delta")
s = replace_required(
    s,
    ".checked_add(supply_delta as u64)",
    ".checked_add(u64::try_from(supply_delta).context(\"Invalid positive native supply delta\")?)",
    "positive supply conversion",
)
s = replace_required(
    s,
    "let amount = change.balance_delta as u64;",
    "let amount = u64::try_from(change.balance_delta)\n                    .context(\"Invalid positive native account balance delta\")?;",
    "positive account conversion",
)
s = replace_required(
    s,
    "let current = account.get_token_balance(&normalized_token_type);\n            let next = current.saturating_add(amount.value());\n            account.set_token_balance_value(&normalized_token_type, next);",
    "// token_balance_sets is an absolute replacement, not a delta.\n            account.set_token_balance_value(&normalized_token_type, amount.value());",
    "absolute token balance",
)
s = replace_required(
    s,
    "        // Record Global Token Supplies to database only once after all processing\n        let mut owners_to_recompute",
    "        // Materialize Move module/resource writes into the same transactional overlay.\n        for (key, value) in &changeset.move_writes {\n            if key.starts_with(b\"module:\") {\n                let module_key = std::str::from_utf8(key)\n                    .context(\"Invalid UTF-8 Move module key\")?\n                    .to_string();\n                if value.is_some() {\n                    self.add_to_index_list(b\"module_index\", module_key)?;\n                } else {\n                    self.remove_from_index_list(b\"module_index\", &module_key)?;\n                }\n            }\n            match value {\n                Some(bytes) => self.save_internal(key, bytes)?,\n                None => { self.overlay.insert(key.clone(), None); }\n            }\n        }\n\n        // Record Global Token Supplies to database only once after all processing\n        let mut owners_to_recompute",
    "canonical Move writes",
)
save(path, s)

# Runtime object persistence: no silent in-memory fallback and all writes return Result.
path = "move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs"
s = load(path)
s = replace_required(
    s,
    """        let object_storage: Arc<dyn ObjectStore> = match shared_store {
            Some(store) if !cfg!(miri) => match ObjectStorage::boxed_with_store(store) {
                Ok(store) => Arc::from(store),
                Err(e) => {
                    log::warn!("[RUNTIME] shared object store load failed: {}", e);
                    Arc::from(ObjectStorage::boxed_inmemory())
                }
            },
            _ if cfg!(miri) => Arc::from(ObjectStorage::boxed_inmemory()),
            _ => match ObjectStorage::boxed_with_store(state.store()) {
                Ok(store) => Arc::from(store),
                Err(e) => {
                    log::warn!("[RUNTIME] shared object store load failed: {}", e);
                    Arc::from(ObjectStorage::boxed_inmemory())
                }
            },
        };""",
    """        let object_storage: Arc<dyn ObjectStore> = match shared_store {
            Some(store) if !cfg!(miri) => Arc::from(
                ObjectStorage::boxed_with_store(store)
                    .require("Failed to load persistent shared object store")?,
            ),
            _ if cfg!(miri) => Arc::from(ObjectStorage::boxed_inmemory()),
            _ => Arc::from(
                ObjectStorage::boxed_with_store(state.store())
                    .require("Failed to load persistent runtime object store")?,
            ),
        };""",
    "persistent object-store initialization",
)
s = replace_required(
    s,
    ".get_all_module_ids()\n            .unwrap_or_default()",
    ".get_all_module_ids()?",
    "module index load",
)
s = replace_required(
    s,
    """        let object_storage: Arc<dyn ObjectStore> =
            match ObjectStorage::boxed_with_store(self.state.store()) {
                Ok(store) => Arc::from(store),
                Err(e) => {
                    log::warn!("[RUNTIME] isolated object store load failed: {}", e);
                    Arc::from(ObjectStorage::boxed_inmemory())
                }
            };""",
    """        let object_storage: Arc<dyn ObjectStore> = Arc::from(
            ObjectStorage::boxed_with_store(self.state.store())
                .require("Failed to load isolated persistent object store")?,
        );""",
    "isolated object-store initialization",
)
s = replace_required(
    s,
    "pub fn persist_created_objects(&self, cs: &ChangeSet) {",
    "pub fn persist_created_objects(&self, cs: &ChangeSet) -> Result<()> {",
    "persist created signature",
)
s = replace_required(
    s,
    """            let _ = self.object_storage.store_object(stored);
        }
    }

    pub fn persist_deleted_objects(&self, cs: &ChangeSet) {""",
    """            self.object_storage
                .store_object(stored)
                .require("Failed to persist created object")?;
        }
        Ok(())
    }

    pub fn persist_deleted_objects(&self, cs: &ChangeSet) -> Result<()> {""",
    "persist created body",
)
s = replace_required(
    s,
    """        for obj_id in &cs.deleted_objects {
            let _ = self.object_storage.delete_object(obj_id);
        }
    }""",
    """        for obj_id in &cs.deleted_objects {
            self.object_storage
                .delete_object(obj_id)
                .require("Failed to persist deleted object")?;
        }
        Ok(())
    }""",
    "persist deleted body",
)
s = s.replace("self.persist_created_objects(&cs);", "self.persist_created_objects(&cs)?;")
s = s.replace("self.persist_deleted_objects(&cs);", "self.persist_deleted_objects(&cs)?;")
save(path, s)

# Core call sites must propagate object persistence errors.
for path in ["crates/kanari-core/src/engine.rs", "crates/kanari-core/src/engine/apply_checkpoint.rs"]:
    s = load(path)
    s = s.replace("runtime.persist_created_objects(&changeset);", "runtime.persist_created_objects(&changeset)?;")
    s = s.replace("runtime.persist_deleted_objects(&changeset);", "runtime.persist_deleted_objects(&changeset)?;")
    s = s.replace("runtime.persist_created_objects(&cs);", "runtime.persist_created_objects(&cs)?;")
    s = s.replace("runtime.persist_deleted_objects(&cs);", "runtime.persist_deleted_objects(&cs)?;")
    save(path, s)

# Bootstrap corruption must fail closed.
path = "crates/kanari-core/src/engine/bootstrap.rs"
s = load(path)
s = replace_required(s, "let mut blockchain = Self::load_blockchain(&persistent_store);", "let mut blockchain = Self::load_blockchain(&persistent_store)?;", "bootstrap blockchain result")
s = replace_required(s, "let persisted_dag_state = Self::load_dag_state(&persistent_store);", "let persisted_dag_state = Self::load_dag_state(&persistent_store)?;", "bootstrap dag result")
s = replace_required(s, "fn load_blockchain(store: &Option<Arc<PersistentStore>>) -> Arc<RwLock<Blockchain>>", "fn load_blockchain(store: &Option<Arc<PersistentStore>>) -> Result<Arc<RwLock<Blockchain>>>", "load blockchain signature")
s = replace_required(s, "fn load_dag_state(store: &Option<Arc<PersistentStore>>) -> Option<PersistentDagState>", "fn load_dag_state(store: &Option<Arc<PersistentStore>>) -> Result<Option<PersistentDagState>>", "load dag signature")
s = s.replace("Arc::new(RwLock::new(blockchain))", "Ok(Arc::new(RwLock::new(blockchain)))")
s = s.replace("Arc::new(RwLock::new(Blockchain::new()))", "Ok(Arc::new(RwLock::new(Blockchain::new())))")
s = s.replace("Some(state)", "Ok(Some(state))")
s = s.replace("Ok(None) => None,", "Ok(None) => Ok(None),")
s = replace_required(
    s,
    """                Err(e) => {
                    error!(
                        "FATAL ERROR loading blockchain: {}. Falling back to fresh genesis.",
                        e
                    );
                    Ok(Arc::new(RwLock::new(Blockchain::new())))
                }""",
    """                Err(e) => anyhow::bail!(
                    "Persistent blockchain metadata is unreadable; refusing to start: {}",
                    e
                ),""",
    "blockchain corruption",
)
s = replace_required(
    s,
    """                Err(e) => {
                    error!(
                        "Failed to load DAG state: {}. Falling back to fresh DAG.",
                        e
                    );
                    None
                }""",
    """                Err(e) => anyhow::bail!(
                    "Persistent DAG state is unreadable; refusing to start: {}",
                    e
                ),""",
    "dag corruption",
)
# Explicit no-store mode remains available for tests/dev, but never as an error fallback.
s = s.replace("            None\n        }\n    }\n}", "            Ok(None)\n        }\n    }\n}")
save(path, s)

# RPC: immediate mode is simulation-only and list size is bounded.
path = "crates/kanari-rpc-server/src/transaction/mod.rs"
s = load(path)
s = s.replace("use tokio::time::{Duration, sleep};\n", "")
if "async fn submit_after_immediate_execution(" in s:
    start = s.index("async fn submit_after_immediate_execution(")
    end = s.index("async fn execute_or_submit_response(", start)
    s = s[:start] + s[end:]
s = replace_required(
    s,
    """            let tx_for_broadcast = signed_tx.clone();
            if let Err(e) = submit_after_immediate_execution(state, signed_tx).await {
                error!("Failed to submit executed transaction: {}", e);
                return RpcResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(RpcError::transaction_error(format!(
                        "Simulation successful, but mempool submission failed: {}",
                        e
                    ))),
                    id: request_id,
                };
            }
            state.broadcast_submitted_transaction(tx_for_broadcast);

            info!(
                "{} executed immediately & submitted: {}",
                action, tx_hash_hex
            );""",
    """            info!("{} simulated without submission: {}", action, tx_hash_hex);""",
    "simulation-only path",
)
s = s.replace('"status": "executed",', '"status": "simulated",\n                    "committed": false,\n                    "submitted": false,')
s = replace_required(
    s,
    """    let limit = request
        .params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;""",
    """    const MAX_TRANSACTION_PAGE_SIZE: u64 = 200;
    let requested_limit = request
        .params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50);
    if requested_limit == 0 || requested_limit > MAX_TRANSACTION_PAGE_SIZE {
        return invalid_params_response(
            request.id,
            format!("limit must be between 1 and {}", MAX_TRANSACTION_PAGE_SIZE),
        );
    }
    let limit = requested_limit as usize;""",
    "RPC transaction page limit",
)
save(path, s)

print("security remediation batch 1 applied or already present")
