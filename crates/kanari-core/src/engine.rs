// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::blockchain::Blockchain;
use crate::consensus::{
    Checkpoint, DagMetrics, DagProductionPolicy, DagVertex, PersistentDagState,
};
use ahash::AHashMap;
use anyhow::{Context, Result};
use kanari_move_runtime_v1::changeset::ChangeSet;
use kanari_move_runtime_v1::move_runtime::MoveRuntime;
use kanari_move_runtime_v1::state::StateManager;
use kanari_move_runtime_v1::storage::persistent_store::PersistentStore;
pub use kanari_rpc_api::{AccountInfo, BlockData, BlockchainStats, FullBlockData, ObjectInfo};
use kanari_types::address::Address as KanariAddress;
use kanari_types::event::Event;
use kanari_types::transaction::{NativeCall, SignedTransaction, Transaction};
use kanari_types::{GasMeter, GasOperation};
use log::{error, info};
use lru::LruCache;
use move_core_types::{
    account_address::AccountAddress,
    identifier::Identifier,
    language_storage::{ModuleId, StructTag, TypeTag},
};
use num_cpus;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::num::NonZeroUsize;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

type ProofCache = LruCache<(u64, usize), (String, Vec<Vec<u8>>)>;

mod apply_checkpoint;
mod bootstrap;
mod mempool;
mod produce_dag_vertex;
mod queries;
mod runtime_guards;
pub use produce_dag_vertex::{CheckpointProductionInfo, DagEngine};
pub use runtime_guards::{RuntimeGuardConfig, RuntimeHealthReport};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointSyncData {
    pub checkpoint: Checkpoint,
}

const MAX_MEMPOOL_SIZE: usize = 1_000_000;
const MAX_PERSISTED_RECENT_TX_HASHES: usize = 100_000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedTransactionLocation {
    checkpoint_sequence: u64,
    state_root: Vec<u8>,
}

#[derive(Debug, Default)]
pub(crate) struct MempoolState {
    pending_txs: Vec<SignedTransaction>,
    pending_tx_hashes: HashSet<Vec<u8>>,
    pending_sender_counts: AHashMap<String, u64>,
}

/// Complete blockchain engine with Move VM integration
pub struct BlockchainEngine {
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub state: Arc<RwLock<StateManager>>,
    mempool: Arc<RwLock<MempoolState>>,
    pub persistent_store: Option<Arc<PersistentStore>>,
    // Reusable pool of MoveRuntime instances for parallel execution
    pub runtime_pool: Vec<kanari_move_runtime_v1::move_runtime::MoveRuntime>,
    // LRU cache for frequently requested merkle proofs
    // Cache key: (block_height, tx_index), Value: (tx_hash, proof)
    pub proof_cache: Arc<RwLock<ProofCache>>,
    // DAG engine for high-throughput consensus (lazy-initialized)
    dag_engine: Arc<RwLock<Option<DagEngine>>>,
    // Authority ID for this node (used in DAG mode)
    authority_id: String,
    // List of all authorities (validators) in the network
    authorities: Vec<String>,
    // Persisted DAG state, loaded on startup
    persisted_dag_state: Option<PersistentDagState>,
    // Optional production-safe DAG signing key. When absent, DAG mode uses
    // deterministic demo keys for tests/local development only.
    consensus_signing_key: Option<ed25519_dalek::SigningKey>,
    consensus_public_keys: BTreeMap<String, Vec<u8>>,
}

// Basic recursive parser for simple type-argument strings used by RPC/tests.
fn parse_type_tag(s: &str) -> Option<TypeTag> {
    fn split_top_level_commas(s: &str) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut depth: usize = 0;
        let mut start = 0usize;
        for (i, ch) in s.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth = depth.saturating_sub(1);
                }
                ',' if depth == 0 => {
                    parts.push(s[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
        parts.push(s[start..].trim());
        parts
    }

    let s = s.trim();
    match s {
        "bool" => return Some(TypeTag::Bool),
        "u8" => return Some(TypeTag::U8),
        "u64" => return Some(TypeTag::U64),
        "u128" => return Some(TypeTag::U128),
        "address" => return Some(TypeTag::Address),
        _ => {}
    }

    if let Some(inner) = s.strip_prefix("vector<")
        && let Some(inner) = inner.strip_suffix('>')
    {
        return parse_type_tag(inner).map(|t| TypeTag::Vector(Box::new(t)));
    }

    if s.contains("::") {
        let parts = s.split("::").collect::<Vec<_>>();
        if parts.len() >= 3 {
            let addr_str = parts[0].trim();
            let module_str = parts[1].trim();
            let name_and_generics = parts[2..].join("::").trim().to_string();

            let (name_str, generics_opt) = if let Some(idx) = name_and_generics.find('<') {
                if !name_and_generics.ends_with('>') || idx + 1 >= name_and_generics.len() {
                    return None;
                }
                let name = &name_and_generics[..idx];
                let generics = &name_and_generics[idx + 1..name_and_generics.len() - 1];
                (name.trim(), Some(generics))
            } else {
                (name_and_generics.as_str(), None)
            };

            let addr = KanariAddress::parse_to_account_address(addr_str).ok()?;
            let module_id = Identifier::new(module_str).ok()?;
            let name_id = Identifier::new(name_str).ok()?;

            let mut type_params = Vec::new();
            if let Some(r#gen) = generics_opt {
                for g in split_top_level_commas(r#gen) {
                    if g.is_empty() {
                        continue;
                    }
                    let parsed = parse_type_tag(g)?;
                    type_params.push(parsed);
                }
            }

            let st = StructTag {
                address: addr,
                module: module_id,
                name: name_id,
                type_params,
            };
            return Some(TypeTag::Struct(Box::new(st)));
        }
    }

    None
}

impl BlockchainEngine {
    fn checkpoint_transactions_key(sequence: u64) -> Vec<u8> {
        format!("checkpoint_txs/{sequence:020}").into_bytes()
    }

    fn checkpoint_metadata_key(sequence: u64) -> Vec<u8> {
        format!("checkpoint_meta/{sequence:020}").into_bytes()
    }

    fn transaction_payload_key(tx_hash: &[u8]) -> Vec<u8> {
        let mut key = b"tx_payload/".to_vec();
        key.extend_from_slice(hex::encode(tx_hash).as_bytes());
        key
    }

    fn transaction_index_key(tx_hash: &[u8]) -> Vec<u8> {
        let mut key = b"tx_index/".to_vec();
        key.extend_from_slice(hex::encode(tx_hash).as_bytes());
        key
    }

    fn recent_transaction_hashes_key() -> &'static [u8] {
        b"tx_recent"
    }

    fn vertex_transactions_key(vertex_id: &[u8; 32]) -> Vec<u8> {
        let mut key = b"dag_vertex_txs/".to_vec();
        key.extend_from_slice(hex::encode(vertex_id).as_bytes());
        key
    }

    fn checkpoint_without_transactions(checkpoint: &Checkpoint) -> Checkpoint {
        Checkpoint::new(
            checkpoint.sequence,
            checkpoint.vertices.clone(),
            Vec::new(),
            checkpoint.state_root.clone(),
            checkpoint.timestamp,
            checkpoint.prev_checkpoint_hash.clone(),
        )
    }

    fn vertex_without_transactions(vertex: &DagVertex) -> DagVertex {
        let mut slim = vertex.clone();
        slim.transactions = Vec::new().into();
        slim
    }

    fn persist_checkpoint_transactions(
        store: &PersistentStore,
        checkpoint: &Checkpoint,
    ) -> Result<()> {
        store
            .save(
                &Self::checkpoint_metadata_key(checkpoint.sequence),
                &Self::checkpoint_without_transactions(checkpoint),
            )
            .context("Failed to persist checkpoint metadata")?;

        if checkpoint.transactions.is_empty() || checkpoint.sequence == 0 {
            return Ok(());
        }

        let mut recent_hashes = store
            .load::<Vec<Vec<u8>>>(Self::recent_transaction_hashes_key())
            .unwrap_or_default()
            .unwrap_or_default();
        let mut recent_set: HashSet<Vec<u8>> = recent_hashes.iter().cloned().collect();

        for tx in checkpoint.transactions.iter() {
            let tx_hash = tx.transaction_hash().to_vec();
            store
                .save(&Self::transaction_payload_key(&tx_hash), tx)
                .context("Failed to persist transaction payload")?;
            store
                .save(
                    &Self::transaction_index_key(&tx_hash),
                    &PersistedTransactionLocation {
                        checkpoint_sequence: checkpoint.sequence,
                        state_root: checkpoint.state_root.clone(),
                    },
                )
                .context("Failed to persist transaction hash index")?;

            if recent_set.insert(tx_hash.clone()) {
                recent_hashes.push(tx_hash);
            }
        }

        if recent_hashes.len() > MAX_PERSISTED_RECENT_TX_HASHES {
            let trim = recent_hashes.len() - MAX_PERSISTED_RECENT_TX_HASHES;
            recent_hashes.drain(0..trim);
        }
        store
            .save(Self::recent_transaction_hashes_key(), &recent_hashes)
            .context("Failed to persist recent transaction index")?;

        store
            .save(
                &Self::checkpoint_transactions_key(checkpoint.sequence),
                &checkpoint.transactions,
            )
            .context("Failed to persist checkpoint transaction payload")?;
        Ok(())
    }

    fn load_checkpoint_metadata(store: &PersistentStore, sequence: u64) -> Option<Checkpoint> {
        store
            .load(&Self::checkpoint_metadata_key(sequence))
            .map_err(|e| {
                tracing::warn!(
                    checkpoint = sequence,
                    "Failed to load checkpoint metadata: {}",
                    e
                );
                e
            })
            .ok()
            .flatten()
    }

    fn load_checkpoint_transactions(
        store: &PersistentStore,
        sequence: u64,
    ) -> Option<crate::consensus::TransactionBatch> {
        store
            .load(&Self::checkpoint_transactions_key(sequence))
            .map_err(|e| {
                tracing::warn!(
                    checkpoint = sequence,
                    "Failed to load checkpoint transaction payload: {}",
                    e
                );
                e
            })
            .ok()
            .flatten()
    }

    fn load_transaction_by_hash_from_index(
        store: &PersistentStore,
        tx_hash: &[u8],
    ) -> Option<(SignedTransaction, PersistedTransactionLocation)> {
        let location = store
            .load::<PersistedTransactionLocation>(&Self::transaction_index_key(tx_hash))
            .map_err(|e| {
                tracing::warn!(
                    tx_hash = %hex::encode(tx_hash),
                    "Failed to load transaction index: {}",
                    e
                );
                e
            })
            .ok()
            .flatten()?;
        let tx = store
            .load::<SignedTransaction>(&Self::transaction_payload_key(tx_hash))
            .map_err(|e| {
                tracing::warn!(
                    tx_hash = %hex::encode(tx_hash),
                    "Failed to load transaction payload: {}",
                    e
                );
                e
            })
            .ok()
            .flatten()?;
        Some((tx, location))
    }

    fn persist_vertex_transactions(store: &PersistentStore, vertex: &DagVertex) -> Result<()> {
        if vertex.transactions.is_empty() {
            return Ok(());
        }
        store
            .save(
                &Self::vertex_transactions_key(&vertex.id),
                &vertex.transactions,
            )
            .context("Failed to persist DAG vertex transaction payload")?;
        Ok(())
    }

    fn persist_blockchain_snapshot_to_store(
        store: &PersistentStore,
        chain: &Blockchain,
    ) -> Result<()> {
        for checkpoint in &chain.dag_checkpoints {
            Self::persist_checkpoint_transactions(store, checkpoint)?;
        }

        let mut slim = chain.clone();
        for checkpoint in &mut slim.dag_checkpoints {
            if !checkpoint.transactions.is_empty() {
                *checkpoint = Self::checkpoint_without_transactions(checkpoint);
            }
        }
        store
            .save(b"blockchain", &slim)
            .context("Failed to persist blockchain metadata")?;
        Ok(())
    }

    pub(crate) fn persist_blockchain_snapshot(&self, chain: &Blockchain) -> Result<()> {
        let Some(store) = &self.persistent_store else {
            return Ok(());
        };
        Self::persist_blockchain_snapshot_to_store(store, chain)
    }

    fn hydrate_blockchain_transactions(store: &PersistentStore, chain: &mut Blockchain) {
        for checkpoint in &mut chain.dag_checkpoints {
            if !checkpoint.transactions.is_empty() || checkpoint.sequence == 0 {
                continue;
            }
            if let Some(transactions) =
                Self::load_checkpoint_transactions(store, checkpoint.sequence)
            {
                checkpoint.transactions = transactions;
            }
        }
    }

    fn slim_persistent_dag_state(state: &PersistentDagState) -> PersistentDagState {
        PersistentDagState {
            vertices: state
                .vertices
                .iter()
                .map(Self::vertex_without_transactions)
                .collect(),
            checkpoints: state
                .checkpoints
                .iter()
                .map(Self::checkpoint_without_transactions)
                .collect(),
            current_round: state.current_round,
            last_checkpoint_round: state.last_checkpoint_round,
        }
    }

    fn persist_dag_payloads(store: &PersistentStore, state: &PersistentDagState) -> Result<()> {
        for vertex in &state.vertices {
            Self::persist_vertex_transactions(store, vertex)?;
        }
        for checkpoint in &state.checkpoints {
            Self::persist_checkpoint_transactions(store, checkpoint)?;
        }
        Ok(())
    }

    fn hydrate_dag_state_transactions(store: &PersistentStore, state: &mut PersistentDagState) {
        for vertex in &mut state.vertices {
            if !vertex.transactions.is_empty() {
                continue;
            }
            match store.load(&Self::vertex_transactions_key(&vertex.id)) {
                Ok(Some(transactions)) => vertex.transactions = transactions,
                Ok(None) => {}
                Err(e) => tracing::warn!(
                    vertex = %hex::encode(vertex.id),
                    "Failed to hydrate DAG vertex transaction payload: {}",
                    e
                ),
            }
        }

        for checkpoint in &mut state.checkpoints {
            if !checkpoint.transactions.is_empty() || checkpoint.sequence == 0 {
                continue;
            }
            if let Some(transactions) =
                Self::load_checkpoint_transactions(store, checkpoint.sequence)
            {
                checkpoint.transactions = transactions;
            }
        }
    }

    pub fn get_committed_transaction_from_history(
        &self,
        tx_hash: &[u8],
    ) -> Option<(SignedTransaction, u64, Vec<u8>)> {
        let store = self.persistent_store.as_ref()?;
        if let Some((tx, location)) = Self::load_transaction_by_hash_from_index(store, tx_hash) {
            return Some((tx, location.checkpoint_sequence, location.state_root));
        }

        let height = self.get_stats().height;

        for sequence in (1..=height).rev() {
            let Some(transactions) = Self::load_checkpoint_transactions(store, sequence) else {
                continue;
            };
            for tx in transactions.iter().rev() {
                if tx.transaction_hash() == tx_hash {
                    let state_root = Self::load_checkpoint_metadata(store, sequence)
                        .map(|checkpoint| checkpoint.state_root)
                        .unwrap_or_default();
                    return Some((tx.clone(), sequence, state_root));
                }
            }
        }

        None
    }

    pub fn list_committed_transactions_from_history<F>(
        &self,
        limit: usize,
        mut matches: F,
    ) -> Vec<(SignedTransaction, u64, Vec<u8>)>
    where
        F: FnMut(&Transaction) -> bool,
    {
        let Some(store) = self.persistent_store.as_ref() else {
            return Vec::new();
        };
        let mut results = Vec::with_capacity(limit);
        let mut seen_hashes = HashSet::new();

        if let Ok(Some(recent_hashes)) =
            store.load::<Vec<Vec<u8>>>(Self::recent_transaction_hashes_key())
        {
            for tx_hash in recent_hashes.iter().rev() {
                if results.len() >= limit {
                    break;
                }
                if !seen_hashes.insert(tx_hash.clone()) {
                    continue;
                }
                let Some((tx, location)) =
                    Self::load_transaction_by_hash_from_index(store, tx_hash)
                else {
                    continue;
                };
                if matches(&tx.transaction) {
                    results.push((tx, location.checkpoint_sequence, location.state_root));
                }
            }
        }

        if results.len() >= limit {
            return results;
        }

        let height = self.get_stats().height;

        for sequence in (1..=height).rev() {
            if results.len() >= limit {
                break;
            }

            let Some(transactions) = Self::load_checkpoint_transactions(store, sequence) else {
                continue;
            };
            let state_root = Self::load_checkpoint_metadata(store, sequence)
                .map(|checkpoint| checkpoint.state_root)
                .unwrap_or_default();

            for tx in transactions.iter().rev() {
                if results.len() >= limit {
                    break;
                }
                if !seen_hashes.insert(tx.transaction_hash().to_vec()) {
                    continue;
                }
                if matches(&tx.transaction) {
                    results.push((tx.clone(), sequence, state_root.clone()));
                }
            }
        }

        results
    }

    pub fn state_read(&self) -> RwLockReadGuard<'_, StateManager> {
        self.state.read().unwrap_or_else(|poisoned| {
            error!("State lock poisoned while reading runtime state; recovering...");
            poisoned.into_inner()
        })
    }

    pub fn state_write(&self) -> RwLockWriteGuard<'_, StateManager> {
        self.state.write().unwrap_or_else(|poisoned| {
            error!("State lock poisoned while writing runtime state; recovering...");
            poisoned.into_inner()
        })
    }

    pub(crate) fn mempool_read(&self) -> RwLockReadGuard<'_, MempoolState> {
        self.mempool.read().unwrap_or_else(|poisoned| {
            error!("Mempool lock poisoned while reading pending state; recovering...");
            poisoned.into_inner()
        })
    }

    pub(crate) fn mempool_write(&self) -> RwLockWriteGuard<'_, MempoolState> {
        self.mempool.write().unwrap_or_else(|poisoned| {
            error!("Mempool lock poisoned while writing pending state; recovering...");
            poisoned.into_inner()
        })
    }

    pub fn pending_transactions_snapshot(&self) -> Vec<SignedTransaction> {
        self.mempool_read().pending_txs.clone()
    }

    pub fn pending_transaction_len(&self) -> usize {
        self.mempool_read().pending_txs.len()
    }

    pub(crate) fn get_expected_sequence(&self, address_hex: &str) -> u64 {
        let mut seq = self
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get_account_by_hex(address_hex)
            .map(|acc| acc.sequence_number)
            .unwrap_or(0);

        seq += self.pending_tx_count_for_sender(address_hex);
        seq
    }

    fn resolve_account_objects(
        &self,
        state: &StateManager,
        owner_addr: &AccountAddress,
    ) -> Vec<ObjectInfo> {
        let mut unique_ids = state.get_owned_objects(owner_addr).unwrap_or_default();
        unique_ids.sort();
        unique_ids.dedup();

        let mut coins = Vec::new();
        let mut others = Vec::new();

        for id in unique_ids {
            if let Ok(Some(obj)) = state.get_object(&id) {
                let info = ObjectInfo {
                    id: id.clone(),
                    owner: format!("{:#x}", obj.owner),
                    type_: obj.type_.clone(),
                    data: obj.data.clone(),
                    version: obj.version,
                };

                if obj.type_.contains("::coin::Coin<") && obj.data.len() >= 40 {
                    let mut arr = [0u8; 8];
                    arr.copy_from_slice(&obj.data[32..40]);
                    let amount = u64::from_le_bytes(arr);
                    if amount > 0 {
                        coins.push((amount, info));
                        continue;
                    }
                }
                others.push(info);
            }
        }

        coins.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        others.sort_by(|a, b| a.id.cmp(&b.id));
        coins
            .into_iter()
            .map(|(_, info)| info)
            .chain(others)
            .collect()
    }

    #[cfg(test)]
    fn execute_tx_waves_parallel(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
        strict_mode: bool,
    ) -> Result<(usize, usize)> {
        self.execute_tx_waves_parallel_inner(
            transactions,
            state_arc,
            timestamp,
            persist_objects,
            strict_mode,
            strict_mode,
        )
    }

    pub(crate) fn execute_tx_waves_deterministic_parallel(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
    ) -> Result<(usize, usize)> {
        self.execute_tx_waves_parallel_inner(
            transactions,
            state_arc,
            timestamp,
            persist_objects,
            false,
            true,
        )
    }

    pub(crate) fn execute_tx_waves_strict_serial(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
    ) -> Result<(usize, usize)> {
        self.execute_tx_waves_parallel_inner(
            transactions,
            state_arc,
            timestamp,
            persist_objects,
            true,
            true,
        )
    }

    pub(crate) fn apply_zero_effect_native_batch(
        &self,
        transactions: &[SignedTransaction],
        state_arc: &Arc<RwLock<StateManager>>,
    ) -> Result<Option<(usize, usize)>> {
        if transactions.is_empty() {
            return Ok(Some((0, 0)));
        }

        let mut sequence_increments: AHashMap<AccountAddress, u64> = AHashMap::default();
        let zero_amount = 0u64.to_le_bytes();

        for signed_tx in transactions {
            let Transaction::ExecuteFunction {
                sender,
                module,
                function,
                args,
                gas_price,
                ..
            } = &signed_tx.transaction
            else {
                return Ok(None);
            };
            if *gas_price != 0 || module != Transaction::KANARI_MODULE {
                return Ok(None);
            }

            let is_zero_native_call = matches!(
                function.as_str(),
                Transaction::BURN_AMOUNT_FUNCTION | Transaction::TRANSFER_AMOUNT_FUNCTION
            ) && args
                .first()
                .is_some_and(|amount| amount.as_slice() == zero_amount);
            if !is_zero_native_call {
                return Ok(None);
            }

            let sender_addr = KanariAddress::parse_to_account_address(sender)?;
            *sequence_increments.entry(sender_addr).or_insert(0) += 1;
        }

        let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
        state_write
            .apply_zero_effect_sequence_batch(sequence_increments)
            .map_err(|e| anyhow::anyhow!("Failed to apply zero-effect native batch: {}", e))?;

        Ok(Some((transactions.len(), 0)))
    }

    fn execute_tx_waves_parallel_inner(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
        serial_execution: bool,
        fail_hard: bool,
    ) -> Result<(usize, usize)> {
        let mut executed_count = 0;
        let mut failed_count = 0;
        let has_module_publish = transactions
            .iter()
            .any(|tx| matches!(tx.transaction, Transaction::PublishModule { .. }));

        if serial_execution {
            if has_module_publish {
                self.runtime_pool[0].reload_vm_cache()?;
            }

            let mut executed_count = 0;
            let failed_count = 0;
            for signed_tx in transactions {
                let changeset = self.execute_transaction_with_runtime_internal(
                    &signed_tx.transaction,
                    &self.runtime_pool[0],
                    state_arc,
                    false,
                    timestamp,
                    persist_objects,
                )?;

                let mut state_write = match state_arc.write() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        log::error!("State lock poisoned during strict execution, recovering...");
                        poisoned.into_inner()
                    }
                };

                if persist_objects {
                    let runtime = &self.runtime_pool[0];
                    runtime.persist_created_objects(&changeset);
                    runtime.persist_deleted_objects(&changeset);
                }

                state_write
                    .apply_changeset(&changeset)
                    .map_err(|e| anyhow::anyhow!("Failed to apply changeset: {}", e))?;
                executed_count += 1;
            }

            return Ok((executed_count, failed_count));
        }

        let waves = kanari_move_runtime_v1::TransactionScheduler::schedule(transactions);

        if has_module_publish {
            // Keep speculative publish execution deterministic across authorities.
            // PublishModule depends on VM/module cache state more heavily than regular
            // user transactions, so we reset the shared cache and execute on one
            // runtime in a fixed serial order for DAG production / validation.
            self.runtime_pool[0].reload_vm_cache()?;
        }

        for wave in waves {
            let results: Vec<Result<ChangeSet>> = if has_module_publish {
                wave.iter()
                    .map(|signed_tx| {
                        self.execute_transaction_with_runtime_internal(
                            &signed_tx.transaction,
                            &self.runtime_pool[0],
                            state_arc,
                            false,
                            timestamp,
                            persist_objects,
                        )
                    })
                    .collect()
            } else {
                wave.par_iter()
                    .enumerate()
                    .map(|(i, signed_tx)| {
                        let runtime = &self.runtime_pool[i % self.runtime_pool.len()];
                        self.execute_transaction_with_runtime_internal(
                            &signed_tx.transaction,
                            runtime,
                            state_arc,
                            false,
                            timestamp,
                            persist_objects,
                        )
                    })
                    .collect()
            };

            if fail_hard {
                let mut wave_changeset = ChangeSet::new();
                let mut wave_executed = 0usize;

                for res in results {
                    let cs = res.map_err(|e| anyhow::anyhow!("Execution failed: {}", e))?;

                    if persist_objects {
                        let runtime = &self.runtime_pool[0];
                        runtime.persist_created_objects(&cs);
                        runtime.persist_deleted_objects(&cs);
                    }

                    wave_changeset.merge(cs);
                    wave_executed += 1;
                }

                if wave_executed == 0 {
                    continue;
                }

                let mut state_write = match state_arc.write() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        log::error!("State lock poisoned during wave execution, recovering...");
                        poisoned.into_inner()
                    }
                };

                state_write
                    .apply_changeset_without_supply_validation(&wave_changeset)
                    .map_err(|e| anyhow::anyhow!("Failed to apply changeset: {}", e))?;
                executed_count += wave_executed;
            } else {
                // Apply changesets with proper error handling to prevent node crashes
                let mut state_write = match state_arc.write() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        log::error!("State lock poisoned during wave execution, recovering...");
                        poisoned.into_inner()
                    }
                };

                for res in results {
                    match res {
                        Ok(cs) => {
                            if persist_objects {
                                let runtime = &self.runtime_pool[0];
                                runtime.persist_created_objects(&cs);
                                runtime.persist_deleted_objects(&cs);
                            }

                            if let Err(e) = state_write.apply_changeset(&cs) {
                                log::warn!("apply_changeset failed: {}", e);
                                failed_count += 1;
                            } else {
                                executed_count += 1;
                            }
                        }
                        Err(e) => {
                            log::warn!("Parallel execution failed: {}", e);
                            failed_count += 1;
                        }
                    }
                }
            }
        }

        Ok((executed_count, failed_count))
    }

    pub(crate) fn checkpoint_root_matches(
        &self,
        checkpoint_sequence: u64,
        computed_root: &[u8],
        checkpoint_root: &[u8],
    ) -> Result<bool> {
        if computed_root == checkpoint_root {
            return Ok(true);
        }

        if Self::strict_checkpoint_roots_required() {
            anyhow::bail!(
                "[ENGINE] Strict checkpoint root verification failed for checkpoint {}",
                checkpoint_sequence
            );
        }

        Ok(false)
    }

    fn apply_gas_and_sequence(
        changeset: &mut ChangeSet,
        sender: AccountAddress,
        gas_cost: u64,
        gas_used: u64,
    ) -> Result<()> {
        let sender_change = changeset.get_or_create_change(sender);
        if sender_change.sequence_increment == 0 {
            sender_change.increment_sequence();
        }
        sender_change.debit(gas_cost);

        let dao_addr = AccountAddress::from_hex_literal(KanariAddress::DAO_ADDRESS)?;
        changeset.collect_gas(dao_addr, gas_cost);
        changeset.set_gas_used(gas_used);
        Ok(())
    }

    pub(crate) fn persist_dag_state(&self, state: PersistentDagState) -> Result<()> {
        if let Some(store) = &self.persistent_store {
            Self::persist_dag_payloads(store, &state)?;
            let slim = Self::slim_persistent_dag_state(&state);
            store
                .save(b"dag_state", &slim)
                .context("Failed to persist DAG state")?;
        }
        Ok(())
    }

    fn execute_transaction_with_runtime(
        &self,
        tx: &Transaction,
        runtime: &kanari_move_runtime_v1::move_runtime::MoveRuntime,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
    ) -> Result<ChangeSet> {
        self.execute_transaction_with_runtime_internal(
            tx, runtime, state_arc, true, timestamp, false,
        )
    }

    pub(crate) fn execute_transaction_with_runtime_internal(
        &self,
        tx: &Transaction,
        runtime: &kanari_move_runtime_v1::move_runtime::MoveRuntime,
        state_arc: &Arc<RwLock<StateManager>>,
        validate_sequence: bool,
        timestamp: Option<u64>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        let sender_addr = KanariAddress::parse_to_account_address(tx.sender_address())?;
        let mut gas_meter = GasMeter::new(tx.gas_limit(), tx.gas_price());
        let mut changeset = ChangeSet::new();

        let native_call = tx.native_call();

        let (gas_op, required_amount) = match tx {
            Transaction::PublishModule { module_bytes, .. } => (
                GasOperation::PublishModule {
                    module_size: module_bytes.len(),
                },
                0,
            ),
            Transaction::ExecuteFunction { .. } => {
                if let Some(native_call) = &native_call {
                    (GasOperation::Transfer, native_call.required_native_amount())
                } else {
                    (GasOperation::ExecuteFunction { complexity: 1 }, 0)
                }
            }
        };

        gas_meter.consume(gas_op.gas_units())?;
        let gas_cost = gas_meter.total_cost();
        let total_required = required_amount.saturating_add(gas_cost);

        if validate_sequence || total_required > 0 {
            let state = match state_arc.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    log::error!("State arc lock poisoned in pre-execution checks, recovering...");
                    poisoned.into_inner()
                }
            };
            if validate_sequence {
                state
                    .validate_sequence(&sender_addr, tx.sequence_number())
                    .context("Sequence number validation failed")?;
            }
            if total_required > 0 {
                let balance = state
                    .get_account(&sender_addr)
                    .map(|acc| acc.native_balance())
                    .unwrap_or(0);
                if balance < total_required {
                    let msg = if required_amount > 0 {
                        format!(
                            "Insufficient balance: need {} (amount: {}, gas: {}) but have {}",
                            total_required, required_amount, gas_cost, balance
                        )
                    } else {
                        format!(
                            "Insufficient balance for gas: need {}, have {}",
                            gas_cost, balance
                        )
                    };
                    changeset.mark_failed(msg);
                    Self::apply_gas_and_sequence(
                        &mut changeset,
                        sender_addr,
                        gas_cost.min(balance),
                        gas_meter.gas_used,
                    )?;
                    return Ok(changeset);
                }
            }
        }

        if gas_cost > i64::MAX as u64 {
            changeset.mark_failed("Gas cost exceeds the supported range".to_string());
            changeset
                .get_or_create_change(sender_addr)
                .increment_sequence();
            changeset.set_gas_used(gas_meter.gas_used);
            return Ok(changeset);
        }

        if native_call.as_ref().is_some_and(|call| {
            call.required_native_amount() > (i64::MAX as u64).saturating_sub(gas_cost)
        }) {
            changeset.mark_failed("Native amount exceeds the supported range".to_string());
            Self::apply_gas_and_sequence(
                &mut changeset,
                sender_addr,
                gas_cost,
                gas_meter.gas_used,
            )?;
            return Ok(changeset);
        }

        match tx {
            Transaction::PublishModule {
                sender,
                module_bytes,
                ..
            } => {
                match runtime.publish_module_with_context_and_persistence(
                    module_bytes.clone(),
                    KanariAddress::parse_to_account_address(sender)?,
                    None,
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,
                ) {
                    Ok(move_cs) => changeset.merge(move_cs),
                    Err(e) => {
                        changeset.mark_failed(format!("Publish failed: {}", e));
                    }
                }
            }

            Transaction::ExecuteFunction {
                module,
                function,
                type_args,
                args,
                ..
            } => {
                if let Some(native_call) = native_call {
                    match native_call {
                        NativeCall::TransferAmount { recipient, amount } => {
                            let to_addr = KanariAddress::parse_to_account_address(&recipient)?;
                            changeset.transfer(sender_addr, to_addr, amount);
                        }
                        NativeCall::BurnAmount { amount } => {
                            changeset.burn(sender_addr, amount);
                        }
                    }
                    Self::apply_gas_and_sequence(
                        &mut changeset,
                        sender_addr,
                        gas_cost,
                        gas_meter.gas_used,
                    )?;
                    return Ok(changeset);
                }

                let parts: Vec<&str> = module.split("::").collect();
                if parts.len() != 2 {
                    changeset.mark_failed(
                        "Invalid module format. Expected: address::module".to_string(),
                    );
                    changeset.set_gas_used(0);
                    return Ok(changeset);
                }

                let addr = KanariAddress::parse_to_account_address(parts[0])?;
                let module_id = ModuleId::new(
                    addr,
                    move_core_types::identifier::Identifier::new(parts[1])?,
                );

                let type_tags: Vec<move_core_types::language_storage::TypeTag> = type_args
                    .iter()
                    .map(|s| {
                        parse_type_tag(s.as_str())
                            .ok_or_else(|| anyhow::anyhow!("Invalid type argument: {}", s))
                    })
                    .collect::<Result<Vec<_>>>()?;

                match runtime.execute_entry_function_with_tx_hash_and_persistence(
                    &module_id,
                    function,
                    type_tags,
                    args.clone(),
                    Some(sender_addr),
                    None,
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,
                ) {
                    Ok(move_cs) => changeset.merge(move_cs),
                    Err(e) => {
                        changeset.mark_failed(format!("Execution failed: {}", e));
                    }
                }
            }
        }

        Self::apply_gas_and_sequence(&mut changeset, sender_addr, gas_cost, gas_meter.gas_used)?;
        Ok(changeset)
    }

    fn dag_engine_instance(&self) -> Result<DagEngine> {
        let mut dag_engine_guard = match self.dag_engine.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("DAG engine lock poisoned while initializing, recovering...");
                poisoned.into_inner()
            }
        };
        if dag_engine_guard.is_none() {
            let signing_key = self.consensus_signing_key.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "DAG consensus requires an explicit signing key. Call set_consensus_signing_key() before producing or syncing DAG vertices."
                )
            })?;
            let engine = DagEngine::new_secure(
                Arc::new(self.clone_for_dag()),
                self.authority_id.clone(),
                self.authorities.clone(),
                signing_key,
                self.consensus_public_keys.clone(),
            )?;
            *dag_engine_guard = Some(engine);
        }

        dag_engine_guard
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Failed to initialize DAG engine"))
    }

    pub fn produce_checkpoint(&self) -> Result<CheckpointProductionInfo> {
        let dag_engine = self.dag_engine_instance()?;
        let has_pending_transactions = self.pending_transaction_len() > 0;

        if !has_pending_transactions {
            anyhow::bail!("No new transactions to checkpoint");
        }

        dag_engine.produce_vertex()
    }

    pub fn dag_production_policy(&self) -> Result<DagProductionPolicy> {
        let dag_engine = self.dag_engine_instance()?;
        let consensus_lock = dag_engine.consensus();
        let consensus = match consensus_lock.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Consensus lock poisoned while reading DAG production policy");
                poisoned.into_inner()
            }
        };
        Ok(consensus.production_policy())
    }

    pub fn latest_own_dag_vertices(&self, limit: usize) -> Result<Vec<DagVertex>> {
        Ok(self.dag_engine_instance()?.latest_own_vertices(limit))
    }

    pub fn add_network_dag_vertex(&self, vertex: DagVertex) -> Result<()> {
        self.dag_engine_instance()?.add_network_vertex(vertex)
    }

    fn clone_for_dag(&self) -> BlockchainEngine {
        BlockchainEngine {
            blockchain: self.blockchain.clone(),
            state: self.state.clone(),
            mempool: self.mempool.clone(),
            persistent_store: self.persistent_store.clone(),
            runtime_pool: self.runtime_pool.clone(),
            proof_cache: self.proof_cache.clone(),
            dag_engine: Arc::new(RwLock::new(None)),
            authority_id: self.authority_id.clone(),
            authorities: self.authorities.clone(),
            persisted_dag_state: self.persisted_dag_state.clone(),
            consensus_signing_key: self.consensus_signing_key.clone(),
            consensus_public_keys: self.consensus_public_keys.clone(),
        }
    }

    pub fn set_authorities(&mut self, authority_id: String, authorities: Vec<String>) {
        fn normalize(s: String) -> String {
            if s.starts_with("0x") {
                s
            } else {
                format!("0x{}", s)
            }
        }
        self.authority_id = normalize(authority_id);
        self.authorities = authorities.into_iter().map(normalize).collect();
        self.consensus_signing_key = None;
        self.consensus_public_keys.clear();
        match self.dag_engine.write() {
            Ok(mut guard) => *guard = None,
            Err(poisoned) => {
                log::error!("DAG engine lock poisoned in set_authorities, recovering...");
                *poisoned.into_inner() = None;
            }
        }
    }

    pub fn authority_id(&self) -> &str {
        &self.authority_id
    }

    pub fn authorities(&self) -> &[String] {
        &self.authorities
    }

    pub fn set_consensus_signing_key(
        &mut self,
        local_signing_key: ed25519_dalek::SigningKey,
        authority_public_keys: BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        let local_public_key = local_signing_key.verifying_key().to_bytes().to_vec();
        let expected_public_key =
            authority_public_keys
                .get(&self.authority_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Missing consensus public key for local authority {}",
                        self.authority_id
                    )
                })?;
        if *expected_public_key != local_public_key {
            anyhow::bail!("Consensus signing key does not match local authority public key");
        }
        for authority in &self.authorities {
            let key = authority_public_keys.get(authority).ok_or_else(|| {
                anyhow::anyhow!("Missing consensus public key for authority {}", authority)
            })?;
            let key_bytes: [u8; 32] = key.as_slice().try_into().map_err(|_| {
                anyhow::anyhow!("Invalid consensus public key length for {}", authority)
            })?;
            ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).map_err(|e| {
                anyhow::anyhow!("Invalid consensus public key for {}: {}", authority, e)
            })?;
        }

        self.consensus_signing_key = Some(local_signing_key);
        self.consensus_public_keys = authority_public_keys;
        match self.dag_engine.write() {
            Ok(mut guard) => *guard = None,
            Err(poisoned) => {
                log::error!("DAG engine lock poisoned while replacing consensus key");
                *poisoned.into_inner() = None;
            }
        }

        Ok(())
    }

    pub fn export_consensus_metrics_prometheus(&self) -> Result<String> {
        let dag_engine_guard = match self.dag_engine.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("DAG engine lock poisoned in export_consensus_metrics_prometheus");
                poisoned.into_inner()
            }
        };

        if let Some(dag_engine) = dag_engine_guard.as_ref() {
            let consensus_lock = dag_engine.consensus();
            let consensus = match consensus_lock.read() {
                Ok(guard) => guard,
                Err(poisoned) => {
                    log::error!("Consensus lock poisoned in metrics export");
                    poisoned.into_inner()
                }
            };
            return consensus.metrics().export_prometheus();
        }

        DagMetrics::default().export_prometheus()
    }
}

#[cfg(test)]
mod tests {
    use super::BlockchainEngine;
    use crate::blockchain::Blockchain;
    use crate::consensus::{Checkpoint, PersistentDagState};
    use kanari_crypto::keys::{CurveType, generate_keypair};
    use kanari_move_runtime_v1::changeset::ChangeSet;
    use kanari_move_runtime_v1::state::Account;
    use kanari_types::balance::BalanceRecord;
    use kanari_types::kanari::KANARI_TOKEN_TYPE;
    use kanari_types::transaction::{SignedTransaction, Transaction};
    use move_core_types::account_address::AccountAddress;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, RwLock};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn signed_transfer_from(
        sender: &kanari_crypto::keys::KeyPair,
        sequence_number: u64,
    ) -> SignedTransaction {
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        let tx = Transaction::new_transfer(
            sender.tagged_address(),
            recipient.address,
            1,
            sequence_number,
        );
        let mut signed_tx = SignedTransaction::new(tx);
        signed_tx
            .sign(&sender.private_key, sender.curve_type)
            .unwrap();
        signed_tx
    }

    fn fund_sender(engine: &BlockchainEngine, address: &str, balance: u64) {
        let addr = AccountAddress::from_hex_literal(address).unwrap();
        let mut account = Account::with_native_balance(addr, balance);
        account.set_token_balance(KANARI_TOKEN_TYPE.to_string(), BalanceRecord::new(balance));
        engine
            .state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .save_account(&account)
            .unwrap();
    }

    fn secure_consensus_keys(
        authorities: &[String],
        local_authority: &str,
    ) -> (ed25519_dalek::SigningKey, BTreeMap<String, Vec<u8>>) {
        let mut public_keys = BTreeMap::new();
        let mut local_signing_key = None;

        for (index, authority) in authorities.iter().enumerate() {
            let seed = [index as u8 + 11; 32];
            let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
            if authority == local_authority {
                local_signing_key = Some(signing_key.clone());
            }
            public_keys.insert(
                authority.clone(),
                signing_key.verifying_key().to_bytes().to_vec(),
            );
        }

        (
            local_signing_key.expect("local authority must be in authority set"),
            public_keys,
        )
    }

    #[test]
    fn mainnet_defaults_enable_strict_runtime_guards() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        unsafe {
            std::env::set_var("KANARI_NETWORK", "mainnet");
            std::env::remove_var("KANARI_REQUIRE_PERSISTENT_STORAGE");
            std::env::remove_var("KANARI_STRICT_CHECKPOINT_ROOTS");
        }

        assert!(BlockchainEngine::strict_persistence_required());
        assert!(BlockchainEngine::strict_checkpoint_roots_required());

        unsafe {
            std::env::remove_var("KANARI_NETWORK");
        }
    }

    #[test]
    fn explicit_env_overrides_strict_runtime_guards() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        unsafe {
            std::env::set_var("KANARI_NETWORK", "mainnet");
            std::env::set_var("KANARI_REQUIRE_PERSISTENT_STORAGE", "false");
            std::env::set_var("KANARI_STRICT_CHECKPOINT_ROOTS", "0");
        }

        assert!(!BlockchainEngine::strict_persistence_required());
        assert!(!BlockchainEngine::strict_checkpoint_roots_required());

        unsafe {
            std::env::remove_var("KANARI_NETWORK");
            std::env::remove_var("KANARI_REQUIRE_PERSISTENT_STORAGE");
            std::env::remove_var("KANARI_STRICT_CHECKPOINT_ROOTS");
        }
    }

    #[test]
    fn dag_engine_requires_explicit_consensus_signing_key() {
        let engine = BlockchainEngine::new_in_memory().unwrap();

        let err = engine.produce_checkpoint().unwrap_err();

        assert!(err.to_string().contains("requires an explicit signing key"));
    }

    #[test]
    fn configured_dag_engine_rejects_empty_checkpoint() {
        let mut engine = BlockchainEngine::new_in_memory().unwrap();
        let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];
        engine.set_authorities("0x1".to_string(), authorities.clone());
        let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
        engine
            .set_consensus_signing_key(local_key, public_keys)
            .unwrap();

        let err = engine.produce_checkpoint().unwrap_err();

        assert!(err.to_string().contains("No new transactions"));
        assert_eq!(engine.get_stats().height, 0);
    }

    #[test]
    fn restarted_engine_does_not_create_empty_dag_progress() {
        let temp_dir = tempfile::tempdir().unwrap();
        let data_dir = temp_dir.path().to_str().unwrap();
        let authorities = vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()];

        {
            let mut engine = BlockchainEngine::new_dir(data_dir).unwrap();
            if engine.persistent_store.is_none() {
                return;
            }
            engine.set_authorities("0x1".to_string(), authorities.clone());
            let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
            engine
                .set_consensus_signing_key(local_key, public_keys)
                .unwrap();

            let err = engine.produce_checkpoint().unwrap_err();
            assert!(err.to_string().contains("No new transactions"));
            assert_eq!(engine.get_stats().height, 0);
        }

        let mut restarted = BlockchainEngine::new_dir(data_dir).unwrap();
        if restarted.persistent_store.is_none() {
            return;
        }
        restarted.set_authorities("0x1".to_string(), authorities.clone());
        let (local_key, public_keys) = secure_consensus_keys(&authorities, "0x1");
        restarted
            .set_consensus_signing_key(local_key, public_keys)
            .unwrap();

        assert_eq!(restarted.get_stats().pending_transactions, 0);
        assert_eq!(restarted.get_stats().height, 0);
        let err = restarted.produce_checkpoint().unwrap_err();
        assert!(err.to_string().contains("No new transactions"));
    }

    #[test]
    fn restart_repairs_missing_transaction_history_from_dag_state() {
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let tx = signed_transfer_from(&sender, 0);

        let genesis = Checkpoint::genesis();
        let prev_hash = genesis.hash().unwrap();
        let vertex_id = [9u8; 32];
        let state_root = vec![7u8; 32];
        let timestamp = 42;

        let broken_checkpoint = Checkpoint::new(
            1,
            vec![vertex_id],
            Vec::new(),
            state_root.clone(),
            timestamp,
            prev_hash.clone(),
        );
        let good_checkpoint = Checkpoint::new(
            1,
            vec![vertex_id],
            vec![tx],
            state_root,
            timestamp,
            prev_hash,
        );

        let mut broken_chain = Blockchain::new();
        broken_chain
            .add_checkpoint_with_validation(broken_checkpoint, false)
            .unwrap();
        let mut chain = Arc::new(RwLock::new(broken_chain));

        let repaired = BlockchainEngine::repair_blockchain_from_dag_state(
            &mut chain,
            Some(&PersistentDagState {
                vertices: Vec::new(),
                checkpoints: vec![genesis, good_checkpoint],
                current_round: 1,
                last_checkpoint_round: 1,
            }),
        )
        .unwrap();

        assert!(repaired);
        let repaired_chain = chain.read().unwrap_or_else(|e| e.into_inner());
        assert_eq!(repaired_chain.height(), 1);
        assert_eq!(repaired_chain.get_transaction_count(), 1);
        assert_eq!(repaired_chain.latest_checkpoint().transactions.len(), 1);
    }

    #[test]
    fn committed_transaction_history_survives_metadata_stripping() {
        let temp_dir = tempfile::tempdir().unwrap();
        let data_dir = temp_dir.path().to_str().unwrap();
        let engine = BlockchainEngine::new_dir(data_dir).unwrap();
        if engine.persistent_store.is_none() {
            return;
        }

        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let tx = signed_transfer_from(&sender, 0);
        let tx_hash = tx.transaction_hash().to_vec();
        let genesis_hash = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let checkpoint = Checkpoint::new(
            1,
            vec![[7u8; 32]],
            vec![tx],
            vec![9u8; 32],
            42,
            genesis_hash,
        );

        {
            let mut chain = engine.blockchain.write().unwrap_or_else(|e| e.into_inner());
            chain
                .add_checkpoint_with_validation(checkpoint, false)
                .unwrap();
            engine.persist_blockchain_snapshot(&chain).unwrap();
        }

        let latest = engine.list_committed_transactions_from_history(10, |_| true);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].1, 1);
        assert_eq!(latest[0].0.transaction_hash(), tx_hash.as_slice());

        let found = engine
            .get_committed_transaction_from_history(&tx_hash)
            .expect("transaction must be found in persistent history");
        assert_eq!(found.1, 1);
        assert_eq!(found.0.transaction_hash(), tx_hash.as_slice());
    }

    #[test]
    fn batch_submit_accepts_contiguous_sequences_for_same_sender() {
        let engine = BlockchainEngine::new().unwrap();
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let tx0 = signed_transfer_from(&sender, 0);
        let tx1 = signed_transfer_from(&sender, 1);

        let hashes = engine.submit_transactions_batch(vec![tx0, tx1]).unwrap();

        assert_eq!(hashes.len(), 2);
        assert_eq!(engine.pending_transaction_len(), 2);
    }

    #[test]
    fn batch_submit_accepts_shuffled_contiguous_sequences_for_same_sender() {
        let engine = BlockchainEngine::new().unwrap();
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let tx0 = signed_transfer_from(&sender, 0);
        let tx1 = signed_transfer_from(&sender, 1);
        let tx2 = signed_transfer_from(&sender, 2);

        let hashes = engine
            .submit_transactions_batch(vec![tx2.clone(), tx0.clone(), tx1.clone()])
            .unwrap();

        assert_eq!(hashes.len(), 3);
        let pending = engine.pending_transactions_snapshot();
        let pending_sequences = pending
            .iter()
            .map(|tx| tx.transaction.sequence_number())
            .collect::<Vec<_>>();
        assert_eq!(pending_sequences, vec![0, 1, 2]);
    }

    #[test]
    fn gas_application_does_not_increment_sequence_twice() {
        let sender = AccountAddress::random();
        let mut changeset = ChangeSet::new();
        changeset.get_or_create_change(sender).increment_sequence();

        BlockchainEngine::apply_gas_and_sequence(&mut changeset, sender, 10, 10).unwrap();

        let sender_change = changeset.account_changes.get(&sender).unwrap();
        assert_eq!(sender_change.sequence_increment, 1);
        assert_eq!(sender_change.balance_delta, -10);
    }

    #[test]
    fn failed_transaction_cannot_mint_unpaid_gas_to_dao() {
        let engine = BlockchainEngine::new_in_memory().unwrap();
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let signed_tx = signed_transfer_from(&sender, 0);
        let dao =
            AccountAddress::from_hex_literal(kanari_types::address::Address::DAO_ADDRESS).unwrap();
        let (supply_before, dao_balance_before) = {
            let state = engine.state_read();
            (
                state.total_supply,
                state
                    .get_account(&dao)
                    .map(|account| account.native_balance())
                    .unwrap_or(0),
            )
        };

        let (_, changeset) = engine.execute_transaction_immediate(signed_tx).unwrap();
        assert!(!changeset.success);
        {
            let mut state = engine.state_write();
            state.apply_changeset(&changeset).unwrap();
            assert_eq!(state.total_supply, supply_before);
            assert_eq!(
                state
                    .get_account(&dao)
                    .map(|account| account.native_balance())
                    .unwrap_or(0),
                dao_balance_before
            );
            let sender_address = AccountAddress::from_hex_literal(&sender.address).unwrap();
            let sender_account = state.get_account(&sender_address).unwrap();
            assert_eq!(sender_account.native_balance(), 0);
            assert_eq!(sender_account.sequence_number, 1);
        }
    }

    #[test]
    fn batch_submit_rejects_duplicate_transactions() {
        let engine = BlockchainEngine::new().unwrap();
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let tx = signed_transfer_from(&sender, 0);

        let err = engine
            .submit_transactions_batch(vec![tx.clone(), tx])
            .unwrap_err();

        assert!(err.to_string().contains("already in pending pool"));
    }

    #[test]
    fn batch_submit_rejects_transaction_already_indexed_in_pending_pool() {
        let engine = BlockchainEngine::new().unwrap();
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let tx = signed_transfer_from(&sender, 0);

        engine.submit_transactions_batch(vec![tx.clone()]).unwrap();
        let err = engine.submit_transactions_batch(vec![tx]).unwrap_err();

        assert!(err.to_string().contains("already in pending pool"));
    }

    #[test]
    fn batch_submit_rejects_sequence_gaps() {
        let engine = BlockchainEngine::new().unwrap();
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let tx = signed_transfer_from(&sender, 1);

        let err = engine.submit_transactions_batch(vec![tx]).unwrap_err();

        assert!(err.to_string().contains("Sequence number too high"));
    }

    #[test]
    fn deterministic_parallel_execution_matches_strict_serial_root() {
        let engine = BlockchainEngine::new_in_memory().unwrap();
        let mut txs = Vec::new();

        for _ in 0..16 {
            let sender = generate_keypair(CurveType::Ed25519).unwrap();
            let recipient = generate_keypair(CurveType::Ed25519).unwrap();
            fund_sender(&engine, &sender.address, 1_000_000);

            let tx =
                Transaction::new_transfer(sender.tagged_address(), recipient.address.clone(), 1, 0);
            let mut signed_tx = SignedTransaction::new(tx);
            signed_tx
                .sign(&sender.private_key, sender.curve_type)
                .unwrap();
            txs.push(signed_tx);
        }

        let base_state = engine
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let strict_state = Arc::new(RwLock::new(base_state.clone()));
        let parallel_state = Arc::new(RwLock::new(base_state));

        let strict_counts = engine
            .execute_tx_waves_parallel(txs.clone(), &strict_state, Some(123), false, true)
            .unwrap();
        let parallel_counts = engine
            .execute_tx_waves_deterministic_parallel(txs, &parallel_state, Some(123), false)
            .unwrap();

        let strict_root = strict_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .compute_state_root();
        let parallel_root = parallel_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .compute_state_root();

        assert_eq!(strict_counts, parallel_counts);
        assert_eq!(strict_root, parallel_root);
    }
}
