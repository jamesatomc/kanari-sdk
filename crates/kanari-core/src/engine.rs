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
use kanari_types::{GasConfig, GasMeter, GasOperation};
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
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

type ProofCache = LruCache<(u64, usize), (String, Vec<Vec<u8>>)>;

mod apply_checkpoint;
mod bootstrap;
mod mempool;
mod produce_dag_vertex;
mod queries;
mod runtime_guards;
pub use produce_dag_vertex::{CheckpointProductionInfo, ConsensusUpdate, DagEngine};
pub use runtime_guards::{RuntimeGuardConfig, RuntimeHealthReport};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointSyncData {
    pub checkpoint: Checkpoint,
}

const MAX_MEMPOOL_SIZE: usize = 50_000;
const MAX_PERSISTED_RECENT_TX_HASHES: usize = 100_000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedTransactionLocation {
    checkpoint_sequence: u64,
    state_root: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TransactionExecutionReceipt {
    pub transaction_hash: Vec<u8>,
    pub success: bool,
    pub gas_used: u64,
    pub error_message: Option<String>,
}

impl TransactionExecutionReceipt {
    fn from_changeset(transaction: &SignedTransaction, changeset: &ChangeSet) -> Self {
        Self {
            transaction_hash: transaction.transaction_hash().to_vec(),
            success: changeset.success,
            gas_used: changeset.gas_used,
            error_message: changeset.error_message.clone(),
        }
    }
}

fn panic_payload_to_string(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[derive(Debug, Default)]
pub(crate) struct TransactionBatchExecution {
    pub executed: usize,
    pub failed: usize,
    pub receipts: Vec<TransactionExecutionReceipt>,
}

#[cfg(test)]
static FORCE_TX_EXECUTION_PANIC: AtomicBool = AtomicBool::new(false);

impl TransactionBatchExecution {
    fn counts(&self) -> (usize, usize) {
        (self.executed, self.failed)
    }
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
    if s.len() > 4096 {
        return None;
    }
    let mut nesting = 0usize;
    for byte in s.bytes() {
        match byte {
            b'<' => {
                nesting = nesting.saturating_add(1);
                if nesting > 16 {
                    return None;
                }
            }
            b'>' => nesting = nesting.saturating_sub(1),
            _ => {}
        }
    }

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

    fn transaction_receipt_key(tx_hash: &[u8]) -> Vec<u8> {
        let mut key = b"tx_receipt/".to_vec();
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
        let mut slim = checkpoint.clone();
        slim.transactions = Vec::new().into();
        slim
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

    fn persist_transaction_receipts(&self, receipts: &[TransactionExecutionReceipt]) -> Result<()> {
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

    pub fn get_transaction_execution_receipt(
        &self,
        tx_hash: &[u8],
    ) -> Option<TransactionExecutionReceipt> {
        let store = self.state_read().store.clone();
        store
            .load::<TransactionExecutionReceipt>(&Self::transaction_receipt_key(tx_hash))
            .map_err(|error| {
                tracing::warn!(
                    tx_hash = %hex::encode(tx_hash),
                    "Failed to load transaction execution receipt: {}",
                    error
                );
                error
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
        Ok(self
            .execute_tx_waves_parallel_inner(
                transactions,
                state_arc,
                timestamp,
                persist_objects,
                strict_mode,
                strict_mode,
            )?
            .counts())
    }

    #[cfg(test)]
    pub(crate) fn execute_tx_waves_deterministic_parallel(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
    ) -> Result<(usize, usize)> {
        Ok(self
            .execute_tx_waves_deterministic_parallel_with_receipts(
                transactions,
                state_arc,
                timestamp,
                persist_objects,
            )?
            .counts())
    }

    pub(crate) fn execute_tx_waves_strict_serial(
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

    pub(crate) fn execute_tx_waves_deterministic_parallel_with_receipts(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
    ) -> Result<TransactionBatchExecution> {
        self.execute_tx_waves_parallel_inner(
            transactions,
            state_arc,
            timestamp,
            persist_objects,
            false,
            true,
        )
    }

    pub(crate) fn execute_tx_waves_strict_serial_with_receipts(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
    ) -> Result<TransactionBatchExecution> {
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
    ) -> Result<TransactionBatchExecution> {
        let mut batch = TransactionBatchExecution::default();
        let has_module_publish = transactions
            .iter()
            .any(|tx| matches!(tx.transaction, Transaction::PublishModule { .. }));

        if let Some((executed, failed)) =
            self.apply_zero_effect_native_batch(&transactions, state_arc)?
        {
            batch.executed = executed;
            batch.failed = failed;
            return Ok(batch);
        }

        if serial_execution {
            if has_module_publish {
                self.runtime_pool[0].reload_vm_cache()?;
            }

            for signed_tx in transactions {
                let changeset = match self.execute_transaction_with_runtime_boundary(
                    &signed_tx.transaction,
                    &self.runtime_pool[0],
                    state_arc,
                    false,
                    timestamp,
                    persist_objects,
                ) {
                    Ok(changeset) => changeset,
                    Err(error) => {
                        log::warn!("Strict execution failed: {}", error);
                        batch.failed += 1;
                        batch.receipts.push(TransactionExecutionReceipt {
                            transaction_hash: signed_tx.transaction_hash().to_vec(),
                            success: false,
                            gas_used: 0,
                            error_message: Some(format!("Execution failed: {}", error)),
                        });
                        continue;
                    }
                };

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
                if changeset.success {
                    batch.executed += 1;
                } else {
                    batch.failed += 1;
                }
                batch
                    .receipts
                    .push(TransactionExecutionReceipt::from_changeset(
                        &signed_tx, &changeset,
                    ));
            }

            return Ok(batch);
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
                        self.execute_transaction_with_runtime_boundary(
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
                        self.execute_transaction_with_runtime_boundary(
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

                for (signed_tx, res) in wave.iter().zip(results) {
                    let cs = res.map_err(|e| anyhow::anyhow!("Execution failed: {}", e))?;

                    if persist_objects {
                        let runtime = &self.runtime_pool[0];
                        runtime.persist_created_objects(&cs);
                        runtime.persist_deleted_objects(&cs);
                    }

                    if cs.success {
                        batch.executed += 1;
                    } else {
                        batch.failed += 1;
                    }
                    batch
                        .receipts
                        .push(TransactionExecutionReceipt::from_changeset(signed_tx, &cs));
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
            } else {
                // Apply changesets with proper error handling to prevent node crashes
                let mut state_write = match state_arc.write() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        log::error!("State lock poisoned during wave execution, recovering...");
                        poisoned.into_inner()
                    }
                };

                for (signed_tx, res) in wave.iter().zip(results) {
                    match res {
                        Ok(cs) => {
                            if persist_objects {
                                let runtime = &self.runtime_pool[0];
                                runtime.persist_created_objects(&cs);
                                runtime.persist_deleted_objects(&cs);
                            }

                            let mut receipt =
                                TransactionExecutionReceipt::from_changeset(signed_tx, &cs);
                            if let Err(e) = state_write.apply_changeset(&cs) {
                                log::warn!("apply_changeset failed: {}", e);
                                receipt.success = false;
                                receipt.error_message =
                                    Some(format!("Failed to apply transaction changeset: {}", e));
                                batch.failed += 1;
                            } else if cs.success {
                                batch.executed += 1;
                            } else {
                                batch.failed += 1;
                            }
                            batch.receipts.push(receipt);
                        }
                        Err(e) => {
                            log::warn!("Parallel execution failed: {}", e);
                            batch.failed += 1;
                            batch.receipts.push(TransactionExecutionReceipt {
                                transaction_hash: signed_tx.transaction_hash().to_vec(),
                                success: false,
                                gas_used: 0,
                                error_message: Some(format!("Execution failed: {}", e)),
                            });
                        }
                    }
                }
            }
        }

        Ok(batch)
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

    fn fail_with_gas_and_sequence(
        changeset: &mut ChangeSet,
        sender: AccountAddress,
        gas_cost: u64,
        gas_used: u64,
        message: String,
    ) -> Result<()> {
        changeset.mark_failed(message);
        Self::apply_gas_and_sequence(changeset, sender, gas_cost, gas_used)
    }

    fn gas_operation_for_transaction(tx: &Transaction) -> GasOperation {
        match tx {
            Transaction::PublishModule { module_bytes, .. } => GasOperation::PublishModule {
                module_size: module_bytes.len(),
            },
            Transaction::ExecuteFunction { .. } if tx.native_call().is_some() => {
                GasOperation::Transfer
            }
            Transaction::ExecuteFunction { .. } => GasOperation::ExecuteFunction { complexity: 1 },
        }
    }

    fn object_native_transfer_amount(tx: &Transaction) -> Option<u64> {
        let Transaction::ExecuteFunction {
            module,
            function,
            args,
            ..
        } = tx
        else {
            return None;
        };

        if module != Transaction::KANARI_MODULE
            || function != Transaction::TRANSFER_AMOUNT_FUNCTION
            || args.len() < 2
        {
            return None;
        }

        bcs::from_bytes::<u64>(&args[1]).ok()
    }

    fn object_transfer_recipient(tx: &Transaction) -> Option<AccountAddress> {
        let Transaction::ExecuteFunction {
            module,
            function,
            args,
            ..
        } = tx
        else {
            return None;
        };

        if module != Transaction::KANARI_MODULE
            || function != Transaction::TRANSFER_AMOUNT_FUNCTION
            || args.len() < 3
        {
            return None;
        }

        bcs::from_bytes::<AccountAddress>(&args[2]).ok()
    }

    fn object_transfer_object_id(tx: &Transaction) -> Option<String> {
        let Transaction::ExecuteFunction {
            module,
            function,
            args,
            ..
        } = tx
        else {
            return None;
        };

        if module != Transaction::KANARI_MODULE
            || function != Transaction::TRANSFER_AMOUNT_FUNCTION
            || args.is_empty()
        {
            return None;
        }

        AccountAddress::from_bytes(&args[0])
            .ok()
            .map(|address| address.to_hex_literal())
    }

    fn is_valid_self_object_transfer_noop(
        tx: &Transaction,
        sender_addr: AccountAddress,
        state: &StateManager,
    ) -> bool {
        let Some(recipient) = Self::object_transfer_recipient(tx) else {
            return false;
        };
        if recipient != sender_addr {
            return false;
        }

        let Some(object_id) = Self::object_transfer_object_id(tx) else {
            return false;
        };
        let Some(amount) = Self::object_native_transfer_amount(tx) else {
            return false;
        };
        let Some(object) = state.get_object(&object_id).ok().flatten() else {
            return false;
        };
        if object.owner != sender_addr || !object.type_.contains("::coin::Coin<") {
            return false;
        }
        if object.data.len() < 40 {
            return false;
        }

        let mut balance_bytes = [0u8; 8];
        balance_bytes.copy_from_slice(&object.data[32..40]);
        let object_balance = u64::from_le_bytes(balance_bytes);
        amount <= object_balance
    }

    fn required_native_amount_for_transaction(tx: &Transaction) -> u64 {
        tx.native_call()
            .map(|call| call.required_native_amount())
            .or_else(|| Self::object_native_transfer_amount(tx))
            .unwrap_or(0)
    }

    fn validate_transaction_gas(tx: &Transaction) -> Result<()> {
        let config = GasConfig::default();
        config.validate_price(tx.gas_price())?;
        anyhow::ensure!(
            tx.gas_limit() <= config.max_gas_per_tx,
            "Gas limit {} exceeds maximum {}",
            tx.gas_limit(),
            config.max_gas_per_tx
        );
        let required = Self::gas_operation_for_transaction(tx).gas_units();
        anyhow::ensure!(
            tx.gas_limit() >= required,
            "Gas limit {} is below required operation cost {}",
            tx.gas_limit(),
            required
        );
        required
            .checked_mul(tx.gas_price())
            .ok_or_else(|| anyhow::anyhow!("Gas cost overflow"))?;
        Ok(())
    }
    fn parse_entry_function_target(
        module: &str,
        type_args: &[String],
    ) -> Result<(ModuleId, Vec<move_core_types::language_storage::TypeTag>)> {
        let parts: Vec<&str> = module.split("::").collect();
        anyhow::ensure!(
            parts.len() == 2,
            "Invalid module format. Expected: address::module"
        );

        let addr = KanariAddress::parse_to_account_address(parts[0])
            .map_err(|error| anyhow::anyhow!("Invalid module address: {}", error))?;
        let module_name = move_core_types::identifier::Identifier::new(parts[1])
            .map_err(|error| anyhow::anyhow!("Invalid module name: {}", error))?;

        let type_tags = type_args
            .iter()
            .map(|type_arg| {
                parse_type_tag(type_arg.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Invalid type argument: {}", type_arg))
            })
            .collect::<Result<Vec<_>>>()?;

        Ok((ModuleId::new(addr, module_name), type_tags))
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

    fn execute_transaction_with_runtime_boundary(
        &self,
        tx: &Transaction,
        runtime: &kanari_move_runtime_v1::move_runtime::MoveRuntime,
        state_arc: &Arc<RwLock<StateManager>>,
        validate_sequence: bool,
        timestamp: Option<u64>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        match catch_unwind(AssertUnwindSafe(|| {
            self.execute_transaction_with_runtime_internal(
                tx,
                runtime,
                state_arc,
                validate_sequence,
                timestamp,
                persist_runtime_state,
            )
        })) {
            Ok(result) => result,
            Err(payload) => {
                let message = panic_payload_to_string(payload);
                log::error!(
                    "[ENGINE] Transaction execution panic isolated: sender={} type={} error={}",
                    tx.sender_address(),
                    tx.tx_type_label(),
                    message
                );
                anyhow::bail!("transaction execution panicked: {}", message);
            }
        }
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
        #[cfg(test)]
        if FORCE_TX_EXECUTION_PANIC.load(Ordering::SeqCst) {
            panic!("forced tx execution panic");
        }

        let sender_addr = KanariAddress::parse_to_account_address(tx.sender_address())?;
        Self::validate_transaction_gas(tx)?;
        let mut gas_meter = GasMeter::new(tx.gas_limit(), tx.gas_price());
        let mut changeset = ChangeSet::new();

        let native_call = tx.native_call();
        let gas_op = Self::gas_operation_for_transaction(tx);
        let required_amount = Self::required_native_amount_for_transaction(tx);

        gas_meter.consume(gas_op.gas_units())?;
        let base_gas_used = gas_meter.gas_used;
        let reserved_gas_cost = tx
            .gas_limit()
            .checked_mul(tx.gas_price())
            .ok_or_else(|| anyhow::anyhow!("Gas cost overflow"))?;
        let gas_cost = base_gas_used
            .checked_mul(tx.gas_price())
            .ok_or_else(|| anyhow::anyhow!("Gas cost overflow"))?;
        let total_required = required_amount.saturating_add(reserved_gas_cost);

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

        if required_amount > (i64::MAX as u64).saturating_sub(gas_cost) {
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
                    Some((tx.gas_limit().saturating_sub(base_gas_used), tx.gas_price())),
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,
                ) {
                    Ok(move_cs) => changeset.merge(move_cs),
                    Err(e) => {
                        changeset.mark_failed(format!("Publish failed: {}", e));
                        changeset.set_gas_used(tx.gas_limit().saturating_sub(base_gas_used));
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
                if function == Transaction::TRANSFER_AMOUNT_FUNCTION
                    && module == Transaction::KANARI_MODULE
                {
                    let state = match state_arc.read() {
                        Ok(guard) => guard,
                        Err(poisoned) => {
                            log::error!(
                                "State arc lock poisoned in self-transfer guard, recovering..."
                            );
                            poisoned.into_inner()
                        }
                    };
                    if Self::is_valid_self_object_transfer_noop(tx, sender_addr, &state) {
                        Self::apply_gas_and_sequence(
                            &mut changeset,
                            sender_addr,
                            gas_cost,
                            gas_meter.gas_used,
                        )?;
                        return Ok(changeset);
                    }
                }

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

                let (module_id, type_tags) =
                    match Self::parse_entry_function_target(module, type_args) {
                        Ok(target) => target,
                        Err(error) => {
                            Self::fail_with_gas_and_sequence(
                                &mut changeset,
                                sender_addr,
                                gas_cost,
                                gas_meter.gas_used,
                                error.to_string(),
                            )?;
                            return Ok(changeset);
                        }
                    };

                match runtime.execute_entry_function_with_tx_hash_and_persistence(
                    &module_id,
                    function,
                    type_tags,
                    args.clone(),
                    Some(sender_addr),
                    Some((tx.gas_limit().saturating_sub(base_gas_used), tx.gas_price())),
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,
                ) {
                    Ok(move_cs) => changeset.merge(move_cs),
                    Err(e) => {
                        changeset.mark_failed(format!("Execution failed: {}", e));
                        changeset.set_gas_used(tx.gas_limit().saturating_sub(base_gas_used));
                    }
                }
            }
        }

        let vm_gas_used = changeset.gas_used;
        let actual_gas_used = base_gas_used
            .saturating_add(vm_gas_used)
            .min(tx.gas_limit());
        let actual_gas_cost = actual_gas_used
            .checked_mul(tx.gas_price())
            .ok_or_else(|| anyhow::anyhow!("Gas cost overflow"))?;
        Self::apply_gas_and_sequence(
            &mut changeset,
            sender_addr,
            actual_gas_cost,
            actual_gas_used,
        )?;
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
        self.dag_engine_instance()?.produce_vertex()
    }

    pub fn dag_needs_progress(&self) -> Result<bool> {
        Ok(self.dag_engine_instance()?.needs_progress())
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

    pub fn add_network_dag_vertex(
        &self,
        vertex: DagVertex,
    ) -> Result<crate::engine::produce_dag_vertex::ConsensusUpdate> {
        self.dag_engine_instance()?.add_network_vertex(vertex)
    }

    pub fn submit_checkpoint_vote(
        &self,
        vote: crate::consensus::CheckpointVote,
    ) -> Result<crate::engine::produce_dag_vertex::ConsensusUpdate> {
        self.dag_engine_instance()?.submit_checkpoint_vote(vote)
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
#[path = "../tests/unit/engine_tests.rs"]
mod tests;
