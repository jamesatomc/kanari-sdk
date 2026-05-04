// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use centauri::blockchain::Blockchain;
use centauri::calculate_quorum;
use centauri::consensus::{Checkpoint, PersistentDagState};
use kanari_move_runtime::changeset::ChangeSet;
use kanari_move_runtime::move_runtime::MoveRuntime;
use kanari_move_runtime::state::StateManager;
use kanari_move_runtime::storage::persistent_store::PersistentStore;
pub use kanari_rpc_api::{AccountInfo, BlockData, BlockchainStats, FullBlockData, ObjectInfo};
use kanari_types::address::Address as KanariAddress;
use kanari_types::event::Event;
use kanari_types::gas_v2::{GasMeter, GasOperation};
use kanari_types::transaction::{SignedTransaction, Transaction};
use log::{error, info, warn};
use lru::LruCache;
use move_core_types::{
    account_address::AccountAddress,
    identifier::Identifier,
    language_storage::{ModuleId, StructTag, TypeTag},
};
use num_cpus;
use rayon::prelude::*;
use std::num::NonZeroUsize;
use std::sync::{Arc, RwLock};

type ProofCache = LruCache<(u64, usize), (String, Vec<Vec<u8>>)>;

mod apply_checkpoint;
mod produce_dag_vertex;
pub use produce_dag_vertex::{CheckpointInfo, DagBlockInfo, DagEngine};

pub type BlockInfo = DagBlockInfo;

pub type ExecutionResult = Result<(Vec<u8>, ChangeSet)>;
pub type ParallelTxResult = (SignedTransaction, ExecutionResult);

const MAX_MEMPOOL_SIZE: usize = 50_000;

/// Complete blockchain engine with Move VM integration
pub struct BlockchainEngine {
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub state: Arc<RwLock<StateManager>>,
    pub pending_txs: Arc<RwLock<Vec<SignedTransaction>>>,
    pub persistent_store: Option<Arc<PersistentStore>>,
    // Reusable pool of MoveRuntime instances for parallel execution
    pub runtime_pool: Vec<kanari_move_runtime::move_runtime::MoveRuntime>,
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
    // =====================================================================
    // ⏰ System Prologue
    // =====================================================================
    pub fn execute_system_prologue(&self, timestamp_ms: u64) -> Result<()> {
        let runtime = &self.runtime_pool[0];

        // Acquire the write lock once to ensure atomicity of the entire operation
        let mut state_write = self.state.write().unwrap_or_else(|e| e.into_inner());

        // Get the clock ID
        let clock_id = runtime.ensure_system_clock(&mut state_write)?;

        // Execute the prologue function to get the changeset
        let changeset = runtime.execute_clock_consensus_commit_prologue(clock_id, timestamp_ms)?;

        // Apply the changeset to the state
        state_write.apply_changeset(&changeset)?;
        runtime.persist_created_objects(&changeset);
        runtime.persist_deleted_objects(&changeset);
        Ok(())
    }

    // =====================================================================
    // 💡 HELPER FUNCTIONS
    // =====================================================================

    pub fn get_expected_sequence(&self, address_hex: &str) -> u64 {
        let mut seq = self
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get_account_by_hex(address_hex)
            .map(|acc| acc.sequence_number)
            .unwrap_or(0);

        self.for_each_pending_tx_from_sender(address_hex, |_| seq += 1);
        seq
    }

    fn preload_objects_for_args(
        args: &[Vec<u8>],
        state: &StateManager,
        runtime: &kanari_move_runtime::move_runtime::MoveRuntime,
    ) {
        for arg in args.iter() {
            let mut possible_ids = Vec::new();

            if arg.len() == 32
                && let Ok(addr) = AccountAddress::from_bytes(arg)
            {
                possible_ids.push(addr.to_hex_literal());
            }

            if let Ok(s) = bcs::from_bytes::<String>(arg)
                .or_else(|_| std::str::from_utf8(arg).map(|s| s.to_string()))
            {
                let s_trim = s.trim();
                let hex_str = if !s_trim.starts_with("0x") {
                    format!("0x{}", s_trim)
                } else {
                    s_trim.to_string()
                };
                if let Ok(addr) = AccountAddress::from_hex_literal(&hex_str) {
                    possible_ids.push(addr.to_hex_literal());
                }
            }

            for object_id in possible_ids {
                if let Ok(Some(obj)) = state.get_object(&object_id) {
                    let _ = runtime.preload_object_snapshot(
                        &object_id,
                        obj.owner,
                        &obj.type_,
                        obj.data.clone(),
                        obj.version,
                    );
                }
            }
        }
    }

    fn resolve_account_objects(
        &self,
        state: &StateManager,
        owner_addr: &AccountAddress,
    ) -> Vec<ObjectInfo> {
        let raw_owned_ids = state.get_owned_objects(owner_addr).unwrap_or_default();
        let unique_ids: std::collections::HashSet<_> = raw_owned_ids.into_iter().collect();

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

        coins.sort_by(|a, b| b.0.cmp(&a.0));
        coins
            .into_iter()
            .map(|(_, info)| info)
            .chain(others)
            .collect()
    }

    pub(crate) fn execute_tx_waves_parallel(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
        strict_mode: bool,
    ) -> Result<(usize, usize)> {
        let mut executed_count = 0;
        let mut failed_count = 0;

        let waves = kanari_move_runtime::TransactionScheduler::schedule(transactions);

        for wave in waves {
            let results: Vec<Result<ChangeSet>> = wave
                .par_iter()
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
                .collect();

            let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
            for res in results {
                match res {
                    Ok(cs) => {
                        if persist_objects {
                            let runtime = &self.runtime_pool[0];
                            runtime.persist_created_objects(&cs);
                            runtime.persist_deleted_objects(&cs);
                        }

                        if let Err(e) = state_write.apply_changeset(&cs) {
                            if strict_mode {
                                anyhow::bail!("Failed to apply changeset: {}", e);
                            }
                            log::warn!("apply_changeset failed: {}", e);
                            failed_count += 1;
                        } else {
                            executed_count += 1;
                        }
                    }
                    Err(e) => {
                        if strict_mode {
                            anyhow::bail!("Execution failed: {}", e);
                        }
                        log::warn!("Parallel execution failed: {}", e);
                        failed_count += 1;
                    }
                }
            }
        }

        Ok((executed_count, failed_count))
    }

    pub fn new_dir(dir: &str) -> Result<Self> {
        let persistent_store = Self::try_open_store(
            || PersistentStore::open_with_path(Some(std::path::PathBuf::from(dir))),
            &format!("at '{}'", dir),
        );
        Self::init(persistent_store)
    }

    pub fn new() -> Result<Self> {
        let persistent_store = Self::try_open_store(PersistentStore::open_default, "default");
        Self::init(persistent_store)
    }

    fn try_open_store<F>(opener: F, context: &str) -> Option<Arc<PersistentStore>>
    where
        F: FnOnce() -> Result<PersistentStore>,
    {
        if cfg!(miri) {
            None
        } else {
            match opener() {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    eprintln!(
                        "WARN: Failed to open {} persistent store: {}. Falling back to in-memory mode.",
                        context, e
                    );
                    None
                }
            }
        }
    }

    fn init(persistent_store: Option<Arc<PersistentStore>>) -> Result<Self> {
        let blockchain = Self::load_blockchain(&persistent_store);
        let state = Self::load_state(&persistent_store);

        let workers = num_cpus::get().max(1);
        let mut runtime_pool = Vec::new();

        let base_runtime = match if persistent_store.is_some() {
            MoveRuntime::new_with_kanari_natives()
        } else {
            MoveRuntime::new_with_kanari_natives_in_memory()
        } {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("FATAL: Failed to initialize base MoveRuntime: {}", e);
                anyhow::bail!("Failed to initialize runtime pool: {}", e);
            }
        };

        log::info!(
            "Initializing runtime pool with {} workers (independent VMs sharing DB)",
            workers
        );
        runtime_pool.push(base_runtime.clone());

        for i in 1..workers {
            match base_runtime.spawn_worker() {
                Ok(rt) => runtime_pool.push(rt),
                Err(e) => {
                    log::error!("Failed to spawn worker runtime #{}: {}", i, e);
                    anyhow::bail!("Failed to initialize runtime pool: {}", e);
                }
            }
        }

        let pending_txs = Arc::new(RwLock::new(Vec::new()));
        let proof_cache = Arc::new(RwLock::new(LruCache::new(
            NonZeroUsize::new(1000).expect("NonZeroUsize::new(1000) should never fail"),
        )));

        let authority_id = "0xDEFAULT_AUTHORITY".to_string();
        let authorities = vec![authority_id.clone()];
        let persisted_dag_state = Self::load_dag_state(&persistent_store);

        Ok(Self {
            blockchain,
            state,
            pending_txs,
            persistent_store,
            runtime_pool,
            proof_cache,
            dag_engine: Arc::new(RwLock::new(None)),
            authority_id,
            authorities,
            persisted_dag_state,
        })
    }

    fn load_blockchain(store: &Option<Arc<PersistentStore>>) -> Arc<RwLock<Blockchain>> {
        if let Some(store) = store {
            match store.load::<Blockchain>(b"blockchain") {
                Ok(Some(mut b)) => {
                    info!(
                        "Successfully loaded blockchain from persistent store (height: {}, checkpoints: {})",
                        b.height(),
                        b.dag_checkpoints.len()
                    );
                    b.rebuild_tx_hash_index();
                    Arc::new(RwLock::new(b))
                }
                Ok(None) => {
                    info!("No persisted blockchain found. Creating fresh genesis.");
                    Arc::new(RwLock::new(Blockchain::new()))
                }
                Err(e) => {
                    error!(
                        "FATAL ERROR loading blockchain: {}. Falling back to fresh genesis.",
                        e
                    );
                    Arc::new(RwLock::new(Blockchain::new()))
                }
            }
        } else {
            info!("Running in-memory mode: No persistent store provided for blockchain.");
            Arc::new(RwLock::new(Blockchain::new()))
        }
    }

    fn load_state(store: &Option<Arc<PersistentStore>>) -> Arc<RwLock<StateManager>> {
        let store = store.clone().unwrap_or_else(|| {
            Arc::new(PersistentStore::open_in_memory().expect("Failed to open in-memory store"))
        });
        info!("Initializing StateManager with persistent store support (RocksDB)");
        Arc::new(RwLock::new(StateManager::new(store)))
    }

    fn load_dag_state(store: &Option<Arc<PersistentStore>>) -> Option<PersistentDagState> {
        if let Some(store) = store {
            match store.load::<PersistentDagState>(b"dag_state") {
                Ok(Some(s)) => {
                    info!("Successfully loaded DAG consensus state from persistent store");
                    Some(s)
                }
                Ok(None) => None,
                Err(e) => {
                    error!(
                        "Failed to load DAG state: {}. Falling back to fresh DAG.",
                        e
                    );
                    None
                }
            }
        } else {
            None
        }
    }

    fn apply_gas_and_sequence(
        changeset: &mut ChangeSet,
        sender: AccountAddress,
        gas_cost: u64,
        gas_used: u64,
    ) -> Result<()> {
        let sender_change = changeset.get_or_create_change(sender);
        sender_change.increment_sequence();
        sender_change.debit(gas_cost);

        let dao_addr = AccountAddress::from_hex_literal(KanariAddress::DAO_ADDRESS)?;
        changeset.collect_gas(dao_addr, gas_cost);
        changeset.set_gas_used(gas_used);
        Ok(())
    }

    pub fn persist_dag_state(&self, state: PersistentDagState) -> Result<()> {
        if let Some(store) = &self.persistent_store {
            store
                .save(b"dag_state", &state)
                .context("Failed to persist DAG state")?;
        }
        Ok(())
    }

    // =====================================================================
    // 🛡️ Mempool Security
    // =====================================================================
    pub fn submit_transaction(&self, signed_tx: SignedTransaction) -> Result<Vec<u8>> {
        let pending_count = self
            .pending_txs
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .len();
        if pending_count >= MAX_MEMPOOL_SIZE {
            log::warn!("[MEMPOOL] Rejecting transaction: Queue is full (Anti-DDoS active)");
            anyhow::bail!("Mempool is currently full. Please try again later.");
        }

        if !signed_tx.verify_signature()? {
            anyhow::bail!("Invalid or missing transaction signature");
        }

        let tx_hash = signed_tx.hash();
        let tx_hash_hex = hex::encode(&tx_hash);
        let sender_address = signed_tx.transaction.sender_address();

        let expected_seq = self.get_expected_sequence(sender_address);
        let tx_seq = signed_tx.transaction.sequence_number();

        if tx_seq < expected_seq {
            anyhow::bail!(
                "Sequence number too low: expected {}, got {}",
                expected_seq,
                tx_seq
            );
        }
        if tx_seq > expected_seq {
            anyhow::bail!(
                "Sequence number too high: expected {}, got {}",
                expected_seq,
                tx_seq
            );
        }

        {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            if chain.is_transaction_executed(&tx_hash_hex) {
                anyhow::bail!("Transaction {} already executed", tx_hash_hex);
            }
        }

        let mut pending = self.pending_txs.write().unwrap_or_else(|e| e.into_inner());
        for ptx in pending.iter() {
            if ptx.hash() == tx_hash {
                anyhow::bail!("Transaction {} already in pending pool", tx_hash_hex);
            }
        }

        pending.push(signed_tx);
        info!(
            "[MEMPOOL] Transaction {} accepted and added to queue",
            tx_hash_hex
        );
        Ok(tx_hash)
    }

    pub fn execute_transaction_immediate(
        &self,
        signed_tx: SignedTransaction,
    ) -> Result<(Vec<u8>, ChangeSet)> {
        if !signed_tx.verify_signature()? {
            anyhow::bail!("Invalid transaction signature");
        }

        let tx_hash = signed_tx.hash();
        let tx = signed_tx.transaction;

        let changeset = {
            let mut state_snapshot =
                { self.state.read().unwrap_or_else(|e| e.into_inner()).clone() };
            let sender_addr = tx.sender_address();
            let addr = KanariAddress::parse_to_account_address(sender_addr)?;

            self.for_each_pending_tx_from_sender(sender_addr, |_| {
                if let Some(mut acct) = state_snapshot.get_account(&addr) {
                    acct.increment_sequence();
                    state_snapshot.save_account(&acct).unwrap_or(());
                }
            });
            let state_arc = Arc::new(RwLock::new(state_snapshot));
            let runtime = &self.runtime_pool[0];
            self.execute_transaction_with_runtime(&tx, runtime, &state_arc, None)?
        };

        Ok((tx_hash, changeset))
    }

    // =====================================================================
    // ⚡ Parallel Execution
    // =====================================================================
    pub fn execute_transactions_parallel(
        &self,
        txs: Vec<SignedTransaction>,
    ) -> Vec<ParallelTxResult> {
        log::info!(
            "[PARALLEL ENGINE] Firing up Rayon to execute {} txs concurrently!",
            txs.len()
        );

        let state_arc = &self.state;

        txs.into_par_iter()
            .map(|tx| {
                let thread_idx = rayon::current_thread_index().unwrap_or(0);
                let runtime = &self.runtime_pool[thread_idx % self.runtime_pool.len()];

                let result = self.execute_transaction_with_runtime_internal(
                    &tx.transaction,
                    runtime,
                    state_arc,
                    true,
                    None,
                    false,
                );

                let final_result = match result {
                    Ok(cs) => Ok((tx.hash(), cs)),
                    Err(e) => Err(anyhow::anyhow!("Parallel execution failed: {}", e)),
                };

                (tx, final_result)
            })
            .collect()
    }

    // =====================================================================
    // 🕸️ DAG Consensus Switch
    // =====================================================================
    pub fn process_dag_checkpoint(
        &self,
        checkpoint_txs: Vec<SignedTransaction>,
        consensus_timestamp_ms: Option<u64>, // Allow timestamp from consensus layer
    ) -> Result<Vec<u8>> {
        log::info!(
            "[DAG CONSENSUS] Applying new checkpoint with {} transactions",
            checkpoint_txs.len()
        );

        info!(
            "Executing {} transactions in checkpoint",
            checkpoint_txs.len()
        );

        // =================================================================
        // 🚨 Update the time on the Blockchain before executing user transactions.
        // =================================================================
        // Use timestamp from consensus layer to ensure all nodes have identical state
        // CRITICAL: All nodes must use the same timestamp to ensure deterministic state transitions
        let current_timestamp_ms = consensus_timestamp_ms.expect(
            "CRITICAL ERROR: Consensus timestamp must be provided for blockchain state consistency. "
        );

        if let Err(e) = self.execute_system_prologue(current_timestamp_ms) {
            log::error!(
                "Critical Error: System clock failed to update! Halt execution. {:?}",
                e
            );
            return Err(e);
        }
        // =================================================================

        let execution_results = self.execute_transactions_parallel(checkpoint_txs);

        // =================================================================
        // 📝 Process all the transaction results and create a checkpoint
        // =================================================================
        let mut successful_txs = Vec::new();
        let mut all_events_for_block = Vec::new();
        let runtime = &self.runtime_pool[0];

        for (tx, result) in execution_results {
            match result {
                Ok((_tx_hash, changeset)) => {
                    runtime.persist_created_objects(&changeset);
                    runtime.persist_deleted_objects(&changeset);

                    let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
                    if let Err(e) = state.apply_changeset(&changeset) {
                        error!("[DAG COMMIT] Failed to apply changeset to state: {}", e);
                    }

                    all_events_for_block.extend(changeset.events.clone());
                    successful_txs.push(tx);
                }
                Err(e) => {
                    log::warn!("[DAG COMMIT] Transaction execution failed: {}", e);
                }
            }
        }

        let mut chain = self.blockchain.write().unwrap_or_else(|e| e.into_inner());
        let height = chain.blocks.len() as u64;
        let prev_hash = chain.blocks.back().map(|b| b.hash()).unwrap_or_default();

        let new_block = kanari_types::block::Block::new(
            height,
            prev_hash,
            vec![0u8; 32],
            successful_txs,
            all_events_for_block,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or(std::time::Duration::ZERO)
                .as_secs(),
        );
        let block_hash = new_block.hash();
        chain.blocks.push_back(new_block);

        log::info!(
            "[DAG CONSENSUS] Checkpoint {} committed! Hash: {}",
            height,
            hex::encode(&block_hash)
        );

        Ok(block_hash)
    }

    fn execute_transaction_with_runtime(
        &self,
        tx: &Transaction,
        runtime: &kanari_move_runtime::move_runtime::MoveRuntime,
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
        runtime: &kanari_move_runtime::move_runtime::MoveRuntime,
        state_arc: &Arc<RwLock<StateManager>>,
        validate_sequence: bool,
        timestamp: Option<u64>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        let sender_addr = KanariAddress::parse_to_account_address(tx.sender_address())?;
        if validate_sequence {
            let state = state_arc.read().unwrap_or_else(|e| e.into_inner());
            state
                .validate_sequence(&sender_addr, tx.sequence_number())
                .context("Sequence number validation failed")?;
        }

        let mut gas_meter = GasMeter::new(tx.gas_limit(), tx.gas_price());
        let mut changeset = ChangeSet::new();

        let (gas_op, required_amount) = match tx {
            Transaction::PublishModule { module_bytes, .. } => (
                GasOperation::PublishModule {
                    module_size: module_bytes.len(),
                },
                0,
            ),
            Transaction::ExecuteFunction { .. } => {
                (GasOperation::ExecuteFunction { complexity: 1 }, 0)
            }
            Transaction::Transfer { amount, .. } | Transaction::Burn { amount, .. } => {
                (GasOperation::Transfer, *amount)
            }
        };

        gas_meter.consume(gas_op.gas_units())?;
        let gas_cost = gas_meter.total_cost();
        let total_required = required_amount.saturating_add(gas_cost);

        {
            let state = state_arc.read().unwrap_or_else(|e| e.into_inner());
            let balance = state
                .get_account(&sender_addr)
                .map(|acc| acc.balance)
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
                    gas_cost,
                    gas_meter.gas_used,
                )?;
                return Ok(changeset);
            }
        }

        match tx {
            Transaction::PublishModule {
                sender,
                module_bytes,
                ..
            } => {
                match runtime.publish_module_with_persistence(
                    module_bytes.clone(),
                    KanariAddress::parse_to_account_address(sender)?,
                    Some((tx.gas_limit(), tx.gas_price())),
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
                {
                    let state = state_arc.read().unwrap_or_else(|e| e.into_inner());
                    Self::preload_objects_for_args(args, &state, runtime);
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
                    Some((tx.gas_limit(), tx.gas_price())),
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

            Transaction::Transfer { to, amount, .. } => {
                let to_addr = KanariAddress::parse_to_account_address(to)?;
                changeset.transfer(sender_addr, to_addr, *amount);
            }

            Transaction::Burn { amount, .. } => {
                changeset.burn(sender_addr, *amount);
            }
        }

        Self::apply_gas_and_sequence(&mut changeset, sender_addr, gas_cost, gas_meter.gas_used)?;
        Ok(changeset)
    }

    pub fn produce_block(&self) -> Result<BlockInfo> {
        let dag_engine = {
            let mut dag_engine_guard = self.dag_engine.write().unwrap_or_else(|e| e.into_inner());
            if dag_engine_guard.is_none() {
                {
                    let mut chain = self.blockchain.write().unwrap_or_else(|e| e.into_inner());
                    if !chain.dag_mode {
                        chain.enable_dag_mode();
                    }
                }

                let engine = DagEngine::new(
                    Arc::new(self.clone_for_dag()),
                    self.authority_id.clone(),
                    self.authorities.clone(),
                )?;
                *dag_engine_guard = Some(engine);
            }
            dag_engine_guard.as_ref().cloned()
        };

        let dag_engine = dag_engine.expect("DAG engine should be initialized");

        {
            let consensus_lock = dag_engine.consensus();
            let consensus = consensus_lock.read().unwrap_or_else(|e| e.into_inner());
            let store = consensus.store();
            let current_round = store.current_round();
            let num_authorities = store.num_authorities();

            let quorum_needed = calculate_quorum(num_authorities);

            let parents_available = store.get_vertices_in_round(current_round).len();

            if current_round > 0 && parents_available < quorum_needed {
                anyhow::bail!(
                    "SYNC_WAITING: have {}/{} vertices in round {} (need quorum for round {})",
                    parents_available,
                    quorum_needed,
                    current_round,
                    current_round + 1
                );
            }
        }

        dag_engine.produce_vertex()
    }

    fn clone_for_dag(&self) -> BlockchainEngine {
        BlockchainEngine {
            blockchain: self.blockchain.clone(),
            state: self.state.clone(),
            pending_txs: self.pending_txs.clone(),
            persistent_store: self.persistent_store.clone(),
            runtime_pool: self.runtime_pool.clone(),
            proof_cache: self.proof_cache.clone(),
            dag_engine: Arc::new(RwLock::new(None)),
            authority_id: self.authority_id.clone(),
            authorities: self.authorities.clone(),
            persisted_dag_state: self.persisted_dag_state.clone(),
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
        *self.dag_engine.write().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn get_authority_id(&self) -> String {
        self.authority_id.clone()
    }

    pub fn get_authorities(&self) -> Vec<String> {
        self.authorities.clone()
    }

    pub fn get_dag_engine(&self) -> Option<Arc<RwLock<Option<DagEngine>>>> {
        Some(self.dag_engine.clone())
    }

    pub fn get_stats(&self) -> BlockchainStats {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        let pending = self.pending_txs.read().unwrap_or_else(|e| e.into_inner());

        BlockchainStats {
            height: chain.height(),
            total_blocks: chain.blocks.len(),
            total_transactions: chain.get_transaction_count(),
            pending_transactions: pending.len(),
            total_accounts: state.account_count(),
            total_supply: state.total_supply,
        }
    }

    pub fn get_account_info(&self, address: &str) -> Option<AccountInfo> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());

        state.get_account_by_hex(address).map(|acc| {
            let final_owned_objects = self.resolve_account_objects(&state, &acc.address);
            let sequence_number = self.get_expected_sequence(address);

            let actual_token_balances = acc
                .token_balances
                .into_iter()
                .map(|(k, v)| (k, v.value()))
                .collect();

            AccountInfo {
                address: format!("{:#x}", acc.address),
                balance: acc.balance,
                sequence_number,
                modules: acc.modules.iter().cloned().collect(),
                token_balances: actual_token_balances,
                owned_objects: Some(final_owned_objects),
            }
        })
    }

    fn for_each_pending_tx_from_sender<F>(&self, sender: &str, mut f: F)
    where
        F: FnMut(&SignedTransaction),
    {
        if let Ok(pending) = self.pending_txs.read() {
            let normalized_sender = Self::normalize_addr(sender);
            for ptx in pending.iter() {
                if Self::normalize_addr(ptx.transaction.sender_address()) == normalized_sender {
                    f(ptx);
                }
            }
        }
    }

    fn normalize_addr(addr: &str) -> String {
        use std::str::FromStr;
        KanariAddress::from_str(addr)
            .map(|a| a.to_hex())
            .unwrap_or_else(|_| addr.trim_start_matches("0x").to_lowercase())
    }

    pub fn get_module_bytecode(&self, address: &str, module_name: &str) -> Option<Vec<u8>> {
        use move_core_types::{identifier::Identifier, language_storage::ModuleId};

        let addr = match KanariAddress::parse_to_account_address(address) {
            Ok(a) => a,
            Err(_) => return None,
        };

        let ident = match Identifier::new(module_name) {
            Ok(i) => i,
            Err(_) => return None,
        };

        let module_id = ModuleId::new(addr, ident);
        let runtime = &self.runtime_pool[0];
        runtime.get_module_bytes(&module_id)
    }

    pub fn list_all_modules(&self) -> Vec<(String, String)> {
        let runtime = &self.runtime_pool[0];
        runtime
            .list_modules()
            .into_iter()
            .map(|module_id| {
                (
                    format!("0x{}", module_id.address()),
                    module_id.name().to_string(),
                )
            })
            .collect()
    }

    pub fn get_block(&self, height: u64) -> Option<BlockData> {
        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain.get_block(height).map(|block| BlockData {
            height: block.header.height,
            timestamp: block.header.timestamp,
            hash: hex::encode(block.hash()),
            prev_hash: hex::encode(&block.header.prev_hash),
            state_root: hex::encode(&block.header.state_root),
            tx_count: block.transactions.len(),
            events: block.events.clone(),
        })
    }

    pub fn get_full_block(&self, height: u64) -> Option<FullBlockData> {
        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        let block = chain.get_block(height)?;
        let checkpoint = chain.get_checkpoint(height);

        let vertices = checkpoint
            .map(|cp| cp.vertices.iter().map(hex::encode).collect())
            .unwrap_or_default();

        Some(FullBlockData {
            height: block.header.height,
            timestamp: block.header.timestamp,
            hash: hex::encode(block.hash()),
            prev_hash: hex::encode(&block.header.prev_hash),
            state_root: hex::encode(&block.header.state_root),
            tx_count: block.transactions.len(),
            events: block.events.clone(),
            transactions: block.transactions.clone(),
            vertices,
        })
    }

    fn decode_hex(s: &str) -> Result<Vec<u8>> {
        hex::decode(s.trim_start_matches("0x")).context("Invalid hex string")
    }

    fn decode_hex_32(s: &str) -> [u8; 32] {
        let bytes = Self::decode_hex(s).unwrap_or_default();
        let mut arr = [0u8; 32];
        if bytes.len() == 32 {
            arr.copy_from_slice(&bytes);
        }
        arr
    }

    pub fn sync_full_block_from_data(&self, block_data: &FullBlockData) -> Result<()> {
        let stats = self.get_stats();
        info!(
            "[SYNC] Attempting to sync block #{} (our height: {})",
            block_data.height, stats.height
        );

        if block_data.height <= stats.height {
            info!("[SYNC] Already have block #{}, skipping", block_data.height);
            return Ok(());
        }

        if block_data.height != stats.height + 1 {
            warn!(
                "[SYNC] Block #{} is not consecutive (need {})",
                block_data.height,
                stats.height + 1
            );
            anyhow::bail!(
                "Cannot sync block #{}: current height is {}",
                block_data.height,
                stats.height
            );
        }

        info!(
            "[SYNC] Verifying {} transaction signatures from block #{}",
            block_data.transactions.len(),
            block_data.height
        );
        for (i, signed_tx) in block_data.transactions.iter().enumerate() {
            let signed_tx: &SignedTransaction = signed_tx;
            if !signed_tx.verify_signature()? {
                anyhow::bail!(
                    "Invalid or missing signature for transaction {} in block #{}",
                    i + 1,
                    block_data.height
                );
            }
        }

        let state_root = Self::decode_hex(&block_data.state_root)
            .context("Invalid state root format in block data")?;

        let prev_hash = {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash()?
        };

        let vertices: Vec<[u8; 32]> = block_data
            .vertices
            .iter()
            .map(|v| Self::decode_hex_32(v))
            .collect();

        let checkpoint = Checkpoint::new(
            block_data.height,
            vertices,
            block_data.transactions.clone(),
            state_root,
            block_data.timestamp,
            prev_hash,
        );

        // Note: This calls the apply_checkpoint.rs file.
        self.apply_checkpoint(checkpoint)?;

        info!(
            "Synced block #{} with {} transactions",
            block_data.height,
            block_data.transactions.len()
        );

        Ok(())
    }
}
