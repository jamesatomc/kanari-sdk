// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! DAG-based block production for Kanari blockchain engine
//! Integrates DAG consensus with parallel transaction execution

use anyhow::Result;
use centauri::consensus::{DagConsensus, VertexId};
use log::{error, info};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Arc, RwLock};

use super::*;

/// DAG Block Production Info
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DagBlockInfo {
    pub vertex_id: String,
    pub round: u64,
    pub tx_count: usize,
    pub executed: usize,
    pub failed: usize,
    pub events: Vec<Event>,
    pub checkpoint: Option<CheckpointInfo>,
    /// The actual DAG vertex for network broadcast
    pub vertex: Option<centauri::consensus::DagVertex>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointInfo {
    pub sequence: u64,
    pub vertex_count: usize,
    pub tx_count: usize,
}

type StateCache = Arc<RwLock<LruCache<Vec<u8>, Arc<RwLock<StateManager>>>>>;

/// DAG-enabled blockchain engine
#[derive(Clone)]
pub struct DagEngine {
    /// Reference to the base blockchain engine
    engine: Arc<BlockchainEngine>,

    /// DAG consensus instance
    consensus: Arc<RwLock<DagConsensus>>,

    /// This node's authority ID
    authority_id: String,

    /// Cache for execution results to avoid re-execution in apply_checkpoint
    /// Maps VertexId -> Post-execution State
    state_cache: StateCache,
}

impl DagEngine {
    /// Create a new DAG engine with default configuration (optimized for 500K TPS)
    pub fn new(
        engine: Arc<BlockchainEngine>,
        authority_id: String,
        authorities: Vec<String>,
    ) -> Result<Self> {
        // Enable DAG mode on blockchain
        {
            let mut blockchain = engine.blockchain.write().unwrap_or_else(|e| e.into_inner());
            blockchain.enable_dag_mode();
        }

        let mut consensus = DagConsensus::new(authority_id.clone(), authorities);

        // Load persisted DAG state if it exists
        if let Some(dag_state) = &engine.persisted_dag_state {
            if let Err(e) = consensus.load_state(dag_state.clone()) {
                error!(
                    "Failed to load persisted DAG state: {}. Creating fresh state.",
                    e
                );
            } else {
                info!("Successfully loaded persisted DAG state.");
            }
        }

        Ok(Self {
            engine,
            consensus: Arc::new(RwLock::new(consensus)),
            authority_id,
            state_cache: Arc::new(RwLock::new(LruCache::new(
                NonZeroUsize::new(10).expect("Cache size must be > 0"),
            ))),
        })
    }

    // =====================================================================
    // 💡 HELPER: Collect unique, unexecuted Transactions (History + Current)
    // =====================================================================
    fn collect_unexecuted_txs<'a>(
        &self,
        history_vertices: &[VertexId],
        current_txs: impl Iterator<Item = &'a kanari_types::transaction::SignedTransaction>,
        chain: &centauri::blockchain::Blockchain,
    ) -> Result<Vec<kanari_types::transaction::SignedTransaction>> {
        let mut seen_tx_hashes = std::collections::HashSet::new();
        let mut all_to_execute = Vec::new();
        let consensus = self.consensus.read().unwrap_or_else(|e| e.into_inner());

        // 1. Fetch from old History (Parent generation Vertices)
        for v_id in history_vertices {
            if let Some(v) = consensus.store().get_vertex(v_id) {
                for signed_tx in &v.transactions {
                    let tx_hash = signed_tx.hash();
                    if seen_tx_hashes.insert(tx_hash.clone())
                        && !chain.is_transaction_executed(&hex::encode(&tx_hash))
                    {
                        all_to_execute.push(signed_tx.clone());
                    }
                }
            }
        }

        // 2. Fetch from Current (Latest Vertex)
        for signed_tx in current_txs {
            let tx_hash = signed_tx.hash();
            if seen_tx_hashes.insert(tx_hash.clone())
                && !chain.is_transaction_executed(&hex::encode(&tx_hash))
            {
                all_to_execute.push(signed_tx.clone());
            }
        }

        Ok(all_to_execute)
    }

    /// Produce a DAG vertex with pending transactions
    pub fn produce_vertex(&self) -> Result<DagBlockInfo> {
        let (history_vertices, history_tx_hashes) = {
            let consensus = self.consensus.read().unwrap_or_else(|e| e.into_inner());
            let current_round = consensus.store().current_round();
            let parents: Vec<VertexId> = consensus
                .store()
                .get_vertices_in_round(current_round)
                .into_iter()
                .map(|v| v.id)
                .collect();

            let history_vertices = consensus.collect_history_for_parents(&parents)?;
            let mut history_tx_hashes = std::collections::BTreeSet::new();
            for v_id in &history_vertices {
                if let Some(v) = consensus.store().get_vertex(v_id) {
                    for tx in &v.transactions {
                        history_tx_hashes.insert(tx.hash());
                    }
                }
            }
            (history_vertices, history_tx_hashes)
        };

        let (transactions, tx_to_remove_from_pending) = {
            let _state = self.engine.state.read().unwrap_or_else(|e| e.into_inner());
            let chain = self
                .engine
                .blockchain
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let pending = self
                .engine
                .pending_txs
                .read()
                .unwrap_or_else(|e| e.into_inner());

            let mut to_include = Vec::new();
            let mut to_remove = Vec::new();

            for tx in pending.iter().take(500_000) {
                let hash = tx.hash();
                let hash_hex = hex::encode(&hash);

                if history_tx_hashes.contains(&hash) {
                    continue;
                } else if chain.is_transaction_executed(&hash_hex) {
                    to_remove.push(hash);
                } else {
                    to_include.push(tx.clone());
                }
            }
            (to_include, to_remove)
        };

        if !tx_to_remove_from_pending.is_empty() {
            let mut pending = self
                .engine
                .pending_txs
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let remove_set: std::collections::HashSet<_> =
                tx_to_remove_from_pending.into_iter().collect();
            pending.retain(|tx| !remove_set.contains(&tx.hash()));
        }

        if transactions.is_empty() && history_vertices.is_empty() {
            anyhow::bail!("No new transactions and no history to commit");
        }

        let tx_count = transactions.len();
        // FIX: Use as_secs() instead of as_millis() to match validation expectations (seconds, not milliseconds)
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs()) // ✅ CORRECT - seconds
            .unwrap_or(0);

        // Process transactions using the engine helper
        let (executed_state, state_root, executed, failed) = {
            let state_guard = self.engine.state.read().unwrap_or_else(|e| e.into_inner());
            let state_clone = state_guard.clone();
            let chain = self
                .engine
                .blockchain
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let state_arc = Arc::new(RwLock::new(state_clone));

            // Fetch all TXs using Helper
            let all_to_execute =
                self.collect_unexecuted_txs(&history_vertices, transactions.iter(), &chain)?;

            info!(
                "[DAG] Executing {} transactions in parallel waves",
                all_to_execute.len()
            );

            // Execute transactions in parallel waves
            let (executed_count, failed_count) = self
                .engine
                .execute_tx_waves_parallel(
                    all_to_execute,
                    &state_arc,
                    Some(timestamp),
                    false, // persist_objects = false (wait for Commit confirmation before saving)
                    false, // strict_mode = false (on failure, just increment failed_count and continue)
                )
                .unwrap_or((0, 0));

            let root = state_arc
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .compute_state_root();
            (state_arc, root, executed_count, failed_count)
        };

        let events: Vec<Event> = Vec::new();
        let vertex = {
            let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
            let v = consensus.create_vertex(transactions.clone(), state_root.clone(), timestamp)?;
            info!(
                "[DAG] Created vertex for round {} with {} transactions",
                v.round,
                transactions.len()
            );
            v
        };

        {
            let mut cache = self.state_cache.write().unwrap_or_else(|e| e.into_inner());
            cache.put(vertex.id.to_vec(), executed_state);
        }

        let vertex_id = hex::encode(vertex.id);
        let round = vertex.round;
        let vertex_for_broadcast = vertex.clone();

        {
            let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
            consensus.add_vertex(vertex)?;
        }

        let checkpoint_info = {
            let checkpoint = {
                let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
                consensus.try_commit()?
            };

            if let Some(checkpoint) = checkpoint {
                let mut applied = false;

                if checkpoint.vertices.len() == 1 {
                    let v_id = checkpoint.vertices[0];
                    let cached_state = {
                        let mut cache = self.state_cache.write().unwrap_or_else(|e| e.into_inner());
                        cache.get(&v_id.to_vec()).cloned()
                    };

                    if cached_state.is_some_and(|state| {
                        self.engine
                            .apply_checkpoint_optimized(checkpoint.clone(), state)
                            .is_ok()
                    }) {
                        applied = true;
                    }
                }

                if !applied {
                    self.engine.apply_checkpoint(checkpoint.clone())?;
                }

                {
                    let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
                    consensus.add_checkpoint(checkpoint.clone())?;
                }

                Some(CheckpointInfo {
                    sequence: checkpoint.sequence,
                    vertex_count: checkpoint.vertices.len(),
                    tx_count: checkpoint.transactions.len(),
                })
            } else {
                None
            }
        };

        Ok(DagBlockInfo {
            vertex_id,
            round,
            tx_count,
            executed,
            failed,
            events,
            checkpoint: checkpoint_info,
            vertex: Some(vertex_for_broadcast),
        })
    }

    pub fn consensus(&self) -> Arc<RwLock<DagConsensus>> {
        self.consensus.clone()
    }

    pub fn engine(&self) -> Arc<BlockchainEngine> {
        self.engine.clone()
    }

    pub fn authority_id(&self) -> &str {
        &self.authority_id
    }

    pub fn sync_checkpoint(&self, checkpoint: centauri::consensus::Checkpoint) -> Result<()> {
        let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
        consensus.add_checkpoint(checkpoint)
    }

    pub fn add_network_vertex(&self, vertex: centauri::consensus::DagVertex) -> Result<()> {
        let vertex_id_hex = hex::encode(vertex.id);
        info!(
            "[DAG SYNC] Received vertex {} for round {} from network with {} transactions",
            vertex_id_hex,
            vertex.round,
            vertex.transactions.len()
        );

        {
            let consensus = self.consensus.read().unwrap();
            if consensus.has_vertex(&vertex.id) {
                info!(
                    "[DAG SYNC] Vertex {} (round {}) already exists, skipping",
                    vertex_id_hex, vertex.round
                );
                return Ok(());
            }
        }

        let transactions = vertex.transactions.clone();

        if !transactions.is_empty() {
            {
                let mut pending = self.engine.pending_txs.write().unwrap();
                let tx_hashes: std::collections::HashSet<Vec<u8>> =
                    transactions.iter().map(|tx| tx.hash()).collect();
                pending.retain(|tx| !tx_hashes.contains(&tx.hash()));

                if !pending.is_empty() {
                    info!(
                        "[DAG SYNC] Removed {} transactions from pending pool (keeping {})",
                        transactions.len(),
                        pending.len()
                    );
                }
            }

            // Process network vertex transactions using the engine helper
            let (computed_state_root, executed, failed) = {
                let state_clone = self.engine.state.read().unwrap().clone();
                let chain = self.engine.blockchain.read().unwrap();
                let state_arc = Arc::new(RwLock::new(state_clone));

                let history_vertices = {
                    let consensus = self.consensus.read().unwrap();
                    consensus.collect_history_for_parents(&vertex.parents)?
                };

                let all_to_execute =
                    self.collect_unexecuted_txs(&history_vertices, transactions.iter(), &chain)?;

                if all_to_execute.is_empty() {
                    info!(
                        "[DAG SYNC] No new transactions to execute for vertex round {} (ID: {})",
                        vertex.round, vertex_id_hex
                    );
                } else {
                    info!(
                        "[DAG SYNC] Validating {} transactions in parallel waves for vertex round {}",
                        all_to_execute.len(),
                        vertex.round
                    );
                }

                // Execute transactions in parallel waves
                let (executed_count, failed_count) = self
                    .engine
                    .execute_tx_waves_parallel(
                        all_to_execute,
                        &state_arc,
                        Some(vertex.timestamp),
                        false, // persist_objects = false
                        false, // strict_mode = false
                    )
                    .unwrap_or((0, 0));

                let root = state_arc.write().unwrap().compute_state_root();
                (root, executed_count, failed_count)
            };

            info!(
                "[DAG SYNC] Validation result for vertex round {}: executed={}, failed={}, computed_root={}",
                vertex.round,
                executed,
                failed,
                hex::encode(&computed_state_root)
            );

            let expected_state_root = &vertex.metadata.state_root;
            if computed_state_root != *expected_state_root {
                error!(
                    "[DAG SYNC] STATE ROOT MISMATCH for vertex round {}!\n  Expected: {}\n  Computed: {}\n  Transactions: {}\nRejecting vertex due to state divergence.",
                    vertex.round,
                    hex::encode(expected_state_root),
                    hex::encode(&computed_state_root),
                    transactions.len()
                );
                anyhow::bail!("STATE ROOT MISMATCH for vertex round {}", vertex.round);
            }

            info!(
                "[DAG SYNC] State root validated successfully for vertex round {}: {}",
                vertex.round,
                hex::encode(&computed_state_root)
            );
        }

        let checkpoint = {
            let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
            consensus.add_vertex(vertex)?;
            consensus.try_commit()?
        };

        if let Some(checkpoint) = checkpoint {
            info!(
                "[DAG SYNC] Committed checkpoint {} with {} transactions",
                checkpoint.sequence,
                checkpoint.transactions.len()
            );

            if let Err(e) = self.engine.apply_checkpoint(checkpoint.clone()) {
                error!(
                    "[DAG SYNC] Failed to apply committed checkpoint to engine: {}",
                    e
                );
            } else {
                let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
                let _ = consensus.add_checkpoint(checkpoint);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dag_engine_creation() {
        let engine = Arc::new(BlockchainEngine::new().unwrap());
        let authorities = vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ];

        let dag_engine = DagEngine::new(engine, "auth1".to_string(), authorities);
        assert!(dag_engine.is_ok());
    }
}
