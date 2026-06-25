// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;

impl BlockchainEngine {
    pub fn new_dir(dir: &str) -> Result<Self> {
        let persistent_store = Self::try_open_store(
            || PersistentStore::open_with_path(Some(std::path::PathBuf::from(dir))),
            &format!("at '{}'", dir),
        )?;
        Self::init_with_options(persistent_store, false)
    }

    pub fn new() -> Result<Self> {
        let persistent_store = Self::try_open_store(PersistentStore::open_default, "default")?;
        Self::init_with_options(persistent_store, false)
    }

    pub fn new_in_memory() -> Result<Self> {
        Self::init_with_options(None, false)
    }

    pub fn new_in_memory_with_smt() -> Result<Self> {
        Self::init_with_options(None, true)
    }

    fn try_open_store<F>(opener: F, context: &str) -> Result<Option<Arc<PersistentStore>>>
    where
        F: FnOnce() -> Result<PersistentStore>,
    {
        if cfg!(miri) {
            Ok(None)
        } else {
            match opener() {
                Ok(store) => Ok(Some(Arc::new(store))),
                Err(e) => {
                    if Self::strict_persistence_required() {
                        anyhow::bail!(
                            "Failed to open {} persistent store in {} mode: {}",
                            context,
                            Self::network_name(),
                            e
                        );
                    }
                    tracing::warn!(
                        "Failed to open {} persistent store: {}. Falling back to in-memory mode.",
                        context,
                        e
                    );
                    Ok(None)
                }
            }
        }
    }

    fn init_with_options(
        persistent_store: Option<Arc<PersistentStore>>,
        enable_in_memory_smt: bool,
    ) -> Result<Self> {
        if let Some(store) = &persistent_store {
            if let Some(checkpoint) = store
                .load::<Checkpoint>(b"pending_checkpoint_commit")
                .context("Failed to inspect checkpoint commit journal")?
            {
                anyhow::bail!(
                    "unclean checkpoint commit detected at sequence {}; refusing startup until recovery is performed",
                    checkpoint.sequence
                );
            }
        }
        tracing::info!("Loading blockchain checkpoints");
        let mut blockchain = Self::load_blockchain(&persistent_store);
        tracing::info!("Loading Mysticeti DAG state");
        let persisted_dag_state = Self::load_dag_state(&persistent_store);
        tracing::info!("Repairing checkpoint index from DAG state");
        if Self::repair_blockchain_from_dag_state(&mut blockchain, persisted_dag_state.as_ref())?
            && let Some(store) = &persistent_store
        {
            let chain = blockchain.read().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = Self::persist_blockchain_snapshot_to_store(store, &chain) {
                tracing::warn!("Failed to persist repaired blockchain: {}", e);
            }
        }
        tracing::info!("Opening state database");
        let state = Self::load_state(&persistent_store, enable_in_memory_smt)?;

        let workers = Self::runtime_worker_count();
        let mut runtime_pool = Vec::new();

        let base_runtime = match if let Some(store) = persistent_store.clone() {
            MoveRuntime::new_with_kanari_natives_and_store(store)
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
        let verbose_startup = std::env::var("KANARI_VERBOSE_STARTUP")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false);
        tracing::info!(workers, "Move runtime pool initializing");
        runtime_pool.push(base_runtime.clone());

        for i in 1..workers {
            if verbose_startup {
                tracing::info!(worker = i + 1, workers, "Starting Move runtime worker");
            }
            match base_runtime.spawn_worker() {
                Ok(rt) => {
                    runtime_pool.push(rt);
                    if verbose_startup {
                        tracing::info!(worker = i + 1, workers, "Move runtime worker ready");
                    }
                }
                Err(e) => {
                    log::error!("Failed to spawn worker runtime #{}: {}", i, e);
                    anyhow::bail!("Failed to initialize runtime pool: {}", e);
                }
            }
        }
        tracing::info!(workers = runtime_pool.len(), "Move runtime pool ready");
        tracing::info!("Preparing mempool, proof cache, and authority defaults");

        let mempool = Arc::new(RwLock::new(MempoolState::default()));
        let proof_cache = Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(1000).unwrap())));

        let authority_id = "0xDEFAULT_AUTHORITY".to_string();
        let authorities = vec![authority_id.clone()];

        let engine = Self {
            blockchain,
            state,
            mempool,
            persistent_store,
            runtime_pool,
            proof_cache,
            dag_engine: Arc::new(RwLock::new(None)),
            authority_id,
            authorities,
            persisted_dag_state,
            consensus_signing_key: None,
            consensus_public_keys: BTreeMap::new(),
        };

        let stats = engine.get_stats();
        tracing::info!(
            height = stats.height,
            txs = stats.total_transactions,
            accounts = stats.total_accounts,
            pending = stats.pending_transactions,
            "Kanari engine ready"
        );
        Ok(engine)
    }

    fn runtime_worker_count() -> usize {
        const DEFAULT_MAX_RUNTIME_WORKERS: usize = 4;

        if let Ok(raw) = std::env::var("KANARI_RUNTIME_WORKERS") {
            match raw.trim().parse::<usize>() {
                Ok(value) if value > 0 => return value,
                _ => tracing::warn!(
                    "Ignoring invalid KANARI_RUNTIME_WORKERS='{}'. Using default.",
                    raw
                ),
            }
        }

        num_cpus::get().clamp(1, DEFAULT_MAX_RUNTIME_WORKERS)
    }

    pub(crate) fn repair_blockchain_from_dag_state(
        blockchain: &mut Arc<RwLock<Blockchain>>,
        persisted_dag_state: Option<&PersistentDagState>,
    ) -> Result<bool> {
        let Some(dag_state) = persisted_dag_state else {
            return Ok(false);
        };

        let latest_dag_sequence = dag_state
            .checkpoints
            .last()
            .map(|checkpoint| checkpoint.sequence)
            .unwrap_or(0);
        let dag_transaction_count: usize = dag_state
            .checkpoints
            .iter()
            .map(|checkpoint| checkpoint.transactions.len())
            .sum();

        let (current_height, current_transaction_count) = {
            let chain = blockchain.read().unwrap_or_else(|e| e.into_inner());
            (chain.height(), chain.get_transaction_count())
        };
        if latest_dag_sequence == 0
            || current_height > latest_dag_sequence
            || (current_height > 0 && current_transaction_count >= dag_transaction_count)
        {
            return Ok(false);
        }

        let mut rebuilt = Blockchain::new();
        for checkpoint in dag_state.checkpoints.iter().skip(1) {
            rebuilt.add_checkpoint_with_validation(checkpoint.clone(), false)?;
        }
        rebuilt.rebuild_tx_hash_index();

        info!(
            "Recovered blockchain from persisted DAG state (height: {}, checkpoints: {}, txs: {})",
            rebuilt.height(),
            rebuilt.dag_checkpoints.len(),
            rebuilt.get_transaction_count()
        );
        *blockchain = Arc::new(RwLock::new(rebuilt));
        Ok(true)
    }

    fn load_blockchain(store: &Option<Arc<PersistentStore>>) -> Arc<RwLock<Blockchain>> {
        if let Some(store) = store {
            match store.load::<Blockchain>(b"blockchain") {
                Ok(Some(mut blockchain)) => {
                    Self::hydrate_blockchain_transactions(store, &mut blockchain);
                    info!(
                        "Successfully loaded blockchain from persistent store (height: {}, checkpoints: {})",
                        blockchain.height(),
                        blockchain.dag_checkpoints.len()
                    );
                    blockchain.rebuild_tx_hash_index();
                    Arc::new(RwLock::new(blockchain))
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

    fn load_state(
        store: &Option<Arc<PersistentStore>>,
        enable_in_memory_smt: bool,
    ) -> Result<Arc<RwLock<StateManager>>> {
        let store = match store.clone() {
            Some(store) => store,
            None => Arc::new(PersistentStore::open_in_memory()?),
        };
        info!("Initializing StateManager with persistent store support (RocksDB)");
        let state = if enable_in_memory_smt && store.get_db().is_none() {
            StateManager::try_new_with_in_memory_smt(store)?
        } else {
            StateManager::try_new(store)?
        };
        Ok(Arc::new(RwLock::new(state)))
    }

    fn load_dag_state(store: &Option<Arc<PersistentStore>>) -> Option<PersistentDagState> {
        if let Some(store) = store {
            match store.load::<PersistentDagState>(b"dag_state") {
                Ok(Some(mut state)) => {
                    Self::hydrate_dag_state_transactions(store, &mut state);
                    info!("Successfully loaded DAG consensus state from persistent store");
                    Some(state)
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
}
