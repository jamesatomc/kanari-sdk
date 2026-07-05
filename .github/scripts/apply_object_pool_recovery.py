from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file = Path(path)
    text = file.read_text()
    if new in text:
        return
    if old not in text:
        raise RuntimeError(f"missing marker in {path}: {old[:120]!r}")
    file.write_text(text.replace(old, new, 1))


# Expose raw reads so versioned object-mempool repair can quarantine stale BCS.
replace_once(
    "move-execution/v1/kanari-move-runtime-v1/src/storage/persistent_store.rs",
    '''    pub fn load<T: DeserializeOwned>(
        &self,
        key: &[u8],
    ) -> std::result::Result<Option<T>, PersistentStoreError> {
        match self.read_raw(key)? {
            Some(bytes) => Ok(Some(bcs::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }
''',
    '''    pub fn load<T: DeserializeOwned>(
        &self,
        key: &[u8],
    ) -> std::result::Result<Option<T>, PersistentStoreError> {
        match self.read_raw(key)? {
            Some(bytes) => Ok(Some(bcs::from_bytes(&bytes)?)),
            None => Ok(None),
        }
    }

    /// Load the encoded value without deserializing it. Protocol migrations use
    /// this to identify and quarantine values written by an older BCS schema.
    pub fn load_raw(
        &self,
        key: &[u8],
    ) -> std::result::Result<Option<Vec<u8>>, PersistentStoreError> {
        self.read_raw(key)
    }
''',
)

engine = Path("crates/kanari-core/src/object_transaction_engine_v2.rs")
text = engine.read_text()

# Repair before every new admission so stale locks from an earlier schema cannot
# reserve a live coin forever.
old = '''    pub fn submit_object_transaction(&self, tx: SignedObjectTransaction) -> Result<Vec<u8>> {
        tx.verify()?;
'''
new = '''    pub fn submit_object_transaction(&self, tx: SignedObjectTransaction) -> Result<Vec<u8>> {
        self.repair_object_transaction_pool()?;
        tx.verify()?;
'''
if new not in text:
    if old not in text:
        raise RuntimeError("submit_object_transaction marker not found")
    text = text.replace(old, new, 1)

marker = '''    pub fn pending_object_transaction_len(&self) -> Result<usize> {
'''
methods = r'''    /// Rebuild the durable object mempool index and mutable-object locks from
    /// transactions that can still be decoded and whose digest matches their
    /// storage key. Entries from an older schema are removed atomically.
    pub fn repair_object_transaction_pool(&self) -> Result<(usize, usize)> {
        let state = self.state_write();
        let store = state.store.clone();

        let raw_index = store.load_raw(INDEX_KEY)?;
        let mut indexed: Vec<Vec<u8>> = raw_index
            .as_deref()
            .and_then(|bytes| bcs::from_bytes(bytes).ok())
            .unwrap_or_default();
        indexed.sort();
        indexed.dedup();

        let mut valid_index = Vec::with_capacity(indexed.len());
        let mut rebuilt_locks = LockMap::new();
        let mut deletes = Vec::new();
        let mut removed = 0usize;

        for digest in indexed {
            let key = pending_key(&digest);
            let Some(bytes) = store.load_raw(&key)? else {
                removed = removed.saturating_add(1);
                continue;
            };
            let transaction = match bcs::from_bytes::<SignedObjectTransaction>(&bytes) {
                Ok(transaction) => transaction,
                Err(error) => {
                    log::warn!(
                        "Removing stale object transaction 0x{}: incompatible BCS payload: {}",
                        hex::encode(&digest),
                        error
                    );
                    deletes.push(key);
                    removed = removed.saturating_add(1);
                    continue;
                }
            };
            let actual_digest = match transaction.digest() {
                Ok(actual_digest) => actual_digest,
                Err(error) => {
                    log::warn!(
                        "Removing stale object transaction 0x{}: invalid digest: {}",
                        hex::encode(&digest),
                        error
                    );
                    deletes.push(key);
                    removed = removed.saturating_add(1);
                    continue;
                }
            };
            if actual_digest != digest || transaction.verify().is_err() {
                log::warn!(
                    "Removing stale object transaction 0x{}: digest or signature mismatch",
                    hex::encode(&digest)
                );
                deletes.push(key);
                removed = removed.saturating_add(1);
                continue;
            }

            let mutable_ids = transaction.data.mutable_input_ids();
            if mutable_ids
                .iter()
                .any(|object_id| rebuilt_locks.contains_key(object_id))
            {
                log::warn!(
                    "Removing conflicting recovered object transaction 0x{}",
                    hex::encode(&digest)
                );
                deletes.push(key);
                removed = removed.saturating_add(1);
                continue;
            }
            for object_id in mutable_ids {
                rebuilt_locks.insert(object_id, digest.clone());
            }
            valid_index.push(digest);
        }

        store.apply_raw_changes(
            &[
                (INDEX_KEY.to_vec(), bcs::to_bytes(&valid_index)?),
                (LOCKS_KEY.to_vec(), bcs::to_bytes(&rebuilt_locks)?),
            ],
            &deletes,
        )?;
        Ok((valid_index.len(), removed))
    }

    pub fn pending_object_transaction(
        &self,
        digest: &[u8],
    ) -> Result<Option<SignedObjectTransaction>> {
        self.state_read()
            .store
            .load::<SignedObjectTransaction>(&pending_key(digest))
            .map_err(Into::into)
    }

'''
if "pub fn repair_object_transaction_pool" not in text:
    if marker not in text:
        raise RuntimeError("pending object length marker not found")
    text = text.replace(marker, methods + marker, 1)

# Repair first so listing cannot be poisoned by one obsolete encoded entry.
old = '''    pub fn pending_object_transactions(&self) -> Result<Vec<SignedObjectTransaction>> {
        let state = self.state_read();
'''
new = '''    pub fn pending_object_transactions(&self) -> Result<Vec<SignedObjectTransaction>> {
        self.repair_object_transaction_pool()?;
        let state = self.state_read();
'''
if new not in text:
    if old not in text:
        raise RuntimeError("pending_object_transactions marker not found")
    text = text.replace(old, new, 1)
engine.write_text(text)

# Consolidate gas calculation, exact-digest execution, network execution and
# restart recovery in the core protocol layer.
Path("crates/kanari-core/src/object_command_executor.rs").write_text(r'''// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::engine::BlockchainEngine;
use anyhow::{Context, Result, ensure};
use kanari_types::object_effects::ObjectTransactionEffectsV1;
use kanari_types::object_transaction::ObjectTransactionKind;
use kanari_types::signed_object_transaction::SignedObjectTransaction;

impl BlockchainEngine {
    pub fn object_command_gas_units(
        &self,
        transaction: &SignedObjectTransaction,
    ) -> Result<u64> {
        let units = match &transaction.data.kind {
            ObjectTransactionKind::Pay { coins, .. } => 100u64
                .checked_add((coins.len() as u64).saturating_mul(10))
                .ok_or_else(|| anyhow::anyhow!("Pay gas overflow"))?,
            ObjectTransactionKind::TransferObjects { objects, .. } => 50u64
                .checked_add((objects.len() as u64).saturating_mul(5))
                .ok_or_else(|| anyhow::anyhow!("Transfer gas overflow"))?,
            ObjectTransactionKind::MoveCall(_) => {
                anyhow::bail!("MoveCall object execution is not enabled yet")
            }
            ObjectTransactionKind::Publish { .. } => {
                anyhow::bail!("Publish object execution is not enabled yet")
            }
        };
        ensure!(
            units <= transaction.data.gas_data.budget,
            "Required gas units {} exceed budget {}",
            units,
            transaction.data.gas_data.budget
        );
        Ok(units)
    }

    pub fn submit_protocol_transaction(
        &self,
        transaction: SignedObjectTransaction,
    ) -> Result<Vec<u8>> {
        transaction.verify()?;
        {
            let state = self.state_read();
            for reference in transaction.data.owned_input_refs() {
                state.validate_address_owned_object_ref(&reference, transaction.data.sender)?;
            }
            for reference in &transaction.data.gas_data.payment {
                state.validate_address_owned_object_ref(
                    reference,
                    transaction.data.gas_data.owner,
                )?;
            }
        }
        self.submit_object_transaction(transaction)
    }

    pub fn execute_submitted_object_command(
        &self,
        digest: &[u8],
        gas_used: u64,
    ) -> Result<ObjectTransactionEffectsV1> {
        let transaction = self
            .pending_object_transaction(digest)?
            .ok_or_else(|| anyhow::anyhow!("Pending object transaction was not found"))?;

        match &transaction.data.kind {
            ObjectTransactionKind::Pay { .. } | ObjectTransactionKind::TransferObjects { .. } => {}
            _ => anyhow::bail!("Submitted transaction is not a direct object command"),
        }

        let effects = match self.build_object_command_effects(&transaction, gas_used) {
            Ok(effects) => effects,
            Err(error) => {
                self.release_object_transaction(digest)?;
                return Err(error);
            }
        };
        if let Err(error) = self.state_write().apply_object_effects(&effects) {
            self.release_object_transaction(digest)?;
            return Err(error).context("Failed to apply object command effects");
        }
        self.finalize_object_transaction(digest)?
            .ok_or_else(|| anyhow::anyhow!("Object transaction disappeared before finalization"))?;
        Ok(effects)
    }

    /// Canonical direct-object execution entry point used by both RPC and P2P.
    pub fn execute_protocol_transaction(
        &self,
        transaction: SignedObjectTransaction,
    ) -> Result<(Vec<u8>, ObjectTransactionEffectsV1)> {
        let digest = transaction.digest()?;
        ensure!(
            !self.is_object_transaction_executed(&digest)?,
            "Object transaction already executed"
        );
        let gas_units = self.object_command_gas_units(&transaction)?;
        if self.pending_object_transaction(&digest)?.is_none() {
            self.submit_protocol_transaction(transaction)?;
        }
        let effects = self.execute_submitted_object_command(&digest, gas_units)?;
        Ok((digest, effects))
    }

    /// Finalize valid durable admissions after restart and release any entry
    /// that can no longer execute against the current object state.
    pub fn recover_pending_object_transactions(&self) -> Result<(usize, usize)> {
        let (_, mut removed) = self.repair_object_transaction_pool()?;
        let pending = self.pending_object_transactions()?;
        let mut executed = 0usize;
        for transaction in pending {
            let digest = match transaction.digest() {
                Ok(digest) => digest,
                Err(_) => continue,
            };
            let result = self
                .object_command_gas_units(&transaction)
                .and_then(|gas| self.execute_submitted_object_command(&digest, gas).map(|_| ()));
            match result {
                Ok(()) => executed = executed.saturating_add(1),
                Err(error) => {
                    log::warn!(
                        "Releasing unrecoverable object transaction 0x{}: {}",
                        hex::encode(&digest),
                        error
                    );
                    self.release_object_transaction(&digest)?;
                    removed = removed.saturating_add(1);
                }
            }
        }
        Ok((executed, removed))
    }

    pub fn execute_object_command_now(
        &self,
        transaction: SignedObjectTransaction,
        _gas_used: u64,
    ) -> Result<ObjectTransactionEffectsV1> {
        self.execute_protocol_transaction(transaction)
            .map(|(_, effects)| effects)
    }
}
''')

# RPC delegates all protocol work to core; no second local gas/execution path.
rpc = Path("crates/kanari-rpc-server/src/object_transaction.rs")
text = rpc.read_text()
start = text.find("fn deterministic_gas_units(")
if start != -1:
    end = text.find("\npub async fn submit", start)
    if end == -1:
        raise RuntimeError("submit handler marker not found")
    text = text[:start] + text[end + 1:]
old_submit = '''pub async fn submit(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let transaction: SignedObjectTransaction = match serde_json::from_value(request.params.clone())
    {
        Ok(transaction) => transaction,
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };
    let broadcast = transaction.clone();
    let gas_units = match deterministic_gas_units(&transaction) {
        Ok(gas_units) => gas_units,
        Err(error) => return invalid_params_response(request.id, error),
    };
    match state.engine.submit_protocol_transaction(transaction) {
        Ok(digest) => match state.engine.execute_submitted_object_command(&digest, gas_units) {
            Ok(effects) => {
                state.broadcast_submitted_transaction(broadcast);
                respond_with_serialize(
                    request.id,
                    SubmitResponse {
                        digest: format!("0x{}", hex::encode(digest)),
                        status: "executed_object_effects",
                        effects: Some(effects),
                    },
                )
            }
            Err(error) => internal_error_response(request.id, error.to_string()),
        },
        Err(error) => invalid_params_response(request.id, error.to_string()),
    }
}
'''
new_submit = '''pub async fn submit(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let transaction: SignedObjectTransaction = match serde_json::from_value(request.params.clone())
    {
        Ok(transaction) => transaction,
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };
    let broadcast = transaction.clone();
    match state.engine.execute_protocol_transaction(transaction) {
        Ok((digest, effects)) => {
            state.broadcast_submitted_transaction(broadcast);
            respond_with_serialize(
                request.id,
                SubmitResponse {
                    digest: format!("0x{}", hex::encode(digest)),
                    status: "executed_object_effects",
                    effects: Some(effects),
                },
            )
        }
        Err(error) => invalid_params_response(request.id, error.to_string()),
    }
}
'''
if new_submit not in text:
    if old_submit not in text:
        raise RuntimeError("old RPC submit handler not found")
    text = text.replace(old_submit, new_submit, 1)
rpc.write_text(text)

# Execute gossip transactions through the same core entry point. A peer no
# longer leaves the object locked and asks the legacy checkpoint producer to
# process a payload it cannot decode.
sync = Path("crates/kanari-node/src/sync.rs")
text = sync.read_text()
old_handler = '''    async fn handle_new_transaction(&self, tx_data: String) {
        if let Some(signed_tx) =
            Self::parse_message::<SignedObjectTransaction>(&tx_data, "object transaction")
        {
            match self.engine.submit_protocol_transaction(signed_tx) {
                Ok(tx_hash) => {
                    info!(
                        "Received object transaction from network: 0x{}",
                        hex::encode(tx_hash)
                    );
                }
                Err(e) => {
                    warn!("Failed to submit object transaction from network: {}", e);
                }
            }
        }
    }
'''
new_handler = '''    async fn handle_new_transaction(&self, tx_data: String) {
        if let Some(signed_tx) =
            Self::parse_message::<SignedObjectTransaction>(&tx_data, "object transaction")
        {
            let digest = match signed_tx.digest() {
                Ok(digest) => digest,
                Err(error) => {
                    warn!("Rejected object transaction with invalid digest: {}", error);
                    return;
                }
            };
            match self.engine.is_object_transaction_executed(&digest) {
                Ok(true) => {
                    info!(
                        "Ignoring already executed object transaction: 0x{}",
                        hex::encode(digest)
                    );
                    return;
                }
                Ok(false) => {}
                Err(error) => {
                    warn!("Failed to check object transaction replay state: {}", error);
                    return;
                }
            }
            match self.engine.execute_protocol_transaction(signed_tx) {
                Ok((tx_hash, _effects)) => {
                    info!(
                        "Executed object transaction from network: 0x{}",
                        hex::encode(tx_hash)
                    );
                }
                Err(e) => {
                    warn!("Failed to execute object transaction from network: {}", e);
                }
            }
        }
    }
'''
if new_handler not in text:
    if old_handler not in text:
        raise RuntimeError("P2P object transaction handler marker not found")
    text = text.replace(old_handler, new_handler, 1)
sync.write_text(text)

# Recover old durable admissions before the node advertises pending state.
app = Path("crates/kanari-node/src/app.rs")
text = app.read_text()
old_create = '''pub fn create_engine(
    data_dir: &Option<std::path::PathBuf>,
    network: &NetworkMode,
) -> Result<BlockchainEngine> {
    configure_engine_environment(data_dir.as_deref(), network)?;
    if let Some(dir) = data_dir {
        tracing::info!("Using data directory: {}", dir.display());
        let dir_str = path_to_env_value(dir)?;
        Ok(BlockchainEngine::new_dir(dir_str)?)
    } else {
        Ok(BlockchainEngine::new()?)
    }
}
'''
new_create = '''pub fn create_engine(
    data_dir: &Option<std::path::PathBuf>,
    network: &NetworkMode,
) -> Result<BlockchainEngine> {
    configure_engine_environment(data_dir.as_deref(), network)?;
    let engine = if let Some(dir) = data_dir {
        tracing::info!("Using data directory: {}", dir.display());
        let dir_str = path_to_env_value(dir)?;
        BlockchainEngine::new_dir(dir_str)?
    } else {
        BlockchainEngine::new()?
    };
    let (executed, removed) = engine.recover_pending_object_transactions()?;
    if executed > 0 || removed > 0 {
        tracing::info!(
            executed,
            removed,
            "Recovered durable object transaction pool"
        );
    }
    Ok(engine)
}
'''
if new_create not in text:
    if old_create not in text:
        raise RuntimeError("create_engine marker not found")
    text = text.replace(old_create, new_create, 1)

# Only account transactions may enter the old DAG/checkpoint producer. Object
# admissions are drained through recover_pending_object_transactions instead.
old_loop = '''        let should_produce_pending = stats.pending_transactions > 0 && pending_gossip_ready;

        if should_produce_pending {
            match engine.produce_checkpoint() {
'''
new_loop = '''        let object_pending = engine.pending_object_transaction_len().unwrap_or_else(|error| {
            tracing::warn!("Failed to read object pending count: {}", error);
            0
        });
        if object_pending > 0 {
            match engine.recover_pending_object_transactions() {
                Ok((executed, removed)) => {
                    if executed > 0 || removed > 0 {
                        did_work = true;
                        tracing::info!(executed, removed, "Processed pending object transactions");
                    }
                }
                Err(error) => tracing::error!("Object transaction recovery failed: {}", error),
            }
        }
        let legacy_pending = engine.pending_transaction_len();
        let should_produce_pending = legacy_pending > 0 && pending_gossip_ready;

        if should_produce_pending {
            match engine.produce_checkpoint() {
'''
if new_loop not in text:
    if old_loop not in text:
        raise RuntimeError("node checkpoint loop marker not found")
    text = text.replace(old_loop, new_loop, 1)
app.write_text(text)
