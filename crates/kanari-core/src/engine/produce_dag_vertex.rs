// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! DAG vertex transport type and DAG engine backed by mysticeti.
//!
//! [`DagVertex`] is a kanari-specific transport type that carries kanari
//! transactions over the network. All core DAG logic uses mysticeti types
//! (`mysticeti_dag::Block`, `mysticeti_consensus::protocol::Protocol`, etc.)
//! directly — no custom DAG wrappers.

use anyhow::{Context, Result};
use log::info;
use mysticeti_consensus::{
    committer::Committer as MysticetiCommitter,
    protocol::{
        ConsensusProtocol as MysticetiConsensusProtocol, Protocol as MysticetiRuntimeProtocol,
    },
};
use mysticeti_dag::{
    authority::Authority as MysticetiAuthority,
    block::{
        Block, BlockReference as MysticetiBlockReference, RoundNumber as MysticetiRound,
        transaction::Transaction as MysticetiTransaction,
    },
    committee::{AuthorityInfo as MysticetiAuthorityInfo, Committee as MysticetiCommittee},
    consensus::{CommittedSubDag as MysticetiCommittedSubDag, Linearizer as MysticetiLinearizer},
    context::TokioCtx as MysticetiTokioCtx,
    core::{Core as MysticetiCore, block_handler::RealBlockHandler as MysticetiBlockHandler},
    crypto::{
        AsBytes, CryptoEngine as MysticetiCryptoEngine, PublicKey as MysticetiPublicKey,
        Signer as MysticetiSigner,
    },
    data::Data as MysticetiBlockData,
    metrics::Metrics as MysticetiMetrics,
    storage::Storage as MysticetiStorage,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::consensus::Checkpoint;
use kanari_types::event::Event;
use kanari_types::transaction::SignedTransaction;

use super::*;

// ---------------------------------------------------------------------------
// Kanari-specific DAG transport type
// ---------------------------------------------------------------------------

/// A DAG vertex for network transport.
///
/// This is a kanari-specific type that carries kanari [`SignedTransaction`]s
/// so they can be serialised (JSON) and exchanged over gossip.  The core DAG
/// engine inside `DagEngine` uses `mysticeti_dag::Block` directly.
///
/// Each parent entry is `(author_hex_id, digest)` — the authority ID of the
/// parent vertex author and its 32-byte digest. This allows the receiving node
/// to reconstruct full mysticeti `BlockReference`s when feeding the vertex
/// into mysticeti's DAG storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagVertex {
    pub id: [u8; 32],
    pub round: u64,
    pub author: String,
    pub chain_id: String,
    pub parents: Vec<(String, u64, [u8; 32])>,
    pub transactions: Arc<[SignedTransaction]>,
    pub timestamp: u64,
    pub signature: Vec<u8>,
    /// Canonical bincode encoding of the signed native Mysticeti block.
    #[serde(default)]
    pub native_block: Vec<u8>,
    #[serde(skip)]
    pub cached_serialized_data: Option<Vec<u8>>,
}

impl DagVertex {
    pub fn new<T>(
        round: u64,
        author: String,
        chain_id: String,
        parents: Vec<(String, u64, [u8; 32])>,
        transactions: T,
        timestamp: u64,
    ) -> Self
    where
        T: Into<Arc<[SignedTransaction]>>,
    {
        Self {
            id: [0u8; 32],
            round,
            author,
            chain_id,
            parents,
            transactions: transactions.into(),
            timestamp,
            signature: Vec::new(),
            native_block: Vec::new(),
            cached_serialized_data: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Checkpoint production info (public API)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointProductionInfo {
    pub vertex_id: String,
    pub round: u64,
    pub tx_count: usize,
    pub executed: usize,
    pub failed: usize,
    pub events: Vec<Event>,
    pub checkpoint: Option<CheckpointInfo>,
    pub vertex: Option<DagVertex>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointInfo {
    pub sequence: u64,
    pub vertex_count: usize,
    pub tx_count: usize,
}

// ---------------------------------------------------------------------------
// DAG production policy (public API)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProductionPolicy {
    pub current_round: u64,
    pub parent_round: u64,
    pub target_round: u64,
    pub parent_ids: Vec<[u8; 32]>,
    pub parent_authors: Vec<String>,
    pub missing_parent_authors: Vec<String>,
    pub parent_author_count: usize,
    pub quorum_size: usize,
    pub local_has_vertex_in_current_round: bool,
    pub using_catch_up_round: bool,
}

impl DagProductionPolicy {
    pub fn should_wait_for_current_round_quorum(&self) -> bool {
        self.current_round > 0
            && self.parent_round == self.current_round
            && self.local_has_vertex_in_current_round
            && self.parent_author_count < self.quorum_size
    }
}

// ---------------------------------------------------------------------------
// Mysticeti backend — thin wrapper around mysticeti's `Core`
// ---------------------------------------------------------------------------

struct MysticetiBackend {
    core: MysticetiCore<MysticetiTokioCtx, MysticetiCommitter>,
    transaction_sender: mpsc::Sender<Vec<MysticetiTransaction>>,
    protocol: MysticetiRuntimeProtocol,
    /// Canonical ordered list of kanari authority IDs (hex).
    authorities: Vec<String>,
    committee: Arc<MysticetiCommittee>,
    linearizer: MysticetiLinearizer,
    submitted_tx_hashes: HashSet<Vec<u8>>,
    pending_committed_subdags: Vec<MysticetiCommittedSubDag>,
    last_cleanup: Instant,
}

impl MysticetiBackend {
    fn block_to_vertex(&self, block: &MysticetiBlockData<Block>) -> Result<DagVertex> {
        let author = self
            .authorities
            .get(block.author().index())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Unknown Mysticeti block author"))?;
        let parents = block
            .includes()
            .iter()
            .map(|reference| {
                let parent_author = self
                    .authorities
                    .get(reference.authority.index())
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("Unknown Mysticeti parent author"))?;
                Ok((
                    parent_author,
                    reference.round,
                    mysticeti_reference_to_vertex_id(reference),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let transactions = block
            .transactions()
            .iter()
            .map(|transaction| {
                bcs::from_bytes(transaction.as_bytes())
                    .context("Mysticeti transaction is not a SignedTransaction")
            })
            .collect::<Result<Vec<SignedTransaction>>>()?;
        let mut vertex = DagVertex::new(
            block.round(),
            author,
            "kanari-v2-mysticeti".to_string(),
            parents,
            transactions,
            block.timestamp_ns() / 1_000_000,
        );
        vertex.id = mysticeti_reference_to_vertex_id(block.reference());
        vertex.signature = block.signature().as_ref().to_vec();
        vertex.native_block = block.serialized_bytes().to_vec();
        Ok(vertex)
    }

    fn new(
        local_authority: &str,
        authorities: Vec<String>,
        local_signing_key: &ed25519_dalek::SigningKey,
        authority_public_keys: &BTreeMap<String, Vec<u8>>,
        wal_dir: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        anyhow::ensure!(
            !authorities.is_empty(),
            "Mysticeti committee cannot be empty"
        );
        let local_index = authorities
            .iter()
            .position(|authority| authority == local_authority)
            .ok_or_else(|| {
                anyhow::anyhow!("Local authority {local_authority} is not in committee")
            })?;
        let local_authority = MysticetiAuthority::new(local_index as u64);
        let committee_members = authorities
            .iter()
            .map(|authority_id| {
                let bytes: [u8; 32] = authority_public_keys
                    .get(authority_id)
                    .ok_or_else(|| {
                        anyhow::anyhow!("Missing consensus public key for {authority_id}")
                    })?
                    .as_slice()
                    .try_into()
                    .map_err(|_| {
                        anyhow::anyhow!("Invalid consensus public key length for {authority_id}")
                    })?;
                let public_key = MysticetiPublicKey::from_bytes(bytes).map_err(|e| {
                    anyhow::anyhow!("Invalid consensus public key for {authority_id}: {e}")
                })?;
                Ok(MysticetiAuthorityInfo::new(1, public_key))
            })
            .collect::<Result<Vec<_>>>()?;
        let committee = MysticetiCommittee::new(committee_members);
        let authority_count = committee.len();
        let protocol_config = MysticetiConsensusProtocol::Mysticeti {
            leader_count: NonZeroUsize::new(authority_count.clamp(1, 2))
                .expect("leader count is non-zero"),
        };
        let protocol = protocol_config
            .to_protocol(&committee)
            .map_err(|e| anyhow::anyhow!("Failed to build Mysticeti protocol: {}", e))?;
        let metrics = MysticetiMetrics::new_for_test(committee.len());
        let (storage, recovered) = if let Some(wal_dir) = wal_dir {
            std::fs::create_dir_all(&wal_dir)
                .context("Failed to create Mysticeti WAL directory")?;
            let wal_path = wal_dir.join(format!("mysticeti-{}.wal", local_authority.as_u64()));
            MysticetiStorage::open(
                local_authority,
                wal_path,
                metrics.clone(),
                committee.as_ref(),
            )
            .context("Failed to open persistent Mysticeti WAL")?
        } else {
            MysticetiStorage::ephemeral(local_authority, metrics.clone(), committee.as_ref())
        };
        let (block_handler, transaction_sender) =
            MysticetiBlockHandler::<MysticetiTokioCtx>::new(metrics.clone());
        // Re-derive protocol config for the committer to avoid consuming `protocol`
        // (Protocol does not implement Clone).
        let committer_protocol = protocol_config.to_protocol(&committee).map_err(|e| {
            anyhow::anyhow!("Failed to build Mysticeti protocol for committer: {}", e)
        })?;
        let committer = MysticetiCommitter::new(
            committee.clone(),
            storage.block_reader().clone(),
            committer_protocol,
        );
        let mut core = MysticetiCore::open(
            block_handler,
            local_authority,
            committee.clone(),
            metrics,
            storage,
            recovered,
            false,
            committer,
            MysticetiCryptoEngine::enabled(MysticetiSigner::from_bytes(
                local_signing_key.to_bytes(),
            )),
        );
        let mut linearizer = MysticetiLinearizer::new();
        linearizer.committed = core.take_recovered_committed_blocks();

        Ok(Self {
            core,
            transaction_sender,
            protocol,
            authorities,
            committee,
            linearizer,
            submitted_tx_hashes: HashSet::new(),
            pending_committed_subdags: Vec::new(),
            // The standard Mysticeti NetworkSyncer runs Core::cleanup every
            // ten seconds. This wrapper drives Core directly, so mirror that
            // maintenance task from the first producer tick.
            last_cleanup: Instant::now() - Duration::from_secs(10),
        })
    }

    fn cleanup_if_due(&mut self) {
        const CLEANUP_INTERVAL: Duration = Duration::from_secs(10);
        if self.last_cleanup.elapsed() >= CLEANUP_INTERVAL {
            self.core.cleanup();
            self.last_cleanup = Instant::now();
        }
    }

    fn propose_block(
        &mut self,
        transactions: &[SignedTransaction],
        _timestamp_ms: u64,
    ) -> Result<Option<MysticetiBlockSummary>> {
        self.cleanup_if_due();
        let candidates = transactions
            .iter()
            .map(|tx| (tx, tx.transaction_hash().to_vec()))
            .filter(|(_, hash)| !self.submitted_tx_hashes.contains(hash))
            .collect::<Vec<_>>();
        let new_transactions = candidates
            .iter()
            .map(|(tx, _)| signed_tx_to_mysticeti_transaction(tx))
            .collect::<Result<Vec<_>>>()?;
        if !new_transactions.is_empty() {
            for (_, hash) in &candidates {
                self.submitted_tx_hashes.insert(hash.clone());
            }
            if let Err(error) = self.transaction_sender.try_send(new_transactions) {
                for (_, hash) in &candidates {
                    self.submitted_tx_hashes.remove(hash);
                }
                anyhow::bail!("Failed to submit transactions to Mysticeti Core: {error}");
            }
        }

        self.core.drain_submitted_transactions();
        let Some(block) = self.core.try_new_block() else {
            // `try_send` only hands the transactions to Mysticeti's local queue;
            // `try_new_block` is the first point where they become anchored in a
            // native block. After a restart or threshold-clock stall, Core may
            // decline to produce a block even though Kanari has marked the hashes
            // as submitted. Roll the marks back so the next producer tick can
            // re-offer still-pending mempool transactions instead of stranding
            // them forever.
            for (_, hash) in &candidates {
                self.submitted_tx_hashes.remove(hash);
            }
            return Ok(None);
        };
        let reference = *block.reference();
        let block_transactions = block
            .transactions()
            .iter()
            .map(|transaction| {
                bcs::from_bytes(transaction.as_bytes())
                    .context("Proposed Mysticeti transaction is not a SignedTransaction")
            })
            .collect::<Result<Vec<SignedTransaction>>>()?;
        let parents: Vec<(String, u64, [u8; 32])> = block
            .includes()
            .iter()
            .map(|block_ref| {
                let digest = mysticeti_reference_to_vertex_id(block_ref);
                let author_id = self
                    .authorities
                    .get(block_ref.authority.index())
                    .cloned()
                    .unwrap_or_else(|| format!("auth{}", block_ref.authority.index() + 1));
                (author_id, block_ref.round, digest)
            })
            .collect();
        Ok(Some(MysticetiBlockSummary {
            vertex_id: mysticeti_reference_to_vertex_id(&reference),
            round: block.round(),
            parents,
            timestamp_ms: block.timestamp_ns() / 1_000_000,
            signature: block.signature().as_ref().to_vec(),
            native_block: block.serialized_bytes().to_vec(),
            transactions: block_transactions,
        }))
    }

    /// Add a block received from the network into mysticeti's storage.
    ///
    /// Constructs a mysticeti `Block` from the kanari `DagVertex` and feeds it
    /// into `Core::add_blocks()`. This is critical for the threshold clock to
    /// advance — without blocks from other authorities the clock stays stuck
    /// and the node cannot produce rounds beyond its own first vertex.
    fn add_network_block(&mut self, vertex: &DagVertex) -> Result<()> {
        anyhow::ensure!(
            !vertex.native_block.is_empty(),
            "Missing canonical Mysticeti block"
        );
        let block = MysticetiBlockData::<Block>::from_bytes(minibytes::Bytes::copy_from_slice(
            &vertex.native_block,
        ))
        .context("Invalid canonical Mysticeti block encoding")?;
        block
            .verify(
                &self.committee,
                self.core.quorum_threshold(),
                &self.core.verifier(),
            )
            .map_err(|e| anyhow::anyhow!("Invalid canonical Mysticeti block: {e}"))?;
        let processed = self.core.add_blocks(vec![block]);
        anyhow::ensure!(
            !processed.is_empty(),
            "Missing parent(s), duplicate, or unprocessable Mysticeti block"
        );
        Ok(())
    }

    fn try_committed_batches(&mut self) -> Result<Vec<MysticetiCommittedBatch>> {
        if self.pending_committed_subdags.is_empty() {
            let leaders = self.core.try_commit();
            if leaders.is_empty() {
                return Ok(Vec::new());
            }
            self.pending_committed_subdags = self
                .linearizer
                .handle_commit(self.core.block_reader(), leaders);
            for subdag in &mut self.pending_committed_subdags {
                subdag.blocks.sort_by_key(|block| *block.reference());
            }
        }
        let mut batches = Vec::with_capacity(self.pending_committed_subdags.len());
        for subdag in &self.pending_committed_subdags {
            let timestamp_ms = subdag
                .blocks
                .iter()
                .find(|block| block.reference() == &subdag.anchor)
                .map(|block| block.timestamp_ns() / 1_000_000)
                .unwrap_or_default();
            let mut transactions = Vec::new();
            let mut vertices = Vec::with_capacity(subdag.blocks.len());
            for block in &subdag.blocks {
                if block.round() > 0 {
                    vertices.push(mysticeti_reference_to_vertex_id(block.reference()));
                }
                for transaction in block.transactions() {
                    let signed: SignedTransaction = bcs::from_bytes(transaction.as_bytes())
                        .context("Committed Mysticeti transaction is not a SignedTransaction")?;
                    signed.verified_transaction_hash()?;
                    transactions.push(signed);
                }
            }
            batches.push(MysticetiCommittedBatch {
                anchor: mysticeti_reference_to_vertex_id(&subdag.anchor),
                vertices,
                transactions,
                timestamp_ms,
            });
        }
        Ok(batches)
    }

    fn ack_committed_batches(&mut self) {
        let committed = std::mem::take(&mut self.pending_committed_subdags);
        if !committed.is_empty() {
            for block in committed.iter().flat_map(|subdag| &subdag.blocks) {
                for transaction in block.transactions() {
                    if let Ok(signed) = bcs::from_bytes::<SignedTransaction>(transaction.as_bytes())
                    {
                        self.submitted_tx_hashes.remove(signed.transaction_hash());
                    }
                }
            }
            self.core.handle_committed_subdag(committed);
        }
    }
}

struct MysticetiCommittedBatch {
    anchor: [u8; 32],
    vertices: Vec<[u8; 32]>,
    transactions: Vec<SignedTransaction>,
    timestamp_ms: u64,
}

struct MysticetiBlockSummary {
    vertex_id: [u8; 32],
    round: MysticetiRound,
    parents: Vec<(String, u64, [u8; 32])>,
    timestamp_ms: u64,
    signature: Vec<u8>,
    native_block: Vec<u8>,
    transactions: Vec<SignedTransaction>,
}

fn signed_tx_to_mysticeti_transaction(tx: &SignedTransaction) -> Result<MysticetiTransaction> {
    let bytes = bcs::to_bytes(tx).context("Failed to encode transaction for Mysticeti")?;
    Ok(MysticetiTransaction::new(
        minibytes::Bytes::copy_from_slice(&bytes),
    ))
}

pub fn mysticeti_reference_to_vertex_id(reference: &MysticetiBlockReference) -> [u8; 32] {
    let mut id = [0u8; 32];
    id.copy_from_slice(reference.digest.as_ref());
    id
}

// ---------------------------------------------------------------------------
// DagEngine — public API for DAG operations
// ---------------------------------------------------------------------------

/// Mutable state behind `DagEngine`'s read-write lock.
struct DagEngineState {
    mysticeti: MysticetiBackend,
}

#[derive(Clone)]
pub struct DagEngine {
    engine: Arc<BlockchainEngine>,
    /// Consensus state (mysticeti backend + vertex cache) behind a
    /// read-write lock for thread-safe access.
    state: Arc<RwLock<DagEngineState>>,
    /// Serializes application checkpoint finalization across the producer and
    /// asynchronous network-ingest paths.
    finalization_lock: Arc<Mutex<()>>,
    /// Local authority ID (hex string).
    authority_id: String,
    /// All authorities in the committee (hex strings).
    authorities: Vec<String>,
}

impl DagEngine {
    pub fn new_secure(
        engine: Arc<BlockchainEngine>,
        authority_id: String,
        authorities: Vec<String>,
        local_signing_key: ed25519_dalek::SigningKey,
        authority_public_keys: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self> {
        anyhow::ensure!(
            !authorities.is_empty(),
            "Consensus authority set cannot be empty"
        );
        let unique_authorities = authorities.iter().collect::<BTreeSet<_>>();
        anyhow::ensure!(
            unique_authorities.len() == authorities.len(),
            "Consensus authority set contains duplicates"
        );
        let local_public_key = local_signing_key.verifying_key().to_bytes().to_vec();
        let expected_public_key = authority_public_keys
            .get(&authority_id)
            .ok_or_else(|| anyhow::anyhow!("Missing consensus public key for {}", authority_id))?;
        if *expected_public_key != local_public_key {
            anyhow::bail!("Consensus signing key does not match local authority public key");
        }
        Self::from_parts(
            engine,
            authority_id,
            authorities,
            local_signing_key,
            authority_public_keys,
        )
    }

    fn from_parts(
        engine: Arc<BlockchainEngine>,
        authority_id: String,
        authorities: Vec<String>,
        local_signing_key: ed25519_dalek::SigningKey,
        authority_public_keys: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self> {
        let mysticeti = MysticetiBackend::new(
            &authority_id,
            authorities.clone(),
            &local_signing_key,
            &authority_public_keys,
            engine
                .persistent_store
                .as_ref()
                .and_then(|_| std::env::var_os("KANARI_DAG_WAL_DIR"))
                .map(std::path::PathBuf::from),
        )?;
        let dag_engine = Self {
            engine,
            state: Arc::new(RwLock::new(DagEngineState { mysticeti })),
            finalization_lock: Arc::new(Mutex::new(())),
            authority_id,
            authorities,
        };
        Ok(dag_engine)
    }

    /// Query the DAG production policy from mysticeti's authoritative storage.
    pub fn production_policy(&self) -> DagProductionPolicy {
        let state = lock_read(&self.state);
        let block_reader = state.mysticeti.core.block_reader();
        let current_round = block_reader.highest_round();
        let parent_round = current_round;
        let target_round = current_round.saturating_add(1);

        let mysticeti_blocks = block_reader.get_blocks_by_round(parent_round);

        let parent_ids: Vec<[u8; 32]> = mysticeti_blocks
            .iter()
            .map(|block| {
                let mut id = [0u8; 32];
                id.copy_from_slice(block.reference().digest.as_ref());
                id
            })
            .collect();

        let parent_authors: Vec<String> = mysticeti_blocks
            .iter()
            .map(|block| self.authority_id(block.author()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        let missing_parent_authors = self
            .authorities
            .iter()
            .filter(|authority| !parent_authors.contains(authority))
            .cloned()
            .collect();

        let local_has_vertex_in_current_round = mysticeti_blocks
            .iter()
            .any(|block| self.authority_id(block.author()) == self.authority_id);

        DagProductionPolicy {
            current_round,
            parent_round,
            target_round,
            parent_ids,
            parent_authors: parent_authors.clone(),
            missing_parent_authors,
            parent_author_count: parent_authors.len(),
            quorum_size: state.mysticeti.protocol.direct_commit_quorum as usize,
            local_has_vertex_in_current_round,
            using_catch_up_round: false,
        }
    }

    /// Map a mysticeti `Authority` to the kanari authority ID string.
    fn authority_id(&self, authority: MysticetiAuthority) -> String {
        let idx = authority.index();
        self.authorities
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("auth{}", idx + 1))
    }

    /// Export metrics as a Prometheus string via mysticeti.
    pub fn metrics_prometheus(&self) -> String {
        let state = lock_read(&self.state);
        let block_reader = state.mysticeti.core.block_reader();
        let round = block_reader.highest_round();
        let chain = lock_read(&self.engine.blockchain);
        let checkpoint_count = chain.height();
        format!(
            "# HELP dag_vertices_created_total Total DAG vertices created\n\
             # TYPE dag_vertices_created_total counter\n\
             dag_vertices_created_total {}\n\
             # HELP dag_checkpoints_created_total Total DAG checkpoints created\n\
             # TYPE dag_checkpoints_created_total counter\n\
             dag_checkpoints_created_total {}\n",
            round, checkpoint_count,
        )
    }

    pub fn produce_vertex(&self) -> Result<CheckpointProductionInfo> {
        let transactions = self.engine.pending_conflict_free_transactions_snapshot();

        // A network block can make a sub-DAG committable without advancing the
        // threshold clock far enough to produce our next block. Drain those
        // commits first; otherwise returning DAG_WAITING below can strand a
        // quorum-certified transaction indefinitely.
        let mut checkpoint_info = self.finalize_ready_commits()?;
        let timestamp = {
            let chain = lock_read(&self.engine.blockchain);
            chain
                .latest_checkpoint()
                .timestamp
                .saturating_add(1)
                .max(chain.height().saturating_add(1))
        };

        let (executed, failed) = (0, 0);
        let block = {
            let mut state = lock_write(&self.state);
            state.mysticeti.propose_block(&transactions, timestamp)?
        };
        let Some(block) = block else {
            if checkpoint_info.is_some() {
                let policy = self.production_policy();
                return Ok(CheckpointProductionInfo {
                    vertex_id: String::new(),
                    round: policy.target_round,
                    tx_count: 0,
                    executed,
                    failed,
                    events: Vec::new(),
                    checkpoint: checkpoint_info,
                    vertex: None,
                });
            }
            anyhow::bail!("DAG_WAITING: Mysticeti threshold clock has not advanced");
        };

        let mut vertex = DagVertex::new(
            block.round,
            self.authority_id.clone(),
            "kanari-v2-mysticeti".to_string(),
            block.parents,
            block.transactions,
            block.timestamp_ms,
        );
        vertex.id = block.vertex_id;
        vertex.signature = block.signature;
        vertex.native_block = block.native_block;
        let tx_count = vertex.transactions.len();

        if let Some(committed) = self.finalize_ready_commits()? {
            checkpoint_info = Some(committed);
        }

        let vertex_id_hex = hex::encode(vertex.id);
        if tx_count == 0 {
            log::debug!(
                "[DAG v2] Produced idle Mysticeti vertex {} round {}",
                vertex_id_hex,
                vertex.round
            );
        } else {
            info!(
                "[DAG v2] Produced Mysticeti-backed vertex {} round {} txs {}",
                vertex_id_hex, vertex.round, tx_count
            );
        }

        Ok(CheckpointProductionInfo {
            vertex_id: vertex_id_hex,
            round: vertex.round,
            tx_count,
            executed,
            failed,
            events: Vec::new(),
            checkpoint: checkpoint_info,
            vertex: Some(vertex),
        })
    }

    fn finalize_ready_commits(&self) -> Result<Option<CheckpointInfo>> {
        let _finalization_guard = self
            .finalization_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let committed_batches = {
            let mut state = lock_write(&self.state);
            state.mysticeti.try_committed_batches()?
        };
        if committed_batches.is_empty() {
            return Ok(None);
        }

        let mut checkpoint_info = None;
        for batch in committed_batches {
            if let Some(checkpoint) = self.finalize_committed_batch(batch)? {
                checkpoint_info = Some(CheckpointInfo {
                    sequence: checkpoint.sequence,
                    vertex_count: checkpoint.vertices.len(),
                    tx_count: checkpoint.transactions.len(),
                });
            }
        }
        lock_write(&self.state).mysticeti.ack_committed_batches();
        Ok(checkpoint_info)
    }

    fn finalize_committed_batch(
        &self,
        batch: MysticetiCommittedBatch,
    ) -> Result<Option<Checkpoint>> {
        if let Some(existing) = lock_read(&self.engine.blockchain)
            .dag_checkpoints
            .iter()
            .find(|checkpoint| checkpoint.vertices == batch.vertices)
            .cloned()
        {
            return Ok(Some(existing));
        }
        let mut seen = HashSet::new();
        let transactions = batch
            .transactions
            .into_iter()
            .filter(|tx| {
                let hash = tx.transaction_hash().to_vec();
                seen.insert(hash.clone()) && !self.engine.is_transaction_committed(&hash)
            })
            .collect::<Vec<_>>();
        if transactions.is_empty() {
            info!(
                "[DAG v2] Committed empty sub-DAG anchor {} without creating a blockchain checkpoint",
                hex::encode(batch.anchor)
            );
            return Ok(None);
        }
        let (sequence, prev_hash, timestamp_ms) = {
            let chain = lock_read(&self.engine.blockchain);
            (
                chain.latest_checkpoint().sequence.saturating_add(1),
                chain.latest_checkpoint().hash()?,
                batch
                    .timestamp_ms
                    .max(chain.latest_checkpoint().timestamp.saturating_add(1)),
            )
        };
        let mut checkpoint = Checkpoint::new(
            sequence,
            batch.vertices,
            transactions,
            Vec::new(),
            timestamp_ms,
            prev_hash,
        );
        let prepared = self.engine.prepare_checkpoint_state(&checkpoint)?;
        checkpoint.state_root = prepared.state_root;
        checkpoint.transaction_effects = prepared.effects.into();
        checkpoint.object_changes =
            BlockchainEngine::aggregate_checkpoint_object_changes(&checkpoint.transaction_effects)
                .into();
        checkpoint.object_graph_edges = BlockchainEngine::aggregate_checkpoint_object_graph_edges(
            &checkpoint.transaction_effects,
        )
        .into();
        self.engine.apply_prepared_checkpoint(
            checkpoint.clone(),
            prepared.state,
            prepared.transactions,
            true,
        )?;
        info!(
            "[DAG v2] Finalized committed sub-DAG anchor {} as checkpoint {}",
            hex::encode(batch.anchor),
            checkpoint.sequence
        );
        Ok(Some(checkpoint))
    }

    pub fn latest_own_vertices(&self, limit: usize) -> Result<Vec<DagVertex>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let state = lock_read(&self.state);
        let block_reader = state.mysticeti.core.block_reader();
        let blocks = block_reader.get_latest_own_blocks(limit);
        blocks
            .iter()
            .map(|block| {
                state
                    .mysticeti
                    .block_to_vertex(block)
                    .context("Failed to convert local Mysticeti block for DAG gossip")
            })
            .collect()
    }

    pub fn vertices_for_sync(&self, limit: usize) -> Result<Vec<DagVertex>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let state = lock_read(&self.state);
        let reader = state.mysticeti.core.block_reader();
        let highest_round = reader.highest_round();
        let mut vertices = Vec::new();
        for round in 1..=highest_round {
            for block in reader.get_blocks_by_round(round) {
                vertices.push(state.mysticeti.block_to_vertex(&block).with_context(|| {
                    format!("Failed to convert Mysticeti block from round {round} for DAG sync")
                })?);
            }
        }
        vertices.sort_by(|left, right| {
            left.round
                .cmp(&right.round)
                .then_with(|| left.author.cmp(&right.author))
                .then_with(|| left.id.cmp(&right.id))
        });
        if vertices.len() > limit {
            vertices.drain(..vertices.len() - limit);
        }
        Ok(vertices)
    }

    /// Return the exact checkpoint vertices plus every available non-genesis
    /// parent required to inject them into Mysticeti. Roots are resolved from
    /// the retained Mysticeti block store; parents are then resolved by their
    /// explicit round/id references rather than silently omitted.
    pub fn checkpoint_vertices_for_sync(
        &self,
        checkpoint_vertices: &[[u8; 32]],
        limit: usize,
    ) -> Result<Vec<DagVertex>> {
        if checkpoint_vertices.is_empty() {
            return Ok(Vec::new());
        }
        anyhow::ensure!(limit > 0, "Checkpoint DAG sync limit must be non-zero");

        let state = lock_read(&self.state);
        let reader = state.mysticeti.core.block_reader();
        let highest_round = reader.highest_round();

        let mut recent = BTreeMap::new();
        for round in 1..=highest_round {
            for block in reader.get_blocks_by_round(round) {
                let vertex = state.mysticeti.block_to_vertex(&block)?;
                recent.insert(vertex.id, vertex);
            }
        }

        // Traverse breadth-first so every checkpoint root is preferred over old
        // ancestry when the bounded transport budget is exhausted. Missing old
        // parents do not weaken checkpoint validation: receivers buffer these
        // vertices until their own Mysticeti store has the parent, and a remote
        // checkpoint is never applied before the local DAG commits it.
        let mut pending = VecDeque::with_capacity(checkpoint_vertices.len());
        for vertex_id in checkpoint_vertices {
            let vertex = recent.get(vertex_id).cloned().ok_or_else(|| {
                anyhow::anyhow!(
                    "Checkpoint DAG vertex {} is missing from retained Mysticeti block store (highest_round={}); use snapshot recovery",
                    hex::encode(vertex_id),
                    highest_round
                )
            })?;
            pending.push_back(vertex);
        }

        let mut closure = BTreeMap::new();
        let mut truncated = false;
        while let Some(vertex) = pending.pop_front() {
            if closure.contains_key(&vertex.id) {
                continue;
            }
            if closure.len() >= limit {
                truncated = true;
                break;
            }
            for (parent_author, parent_round, parent_id) in &vertex.parents {
                // Mysticeti genesis references are implicit and are never sent as
                // network blocks.
                if *parent_round == 0 || closure.contains_key(parent_id) {
                    continue;
                }
                let parent = reader
                    .get_blocks_by_round(*parent_round)
                    .into_iter()
                    .find(|block| mysticeti_reference_to_vertex_id(block.reference()) == *parent_id)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Missing DAG parent {} for checkpoint vertex {}",
                            hex::encode(parent_id),
                            hex::encode(vertex.id)
                        )
                    })?;
                let parent_vertex = state.mysticeti.block_to_vertex(&parent)?;
                anyhow::ensure!(
                    &parent_vertex.author == parent_author,
                    "DAG parent author mismatch for {}",
                    hex::encode(parent_id)
                );
                pending.push_back(parent_vertex);
            }
            closure.insert(vertex.id, vertex);
        }
        if truncated {
            log::warn!(
                "Checkpoint DAG parent closure exceeded sync limit {}; sending the newest bounded evidence and relying on the receiver's verified Mysticeti parent store",
                limit
            );
        }

        let mut vertices = closure.into_values().collect::<Vec<_>>();
        vertices.sort_by(|left, right| {
            left.round
                .cmp(&right.round)
                .then_with(|| left.author.cmp(&right.author))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(vertices)
    }

    /// Validate a network `DagVertex` against both kanari rules and mysticeti storage.
    ///
    /// Parent existence is checked against mysticeti's authoritative `block_reader()`
    /// rather than the local kanari vertex cache, ensuring DAG integrity is
    /// verified against the actual consensus state.
    fn validate_network_vertex(&self, state: &DagEngineState, vertex: &DagVertex) -> Result<()> {
        const DAG_CHAIN_ID: &str = "kanari-v2-mysticeti";

        if vertex.chain_id != DAG_CHAIN_ID {
            anyhow::bail!("Invalid DAG vertex chain id: {}", vertex.chain_id);
        }
        if vertex.round == 0 {
            anyhow::bail!("Invalid DAG vertex round 0");
        }
        if !self.authorities.contains(&vertex.author) {
            anyhow::bail!("Unknown DAG vertex author: {}", vertex.author);
        }
        anyhow::ensure!(
            !vertex.native_block.is_empty(),
            "Missing canonical Mysticeti block"
        );
        let block = MysticetiBlockData::<Block>::from_bytes(minibytes::Bytes::copy_from_slice(
            &vertex.native_block,
        ))
        .context("Invalid canonical Mysticeti block encoding")?;
        block
            .verify(
                &state.mysticeti.committee,
                state.mysticeti.core.quorum_threshold(),
                &state.mysticeti.core.verifier(),
            )
            .map_err(|e| anyhow::anyhow!("Invalid canonical Mysticeti block: {e}"))?;

        anyhow::ensure!(
            self.authority_id(block.author()) == vertex.author,
            "Mysticeti block author does not match transport author"
        );
        anyhow::ensure!(
            block.round() == vertex.round,
            "Mysticeti block round mismatch"
        );
        anyhow::ensure!(
            mysticeti_reference_to_vertex_id(block.reference()) == vertex.id,
            "Mysticeti block digest mismatch"
        );
        anyhow::ensure!(
            block.signature().as_ref() == vertex.signature.as_slice(),
            "Mysticeti block signature mismatch"
        );
        anyhow::ensure!(
            block.timestamp_ns() / 1_000_000 == vertex.timestamp,
            "Mysticeti block timestamp mismatch"
        );
        let canonical_parents = block
            .includes()
            .iter()
            .map(|reference| {
                (
                    self.authority_id(reference.authority),
                    reference.round,
                    mysticeti_reference_to_vertex_id(reference),
                )
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(
            canonical_parents == vertex.parents,
            "Mysticeti block parents do not match transport parents"
        );
        anyhow::ensure!(
            block.transactions().len() == vertex.transactions.len(),
            "Mysticeti block transaction count mismatch"
        );
        for (native, transported) in block.transactions().iter().zip(vertex.transactions.iter()) {
            let encoded =
                bcs::to_bytes(transported).context("Failed to encode transported transaction")?;
            anyhow::ensure!(
                native.as_bytes() == encoded.as_slice(),
                "Mysticeti block transaction payload mismatch"
            );
            transported.verified_transaction_hash()?;
        }
        Ok(())
    }

    pub fn add_network_vertex(&self, vertex: DagVertex) -> Result<()> {
        {
            let mut state = lock_write(&self.state);
            let already_in_mysticeti = state
                .mysticeti
                .core
                .block_reader()
                .get_blocks_by_round(vertex.round)
                .iter()
                .any(|block| mysticeti_reference_to_vertex_id(block.reference()) == vertex.id);
            if already_in_mysticeti {
                return Ok(());
            }
            self.validate_network_vertex(&state, &vertex)?;
            state.mysticeti.add_network_block(&vertex).context(
                "Failed to inject network vertex into Mysticeti (missing parents or duplicate)",
            )?;
        }
        info!(
            "[DAG v2 SYNC] Accepted + injected network vertex {} round {} txs {}",
            hex::encode(vertex.id),
            vertex.round,
            vertex.transactions.len()
        );
        self.finalize_ready_commits()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Lock helpers with poison recovery
// ---------------------------------------------------------------------------

fn lock_read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|e| {
        log::error!("RwLock poisoned, recovering");
        e.into_inner()
    })
}

fn lock_write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|e| {
        log::error!("RwLock poisoned, recovering");
        e.into_inner()
    })
}

#[cfg(test)]
#[path = "../../tests/unit/produce_dag_vertex_tests.rs"]
mod tests;
