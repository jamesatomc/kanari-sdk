// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! DAG-based Consensus Implementation (Narwhal & Tusk / Bullshark style)
//!
//! This module implements a Directed Acyclic Graph consensus mechanism that separates:
//! - Data Availability (DA): Broadcasting and storing transaction data
//! - Ordering: Determining the total order of transactions
//!
//! Inspired by Sui's Narwhal & Bullshark consensus, this design enables:
//! - High throughput through parallel block production
//! - Low latency by decoupling DA from consensus
//! - Byzantine fault tolerance
//! - Efficient parallel execution (already supported in Kanari's produce_block.rs)

use crate::calculate_quorum;

use super::byzantine_detector::ByzantineDetector;
use super::cache::DagCaches;
use super::committee::{Committee, ValidatorInfo};
use super::crypto_signatures::Ed25519Keypair;
use super::metrics::DagMetrics;
use super::parallel_validator::{ParallelValidator, ParallelValidatorConfig};
use super::persistent_store::PersistentDagStore;
use super::pruning::{DagPruner, PruningConfig};
use super::state_sync::StateSynchronizer;
use super::vertex_broadcast::VertexBroadcaster;
use super::vrf_leader::VrfLeaderElection;
use anyhow::Result;
use kanari_crypto::hash_data_blake3;
use kanari_types::transaction::SignedTransaction;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
// Use fully-qualified time APIs where needed to avoid unused-import warnings
use tokio::sync::mpsc;

/// Unique identifier for a DAG vertex (block)
/// Fixed-size [u8; 32] for zero heap allocations (500K TPS optimization)
/// Blake3 hash output is always 32 bytes
pub type VertexId = [u8; 32];

/// Round number in the DAG consensus protocol
pub type Round = u64;

/// Authority/validator identifier
pub type AuthorityId = String;

/// DAG Vertex - represents a batch of transactions in the DAG
/// Each vertex can reference multiple parent vertices, forming a DAG
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagVertex {
    /// Unique identifier (hash of the vertex) - fixed [u8; 32]
    pub id: VertexId,

    /// Round number in the consensus protocol
    pub round: Round,

    /// Authority that created this vertex
    pub author: AuthorityId,

    /// Chain ID for cross-chain replay protection
    pub chain_id: String,

    /// References to parent vertices (from previous round)
    /// Requires 2f+1 parents for quorum (where f is max faulty nodes)
    pub parents: Vec<VertexId>,

    /// Transactions included in this vertex
    pub transactions: Vec<SignedTransaction>,

    /// Timestamp when vertex was created
    pub timestamp: u64,

    /// Signature from the authority
    pub signature: Vec<u8>,

    /// Metadata for consensus
    pub metadata: VertexMetadata,

    /// Cached serialized data (500K TPS optimization - avoid repeated serialization)
    #[serde(skip)]
    pub cached_serialized_data: Option<Vec<u8>>,

    /// Cached hash for quick lookups (computed once)
    #[serde(skip)]
    pub cached_hash: Option<Vec<u8>>,
}

/// Metadata for DAG vertex
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertexMetadata {
    /// Total number of transactions in this vertex
    pub tx_count: usize,

    /// Gas used by transactions
    pub total_gas_used: u64,

    /// State root after executing transactions
    pub state_root: Vec<u8>,

    /// Whether this vertex is a checkpoint (committed)
    pub is_checkpoint: bool,

    /// Checkpoint sequence number (if committed)
    pub checkpoint_seq: Option<u64>,
}

impl DagVertex {
    fn hash_material(&self) -> Result<Vec<u8>> {
        let tx_hashes: Vec<Vec<u8>> = self.transactions.iter().map(|tx| tx.hash()).collect();
        let bytes = bcs::to_bytes(&(
            &self.chain_id, // Chain ID for replay protection
            self.round,
            &self.author,
            &self.parents,
            tx_hashes,
            self.timestamp,
            &self.metadata.state_root,
        ))
        .map_err(|e| anyhow::anyhow!("Failed to serialize vertex for hashing: {}", e))?;

        Ok(bytes)
    }

    /// Create a new DAG vertex (panics on serialization failure - use try_new() for production)
    pub fn new(
        round: Round,
        author: AuthorityId,
        chain_id: String, // FIX #3: Chain ID for cross-chain replay protection
        parents: Vec<VertexId>,
        transactions: Vec<SignedTransaction>,
        state_root: Vec<u8>,
        timestamp: u64,
    ) -> Self {
        Self::try_new(
            round,
            author,
            chain_id,
            parents,
            transactions,
            state_root,
            timestamp,
        )
        .expect("DagVertex::new failed - this should never happen with valid inputs")
    }

    /// Try to create a new DAG vertex (returns Result for safe error handling)
    pub fn try_new(
        round: Round,
        author: AuthorityId,
        chain_id: String,
        parents: Vec<VertexId>,
        transactions: Vec<SignedTransaction>,
        state_root: Vec<u8>,
        timestamp: u64,
    ) -> Result<Self> {
        let tx_count = transactions.len();

        let metadata = VertexMetadata {
            tx_count,
            total_gas_used: 0,
            state_root,
            is_checkpoint: false,
            checkpoint_seq: None,
        };

        let mut vertex = Self {
            id: [0u8; 32], // Will be computed
            round,
            author,
            chain_id,
            parents,
            transactions,
            timestamp,
            signature: Vec::new(),
            metadata,
            cached_serialized_data: None,
            cached_hash: None,
        };

        // Compute vertex ID (hash) and cache it
        let hash = vertex.compute_hash()?;
        vertex.cached_hash = Some(hash.to_vec());
        vertex.id = hash;
        Ok(vertex)
    }

    /// Create a new DAG vertex with default chain_id (for testing)
    #[cfg(test)]
    pub fn new_for_test(
        round: Round,
        author: AuthorityId,
        parents: Vec<VertexId>,
        transactions: Vec<SignedTransaction>,
        state_root: Vec<u8>,
        timestamp: u64,
    ) -> Self {
        Self::new(
            round,
            author,
            "test-chain".to_string(), // Default test chain_id
            parents,
            transactions,
            state_root,
            timestamp,
        )
    }

    /// Compute hash of the vertex (excluding id and signature)
    /// 500K TPS optimization: caches result to avoid repeated serialization
    /// Returns fixed-size [u8; 32] array (no heap allocation)
    pub fn compute_hash(&self) -> Result<VertexId> {
        // Return cached hash if available
        if let Some(ref cached) = self.cached_hash {
            // Convert Vec<u8> cache to [u8; 32]
            let mut result = [0u8; 32];
            result.copy_from_slice(&cached[..32]);
            return Ok(result);
        }

        let hash_vec = hash_data_blake3(&self.hash_material()?);
        let mut result = [0u8; 32];
        result.copy_from_slice(&hash_vec[..32]);
        Ok(result)
    }

    /// Get serialized data with caching (500K TPS optimization)
    pub fn get_serialized_data(&mut self) -> Result<&[u8]> {
        if self.cached_serialized_data.is_none() {
            let serialized = bcs::to_bytes(self)?;
            self.cached_serialized_data = Some(serialized);
        }
        // Safe unwrap - we just ensured it's Some above
        Ok(self.cached_serialized_data.as_ref().unwrap())
    }

    /// Verify vertex integrity
    pub fn verify(&self) -> Result<()> {
        let computed_hash = self.compute_hash()?;
        if self.id != computed_hash {
            anyhow::bail!("Vertex hash mismatch");
        }

        // Verify transaction count
        if self.transactions.len() != self.metadata.tx_count {
            anyhow::bail!("Transaction count mismatch");
        }

        Ok(())
    }

    /// Check if this vertex has quorum support from unique, trusted authors
    ///
    /// This validates that at least `2f+1` unique trusted authorities have referenced
    /// this vertex as a parent. Excludes banned/untrusted authorities to prevent
    /// Byzantine nodes from influencing consensus.
    ///
    /// # Arguments
    /// * `store` - DAG store with vertex data and trust information
    /// * `total_authorities` - Total validators in committee
    ///
    /// # Returns
    /// `true` if quorum is reached from trusted authorities only
    pub fn has_quorum_unique_authors(&self, store: &DagStore, total_authorities: usize) -> bool {
        // Prevent underflow and ensure meaningful quorum calculation
        if total_authorities == 0 {
            return false;
        }

        let quorum_size = calculate_quorum(total_authorities);

        // FIX #2 & #3: CRITICAL - Filter out banned/untrusted authorities before counting quorum
        // Previously counted ALL authors including Byzantine ones that should be excluded
        let mut unique_authors = HashSet::new();
        for parent_id in &self.parents {
            if let Some(parent_vertex) = store.get_vertex(parent_id) {
                // FIX #2: Check if author is trusted (not banned/slash to reputation 0)
                // This prevents Byzantine nodes from participating in quorum
                if store.is_authority_trusted(&parent_vertex.author) {
                    unique_authors.insert(parent_vertex.author.clone());
                } else {
                    tracing::warn!(
                        "[Security] Excluding untrusted authority {} from quorum calculation",
                        parent_vertex.author
                    );
                }
            }
        }

        // Must have quorum from unique TRUSTED authors only
        unique_authors.len() >= quorum_size
    }
}

/// Checkpoint represents a committed sequence of vertices
/// This is the output of the consensus protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Checkpoint sequence number
    pub sequence: u64,

    /// Vertices included in this checkpoint (in total order)
    pub vertices: Vec<VertexId>,

    /// All transactions in order
    pub transactions: Vec<SignedTransaction>,

    /// State root after executing all transactions
    pub state_root: Vec<u8>,

    /// Timestamp of checkpoint creation
    pub timestamp: u64,

    /// Previous checkpoint hash
    pub prev_checkpoint_hash: Vec<u8>,
}

impl Checkpoint {
    pub fn new(
        sequence: u64,
        vertices: Vec<VertexId>,
        transactions: Vec<SignedTransaction>,
        state_root: Vec<u8>,
        timestamp: u64,
        prev_checkpoint_hash: Vec<u8>,
    ) -> Self {
        Self {
            sequence,
            vertices,
            transactions,
            state_root,
            timestamp,
            prev_checkpoint_hash,
        }
    }

    pub fn hash(&self) -> Result<Vec<u8>> {
        let serialized = bcs::to_bytes(self)?;
        Ok(hash_data_blake3(&serialized))
    }

    /// Genesis checkpoint
    pub fn genesis() -> Self {
        let genesis_state_root = smt::default_hashes()[0].to_vec();
        Self {
            sequence: 0,
            vertices: Vec::new(),
            transactions: Vec::new(),
            state_root: genesis_state_root,
            timestamp: 0,
            prev_checkpoint_hash: vec![0u8; 32],
        }
    }
}

/// Checkpoint configuration
#[derive(Debug, Clone)]
pub struct CheckpointConfig {
    /// Minimum rounds between checkpoints
    pub min_rounds: u64,

    /// Maximum rounds between checkpoints (force checkpoint after this)
    pub max_rounds: u64,

    /// Minimum vertices to justify creating a checkpoint
    pub min_vertices: usize,

    /// Maximum pending vertices before forcing a checkpoint
    pub max_vertices: usize,
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self {
            min_rounds: 5,       // Faster checkpointing for 500K TPS
            max_rounds: 50,      // More frequent forced checkpoints
            min_vertices: 1000,  // Larger batches for efficiency
            max_vertices: 50000, // 50k pending before force (handle bursts)
        }
    }
}

impl CheckpointConfig {
    fn from_values(
        min_rounds: u64,
        max_rounds: u64,
        min_vertices: usize,
        max_vertices: usize,
    ) -> Self {
        Self {
            min_rounds,
            max_rounds,
            min_vertices,
            max_vertices,
        }
    }

    /// Create high-throughput config for 500K+ TPS
    pub fn high_throughput() -> Self {
        Self::from_values(2, 20, 5000, 100000)
    }

    /// Create conservative config (frequent checkpoints, low latency)
    pub fn conservative() -> Self {
        Self::from_values(5, 50, 50, 5000)
    }

    /// Create aggressive config (infrequent checkpoints, high throughput)
    pub fn aggressive() -> Self {
        Self::from_values(20, 200, 500, 50000)
    }

    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if self.min_rounds >= self.max_rounds {
            anyhow::bail!("min_rounds must be less than max_rounds");
        }
        if self.min_vertices >= self.max_vertices {
            anyhow::bail!("min_vertices must be less than max_vertices");
        }
        if self.max_rounds == 0 {
            anyhow::bail!("max_rounds must be greater than 0");
        }
        Ok(())
    }
}

/// DAG Storage - maintains the DAG structure
#[derive(Clone)]
pub struct DagStore {
    /// All vertices indexed by their ID (Arc for zero-copy sharing - 500K TPS)
    /// Optimized: HashMap for O(1) lookup instead of BTreeMap O(log n)
    vertices: HashMap<VertexId, Arc<DagVertex>>,
    /// Vertices indexed by round number
    vertices_by_round: BTreeMap<Round, Vec<VertexId>>,
    /// Vertices indexed by authority
    vertices_by_authority: BTreeMap<AuthorityId, Vec<VertexId>>,
    /// Checkpoints (committed state)
    checkpoints: VecDeque<Checkpoint>,
    /// Pending vertices (not yet checkpointed)
    pending_vertices: VecDeque<VertexId>,
    /// Executed transaction hashes to prevent replay across checkpoints
    /// Optimized: HashSet for O(1) lookup instead of BTreeSet O(log n)
    executed_tx_hashes: HashSet<Vec<u8>>,
    /// Current round number
    current_round: Round,
    /// Set of authority IDs
    /// Optimized: HashSet for O(1) lookup instead of BTreeSet O(log n)
    authorities: HashSet<AuthorityId>,
    /// Map of vertex ID to its checkpoint sequence number (for GC)
    vertex_checkpoint_map: HashMap<VertexId, u64>,
    /// Checkpoint configuration
    checkpoint_config: CheckpointConfig,
    /// Round of last checkpoint
    last_checkpoint_round: Round,
    /// Backpressure limit for pending vertices (500K TPS protection)
    max_pending_vertices: usize,
    /// FIX #2 & #3: Track banned/untrusted authorities (reputation = 0)
    banned_authorities: HashSet<AuthorityId>,
}

impl DagStore {
    fn validate_pending_transactions(&self, vertex: &DagVertex) -> Result<()> {
        let mut local_hashes = HashSet::new();
        for tx in &vertex.transactions {
            let tx_hash = tx.hash();
            if self.executed_tx_hashes.contains(&tx_hash) {
                anyhow::bail!("Duplicate committed transaction");
            }
            if !local_hashes.insert(tx_hash) {
                anyhow::bail!("Duplicate transaction inside vertex");
            }
        }
        Ok(())
    }

    fn index_vertex(&mut self, vertex_id: VertexId, round: Round, author: AuthorityId) {
        self.vertices_by_round
            .entry(round)
            .or_default()
            .push(vertex_id);
        self.vertices_by_authority
            .entry(author)
            .or_default()
            .push(vertex_id);
    }

    fn insert_vertex_arc(&mut self, vertex_id: VertexId, vertex_arc: Arc<DagVertex>) {
        let round = vertex_arc.round;
        let author = vertex_arc.author.clone();
        self.vertices.insert(vertex_id, vertex_arc);
        self.index_vertex(vertex_id, round, author);
    }

    fn validate_new_vertex(&self, vertex: &DagVertex, total_authorities: usize) -> Result<()> {
        vertex.verify()?;
        if self.vertices.contains_key(&vertex.id) {
            anyhow::bail!("Vertex already exists");
        }
        self.validate_pending_transactions(vertex)?;

        // FIX #2: CRITICAL - Remove SystemTime-based validation that causes consensus splits
        // Different nodes have different system clocks. If a node's clock is behind,
        // it will reject valid blocks from other nodes, causing permanent network partition.
        //
        // Instead, use median timestamp from parents for time-jacking protection.
        // This ensures all nodes agree on timestamp validity based on DAG structure alone.

        if vertex.round > 0 {
            if !vertex.has_quorum_unique_authors(self, total_authorities) {
                anyhow::bail!("Vertex does not have quorum from unique authors");
            }

            let mut max_parent_timestamp = 0;
            let mut parent_timestamps = Vec::new();

            for parent_id in &vertex.parents {
                let parent = self
                    .vertices
                    .get(parent_id)
                    .ok_or_else(|| anyhow::anyhow!("Parent vertex not found"))?;

                if parent.round != vertex.round - 1 {
                    anyhow::bail!("Parent from wrong round");
                }

                if parent.timestamp > max_parent_timestamp {
                    max_parent_timestamp = parent.timestamp;
                }

                // Collect parent timestamps for median calculation
                parent_timestamps.push(parent.timestamp);
            }

            // FIX #2: Validate timestamp against median of parents (not SystemTime)
            // This prevents consensus splits due to clock skew between nodes
            if !parent_timestamps.is_empty() {
                parent_timestamps.sort_unstable();
                let median_timestamp = parent_timestamps[parent_timestamps.len() / 2];

                // Vertex timestamp must be >= median parent timestamp
                if vertex.timestamp < median_timestamp {
                    anyhow::bail!(
                        "Vertex timestamp {} is older than median parent timestamp {}",
                        vertex.timestamp,
                        median_timestamp
                    );
                }

                // Allow reasonable future tolerance based on parent median
                // FIX #5: Handle node restart scenarios where parents are very old
                // If parent median is more than 10 minutes behind current time, assume node just restarted/paused
                // and use current time as baseline instead of old parent timestamps
                let current_time = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                const MAX_TIMESTAMP_DRIFT_SECS: u64 = 300; // 5 minutes for normal operation
                const RESTART_THRESHOLD_SECS: u64 = 600; // 10 minutes threshold for restart detection (reduced from 15min)

                let max_allowed =
                    if current_time.saturating_sub(median_timestamp) > RESTART_THRESHOLD_SECS {
                        // Node likely just restarted or paused - use current time as baseline
                        // Allow vertex timestamp to be close to current time (±30 seconds)
                        current_time.saturating_add(30)
                    } else {
                        // Normal operation - use parent median as baseline
                        median_timestamp.saturating_add(MAX_TIMESTAMP_DRIFT_SECS)
                    };

                if vertex.timestamp > max_allowed {
                    anyhow::bail!(
                        "Vertex timestamp {} too far ahead of parent median {} (drift: {}s, max allowed: {}s)",
                        vertex.timestamp,
                        median_timestamp,
                        vertex.timestamp.saturating_sub(median_timestamp),
                        max_allowed.saturating_sub(median_timestamp)
                    );
                }
            }

            if vertex.timestamp < max_parent_timestamp {
                anyhow::bail!("Vertex timestamp is older than its newest parent");
            }
        }

        Ok(())
    }

    pub fn new(authorities: Vec<AuthorityId>) -> Self {
        Self::with_config(authorities, CheckpointConfig::default())
    }

    pub fn with_config(authorities: Vec<AuthorityId>, config: CheckpointConfig) -> Self {
        let genesis_checkpoint = Checkpoint::genesis();

        // Backpressure limit: high-throughput mode allows 1M pending, default 100K
        let max_pending = if config.min_vertices >= 1000 {
            1_000_000 // High-throughput mode (500K TPS)
        } else {
            100_000 // Default mode
        };

        Self {
            vertices: HashMap::new(),
            vertices_by_round: BTreeMap::new(),
            vertices_by_authority: BTreeMap::new(),
            checkpoints: VecDeque::from([genesis_checkpoint]),
            pending_vertices: VecDeque::new(),
            executed_tx_hashes: HashSet::new(),
            current_round: 0,
            authorities: authorities.into_iter().collect(),
            vertex_checkpoint_map: HashMap::new(),
            checkpoint_config: config,
            last_checkpoint_round: 0,
            max_pending_vertices: max_pending,
            banned_authorities: HashSet::new(),
        }
    }

    /// Check if backpressure should be applied (500K TPS protection)
    pub fn should_apply_backpressure(&self) -> bool {
        self.pending_vertices.len() >= self.max_pending_vertices
    }

    /// Check if a checkpoint should be created based on configuration
    pub fn should_create_checkpoint(&self) -> bool {
        let rounds_since_last = self
            .current_round
            .saturating_sub(self.last_checkpoint_round);
        let pending_count = self.pending_vertices.len();

        // Force checkpoint if max rounds reached
        if rounds_since_last >= self.checkpoint_config.max_rounds {
            return true;
        }

        // Too soon for checkpoint
        if rounds_since_last < self.checkpoint_config.min_rounds {
            return false;
        }

        // Force checkpoint if too many pending vertices
        if pending_count >= self.checkpoint_config.max_vertices {
            return true;
        }

        // Create checkpoint if enough vertices accumulated
        pending_count >= self.checkpoint_config.min_vertices
    }

    /// Get checkpoint configuration
    pub fn get_checkpoint_config(&self) -> &CheckpointConfig {
        &self.checkpoint_config
    }

    /// Check if a vertex is already checkpointed
    pub fn is_vertex_checkpointed(&self, id: &VertexId) -> bool {
        self.vertex_checkpoint_map.contains_key(id)
    }

    /// Update checkpoint configuration
    pub fn set_checkpoint_config(&mut self, config: CheckpointConfig) -> Result<()> {
        config.validate()?;
        self.checkpoint_config = config;
        Ok(())
    }

    /// Add a new vertex to the DAG
    pub fn add_vertex(&mut self, vertex: DagVertex, total_authorities: usize) -> Result<()> {
        self.add_vertex_arc(Arc::new(vertex), total_authorities)
    }

    /// Add a new vertex to the DAG using shared ownership.
    pub fn add_vertex_arc(
        &mut self,
        vertex: Arc<DagVertex>,
        total_authorities: usize,
    ) -> Result<()> {
        if self.should_apply_backpressure() {
            anyhow::bail!(
                "Backpressure applied: {} pending vertices (max: {})",
                self.pending_vertices.len(),
                self.max_pending_vertices
            );
        }
        self.validate_new_vertex(&vertex, total_authorities)?;
        if vertex.round > self.current_round {
            self.current_round = vertex.round;
        }
        let vertex_id = vertex.id;
        self.insert_vertex_arc(vertex_id, vertex);
        self.pending_vertices.push_back(vertex_id);
        Ok(())
    }

    /// Get vertex by ID (returns Arc reference for zero-copy)
    pub fn get_vertex(&self, id: &VertexId) -> Option<&Arc<DagVertex>> {
        self.vertices.get(id)
    }

    /// Get vertex by ID (mutable) - use sparingly with Arc
    pub fn get_vertex_mut(&mut self, id: &VertexId) -> Option<&mut Arc<DagVertex>> {
        self.vertices.get_mut(id)
    }

    /// Get all vertices in a round (returns Arc for zero-copy)
    pub fn get_vertices_in_round(&self, round: Round) -> Vec<Arc<DagVertex>> {
        self.vertices_by_round
            .get(&round)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.vertices.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all vertex IDs in a round (cheap path, avoids cloning `Arc<DagVertex>` values).
    pub fn get_vertex_ids_in_round(&self, round: Round) -> Vec<VertexId> {
        self.vertices_by_round
            .get(&round)
            .cloned()
            .unwrap_or_default()
    }

    /// Get vertices by authority (returns Arc for zero-copy)
    pub fn get_vertices_by_authority(&self, authority: &AuthorityId) -> Vec<Arc<DagVertex>> {
        self.vertices_by_authority
            .get(authority)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.vertices.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get latest checkpoint
    pub fn latest_checkpoint(&self) -> Checkpoint {
        self.checkpoints
            .back()
            .cloned()
            .unwrap_or_else(Checkpoint::genesis)
    }

    pub fn last_checkpoint_round(&self) -> Round {
        self.last_checkpoint_round
    }

    /// Get checkpoint by sequence number
    pub fn get_checkpoint(&self, sequence: u64) -> Option<&Checkpoint> {
        self.checkpoints.iter().find(|cp| cp.sequence == sequence)
    }

    /// Add a new checkpoint (commits vertices)
    pub fn add_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<()> {
        let latest = self.latest_checkpoint();
        let expected_seq = latest.sequence + 1;

        if checkpoint.sequence != expected_seq {
            // Handle duplicate checkpoint submission
            if checkpoint.sequence == latest.sequence {
                let checkpoint_hash = checkpoint.hash()?;
                let latest_hash = latest.hash()?;
                if checkpoint_hash == latest_hash {
                    return Ok(());
                }
            }
            anyhow::bail!(
                "Invalid checkpoint sequence: expected {}, got {}",
                expected_seq,
                checkpoint.sequence
            );
        }

        let prev_hash = latest.hash()?;
        if checkpoint.prev_checkpoint_hash != prev_hash {
            anyhow::bail!("Invalid previous checkpoint hash");
        }

        for tx in &checkpoint.transactions {
            self.executed_tx_hashes.insert(tx.hash());
        }

        for vertex_id in &checkpoint.vertices {
            self.vertex_checkpoint_map
                .insert(*vertex_id, checkpoint.sequence);
        }

        // FIX 1: O(N) Pending Vertices Cleanup
        let checkpoint_vertices_set: std::collections::HashSet<_> =
            checkpoint.vertices.iter().collect();
        self.pending_vertices
            .retain(|id| !checkpoint_vertices_set.contains(id));

        self.last_checkpoint_round = self.current_round;
        self.checkpoints.push_back(checkpoint.clone());

        if checkpoint.sequence > 10 {
            let cutoff_seq = checkpoint.sequence.saturating_sub(10);
            let vertices_to_remove: Vec<VertexId> = self
                .vertex_checkpoint_map
                .iter()
                .filter(|&(_, &seq)| seq <= cutoff_seq)
                .map(|(id, _)| *id)
                .collect();

            for vertex_id in vertices_to_remove {
                if let Some(vertex) = self.vertices.remove(&vertex_id) {
                    // FIX #1: Clean up all indexes properly to prevent memory leak
                    // Remove from round index
                    if let Some(round_vertices) = self.vertices_by_round.get_mut(&vertex.round) {
                        round_vertices.retain(|id| id != &vertex_id);
                        // Remove empty round entries to prevent BTreeMap growth
                        if round_vertices.is_empty() {
                            self.vertices_by_round.remove(&vertex.round);
                        }
                    }
                    // Remove from authority index
                    if let Some(auth_vertices) = self.vertices_by_authority.get_mut(&vertex.author)
                    {
                        auth_vertices.retain(|id| id != &vertex_id);
                        // Remove empty authority entries to prevent BTreeMap growth
                        if auth_vertices.is_empty() {
                            self.vertices_by_authority.remove(&vertex.author);
                        }
                    }
                }
                self.vertex_checkpoint_map.remove(&vertex_id);
            }
        }

        // Clear out old and never-committed Vertex (Orphan) instances. (Prevent OOM)
        const MAX_RETAIN_ROUNDS: u64 = 100;
        if self.current_round > MAX_RETAIN_ROUNDS {
            let cutoff_round = self.current_round.saturating_sub(MAX_RETAIN_ROUNDS);
            let orphaned_vertices: Vec<VertexId> = self
                .vertices
                .iter()
                .filter(|(_, v)| v.round < cutoff_round)
                .map(|(id, _)| *id)
                .collect();

            for id in orphaned_vertices {
                if let Some(vertex) = self.vertices.remove(&id) {
                    // FIX #1: Consistent cleanup for orphaned vertices
                    if let Some(round_vertices) = self.vertices_by_round.get_mut(&vertex.round) {
                        round_vertices.retain(|v_id| v_id != &id);
                        if round_vertices.is_empty() {
                            self.vertices_by_round.remove(&vertex.round);
                        }
                    }
                    if let Some(auth_vertices) = self.vertices_by_authority.get_mut(&vertex.author)
                    {
                        auth_vertices.retain(|v_id| v_id != &id);
                        if auth_vertices.is_empty() {
                            self.vertices_by_authority.remove(&vertex.author);
                        }
                    }
                }
            }
        }

        // Prevent memory leaks from checkpoint arrays + TX retention GC
        const TX_RETENTION_WINDOW: usize = 10_000;
        if self.checkpoints.len() > TX_RETENTION_WINDOW {
            // FIX #3: Use pop_front() for O(1) removal instead of remove(0) which is O(N)
            if let Some(old_checkpoint) = self.checkpoints.pop_front() {
                for tx in &old_checkpoint.transactions {
                    self.executed_tx_hashes.remove(&tx.hash());
                }
            }
        }

        Ok(())
    }

    /// Get statistics about pending vertices
    pub fn get_checkpoint_stats(&self) -> CheckpointStats {
        CheckpointStats {
            pending_vertices: self.pending_vertices.len(),
            rounds_since_last: self
                .current_round
                .saturating_sub(self.last_checkpoint_round),
            total_checkpoints: self.checkpoints.len(),
            should_checkpoint: self.should_create_checkpoint(),
        }
    }

    /// Current round
    pub fn current_round(&self) -> Round {
        self.current_round
    }

    /// Number of authorities
    pub fn num_authorities(&self) -> usize {
        self.authorities.len()
    }

    // FIX #2 & #3: Methods to manage banned/untrusted authorities

    /// Ban an authority (set reputation to 0, exclude from quorum)
    pub fn ban_authority(&mut self, authority: &AuthorityId) {
        self.banned_authorities.insert(authority.clone());
        tracing::warn!(
            "[Security] Authority {} has been BANNED - excluded from quorum",
            authority
        );
    }

    /// Unban an authority (restore participation rights)
    pub fn unban_authority(&mut self, authority: &AuthorityId) {
        self.banned_authorities.remove(authority);
        tracing::info!("[Security] Authority {} has been UNBANNED", authority);
    }

    /// Check if an authority is trusted (not banned)
    pub fn is_authority_trusted(&self, authority: &AuthorityId) -> bool {
        !self.banned_authorities.contains(authority)
    }

    /// Get list of all banned authorities
    pub fn get_banned_authorities(&self) -> &HashSet<AuthorityId> {
        &self.banned_authorities
    }
}

/// Statistics for checkpoint creation
#[derive(Debug, Clone)]
pub struct CheckpointStats {
    pub pending_vertices: usize,
    pub rounds_since_last: u64,
    pub total_checkpoints: usize,
    pub should_checkpoint: bool,
}

/// Serializable state for persisting DAG data across restarts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentDagState {
    pub vertices: Vec<DagVertex>,
    pub checkpoints: Vec<Checkpoint>,
    pub current_round: Round,
    pub last_checkpoint_round: Round,
}

/// DAG Consensus Protocol (Bullshark-style with VRF leader election)
pub struct DagConsensus {
    /// DAG storage
    store: DagStore,

    /// This node's authority ID
    authority_id: AuthorityId,

    /// Chain ID for cross-chain replay protection
    chain_id: String,

    /// VRF-based leader election (replaces round-robin)
    vrf_election: VrfLeaderElection,

    /// Byzantine fault detector
    byzantine_detector: ByzantineDetector,

    /// Caching layer for performance optimization
    caches: DagCaches,

    /// Committee management for dynamic validator sets
    committee: Committee,

    /// Metrics collection for monitoring
    metrics: DagMetrics,

    /// State synchronization for new nodes
    state_sync: StateSynchronizer,

    /// Vertex broadcasting with compression
    broadcaster: VertexBroadcaster,

    /// Persistent storage backend for DAG vertices
    persistent_store: Option<PersistentDagStore>,

    /// Async channel for background disk writes (500K TPS optimization - replaced raw thread with tokio)
    disk_writer_tx: Option<mpsc::Sender<Arc<DagVertex>>>,

    /// DAG pruning to manage storage growth
    pruner: DagPruner,

    /// Parallel vertex validator for high throughput
    parallel_validator: ParallelValidator,

    /// Local keypair for signing vertices created by this authority (demo key management).
    local_signing_key: ed25519_dalek::SigningKey,
}

impl DagConsensus {
    fn authority_seed(authority: &str) -> [u8; 32] {
        let mut seed = [0u8; 32];
        let digest = hash_data_blake3(authority.as_bytes());
        seed.copy_from_slice(&digest[..32]);
        seed
    }

    fn authority_public_key(authority: &str) -> [u8; 32] {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&Self::authority_seed(authority));
        signing_key.verifying_key().to_bytes()
    }

    fn signing_payload(vertex: &DagVertex) -> Result<Vec<u8>> {
        let mut to_sign = vertex.clone();
        to_sign.signature.clear();
        bcs::to_bytes(&to_sign).map_err(|e| anyhow::anyhow!("Failed to serialize vertex: {}", e))
    }

    fn sign_vertex_with_key(
        signing_key: &ed25519_dalek::SigningKey,
        vertex: &mut DagVertex,
    ) -> Result<()> {
        let payload = Self::signing_payload(vertex)?;
        let keypair = Ed25519Keypair {
            signing_key: signing_key.clone(),
            verifying_key: signing_key.verifying_key(),
        };
        vertex.signature = keypair.sign(&payload);
        Ok(())
    }

    fn verify_vertex_signature(&mut self, vertex: &DagVertex) -> Result<()> {
        let validator = self
            .committee
            .get_validator(&vertex.author)
            .ok_or_else(|| anyhow::anyhow!("Unknown vertex author {}", vertex.author))?;
        let key_bytes: [u8; 32] = validator
            .public_key
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid public key length for {}", vertex.author))?;
        let public_key = ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)
            .map_err(|e| anyhow::anyhow!("Invalid public key for {}: {}", vertex.author, e))?;
        let result = self
            .parallel_validator
            .validate_vertex_with_public_key(vertex, &public_key)?;
        if result.is_valid {
            return Ok(());
        }
        anyhow::bail!(
            "Vertex validation failed: {}",
            result.error.as_deref().unwrap_or("unknown error")
        )
    }

    pub fn new(authority_id: AuthorityId, authorities: Vec<AuthorityId>) -> Self {
        // Use default chain ID for backward compatibility
        Self::with_chain_id(authority_id, authorities, "kanari-default".to_string())
    }

    fn with_chain_id_internal(
        authority_id: AuthorityId,
        authorities: Vec<AuthorityId>,
        chain_id: String,
        committee_public_keys: BTreeMap<AuthorityId, Vec<u8>>,
        // FIX #1: CRITICAL SECURITY - Remove vrf_secrets parameter
        // Each node should only know its OWN VRF secret, not all authorities' secrets
        // This prevents any admin/config leak from compromising all validators
        local_vrf_secret: Option<[u8; 32]>,
        local_signing_key: ed25519_dalek::SigningKey,
    ) -> Self {
        tracing::info!(
            "[DAG Consensus] Initializing with authority_id: {}, chain_id: {}, committee: {:?}",
            authority_id,
            chain_id,
            authorities
        );
        let mut store = DagStore::new(authorities.clone());

        // Initialize VRF-based leader election
        let mut vrf_election = VrfLeaderElection::new();

        // FIX #1: Only register this node's own VRF secret
        if let Some(secret) = local_vrf_secret {
            vrf_election.register_authority_bytes(authority_id.clone(), &secret);
            tracing::info!("[VRF] Registered local VRF secret for {}", authority_id);
        } else {
            tracing::warn!("[VRF] No VRF secret provided - will use fallback round-robin");
        }

        // Fallback: Create simple round-robin leader schedule
        let mut leader_schedule = BTreeMap::new();
        for round in 0..1000 {
            let leader_idx = (round as usize) % authorities.len();
            leader_schedule.insert(round, authorities[leader_idx].clone());
        }

        // Create genesis vertices (round 0) for all authorities
        let genesis_state_root = smt::default_hashes()[0].to_vec();
        let total_auths = authorities.len();

        // FIX: Use current system time for genesis to avoid timestamp validation issues
        let genesis_timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        for authority in &authorities {
            let genesis_vertex = DagVertex::new(
                0,
                authority.clone(),
                chain_id.clone(),
                vec![],
                vec![],
                genesis_state_root.clone(),
                genesis_timestamp,
            );
            let _ = store.add_vertex(genesis_vertex, total_auths);
        }

        // Initialize Byzantine detector
        let mut byzantine_detector = ByzantineDetector::new();
        for authority in &authorities {
            byzantine_detector.init_authority(authority.clone());
        }

        let caches = DagCaches::extreme_throughput();

        let validator_infos: Vec<ValidatorInfo> = authorities
            .iter()
            .enumerate()
            .map(|(i, auth)| ValidatorInfo {
                authority_id: auth.clone(),
                public_key: committee_public_keys
                    .get(auth)
                    .cloned()
                    .unwrap_or_else(|| Self::authority_public_key(auth).to_vec()),
                network_address: format!("validator-{}", i),
                active: true,
            })
            .collect();
        let committee = Committee::new(0, validator_infos);

        let metrics = DagMetrics::new();
        let state_sync = StateSynchronizer::new();

        use super::vertex_broadcast::AdaptiveBatchConfig;
        let broadcaster = VertexBroadcaster::with_adaptive_config(
            10000,
            std::time::Duration::from_millis(50),
            AdaptiveBatchConfig::extreme_throughput(),
        );

        let persistent_store: Option<PersistentDagStore> = None;
        let pruner = DagPruner::new(PruningConfig::default()).unwrap_or_else(|e| {
            panic!(
                "Failed to create pruner with default config: {}. This is a programming error.",
                e
            )
        });
        let parallel_validator = ParallelValidator::new(ParallelValidatorConfig::high_throughput())
            .unwrap_or_else(|e| {
                panic!(
                    "Failed to create parallel validator: {}. This is a programming error.",
                    e
                )
            });

        let disk_writer_tx = if persistent_store.is_some() {
            // Use async channel instead of sync mpsc (better performance, no thread overhead)
            let (tx, mut rx) = mpsc::channel::<Arc<DagVertex>>(100_000);
            let persistent_clone = persistent_store.clone();

            // Spawn async task instead of OS thread (lighter weight, better scheduling)
            tokio::spawn(async move {
                while let Some(vertex) = rx.recv().await {
                    if let Some(ref store) = persistent_clone
                        && let Err(e) = store.put_vertex(&vertex)
                    {
                        tracing::error!(
                            "Failed to persist vertex {}: {}",
                            hex::encode(vertex.id),
                            e
                        );
                    }
                }
            });

            Some(tx)
        } else {
            None
        };

        Self {
            store,
            authority_id,
            chain_id,
            vrf_election,
            byzantine_detector,
            caches,
            committee,
            metrics,
            state_sync,
            broadcaster,
            persistent_store,
            disk_writer_tx,
            pruner,
            parallel_validator,
            local_signing_key,
        }
    }

    /// Create new DagConsensus with explicit chain ID for replay protection
    pub fn with_chain_id(
        authority_id: AuthorityId,
        authorities: Vec<AuthorityId>,
        chain_id: String,
    ) -> Self {
        tracing::warn!(
            "[DAG Consensus] with_chain_id() uses deterministic demo keys. \
             Use with_chain_id_secure() for production-safe key management."
        );
        let committee_public_keys: BTreeMap<AuthorityId, Vec<u8>> = authorities
            .iter()
            .map(|auth| (auth.clone(), Self::authority_public_key(auth).to_vec()))
            .collect();

        // FIX #1: Only provide local VRF secret, not all secrets
        let local_vrf_secret = Some(Self::authority_seed(&authority_id));

        let local_signing_key =
            ed25519_dalek::SigningKey::from_bytes(&Self::authority_seed(&authority_id));
        Self::with_chain_id_internal(
            authority_id,
            authorities,
            chain_id,
            committee_public_keys,
            local_vrf_secret, // FIX #1: Pass only local secret
            local_signing_key,
        )
    }

    /// Create a production-safe consensus instance with explicit cryptographic material.
    ///
    /// - `authority_public_keys` must contain every authority from `authorities`
    /// - `local_signing_key` must match `authority_id` public key in `authority_public_keys`
    /// - `local_vrf_secret` should be THIS NODE'S OWN VRF secret ONLY (never other nodes' secrets)
    pub fn with_chain_id_secure(
        authority_id: AuthorityId,
        authorities: Vec<AuthorityId>,
        chain_id: String,
        local_signing_key: ed25519_dalek::SigningKey,
        authority_public_keys: BTreeMap<AuthorityId, Vec<u8>>,
        // FIX #1: Changed from vrf_secrets (all nodes) to local_vrf_secret (this node only)
        local_vrf_secret: Option<[u8; 32]>,
    ) -> Result<Self> {
        for auth in &authorities {
            let key = authority_public_keys
                .get(auth)
                .ok_or_else(|| anyhow::anyhow!("Missing public key for authority {}", auth))?;
            let key_bytes: [u8; 32] = key
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid public key length for {}", auth))?;
            ed25519_dalek::VerifyingKey::from_bytes(&key_bytes)
                .map_err(|e| anyhow::anyhow!("Invalid public key for {}: {}", auth, e))?;
        }
        let local_pk = local_signing_key.verifying_key().to_bytes().to_vec();
        let expected_local = authority_public_keys.get(&authority_id).ok_or_else(|| {
            anyhow::anyhow!("Missing local authority public key {}", authority_id)
        })?;
        if *expected_local != local_pk {
            anyhow::bail!("Local signing key does not match authority public key");
        }

        Ok(Self::with_chain_id_internal(
            authority_id,
            authorities,
            chain_id,
            authority_public_keys,
            local_vrf_secret, // FIX #1: Only this node's VRF secret
            local_signing_key,
        ))
    }

    /// Create a new vertex for current round
    pub fn create_vertex(
        &mut self,
        transactions: Vec<SignedTransaction>,
        state_root: Vec<u8>,
        timestamp: u64,
    ) -> Result<DagVertex> {
        let current_round = self.store.current_round();
        let next_round = current_round + 1;

        // Get parent vertices from current round
        let mut parents = self.store.get_vertex_ids_in_round(current_round);

        let mut unique_authors = HashSet::new();
        for parent_id in &parents {
            if let Some(parent_vertex) = self.store.get_vertex(parent_id) {
                unique_authors.insert(parent_vertex.author.clone());
            }
        }

        let total_authorities = self.committee.validators.len();
        let quorum_size = calculate_quorum(total_authorities);

        if unique_authors.len() < quorum_size {
            anyhow::bail!(
                "Cannot create vertex for round {}: Not enough parents for quorum. Have {}, need {}",
                next_round,
                unique_authors.len(),
                quorum_size
            );
        }

        parents.sort(); // Ensure deterministic parent order for state root consistency

        let mut vertex = DagVertex::new(
            next_round,
            self.authority_id.clone(),
            self.chain_id.clone(), // FIX #3: Use consensus chain_id for replay protection
            parents,
            transactions,
            state_root,
            timestamp,
        );
        Self::sign_vertex_with_key(&self.local_signing_key, &mut vertex)?;
        Ok(vertex)
    }

    /// Add vertex to the DAG
    pub fn add_vertex(&mut self, vertex: DagVertex) -> Result<()> {
        // Cross-Chain Replay Attack
        if vertex.chain_id != self.chain_id {
            anyhow::bail!(
                "Cross-chain replay attack detected! Expected chain_id '{}', got '{}'",
                self.chain_id,
                vertex.chain_id
            );
        }

        let vertex_id = vertex.id;
        let author = vertex.author.clone();

        // 1. Verify author is in current committee
        if !self.committee.contains(&author) {
            tracing::error!(
                "[DAG Consensus] Committee check failed for author: '{}'. Committee members: {:?}",
                author,
                self.committee.validators.keys().collect::<Vec<_>>()
            );
            anyhow::bail!("Vertex author '{}' is not in current committee", author);
        }

        // FIX #3: CRITICAL - Reject vertices from banned/untrusted authorities
        // Previously allowed Byzantine nodes (reputation = 0) to keep producing blocks
        if !self.store.is_authority_trusted(&author) {
            tracing::warn!(
                "[Security] REJECTED vertex from BANNED authority: {}",
                author
            );
            return Err(anyhow::anyhow!(
                "Vertex from banned authority '{}' rejected",
                author
            ));
        }

        // 1.5. Fast-path validation + signature verification
        self.verify_vertex_signature(&vertex)?;

        // 2. Fast parent existence check using cache (500K TPS optimization)
        for parent_id in &vertex.parents {
            if self.caches.vertices.get(parent_id).is_none()
                && !self.store.vertices.contains_key(parent_id)
            {
                anyhow::bail!("Parent vertex {} not found", hex::encode(parent_id));
            }
        }

        let total_authorities = self.committee.validators.len();

        // 3. Check for Byzantine faults before adding
        // FIX #2 & #3: Automatically ban authorities that are slashed to reputation 0
        if let Err(e) = self.byzantine_detector.check_double_voting(&vertex) {
            // Double voting detected - check if authority should be banned
            if self.byzantine_detector.get_reputation(&author) == 0 {
                tracing::error!(
                    "[Security] Authority {} SLASHED to 0 reputation - BANNING from consensus",
                    author
                );
                self.store.ban_authority(&author);
            }
            return Err(e);
        }

        if let Err(e) = self
            .byzantine_detector
            .check_vertex_validity(&vertex, total_authorities)
        {
            // Invalid vertex detected - check if authority should be banned
            if self.byzantine_detector.get_reputation(&author) == 0 {
                tracing::error!(
                    "[Security] Authority {} SLASHED to 0 reputation for invalid vertex - BANNING",
                    author
                );
                self.store.ban_authority(&author);
            }
            return Err(e);
        }

        let vertex_arc = Arc::new(vertex);

        // FIX #6: CRITICAL - Send to disk queue BEFORE adding to memory store
        // This prevents state inconsistency where vertex exists in RAM but not on disk
        if let Some(ref tx) = self.disk_writer_tx {
            match tx.try_send(Arc::clone(&vertex_arc)) {
                Ok(()) => {
                    // Successfully queued for disk write, proceed to add to memory
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    // FIX #6: Reject vertex if disk queue is full to prevent data loss
                    self.metrics.inc_disk_queue_full_count();
                    tracing::error!(
                        "[CRITICAL] Disk write queue FULL! Rejecting vertex {} to prevent data loss. \
                         Node must slow down or increase queue capacity.",
                        hex::encode(vertex_id)
                    );
                    return Err(anyhow::anyhow!(
                        "Disk write queue saturated. Vertex {} rejected to prevent data loss",
                        hex::encode(vertex_id)
                    ));
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    // Disk writer task has crashed - this is a fatal error
                    tracing::error!(
                        "[FATAL] Disk writer task closed! Vertex {} will not be persisted.",
                        hex::encode(vertex_id)
                    );

                    return Err(anyhow::anyhow!(
                        "Disk writer task crashed - system unhealthy"
                    ));
                }
            }
        }

        // 4. Add to store (shared ownership) - ONLY after disk queue succeeds
        self.store
            .add_vertex_arc(Arc::clone(&vertex_arc), total_authorities)?;

        // 5. Update metrics and caches (disk write already queued above)
        self.caches
            .vertices
            .insert(vertex_id, (*vertex_arc).clone());

        // 7. Determine if this is a priority vertex
        let is_priority = self.vrf_election.is_leader(vertex_arc.round, &author);

        // 8. Add to broadcaster and state sync using shared vertex
        self.broadcaster
            .add_vertex_arc(Arc::clone(&vertex_arc), is_priority);
        self.state_sync.add_vertex_arc(vertex_arc);

        // 9. Check if pruning should run
        let current_round = self.store.current_round();
        if self.pruner.should_prune(current_round)
            && let Some(persistent) = &self.persistent_store
        {
            let latest_checkpoint = self.store.latest_checkpoint();
            if let Ok(prune_stats) =
                self.pruner
                    .prune(persistent, current_round, Some(latest_checkpoint.sequence))
            {
                self.parallel_validator
                    .invalidate_pruned_vertices(&prune_stats.pruned_vertex_ids);

                for vertex_id in &prune_stats.pruned_vertex_ids {
                    self.caches.vertices.remove(vertex_id);
                }

                self.byzantine_detector
                    .prune_old_rounds(prune_stats.cutoff_round);

                // FIX #13: Update VRF current round for future-round DoS protection
                self.vrf_election.update_current_round(current_round);
                self.vrf_election.prune_old_rounds(prune_stats.cutoff_round);

                let keep_checkpoints = latest_checkpoint.sequence.saturating_sub(100);
                self.state_sync
                    .prune_old_data(keep_checkpoints, prune_stats.cutoff_round);
            }
        }

        Ok(())
    }

    /// Try to commit vertices to a checkpoint
    /// Uses Bullshark-style leader-based ordering with VRF
    pub fn try_commit(&mut self) -> Result<Option<Checkpoint>> {
        let current_round = self.store.current_round();
        tracing::debug!(
            "[DAG Consensus] try_commit: current_round = {}",
            current_round
        );

        // Need at least 3 rounds to commit (leader round + 2 acknowledgment rounds)
        if current_round < 3 {
            return Ok(None);
        }

        let mut start_round = self.store.last_checkpoint_round() + 1;

        let max_commit_round = current_round.saturating_sub(2);

        // If we're already caught up, nothing to do
        if start_round > max_commit_round {
            return Ok(None);
        }

        tracing::info!(
            "[DAG Consensus] Catching up on missed rounds ({} to {})",
            start_round,
            max_commit_round
        );

        // Try to commit each missed round in order
        while start_round <= max_commit_round {
            let commit_round = start_round;

            // Try VRF-based leader election first
            let leader_id = if let Some(vrf_leader) = self.vrf_election.elect_leader(commit_round) {
                vrf_leader
            } else {
                let authorities: Vec<_> = self.committee.validators.keys().cloned().collect();
                if authorities.is_empty() {
                    tracing::warn!(
                        "[DAG Consensus] Empty committee at round {}, skipping",
                        commit_round
                    );
                    start_round += 1;
                    continue;
                }
                let leader_idx = (commit_round as usize) % authorities.len();
                authorities[leader_idx].clone()
            };

            // Find leader's vertex in commit round
            let leader_vertex = self
                .store
                .get_vertices_in_round(commit_round)
                .into_iter()
                .find(|v| v.author == *leader_id);

            if let Some(leader_vertex) = leader_vertex {
                // FIX #7: CRITICAL - Count unique authors, not vertices (prevent Sybil attack)
                // Previously counted vertex count which allowed one attacker to pump votes
                let next_round_vertices = self.store.get_vertices_in_round(commit_round + 1);

                // Collect unique trusted authors who support this leader
                let trusted_support_count = next_round_vertices
                    .iter()
                    .filter(|v| v.parents.contains(&leader_vertex.id))
                    .map(|v| &v.author)
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .filter(|auth| self.store.is_authority_trusted(auth))
                    .count();

                let total_authorities = self.committee.validators.len();
                let quorum = calculate_quorum(total_authorities);

                if trusted_support_count >= quorum {
                    tracing::info!(
                        "[Consensus] Quorum reached! Support: {} / {} (threshold: {})",
                        trusted_support_count,
                        total_authorities,
                        quorum
                    );

                    // Commit! Collect all uncommitted vertices up to and including leader vertex
                    let vertices_to_commit = self.collect_vertices_to_commit(leader_vertex.id)?;

                    // Order transactions from vertices (with deduplication)
                    let mut seen_tx_hashes = HashSet::new();
                    let mut all_transactions = Vec::new();

                    for vertex_id in &vertices_to_commit {
                        if let Some(vertex) = self.store.get_vertex(vertex_id) {
                            for tx in &vertex.transactions {
                                let tx_hash = tx.hash();

                                if self.store.executed_tx_hashes.contains(&tx_hash) {
                                    continue;
                                }

                                // Only add if not seen before in this batch (dedup)
                                if seen_tx_hashes.insert(tx_hash) {
                                    all_transactions.push(tx.clone());
                                }
                            }
                        }
                    }

                    // Create checkpoint
                    let latest = self.store.latest_checkpoint();

                    let prev_hash = latest.hash()?;
                    let checkpoint = Checkpoint::new(
                        latest.sequence + 1,
                        vertices_to_commit.clone(),
                        all_transactions,
                        vec![0u8; 32],
                        leader_vertex.timestamp,
                        prev_hash,
                    );

                    return Ok(Some(checkpoint));
                }
            }

            start_round += 1;
        }

        Ok(None)
    }

    /// Add a new checkpoint (delegates to store)
    pub fn add_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<()> {
        self.store.add_checkpoint(checkpoint)
    }

    /// Get latest checkpoint (delegates to store)
    pub fn latest_checkpoint(&self) -> Checkpoint {
        self.store.latest_checkpoint()
    }

    /// Internal helper: collect reachable, uncheckpointed vertices in post-order.
    /// This preserves deterministic traversal by sorting roots and parent edges.
    fn collect_vertices_post_order(
        &self,
        mut roots: Vec<VertexId>,
        include_vertex_in_cycle_error: bool,
    ) -> Result<Vec<VertexId>> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut in_progress = HashSet::new();
        let mut stack = Vec::new();

        roots.sort();
        for root in roots {
            stack.push((root, false));
        }

        while let Some((vertex_id, processed)) = stack.pop() {
            if processed {
                if !visited.contains(&vertex_id) {
                    visited.insert(vertex_id);
                    result.push(vertex_id);
                }
                in_progress.remove(&vertex_id);
                continue;
            }

            if visited.contains(&vertex_id) {
                continue;
            }

            if in_progress.contains(&vertex_id) {
                if include_vertex_in_cycle_error {
                    anyhow::bail!("Cycle detected in DAG at vertex {}", hex::encode(vertex_id));
                }
                anyhow::bail!("Cycle detected in DAG");
            }

            if let Some(vertex) = self.store.get_vertex(&vertex_id) {
                if self.store.is_vertex_checkpointed(&vertex_id) {
                    visited.insert(vertex_id);
                    continue;
                }

                in_progress.insert(vertex_id);
                stack.push((vertex_id, true));

                let mut sorted_parents = vertex.parents.clone();
                sorted_parents.sort();
                for parent_id in sorted_parents {
                    if !visited.contains(&parent_id) {
                        stack.push((parent_id, false));
                    }
                }
            }
        }

        Ok(result)
    }

    /// Collect all vertices that would be committed if a vertex with given parents became a leader
    pub fn collect_history_for_parents(&self, parents: &[VertexId]) -> Result<Vec<VertexId>> {
        self.collect_vertices_post_order(parents.to_vec(), false)
    }

    /// Collect all vertices that should be committed (topotracingical sort with cycle detection)
    fn collect_vertices_to_commit(&self, leader_vertex_id: VertexId) -> Result<Vec<VertexId>> {
        self.collect_vertices_post_order(vec![leader_vertex_id], true)
    }

    /// Get current DAG store (read-only)
    pub fn store(&self) -> &DagStore {
        &self.store
    }

    /// Get committee (read-only)
    pub fn committee(&self) -> &Committee {
        &self.committee
    }

    /// Check if vertex exists in DAG
    pub fn has_vertex(&self, vertex_id: &VertexId) -> bool {
        self.store.vertices.contains_key(vertex_id)
    }

    /// Save the essential state of the DAG to a serializable struct
    pub fn save_state(&self) -> Result<PersistentDagState> {
        let vertices = self
            .store
            .vertices
            .values()
            .map(|v| (**v).clone())
            .collect();
        let state = PersistentDagState {
            vertices,
            checkpoints: self.store.checkpoints.iter().cloned().collect(),
            current_round: self.store.current_round,
            last_checkpoint_round: self.store.last_checkpoint_round,
        };
        Ok(state)
    }

    /// Load the state of the DAG from a serializable struct
    pub fn load_state(&mut self, state: PersistentDagState) -> Result<()> {
        let mut new_store = DagStore::with_config(
            self.store.authorities.iter().cloned().collect(),
            self.store.checkpoint_config.clone(),
        );

        new_store.checkpoints = state.checkpoints.into_iter().collect();
        new_store.current_round = state.current_round;
        new_store.last_checkpoint_round = state.last_checkpoint_round;

        for vertex in state.vertices {
            let vertex_id = vertex.id;
            new_store.insert_vertex_arc(vertex_id, Arc::new(vertex));
        }

        // Reconstruct pending_vertices
        let committed_vertices: BTreeSet<VertexId> = new_store
            .checkpoints
            .iter()
            .flat_map(|cp| cp.vertices.iter().cloned())
            .collect();

        new_store.pending_vertices = new_store
            .vertices
            .keys()
            .filter(|&id| !committed_vertices.contains(id))
            .cloned()
            .collect();
        for checkpoint in &new_store.checkpoints {
            for tx in &checkpoint.transactions {
                new_store.executed_tx_hashes.insert(tx.hash());
            }
        }

        self.store = new_store;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use kanari_types::transaction::Transaction;

    use super::*;

    fn round0_vertex(i: u64) -> DagVertex {
        let transaction = Transaction::Transfer {
            from: format!("sender{}", i),
            to: "receiver".to_string(),
            amount: i,
            gas_limit: 1000,
            gas_price: 1,
            sequence_number: i,
        };
        DagVertex::new_for_test(
            0,
            "auth1".to_string(),
            vec![],
            vec![SignedTransaction::new(transaction)],
            vec![i as u8; 32],
            0,
        )
    }

    fn add_round0_vertices(store: &mut DagStore, count: u64) {
        let total_auths = store.num_authorities();
        for i in 0..count {
            store.add_vertex(round0_vertex(i), total_auths).ok();
        }
    }

    #[test]
    fn test_dag_vertex_creation() {
        let parent = [0u8; 32];
        let vertex = DagVertex::new_for_test(
            1,
            "authority1".to_string(),
            vec![parent],
            vec![],
            vec![0u8; 32],
            0,
        );

        assert_eq!(vertex.round, 1);
        assert_eq!(vertex.author, "authority1");
        assert_ne!(vertex.id, [0u8; 32]); // ID should be computed hash
    }

    #[test]
    fn test_dag_store() {
        let authorities = vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ];

        let mut store = DagStore::new(authorities);

        let vertex0 =
            DagVertex::new_for_test(0, "auth1".to_string(), vec![], vec![], vec![0u8; 32], 0);

        store
            .add_vertex(vertex0.clone(), store.num_authorities())
            .unwrap();

        assert_eq!(store.current_round(), 0);
        assert!(store.get_vertex(&vertex0.id).is_some());
    }

    #[test]
    fn test_reject_duplicate_transaction_across_checkpoints() {
        let mut store = DagStore::new(vec!["auth1".to_string()]);
        let tx = SignedTransaction::new(Transaction::Transfer {
            from: "alice".to_string(),
            to: "bob".to_string(),
            amount: 1,
            gas_limit: 1000,
            gas_price: 1,
            sequence_number: 1,
        });
        let vertex = DagVertex::new_for_test(
            0,
            "auth1".to_string(),
            vec![],
            vec![tx.clone()],
            vec![1u8; 32],
            0,
        );
        let vertex_id = vertex.id;
        store.add_vertex(vertex, store.num_authorities()).unwrap();

        let latest = store.latest_checkpoint();
        let prev_hash = latest.hash().expect("Checkpoint hash should succeed");
        store
            .add_checkpoint(Checkpoint::new(
                latest.sequence + 1,
                vec![vertex_id],
                vec![tx.clone()],
                vec![2u8; 32],
                1,
                prev_hash,
            ))
            .unwrap();

        let replay =
            DagVertex::new_for_test(0, "auth1".to_string(), vec![], vec![tx], vec![3u8; 32], 1);
        assert!(store.add_vertex(replay, store.num_authorities()).is_err());
    }

    #[test]
    fn test_reject_future_timestamp_vertex() {
        let authorities = vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ];
        let mut store = DagStore::new(authorities);

        // Add parent vertices with timestamp 100
        let parent1 =
            DagVertex::new_for_test(0, "auth1".to_string(), vec![], vec![], vec![1u8; 32], 100);
        let parent2 =
            DagVertex::new_for_test(0, "auth2".to_string(), vec![], vec![], vec![2u8; 32], 100);

        store.add_vertex(parent1, store.num_authorities()).unwrap();
        store.add_vertex(parent2, store.num_authorities()).unwrap();

        // Try to add child vertex with timestamp way too far in the future
        // Max allowed is median + 300 (MAX_TIMESTAMP_DRIFT_SECS), so 500 should be rejected
        let child_parents = store.get_vertex_ids_in_round(0);
        let future_vertex = DagVertex::new_for_test(
            1,
            "auth1".to_string(),
            child_parents,
            vec![],
            vec![3u8; 32],
            500, // Exceeds median (100) + max drift (300) = 400
        );

        assert!(
            store
                .add_vertex(future_vertex, store.num_authorities())
                .is_err()
        );
    }

    #[test]
    fn test_checkpoint_creation() {
        let checkpoint = Checkpoint::genesis();
        assert_eq!(checkpoint.sequence, 0);
        assert!(checkpoint.transactions.is_empty());
    }

    #[test]
    fn test_node_restart_timestamp_recovery() {
        // Test FIX #5: Node restart scenario where parent timestamps are very old
        // This simulates a node that was offline for a long time and then comes back online

        let authorities = vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ];
        let mut store = DagStore::new(authorities);

        // Get current time
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Simulate very old parents (node was offline) - need 3 for quorum
        let old_timestamp = current_time - 1200; // 20 minutes ago (beyond 15-min threshold)

        // Add parent vertices with very old timestamps from ALL authorities to satisfy quorum
        // Need at least 2f+1 = 3 out of 4 authorities for quorum
        let parent1 = DagVertex::new_for_test(
            0,
            "auth1".to_string(),
            vec![],
            vec![],
            vec![1u8; 32],
            old_timestamp,
        );
        let parent2 = DagVertex::new_for_test(
            0,
            "auth2".to_string(),
            vec![],
            vec![],
            vec![2u8; 32],
            old_timestamp,
        );
        let parent3 = DagVertex::new_for_test(
            0,
            "auth3".to_string(),
            vec![],
            vec![],
            vec![3u8; 32],
            old_timestamp,
        );

        store.add_vertex(parent1, store.num_authorities()).unwrap();
        store.add_vertex(parent2, store.num_authorities()).unwrap();
        store.add_vertex(parent3, store.num_authorities()).unwrap();

        // Now simulate node restart: create a new vertex with current timestamp
        // This should be ACCEPTED because we detect the restart scenario
        let child_parents = store.get_vertex_ids_in_round(0);

        // Use current time as vertex timestamp (simulating fresh vertex after restart)
        let restart_vertex = DagVertex::new_for_test(
            1,
            "auth1".to_string(),
            child_parents.clone(),
            vec![],
            vec![4u8; 32],
            current_time, // Current time after restart
        );

        // Debug: Print validation parameters
        eprintln!("Current time: {}", current_time);
        eprintln!("Old timestamp: {}", old_timestamp);
        eprintln!("Restart vertex timestamp: {}", restart_vertex.timestamp);
        eprintln!("Drift from old: {} seconds", current_time - old_timestamp);
        eprintln!("RESTART_THRESHOLD_SECS: 900 (15 minutes)");

        // This should succeed because of restart detection logic
        // The validation will use current_time + 30 as max_allowed instead of old_median + 300
        let result = store.add_vertex(restart_vertex, store.num_authorities());

        if let Err(ref e) = result {
            eprintln!("❌ Validation failed with error: {}", e);
        }

        assert!(
            result.is_ok(),
            "Node restart vertex should be accepted when parent timestamps are very old"
        );
    }

    #[test]
    fn test_acceptable_timestamp_after_restart() {
        // Test that vertices with reasonable timestamps are accepted after restart

        let authorities = vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ];
        let mut store = DagStore::new(authorities);

        // Get current time
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Simulate very old parents (node was offline) - need 3 for quorum
        let old_timestamp = current_time - 7200; // 2 hours ago

        let parent1 = DagVertex::new_for_test(
            0,
            "auth1".to_string(),
            vec![],
            vec![],
            vec![1u8; 32],
            old_timestamp,
        );
        let parent2 = DagVertex::new_for_test(
            0,
            "auth2".to_string(),
            vec![],
            vec![],
            vec![2u8; 32],
            old_timestamp,
        );
        let parent3 = DagVertex::new_for_test(
            0,
            "auth3".to_string(),
            vec![],
            vec![],
            vec![3u8; 32],
            old_timestamp,
        );

        store.add_vertex(parent1, store.num_authorities()).unwrap();
        store.add_vertex(parent2, store.num_authorities()).unwrap();
        store.add_vertex(parent3, store.num_authorities()).unwrap();

        // Create vertex with timestamp slightly in the future (within ±30s tolerance)
        let child_parents = store.get_vertex_ids_in_round(0);
        let acceptable_vertex = DagVertex::new_for_test(
            1,
            "auth1".to_string(),
            child_parents.clone(),
            vec![],
            vec![4u8; 32],
            current_time + 20, // Within the ±30s tolerance for restart scenario
        );

        // Debug output
        eprintln!("Current time: {}", current_time);
        eprintln!("Vertex timestamp: {}", acceptable_vertex.timestamp);
        eprintln!(
            "Drift from current: {} seconds",
            acceptable_vertex.timestamp as i64 - current_time as i64
        );

        // Should succeed
        let result = store.add_vertex(acceptable_vertex, store.num_authorities());

        if let Err(ref e) = result {
            eprintln!("❌ Validation failed: {}", e);
        }

        assert!(
            result.is_ok(),
            "Vertex within ±30s of current time should be accepted after restart"
        );
    }

    #[test]
    fn test_normal_operation_timestamp_validation() {
        // Test that normal operation still enforces strict timestamp validation
        // (without restart detection trigger)

        let authorities = vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ];
        let mut store = DagStore::new(authorities);

        // Get current time
        let current_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Add parent vertices with recent timestamps (within last minute)
        let recent_timestamp = current_time - 30; // 30 seconds ago

        let parent1 = DagVertex::new_for_test(
            0,
            "auth1".to_string(),
            vec![],
            vec![],
            vec![1u8; 32],
            recent_timestamp,
        );
        let parent2 = DagVertex::new_for_test(
            0,
            "auth2".to_string(),
            vec![],
            vec![],
            vec![2u8; 32],
            recent_timestamp,
        );

        store.add_vertex(parent1, store.num_authorities()).unwrap();
        store.add_vertex(parent2, store.num_authorities()).unwrap();

        // Try to add a vertex with timestamp too far ahead (> 300 seconds from median)
        let child_parents = store.get_vertex_ids_in_round(0);
        let future_vertex = DagVertex::new_for_test(
            1,
            "auth1".to_string(),
            child_parents,
            vec![],
            vec![3u8; 32],
            recent_timestamp + 400, // 400 seconds ahead, exceeds MAX_TIMESTAMP_DRIFT_SECS (300)
        );

        // This should fail because it's within normal operation window (< 1 hour)
        // and exceeds the 300 second drift limit
        assert!(
            store
                .add_vertex(future_vertex, store.num_authorities())
                .is_err(),
            "Normal operation should reject vertices with > 300s timestamp drift"
        );
    }

    #[test]
    fn test_checkpoint_config_default() {
        let config = CheckpointConfig::default();
        // Updated for 500K TPS optimization
        assert_eq!(config.min_rounds, 5);
        assert_eq!(config.max_rounds, 50);
        assert_eq!(config.min_vertices, 1000);
        assert_eq!(config.max_vertices, 50000);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_checkpoint_config_conservative() {
        let config = CheckpointConfig::conservative();
        // Conservative should still have smaller values than default
        assert!(config.min_rounds <= CheckpointConfig::default().min_rounds);
        assert!(config.max_rounds <= CheckpointConfig::default().max_rounds);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_checkpoint_config_aggressive() {
        let config = CheckpointConfig::aggressive();
        assert!(config.min_rounds > CheckpointConfig::default().min_rounds);
        assert!(config.max_rounds > CheckpointConfig::default().max_rounds);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_checkpoint_config_validation() {
        // Invalid: min >= max rounds
        let config = CheckpointConfig {
            min_rounds: 100,
            max_rounds: 50,
            ..Default::default()
        };
        assert!(config.validate().is_err());

        // Invalid: min >= max vertices
        let config = CheckpointConfig {
            min_rounds: 10,
            max_rounds: 100,
            min_vertices: 5000,
            max_vertices: 1000,
        };
        assert!(config.validate().is_err());

        // Invalid: max_rounds = 0
        let config = CheckpointConfig {
            min_vertices: 100,
            max_vertices: 1000,
            max_rounds: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_should_create_checkpoint_min_rounds() {
        let config = CheckpointConfig {
            min_rounds: 10,
            max_rounds: 100,
            min_vertices: 100,
            max_vertices: 1000,
        };

        let store = DagStore::with_config(vec!["auth1".to_string()], config);

        // Too early - should not checkpoint
        assert!(!store.should_create_checkpoint());
    }

    #[test]
    fn test_should_create_checkpoint_max_rounds() {
        let config = CheckpointConfig {
            min_rounds: 5,
            max_rounds: 10,
            min_vertices: 100,
            max_vertices: 1000,
        };

        let mut store = DagStore::with_config(vec!["auth1".to_string()], config);

        // Add enough rounds to trigger max_rounds
        // Note: Using round 0 for all vertices to avoid quorum requirements
        store.current_round = 10; // Simulate 10 rounds passing
        add_round0_vertices(&mut store, 11);

        // Should force checkpoint due to max_rounds
        assert!(store.should_create_checkpoint());
    }

    #[test]
    fn test_should_create_checkpoint_min_vertices() {
        let config = CheckpointConfig {
            min_rounds: 1,
            max_rounds: 100,
            min_vertices: 5,
            max_vertices: 1000,
        };

        let mut store = DagStore::with_config(vec!["auth1".to_string()], config);

        // Add vertices at round 0 to avoid quorum requirements
        store.current_round = 1; // Simulate passing minimum rounds
        add_round0_vertices(&mut store, 10);

        // Should checkpoint due to min_vertices reached
        assert!(store.should_create_checkpoint());
    }

    #[test]
    fn test_should_create_checkpoint_max_vertices() {
        let config = CheckpointConfig {
            min_rounds: 1,
            max_rounds: 100,
            min_vertices: 100,
            max_vertices: 10, // Force at 10 vertices
        };

        let mut store = DagStore::with_config(vec!["auth1".to_string()], config);

        // Add many vertices at round 0
        store.current_round = 1; // Simulate passing minimum rounds
        add_round0_vertices(&mut store, 15);

        // Should force checkpoint due to max_vertices
        assert!(store.should_create_checkpoint());
    }

    #[test]
    fn test_checkpoint_stats() {
        let config = CheckpointConfig::default();
        let mut store = DagStore::with_config(vec!["auth1".to_string()], config);

        // Get initial stats
        let initial_stats = store.get_checkpoint_stats();
        assert_eq!(initial_stats.total_checkpoints, 1); // Genesis
        assert_eq!(initial_stats.pending_vertices, 0);

        // Add some vertices at round 0
        store.current_round = 1; // Simulate passing minimum rounds
        add_round0_vertices(&mut store, 5);

        let stats = store.get_checkpoint_stats();
        assert!(stats.pending_vertices > 0);
        assert_eq!(stats.total_checkpoints, 1); // Still just genesis
        assert!(stats.rounds_since_last > 0);
    }

    #[test]
    fn test_set_checkpoint_config() {
        let mut store = DagStore::new(vec!["auth1".to_string()]);

        let new_config = CheckpointConfig::aggressive();
        assert!(store.set_checkpoint_config(new_config.clone()).is_ok());

        assert_eq!(
            store.get_checkpoint_config().min_rounds,
            new_config.min_rounds
        );

        // Try invalid config
        let invalid_config = CheckpointConfig {
            min_rounds: 100,
            max_rounds: 50,
            min_vertices: 100,
            max_vertices: 1000,
        };
        assert!(store.set_checkpoint_config(invalid_config).is_err());
    }

    #[test]
    fn test_add_vertex_rejects_missing_signature() {
        let authorities = vec![
            "auth1".to_string(),
            "auth2".to_string(),
            "auth3".to_string(),
            "auth4".to_string(),
        ];
        let mut consensus = DagConsensus::new("auth1".to_string(), authorities.clone());

        let vertex = DagVertex::new_for_test(
            1,
            "auth2".to_string(),
            Vec::new(),
            Vec::new(),
            vec![7u8; 32],
            1,
        );
        assert!(consensus.add_vertex(vertex).is_err());
    }

    #[test]
    fn test_with_chain_id_secure_accepts_explicit_keys() {
        let authorities = vec!["auth1".to_string(), "auth2".to_string()];
        let sk1 = ed25519_dalek::SigningKey::from_bytes(&[11u8; 32]);
        let sk2 = ed25519_dalek::SigningKey::from_bytes(&[22u8; 32]);
        let mut public_keys = std::collections::BTreeMap::new();
        public_keys.insert("auth1".to_string(), sk1.verifying_key().to_bytes().to_vec());
        public_keys.insert("auth2".to_string(), sk2.verifying_key().to_bytes().to_vec());

        // FIX #1: Only provide local VRF secret (for auth1), not all secrets
        let local_vrf_secret = Some([1u8; 32]);

        let consensus = DagConsensus::with_chain_id_secure(
            "auth1".to_string(),
            authorities,
            "chain-secure".to_string(),
            sk1,
            public_keys,
            local_vrf_secret, // FIX #1: Pass only local secret
        )
        .unwrap();
        assert!(consensus.committee().contains("auth1"));
        assert!(consensus.committee().contains("auth2"));
    }

    #[test]
    fn test_with_chain_id_secure_rejects_mismatched_local_key() {
        let authorities = vec!["auth1".to_string()];
        let expected = ed25519_dalek::SigningKey::from_bytes(&[33u8; 32]);
        let wrong = ed25519_dalek::SigningKey::from_bytes(&[44u8; 32]);
        let mut public_keys = std::collections::BTreeMap::new();
        public_keys.insert(
            "auth1".to_string(),
            expected.verifying_key().to_bytes().to_vec(),
        );

        // FIX #1: Provide None for local VRF secret (not required for this test)
        let result = DagConsensus::with_chain_id_secure(
            "auth1".to_string(),
            authorities,
            "chain-secure".to_string(),
            wrong,
            public_keys,
            None, // FIX #1: No VRF secret needed for this test
        );
        assert!(result.is_err());
    }
}
