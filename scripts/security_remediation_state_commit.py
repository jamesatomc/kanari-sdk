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


# ---------------------------------------------------------------------------
# StateManager raw overlay access for crash-journal replay.
# ---------------------------------------------------------------------------
path = "move-execution/v1/kanari-move-runtime-v1/src/state.rs"
s = load(path)
marker = "    /// Commit pending overlay changes to the persistent store and update SMT\n    pub fn commit(&mut self) -> Result<()> {"
if "pub fn pending_raw_changes(" not in s:
    methods = """    /// Snapshot the exact pending database write-set. Used by checkpoint
    /// crash recovery; values are already canonical BCS bytes.
    pub fn pending_raw_changes(&self) -> OverlaySmtChanges {
        let mut updates = Vec::new();
        let mut deletes = Vec::new();
        for (key, value) in &self.overlay {
            match value {
                Some(bytes) => updates.push((key.clone(), bytes.clone())),
                None => deletes.push(key.clone()),
            }
        }
        (updates, deletes)
    }

    /// Stage an exact raw write-set for idempotent checkpoint-journal recovery.
    pub fn stage_raw_changes(
        &mut self,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
    ) {
        for (key, value) in updates {
            self.overlay.insert(key.clone(), Some(value.clone()));
        }
        for key in deletes {
            self.overlay.insert(key.clone(), None);
        }
    }

"""
    if marker not in s:
        raise RuntimeError("StateManager commit marker missing")
    s = s.replace(marker, methods + marker, 1)
save(path, s)

# ---------------------------------------------------------------------------
# Engine protocol budgets and persistent checkpoint journal schema.
# ---------------------------------------------------------------------------
path = "crates/kanari-core/src/engine.rs"
s = load(path)
s = replace_required(
    s,
    "const MAX_MEMPOOL_SIZE: usize = 1_000_000;\nconst MAX_PERSISTED_RECENT_TX_HASHES: usize = 100_000;",
    "const MAX_MEMPOOL_SIZE: usize = 100_000;\nconst MAX_PERSISTED_RECENT_TX_HASHES: usize = 1_000_000;\nconst MAX_TRANSACTION_ARGUMENTS: usize = 64;\nconst MAX_TRANSACTION_ARGUMENT_BYTES: usize = 1024 * 1024;\nconst MAX_MODULE_BYTES: usize = 2 * 1024 * 1024;\nconst MAX_TRANSACTION_GAS_UNITS: u64 = 50_000_000;\nconst MAX_CHECKPOINT_TRANSACTIONS: usize = 10_000;\nconst MAX_CHECKPOINT_BYTES: usize = 16 * 1024 * 1024;\nconst MAX_CHECKPOINT_GAS_UNITS: u64 = 1_000_000_000;\nconst CHECKPOINT_COMMIT_JOURNAL_KEY: &[u8] = b\"checkpoint_commit_journal_v1\";",
    "protocol budget constants",
)
if "struct CheckpointCommitJournal" not in s:
    marker = "#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]\nstruct PersistedTransactionLocation"
    journal = """#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CheckpointCommitJournal {
    checkpoint: Checkpoint,
    state_updates: Vec<(Vec<u8>, Vec<u8>)>,
    state_deletes: Vec<Vec<u8>>,
}

"""
    if marker not in s:
        raise RuntimeError("journal type insertion marker missing")
    s = s.replace(marker, journal + marker, 1)
# Add validation helpers near impl start.
marker = "impl BlockchainEngine {\n    fn checkpoint_transactions_key"
if "fn validate_protocol_transaction(" not in s:
    helper = """impl BlockchainEngine {
    fn validate_protocol_transaction(tx: &Transaction) -> Result<()> {
        if tx.gas_limit() == 0 || tx.gas_limit() > MAX_TRANSACTION_GAS_UNITS {
            anyhow::bail!(
                "Transaction gas limit must be between 1 and {}",
                MAX_TRANSACTION_GAS_UNITS
            );
        }
        match tx {
            Transaction::PublishModule { module_bytes, .. } => {
                if module_bytes.is_empty() || module_bytes.len() > MAX_MODULE_BYTES {
                    anyhow::bail!("Module bytecode exceeds protocol size limit");
                }
            }
            Transaction::ExecuteFunction {
                type_args, args, ..
            } => {
                if type_args.len() > MAX_TRANSACTION_ARGUMENTS
                    || args.len() > MAX_TRANSACTION_ARGUMENTS
                {
                    anyhow::bail!("Transaction has too many arguments");
                }
                let argument_bytes = args.iter().try_fold(0usize, |total, arg| {
                    total
                        .checked_add(arg.len())
                        .ok_or_else(|| anyhow::anyhow!("Transaction argument size overflow"))
                })?;
                if argument_bytes > MAX_TRANSACTION_ARGUMENT_BYTES {
                    anyhow::bail!("Transaction arguments exceed protocol byte limit");
                }
            }
        }
        Ok(())
    }

    fn validate_checkpoint_budget(checkpoint: &Checkpoint) -> Result<()> {
        if checkpoint.transactions.len() > MAX_CHECKPOINT_TRANSACTIONS {
            anyhow::bail!("Checkpoint exceeds transaction-count limit");
        }
        let mut bytes = 0usize;
        let mut gas = 0u64;
        for signed in checkpoint.transactions.iter() {
            Self::validate_protocol_transaction(&signed.transaction)?;
            bytes = bytes
                .checked_add(bcs::serialized_size(signed)? as usize)
                .ok_or_else(|| anyhow::anyhow!("Checkpoint byte count overflow"))?;
            gas = gas
                .checked_add(signed.transaction.gas_limit())
                .ok_or_else(|| anyhow::anyhow!("Checkpoint gas count overflow"))?;
        }
        if bytes > MAX_CHECKPOINT_BYTES {
            anyhow::bail!("Checkpoint exceeds serialized byte limit");
        }
        if gas > MAX_CHECKPOINT_GAS_UNITS {
            anyhow::bail!("Checkpoint exceeds protocol gas-unit limit");
        }
        Ok(())
    }

    fn checkpoint_transactions_key"""
    if marker not in s:
        raise RuntimeError("engine impl insertion marker missing")
    s = s.replace(marker, helper, 1)
# Journal recovery helper before history methods.
marker = "    pub fn get_committed_transaction_from_history("
if "pub(crate) fn recover_checkpoint_commit_journal(" not in s:
    helper = """    pub(crate) fn recover_checkpoint_commit_journal(
        store: &Arc<PersistentStore>,
        blockchain: &Arc<RwLock<Blockchain>>,
        state: &Arc<RwLock<StateManager>>,
    ) -> Result<()> {
        let Some(journal) = store.load::<CheckpointCommitJournal>(CHECKPOINT_COMMIT_JOURNAL_KEY)?
        else {
            return Ok(());
        };
        tracing::warn!(
            checkpoint = journal.checkpoint.sequence,
            "Recovering interrupted checkpoint commit"
        );
        {
            let mut state = state.write().unwrap_or_else(|e| e.into_inner());
            state.stage_raw_changes(&journal.state_updates, &journal.state_deletes);
            state.commit()?;
        }
        {
            let mut chain = blockchain.write().unwrap_or_else(|e| e.into_inner());
            if chain.height() < journal.checkpoint.sequence {
                chain.add_checkpoint_with_validation(journal.checkpoint.clone(), true)?;
            } else if chain
                .get_checkpoint(journal.checkpoint.sequence)
                .is_none_or(|existing| existing.hash().ok() != journal.checkpoint.hash().ok())
            {
                anyhow::bail!("Checkpoint journal conflicts with persisted chain history");
            }
            Self::persist_blockchain_snapshot_to_store(store, &chain)?;
        }
        store.delete(CHECKPOINT_COMMIT_JOURNAL_KEY)?;
        Ok(())
    }

"""
    if marker not in s:
        raise RuntimeError("journal recovery insertion marker missing")
    s = s.replace(marker, helper + marker, 1)
save(path, s)

# ---------------------------------------------------------------------------
# Mempool: protocol validation plus persistent replay protection.
# ---------------------------------------------------------------------------
path = "crates/kanari-core/src/engine/mempool.rs"
s = load(path)
s = replace_required(
    s,
    "                    let verified = signed_tx.into_verified()?;",
    "                    let verified = signed_tx.into_verified()?;\n                    Self::validate_protocol_transaction(verified.transaction())?;",
    "mempool protocol validation",
)
old = """        let executed_hashes = {
            let chain = match self.blockchain.read() {"""
new = """        let persisted_hashes = if let Some(store) = &self.persistent_store {
            let mut hashes = AHashSet::new();
            for (hash, _, _) in &batch_metadata {
                if store
                    .load::<PersistedTransactionLocation>(&Self::transaction_index_key(hash))?
                    .is_some()
                {
                    hashes.insert(hash.clone());
                }
            }
            hashes
        } else {
            AHashSet::new()
        };

        let executed_hashes = {
            let chain = match self.blockchain.read() {"""
s = replace_required(s, old, new, "persistent replay lookup")
s = replace_required(
    s,
    "            if executed_hashes.contains(tx_hash) {",
    "            if executed_hashes.contains(tx_hash) || persisted_hashes.contains(tx_hash) {",
    "persistent replay rejection",
)
save(path, s)

# ---------------------------------------------------------------------------
# Checkpoint execution: one execution only, journal before state commit, no replay.
# ---------------------------------------------------------------------------
path = "crates/kanari-core/src/engine/apply_checkpoint.rs"
s = load(path)
s = replace_required(
    s,
    "        if !checkpoint.transactions.is_empty() {\n            self.apply_system_prologue_to_state(&state_arc, checkpoint.timestamp, false)?;\n        }",
    "        Self::validate_checkpoint_budget(checkpoint)?;\n        if !checkpoint.transactions.is_empty() {\n            self.apply_system_prologue_to_state(&state_arc, checkpoint.timestamp, false)?;\n        }",
    "checkpoint budget validation",
)
# Journal before mutating live state.
old = """        {
            let mut state = self.state_write();
            *state = new_state;
            state
                .commit()
                .context("Failed to commit state to RocksDB")?;
        }"""
new = """        if let Some(store) = &self.persistent_store {
            let (state_updates, state_deletes) = new_state.pending_raw_changes();
            store.save(
                CHECKPOINT_COMMIT_JOURNAL_KEY,
                &CheckpointCommitJournal {
                    checkpoint: checkpoint.clone(),
                    state_updates,
                    state_deletes,
                },
            )?;
        }

        {
            let mut state = self.state_write();
            *state = new_state;
            state
                .commit()
                .context("Failed to commit state to RocksDB")?;
        }"""
s = replace_required(s, old, new, "checkpoint journal creation")
s = replace_required(
    s,
    "        self.finalize_checkpoint_metadata(checkpoint)",
    "        self.finalize_checkpoint_metadata(checkpoint)?;\n        if let Some(store) = &self.persistent_store {\n            store.delete(CHECKPOINT_COMMIT_JOURNAL_KEY)?;\n        }\n        Ok(())",
    "checkpoint journal completion",
)
# Eliminate second VM execution; verified state is the state committed.
start_marker = "        if !to_execute.is_empty() && Self::requires_runtime_side_effect_persistence(&to_execute) {"
if start_marker in s:
    start = s.index(start_marker)
    end_marker = "\n\n        self.finalize_checkpoint(checkpoint, verified_state, validate_supply)"
    end = s.index(end_marker, start)
    s = s[:start] + "        let _ = to_execute; // execution already materialized in verified_state\n" + s[end:]
save(path, s)

# ---------------------------------------------------------------------------
# Startup recovery after state opens, before accepting work.
# ---------------------------------------------------------------------------
path = "crates/kanari-core/src/engine/bootstrap.rs"
s = load(path)
s = replace_required(
    s,
    "        let state = Self::load_state(&persistent_store)?;",
    "        let state = Self::load_state(&persistent_store)?;\n        if let Some(store) = &persistent_store {\n            Self::recover_checkpoint_commit_journal(store, &blockchain, &state)?;\n        }",
    "startup journal recovery",
)
save(path, s)

# ---------------------------------------------------------------------------
# RPC transport limits and non-wildcard CORS.
# ---------------------------------------------------------------------------
path = "crates/kanari-rpc-server/Cargo.toml"
s = load(path)
if "tower = { version = \"0.5.3\", features = [\"limit\", \"timeout\"] }" not in s:
    s = s.replace(
        "tower-http = { workspace = true }",
        "tower-http = { workspace = true }\ntower = { version = \"0.5.3\", features = [\"limit\", \"timeout\"] }",
        1,
    )
save(path, s)

path = "crates/kanari-rpc-server/src/lib.rs"
s = load(path)
s = s.replace("use tower_http::cors::{Any, CorsLayer};", "use tower::limit::ConcurrencyLimitLayer;\nuse tower_http::cors::CorsLayer;\nuse tower_http::limit::RequestBodyLimitLayer;\nuse tower_http::timeout::TimeoutLayer;")
old = """    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);"""
new = """    let cors = if let Ok(origin) = std::env::var("KANARI_RPC_ALLOWED_ORIGIN") {
        if origin == "*" {
            panic!("KANARI_RPC_ALLOWED_ORIGIN cannot be wildcard");
        }
        CorsLayer::new()
            .allow_origin(origin.parse::<axum::http::HeaderValue>().expect("valid RPC origin"))
            .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
            .allow_headers([header::CONTENT_TYPE])
    } else {
        CorsLayer::new()
    };"""
s = replace_required(s, old, new, "RPC CORS restriction")
s = replace_required(
    s,
    "        .route(\"/metrics\", get(handle_metrics))\n        .layer(cors)",
    "        .route(\"/metrics\", get(handle_metrics))\n        .layer(RequestBodyLimitLayer::new(1024 * 1024))\n        .layer(TimeoutLayer::new(std::time::Duration::from_secs(15)))\n        .layer(ConcurrencyLimitLayer::new(128))\n        .layer(cors)",
    "RPC transport limits",
)
save(path, s)

print("transactional checkpoint, replay, budget, and RPC hardening applied or already present")
