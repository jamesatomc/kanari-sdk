// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use log::{info, warn};
use mysticeti_consensus::{
    committer::Committer as MysticetiCommitter,
    protocol::{
        ConsensusProtocol as MysticetiConsensusProtocol, Protocol as MysticetiRuntimeProtocol,
    },
};
use mysticeti_dag::{
    authority::Authority as MysticetiAuthority,
    block::{
        Block as MysticetiBlock, BlockReference as MysticetiBlockReference,
        RoundNumber as MysticetiRound, transaction::Transaction as MysticetiTransaction,
    },
    committee::Committee as MysticetiCommittee,
    consensus::Linearizer as MysticetiLinearizer,
    context::TokioCtx as MysticetiTokioCtx,
    core::{Core as MysticetiCore, block_handler::RealBlockHandler as MysticetiBlockHandler},
    crypto::{AsBytes as MysticetiAsBytes, CryptoEngine as MysticetiCryptoEngine},
    data::Data as MysticetiData,
    metrics::Metrics as MysticetiMetrics,
    storage::Storage as MysticetiStorage,
};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::num::NonZeroUsize;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;

use crate::consensus::{
    Checkpoint, CheckpointCertificate, CheckpointVote, ConsensusRuntimeProtocol, DagMetrics,
    DagProductionPolicy, DagVertex, PersistentDagState, VertexId,
};

use super::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointProductionInfo {
    pub vertex_id: String,
    pub round: u64,
    pub tx_count: usize,
    pub executed: usize,
    pub failed: usize,
    pub events: Vec<Event>,
    pub checkpoint: Option<CheckpointInfo>,
    pub checkpoint_votes: Vec<CheckpointVote>,
    pub vertex: Option<DagVertex>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointInfo {
    pub sequence: u64,
    pub vertex_count: usize,
    pub tx_count: usize,
}

struct StagedCheckpoint {
    checkpoint: Checkpoint,
    verified_state: StateManager,
    to_execute: Vec<SignedTransaction>,
    receipts: Vec<TransactionExecutionReceipt>,
    validate_supply: bool,
    epoch: u64,
    round: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ConsensusUpdate {
    pub checkpoint_votes: Vec<CheckpointVote>,
    pub finalized_checkpoints: Vec<Checkpoint>,
}

pub struct MysticetiBackend {
    _committee: Arc<MysticetiCommittee>,
    core: MysticetiCore<MysticetiTokioCtx, MysticetiCommitter>,
    transaction_sender: mpsc::Sender<Vec<MysticetiTransaction>>,
    protocol: MysticetiRuntimeProtocol,
    linearizer: MysticetiLinearizer,
}

impl MysticetiBackend {
    fn new(local_authority: usize, authority_count: usize) -> Result<Self> {
        let authority_count = authority_count.max(1);
        anyhow::ensure!(
            local_authority < authority_count,
            "local Mysticeti authority is outside the committee"
        );
        let local_authority = MysticetiAuthority::from(local_authority);
        let committee = MysticetiCommittee::new_test(vec![1; authority_count]);
        let protocol_config = MysticetiConsensusProtocol::Mysticeti {
            leader_count: NonZeroUsize::new(authority_count.clamp(1, 2))
                .expect("leader count is non-zero"),
        };
        let protocol = protocol_config
            .to_protocol(&committee)
            .map_err(|e| anyhow::anyhow!("Failed to build Mysticeti protocol: {}", e))?;
        let metrics = MysticetiMetrics::new_for_test(committee.len());
        let (storage, recovered) =
            MysticetiStorage::ephemeral(local_authority, metrics.clone(), committee.as_ref());
        let (block_handler, transaction_sender) =
            MysticetiBlockHandler::<MysticetiTokioCtx>::new(metrics.clone());
        let committer =
            MysticetiCommitter::new(committee.clone(), storage.block_reader().clone(), protocol);
        let protocol = protocol_config
            .to_protocol(&committee)
            .map_err(|e| anyhow::anyhow!("Failed to rebuild Mysticeti protocol: {}", e))?;
        let core = MysticetiCore::open(
            block_handler,
            local_authority,
            committee.clone(),
            metrics,
            storage,
            recovered,
            false,
            committer,
            MysticetiCryptoEngine::disabled(),
        );

        Ok(Self {
            _committee: committee,
            core,
            transaction_sender,
            protocol,
            linearizer: MysticetiLinearizer::new(),
        })
    }

    fn runtime_protocol(&self) -> ConsensusRuntimeProtocol {
        ConsensusRuntimeProtocol::from_mysticeti(&self.protocol)
    }

    fn quorum_threshold(&self) -> usize {
        self.protocol.direct_commit_quorum as usize
    }

    fn try_advance(&mut self) -> Vec<Vec<VertexId>> {
        let leaders = self.core.try_commit();
        if leaders.is_empty() {
            return Vec::new();
        }
        let subdags = self
            .linearizer
            .handle_commit(self.core.block_reader(), leaders);
        let committed_vertices = subdags
            .iter()
            .map(|subdag| {
                let anchor = mysticeti_reference_to_vertex_id(&subdag.anchor);
                let mut vertices = vec![anchor];
                vertices.extend(
                    subdag
                        .blocks
                        .iter()
                        .filter(|block| block.round() > 0 && block.reference() != &subdag.anchor)
                        .map(|block| mysticeti_reference_to_vertex_id(block.reference())),
                );
                vertices
            })
            .collect::<Vec<_>>();
        self.core.handle_committed_subdag(subdags);
        committed_vertices
    }

    fn import_block(
        &mut self,
        serialized_block: &[u8],
    ) -> Result<(MysticetiData<MysticetiBlock>, Vec<Vec<VertexId>>)> {
        anyhow::ensure!(
            !serialized_block.is_empty(),
            "missing serialized Mysticeti block"
        );
        let block = MysticetiData::<MysticetiBlock>::from_bytes(minibytes::Bytes::from(
            serialized_block.to_vec(),
        ))?;
        block
            .verify(
                &self._committee,
                self.core.quorum_threshold(),
                &self.core.verifier(),
            )
            .map_err(|error| anyhow::anyhow!("Mysticeti block verification failed: {error:?}"))?;
        let mut processed = self.core.add_blocks(vec![block]);
        anyhow::ensure!(
            processed.len() == 1,
            "Missing parent or rejected Mysticeti block"
        );
        let imported = processed.remove(0);
        let committed = self.try_advance();
        Ok((imported, committed))
    }

    fn propose_block(
        &mut self,
        transactions: &[SignedTransaction],
        timestamp_ms: u64,
    ) -> Result<Option<MysticetiBlockSummary>> {
        if !transactions.is_empty() {
            self.transaction_sender
                .try_send(vec![signed_tx_batch_to_mysticeti_transaction(
                    transactions,
                    timestamp_ms,
                )])
                .map_err(|e| {
                    anyhow::anyhow!("Failed to submit transactions to Mysticeti Core: {}", e)
                })?;
        }

        self.core.drain_submitted_transactions();
        let Some(block) = self.core.try_new_block() else {
            return Ok(None);
        };
        let reference = *block.reference();
        let serialized_block = block.serialized_bytes().to_vec();
        let committed_subdags = self.try_advance();
        Ok(Some(MysticetiBlockSummary {
            vertex_id: mysticeti_reference_to_vertex_id(&reference),
            round: block.round(),
            parents: block
                .includes()
                .iter()
                .filter(|reference| reference.round > 0)
                .map(mysticeti_reference_to_vertex_id)
                .collect(),
            serialized_block,
            committed_subdags,
        }))
    }
}

struct MysticetiBlockSummary {
    vertex_id: VertexId,
    round: MysticetiRound,
    parents: Vec<VertexId>,
    serialized_block: Vec<u8>,
    committed_subdags: Vec<Vec<VertexId>>,
}

fn signed_tx_batch_to_mysticeti_transaction(
    transactions: &[SignedTransaction],
    timestamp_ms: u64,
) -> MysticetiTransaction {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kanari:mysticeti-batch:v1");
    hasher.update(&timestamp_ms.to_le_bytes());
    hasher.update(&(transactions.len() as u64).to_le_bytes());
    for tx in transactions {
        hasher.update(tx.transaction_hash());
    }

    MysticetiTransaction::new(minibytes::Bytes::copy_from_slice(
        hasher.finalize().as_bytes(),
    ))
}

fn mysticeti_reference_to_vertex_id(reference: &MysticetiBlockReference) -> VertexId {
    let mut id = [0u8; 32];
    id.copy_from_slice(reference.digest.as_ref());
    id
}

pub struct CoreDagConsensus {
    authority_id: String,
    authorities: Vec<String>,
    vertices: Vec<DagVertex>,
    checkpoints: Vec<Checkpoint>,
    current_round: u64,
    last_checkpoint_round: u64,
    metrics: DagMetrics,
    mysticeti: MysticetiBackend,
}

impl CoreDagConsensus {
    fn new(
        authority_id: String,
        authorities: Vec<String>,
        state: Option<PersistentDagState>,
    ) -> Result<Self> {
        let mut checkpoints = vec![Checkpoint::genesis()];
        let mut vertices = Vec::new();
        let mut current_round = 0;
        let mut last_checkpoint_round = 0;

        if let Some(state) = state {
            if !state.checkpoints.is_empty() {
                checkpoints = state.checkpoints;
            }
            vertices = state.vertices;
            current_round = state.current_round;
            last_checkpoint_round = state.last_checkpoint_round;
        }

        let local_index = authorities
            .iter()
            .position(|authority| authority == &authority_id)
            .ok_or_else(|| anyhow::anyhow!("local authority is missing from the committee"))?;
        let mysticeti = MysticetiBackend::new(local_index, authorities.len())?;

        Ok(Self {
            authority_id,
            authorities,
            vertices,
            checkpoints,
            current_round,
            last_checkpoint_round,
            metrics: DagMetrics::default(),
            mysticeti,
        })
    }

    pub fn production_policy(&self) -> DagProductionPolicy {
        let parent_round = self.current_round;
        let target_round = self.current_round.saturating_add(1);
        let parent_vertices: Vec<&DagVertex> = self
            .vertices
            .iter()
            .filter(|vertex| vertex.round == parent_round)
            .collect();
        let parent_ids = parent_vertices.iter().map(|vertex| vertex.id).collect();
        let parent_authors: Vec<String> = parent_vertices
            .iter()
            .map(|vertex| vertex.author.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let missing_parent_authors = self
            .authorities
            .iter()
            .filter(|authority| !parent_authors.contains(authority))
            .cloned()
            .collect();
        let local_has_vertex_in_current_round = parent_vertices
            .iter()
            .any(|vertex| vertex.author == self.authority_id);

        DagProductionPolicy {
            current_round: self.current_round,
            parent_round,
            target_round,
            parent_ids,
            parent_authors: parent_authors.clone(),
            missing_parent_authors,
            parent_author_count: parent_authors.len(),
            quorum_size: self.mysticeti.quorum_threshold(),
            local_has_vertex_in_current_round,
            using_catch_up_round: false,
        }
    }

    pub fn metrics(&self) -> &DagMetrics {
        &self.metrics
    }

    fn needs_progress(&self) -> bool {
        self.current_round > self.last_checkpoint_round
    }

    pub fn protocol(&self) -> ConsensusRuntimeProtocol {
        self.mysticeti.runtime_protocol()
    }

    fn save_state(&self) -> PersistentDagState {
        PersistentDagState {
            vertices: self.vertices.clone(),
            checkpoints: self.checkpoints.clone(),
            current_round: self.current_round,
            last_checkpoint_round: self.last_checkpoint_round,
        }
    }

    fn latest_vertices_by_authority(&self, authority: &str, limit: usize) -> Vec<DagVertex> {
        self.vertices
            .iter()
            .rev()
            .filter(|vertex| vertex.author == authority)
            .take(limit)
            .cloned()
            .collect()
    }

    fn known_vertex(&self, id: &VertexId) -> bool {
        self.vertices.iter().any(|vertex| &vertex.id == id)
    }

    fn add_vertex(&mut self, vertex: DagVertex) -> Result<bool> {
        if vertex.transactions.len() != vertex.metadata.tx_count {
            anyhow::bail!("Transaction count mismatch");
        }
        const MAX_VERTEX_TRANSACTIONS: usize = 10_000;
        const MAX_VERTEX_BYTES: usize = 128 * 1024;
        anyhow::ensure!(
            vertex.transactions.len() <= MAX_VERTEX_TRANSACTIONS,
            "DAG vertex transaction limit exceeded"
        );
        let mut vertex_bytes = 0usize;
        let mut declared_gas = 0u64;
        let gas_config = GasConfig::default();
        for tx in vertex.transactions.iter() {
            vertex_bytes = vertex_bytes.saturating_add(bcs::to_bytes(tx)?.len());
            declared_gas = declared_gas.saturating_add(tx.transaction.gas_limit());
        }
        anyhow::ensure!(
            vertex_bytes <= MAX_VERTEX_BYTES,
            "DAG vertex byte limit exceeded"
        );
        anyhow::ensure!(
            declared_gas <= gas_config.max_gas_per_block,
            "DAG vertex gas limit exceeded"
        );
        if self.known_vertex(&vertex.id) {
            return Ok(false);
        }
        self.current_round = self.current_round.max(vertex.round);
        self.metrics.vertices_created = self.metrics.vertices_created.saturating_add(1);
        self.vertices.push(vertex);
        Ok(true)
    }

    fn record_checkpoint(&mut self, checkpoint: Checkpoint) {
        self.last_checkpoint_round = self.current_round;
        self.metrics.checkpoints_created = self.metrics.checkpoints_created.saturating_add(1);
        self.checkpoints.push(checkpoint);
        let _ = self.mysticeti.try_advance();
    }
}

#[derive(Clone)]
pub struct DagEngine {
    engine: Arc<BlockchainEngine>,
    consensus: Arc<RwLock<CoreDagConsensus>>,
    authority_id: String,
    local_signing_key: ed25519_dalek::SigningKey,
    authority_public_keys: BTreeMap<String, Vec<u8>>,
    staged_checkpoints: Arc<RwLock<BTreeMap<VertexId, StagedCheckpoint>>>,
    checkpoint_votes: Arc<RwLock<BTreeMap<VertexId, BTreeMap<String, Vec<u8>>>>>,
    pending_checkpoint_votes: Arc<RwLock<BTreeMap<VertexId, BTreeMap<String, CheckpointVote>>>>,
    committed_anchors: Arc<RwLock<VecDeque<(VertexId, Vec<VertexId>)>>>,
}

impl DagEngine {
    pub fn new_secure(
        engine: Arc<BlockchainEngine>,
        authority_id: String,
        authorities: Vec<String>,
        local_signing_key: ed25519_dalek::SigningKey,
        authority_public_keys: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self> {
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
        let state = Self::aligned_dag_state(&engine);
        let consensus = CoreDagConsensus::new(authority_id.clone(), authorities, state)?;
        let dag_engine = Self {
            engine,
            consensus: Arc::new(RwLock::new(consensus)),
            authority_id,
            local_signing_key,
            authority_public_keys,
            staged_checkpoints: Arc::new(RwLock::new(BTreeMap::new())),
            checkpoint_votes: Arc::new(RwLock::new(BTreeMap::new())),
            pending_checkpoint_votes: Arc::new(RwLock::new(BTreeMap::new())),
            committed_anchors: Arc::new(RwLock::new(VecDeque::new())),
        };
        dag_engine.persist_consensus_state()?;
        Ok(dag_engine)
    }

    fn aligned_dag_state(engine: &BlockchainEngine) -> Option<PersistentDagState> {
        let blockchain_checkpoints = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.dag_checkpoints.iter().cloned().collect::<Vec<_>>()
        };
        if blockchain_checkpoints.is_empty() {
            return engine.persisted_dag_state.clone();
        }

        let dag_seq = engine
            .persisted_dag_state
            .as_ref()
            .and_then(|state| state.checkpoints.last())
            .map(|checkpoint| checkpoint.sequence)
            .unwrap_or(0);
        let chain_seq = blockchain_checkpoints
            .last()
            .map(|checkpoint| checkpoint.sequence)
            .unwrap_or(0);
        if dag_seq != chain_seq {
            warn!(
                "Persisted DAG checkpoint sequence ({}) does not match blockchain ({}); using blockchain checkpoints.",
                dag_seq, chain_seq
            );
            return Some(PersistentDagState {
                vertices: engine
                    .persisted_dag_state
                    .as_ref()
                    .map(|state| state.vertices.clone())
                    .unwrap_or_default(),
                checkpoints: blockchain_checkpoints,
                current_round: 0,
                last_checkpoint_round: 0,
            });
        }

        engine.persisted_dag_state.clone()
    }

    fn persist_consensus_state(&self) -> Result<()> {
        let state = self
            .consensus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .save_state();
        self.engine.persist_dag_state(state)
    }

    pub fn produce_vertex(&self) -> Result<CheckpointProductionInfo> {
        let mut pending = self.engine.pending_transactions_snapshot();
        pending.sort_by(|a, b| {
            a.transaction
                .sender_address()
                .cmp(b.transaction.sender_address())
                .then_with(|| {
                    a.transaction
                        .sequence_number()
                        .cmp(&b.transaction.sequence_number())
                })
                .then_with(|| a.transaction_hash().cmp(b.transaction_hash()))
        });
        const MAX_VERTEX_TRANSACTIONS: usize = 10_000;
        const MAX_VERTEX_BYTES: usize = 128 * 1024;
        let gas_config = GasConfig::default();
        let mut transactions = Vec::new();
        let mut declared_gas = 0u64;
        let mut encoded_bytes = 0usize;
        for tx in pending {
            let next_gas = declared_gas.saturating_add(tx.transaction.gas_limit());
            let tx_bytes = bcs::to_bytes(&tx)?.len();
            if transactions.len() >= MAX_VERTEX_TRANSACTIONS
                || next_gas > gas_config.max_gas_per_block
                || encoded_bytes.saturating_add(tx_bytes) > MAX_VERTEX_BYTES
            {
                break;
            }
            declared_gas = next_gas;
            encoded_bytes = encoded_bytes.saturating_add(tx_bytes);
            transactions.push(tx);
        }
        let tx_count = transactions.len();
        let timestamp = {
            let chain = self
                .engine
                .blockchain
                .read()
                .unwrap_or_else(|e| e.into_inner());
            chain
                .latest_checkpoint()
                .timestamp
                .saturating_add(1)
                .max(chain.height().saturating_add(1))
        };
        let (
            state_root,
            executed,
            failed,
            _verified_state,
            _to_execute,
            _receipts,
            _validate_supply,
        ) = {
            let state_snapshot = self.engine.state_read().clone();
            let state_arc = Arc::new(RwLock::new(state_snapshot));
            self.engine
                .execute_system_prologue_to_state_for_dag_v2(&state_arc, timestamp)?;
            let mut validate_supply = true;
            let execution = match self
                .engine
                .apply_zero_effect_native_batch(&transactions, &state_arc)?
            {
                Some((executed, failed)) => {
                    validate_supply = false;
                    TransactionBatchExecution {
                        executed,
                        failed,
                        receipts: Vec::new(),
                    }
                }
                None => self.engine.execute_tx_waves_strict_serial_with_receipts(
                    transactions.clone(),
                    &state_arc,
                    Some(timestamp),
                    false,
                )?,
            };
            let verified_state = state_arc.read().unwrap_or_else(|e| e.into_inner()).clone();
            let state_root = verified_state.compute_state_root();
            (
                state_root,
                execution.executed,
                execution.failed,
                verified_state,
                if validate_supply {
                    transactions.clone()
                } else {
                    Vec::new()
                },
                execution.receipts,
                validate_supply,
            )
        };

        let mysticeti_block = {
            let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
            consensus
                .mysticeti
                .propose_block(&transactions, timestamp)?
        };
        let block = mysticeti_block.ok_or_else(|| {
            anyhow::anyhow!("DAG_WAITING: Mysticeti threshold clock is not ready")
        })?;
        let vertex_id = block.vertex_id;
        let round = block.round;
        let parents = block.parents;
        let serialized_block = block.serialized_block;
        let committed_subdags = block.committed_subdags;

        let mut vertex = DagVertex::new(
            round,
            self.authority_id.clone(),
            "kanari-v2-mysticeti".to_string(),
            parents,
            transactions,
            state_root,
            timestamp,
        );
        vertex.bind_mysticeti_block(serialized_block, vertex_id)?;
        let signing_digest = vertex.signing_digest()?;
        vertex.cached_signing_digest = Some(signing_digest.to_vec());
        use ed25519_dalek::Signer;
        vertex.signature = self
            .local_signing_key
            .sign(&signing_digest)
            .to_bytes()
            .to_vec();

        {
            let mut consensus = self
                .consensus
                .write()
                .unwrap_or_else(|error| error.into_inner());
            consensus.add_vertex(vertex.clone())?;
        }
        let update = self.process_committed_subdags(committed_subdags)?;
        let checkpoint_info =
            update
                .finalized_checkpoints
                .last()
                .map(|checkpoint| CheckpointInfo {
                    sequence: checkpoint.sequence,
                    vertex_count: checkpoint.vertices.len(),
                    tx_count: checkpoint.transactions.len(),
                });

        let vertex_id = hex::encode(vertex.id);
        info!(
            "[DAG v2] Produced Mysticeti-backed vertex {} round {} txs {}",
            vertex_id, vertex.round, tx_count
        );

        Ok(CheckpointProductionInfo {
            vertex_id,
            round: vertex.round,
            tx_count,
            executed,
            failed,
            events: Vec::new(),
            checkpoint: checkpoint_info,
            checkpoint_votes: update.checkpoint_votes,
            vertex: Some(vertex),
        })
    }
    fn checkpoint_quorum(&self) -> usize {
        self.authority_public_keys.len().saturating_mul(2) / 3 + 1
    }

    fn enqueue_committed_subdags(&self, subdags: Vec<Vec<VertexId>>) {
        let consensus = self
            .consensus
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let staged: HashSet<VertexId> = self
            .staged_checkpoints
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .flat_map(|draft| draft.checkpoint.vertices.iter().copied())
            .collect();
        let mut queue = self
            .committed_anchors
            .write()
            .unwrap_or_else(|error| error.into_inner());
        for vertices in subdags {
            let Some(anchor) = vertices.first().copied() else {
                continue;
            };
            let finalized = consensus
                .checkpoints
                .iter()
                .any(|checkpoint| checkpoint.vertices.first() == Some(&anchor));
            let queued = queue.iter().any(|(known, _)| *known == anchor);
            if !finalized && !queued && !staged.contains(&anchor) {
                queue.push_back((anchor, vertices));
            }
        }
    }

    fn prepare_next_committed_anchor(&self) -> Result<Option<CheckpointVote>> {
        if !self
            .staged_checkpoints
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
        {
            return Ok(None);
        }
        loop {
            let Some((anchor, mut support)) = self
                .committed_anchors
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front()
            else {
                return Ok(None);
            };
            support.sort();
            support.dedup();
            support.retain(|id| *id != anchor);
            support.insert(0, anchor);
            let vertex = {
                let consensus = self
                    .consensus
                    .read()
                    .unwrap_or_else(|error| error.into_inner());
                consensus
                    .vertices
                    .iter()
                    .find(|vertex| vertex.id == anchor)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Committed Mysticeti anchor {} has no Kanari vertex",
                            hex::encode(anchor)
                        )
                    })?
            };
            if vertex.transactions.is_empty() {
                continue;
            }
            let (sequence, previous_hash, previous_timestamp) = {
                let chain = self
                    .engine
                    .blockchain
                    .read()
                    .unwrap_or_else(|error| error.into_inner());
                (
                    chain.height().saturating_add(1),
                    chain.latest_checkpoint().hash()?,
                    chain.latest_checkpoint().timestamp,
                )
            };
            anyhow::ensure!(
                vertex.timestamp > previous_timestamp,
                "Committed anchor timestamp is not monotonic"
            );
            let mut checkpoint = Checkpoint::new(
                sequence,
                support,
                vertex.transactions.clone(),
                Vec::new(),
                vertex.timestamp,
                previous_hash,
            );
            let (root, verified_state, to_execute, receipts) =
                self.engine.prepare_checkpoint_state(&checkpoint)?;
            anyhow::ensure!(
                root == vertex.metadata.state_root,
                "Committed anchor root differs from deterministic execution"
            );
            checkpoint.state_root = root;
            let vote = CheckpointVote::new(
                checkpoint.clone(),
                0,
                vertex.round,
                self.authority_id.clone(),
                &self.local_signing_key,
                &self.authority_public_keys,
            )?;
            let checkpoint_id = vote.checkpoint_id;
            self.staged_checkpoints
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .insert(
                    checkpoint_id,
                    StagedCheckpoint {
                        checkpoint,
                        verified_state,
                        to_execute,
                        receipts,
                        validate_supply: true,
                        epoch: 0,
                        round: vertex.round,
                    },
                );
            return Ok(Some(vote));
        }
    }

    fn accept_checkpoint_vote(&self, vote: CheckpointVote) -> Result<ConsensusUpdate> {
        anyhow::ensure!(
            self.authority_public_keys.contains_key(&vote.authority),
            "Checkpoint vote is from a non-committee authority"
        );
        let checkpoint_id = vote.checkpoint_id;
        {
            let staged = self
                .staged_checkpoints
                .read()
                .unwrap_or_else(|error| error.into_inner());
            let expected = staged
                .get(&checkpoint_id)
                .ok_or_else(|| anyhow::anyhow!("Checkpoint vote has no locally committed draft"))?;
            anyhow::ensure!(
                expected.epoch == vote.epoch && expected.round == vote.round,
                "Checkpoint vote epoch or round mismatch"
            );
            vote.verify_for_checkpoint(&expected.checkpoint, &self.authority_public_keys)?;
        }
        let vote_count = {
            let mut all_votes = self
                .checkpoint_votes
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let votes = all_votes.entry(checkpoint_id).or_default();
            if let Some(existing) = votes.get(&vote.authority) {
                anyhow::ensure!(existing == &vote.signature, "Conflicting checkpoint vote");
            } else {
                votes.insert(vote.authority.clone(), vote.signature.clone());
            }
            votes.len()
        };
        if vote_count < self.checkpoint_quorum() {
            return Ok(ConsensusUpdate::default());
        }

        let signatures = self
            .checkpoint_votes
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&checkpoint_id)
            .unwrap_or_default()
            .into_iter()
            .map(
                |(authority, signature)| crate::consensus::CheckpointAuthoritySignature {
                    authority,
                    signature,
                },
            )
            .collect::<Vec<_>>();
        self.pending_checkpoint_votes
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&checkpoint_id);
        let mut staged = self
            .staged_checkpoints
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&checkpoint_id)
            .ok_or_else(|| anyhow::anyhow!("Certified checkpoint draft disappeared"))?;
        staged.checkpoint.certificate = Some(CheckpointCertificate {
            epoch: staged.epoch,
            round: staged.round,
            committee_digest: Checkpoint::committee_digest(&self.authority_public_keys)?,
            signatures,
        });
        staged.checkpoint.verify_certificate(
            &self.authority_public_keys,
            self.authority_public_keys.len(),
        )?;
        self.engine.apply_prepared_checkpoint(
            staged.checkpoint.clone(),
            staged.verified_state,
            staged.to_execute,
            staged.receipts,
            staged.validate_supply,
        )?;
        {
            let mut consensus = self
                .consensus
                .write()
                .unwrap_or_else(|error| error.into_inner());
            consensus.record_checkpoint(staged.checkpoint.clone());
        }
        self.persist_consensus_state()?;
        let mut update = ConsensusUpdate {
            checkpoint_votes: Vec::new(),
            finalized_checkpoints: vec![staged.checkpoint],
        };
        if let Some(next_vote) = self.prepare_next_committed_anchor()? {
            let next_update = self.accept_checkpoint_vote(next_vote.clone())?;
            update.checkpoint_votes.push(next_vote);
            update.checkpoint_votes.extend(next_update.checkpoint_votes);
            update
                .finalized_checkpoints
                .extend(next_update.finalized_checkpoints);
        }
        Ok(update)
    }

    fn process_committed_subdags(&self, subdags: Vec<Vec<VertexId>>) -> Result<ConsensusUpdate> {
        self.enqueue_committed_subdags(subdags);
        let Some(vote) = self.prepare_next_committed_anchor()? else {
            return Ok(ConsensusUpdate::default());
        };
        let checkpoint_id = vote.checkpoint_id;
        let mut update = self.accept_checkpoint_vote(vote.clone())?;
        update.checkpoint_votes.insert(0, vote);
        if self
            .staged_checkpoints
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&checkpoint_id)
        {
            let buffered = self
                .pending_checkpoint_votes
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&checkpoint_id)
                .unwrap_or_default();
            for buffered_vote in buffered.into_values() {
                let next = self.accept_checkpoint_vote(buffered_vote)?;
                update.checkpoint_votes.extend(next.checkpoint_votes);
                update
                    .finalized_checkpoints
                    .extend(next.finalized_checkpoints);
                if !self
                    .staged_checkpoints
                    .read()
                    .unwrap_or_else(|error| error.into_inner())
                    .contains_key(&checkpoint_id)
                {
                    break;
                }
            }
        }
        Ok(update)
    }

    pub fn submit_checkpoint_vote(&self, vote: CheckpointVote) -> Result<ConsensusUpdate> {
        if self
            .staged_checkpoints
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(&vote.checkpoint_id)
        {
            return self.accept_checkpoint_vote(vote);
        }
        anyhow::ensure!(
            self.authority_public_keys.contains_key(&vote.authority),
            "Checkpoint vote is from a non-committee authority"
        );
        anyhow::ensure!(
            vote.signature.len() == 64,
            "Invalid pending checkpoint vote signature length"
        );
        anyhow::ensure!(
            vote.sequence == self.engine.get_stats().height.saturating_add(1),
            "Pending checkpoint vote sequence is not the next local sequence"
        );
        let mut pending = self
            .pending_checkpoint_votes
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if !pending.contains_key(&vote.checkpoint_id) && pending.len() >= 256 {
            if let Some(oldest) = pending.keys().next().copied() {
                pending.remove(&oldest);
            }
        }
        let votes = pending.entry(vote.checkpoint_id).or_default();
        if votes.len() < self.authority_public_keys.len() {
            votes.entry(vote.authority.clone()).or_insert(vote);
        }
        Ok(ConsensusUpdate::default())
    }

    pub fn consensus(&self) -> Arc<RwLock<CoreDagConsensus>> {
        self.consensus.clone()
    }

    pub fn latest_own_vertices(&self, limit: usize) -> Vec<DagVertex> {
        let consensus = self.consensus.read().unwrap_or_else(|e| e.into_inner());
        consensus.latest_vertices_by_authority(&self.authority_id, limit)
    }

    pub fn needs_progress(&self) -> bool {
        let consensus_progress = self
            .consensus
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .needs_progress();
        consensus_progress
            || !self
                .staged_checkpoints
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
            || !self
                .committed_anchors
                .read()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
    }

    fn verify_full_network_vertex(
        &self,
        consensus: &CoreDagConsensus,
        vertex: &DagVertex,
    ) -> Result<()> {
        const DAG_CHAIN_ID: &str = "kanari-v2-mysticeti";

        if vertex.chain_id != DAG_CHAIN_ID {
            anyhow::bail!("Invalid DAG vertex chain id: {}", vertex.chain_id);
        }
        if vertex.round == 0 {
            anyhow::bail!("Invalid DAG vertex round 0");
        }
        if !consensus.authorities.contains(&vertex.author) {
            anyhow::bail!("Unknown DAG vertex author: {}", vertex.author);
        }
        if vertex.transactions.len() != vertex.metadata.tx_count {
            anyhow::bail!("Transaction count mismatch");
        }
        if vertex.metadata.is_checkpoint || vertex.metadata.checkpoint_seq.is_some() {
            anyhow::bail!("Network DAG vertex must not carry checkpoint metadata");
        }
        if consensus
            .vertices
            .iter()
            .any(|existing| existing.author == vertex.author && existing.round == vertex.round)
        {
            anyhow::bail!(
                "Duplicate DAG vertex for author {} at round {}",
                vertex.author,
                vertex.round
            );
        }

        let public_key_bytes = self
            .authority_public_keys
            .get(&vertex.author)
            .ok_or_else(|| anyhow::anyhow!("Missing consensus public key for {}", vertex.author))?;
        let public_key_bytes: [u8; 32] = public_key_bytes.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!("Invalid consensus public key length for {}", vertex.author)
        })?;
        let verifying_key =
            ed25519_dalek::VerifyingKey::from_bytes(&public_key_bytes).map_err(|e| {
                anyhow::anyhow!("Invalid consensus public key for {}: {}", vertex.author, e)
            })?;
        let signature_bytes: [u8; 64] = vertex.signature.as_slice().try_into().map_err(|_| {
            anyhow::anyhow!("Invalid DAG vertex signature length for {}", vertex.author)
        })?;
        let signature = ed25519_dalek::Signature::from_bytes(&signature_bytes);
        let signing_digest = vertex.signing_digest()?;
        use ed25519_dalek::Verifier;
        verifying_key
            .verify(&signing_digest, &signature)
            .map_err(|e| {
                anyhow::anyhow!("Invalid DAG vertex signature for {}: {}", vertex.author, e)
            })?;

        let mut seen_tx_hashes = HashSet::new();
        for (index, tx) in vertex.transactions.iter().enumerate() {
            let tx_hash = tx.verified_transaction_hash().map_err(|e| {
                anyhow::anyhow!(
                    "Invalid transaction {} in DAG vertex {}: {}",
                    index + 1,
                    hex::encode(vertex.id),
                    e
                )
            })?;
            if !seen_tx_hashes.insert(tx_hash.to_vec()) {
                anyhow::bail!(
                    "Duplicate transaction inside DAG vertex {}",
                    hex::encode(vertex.id)
                );
            }
        }

        let mut seen_parents = HashSet::new();
        for parent in &vertex.parents {
            if !seen_parents.insert(*parent) {
                anyhow::bail!("Duplicate parent in DAG vertex {}", hex::encode(vertex.id));
            }
        }

        if vertex.round == 1 {
            if !vertex.parents.is_empty() {
                anyhow::bail!("Round-1 DAG vertex must not have parents");
            }
            return Ok(());
        }

        if vertex.parents.is_empty() {
            anyhow::bail!("DAG vertex round {} is missing parents", vertex.round);
        }

        let mut parent_authors = HashSet::new();
        for parent_id in &vertex.parents {
            let parent = consensus
                .vertices
                .iter()
                .find(|candidate| candidate.id == *parent_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Missing parent {} for DAG vertex {}",
                        hex::encode(parent_id),
                        hex::encode(vertex.id)
                    )
                })?;
            if parent.round >= vertex.round {
                anyhow::bail!(
                    "Invalid parent round for DAG vertex {}: parent {} is not earlier than round {}",
                    hex::encode(vertex.id),
                    parent.round,
                    vertex.round
                );
            }
            if !parent_authors.insert(parent.author.clone()) {
                anyhow::bail!(
                    "Duplicate parent author in DAG vertex {}",
                    hex::encode(vertex.id)
                );
            }
        }

        anyhow::ensure!(
            parent_authors.len() >= consensus.mysticeti.quorum_threshold(),
            "DAG vertex does not reference a quorum of parent authorities"
        );
        Ok(())
    }

    pub fn add_network_vertex(&self, vertex: DagVertex) -> Result<ConsensusUpdate> {
        let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
        if consensus.known_vertex(&vertex.id) {
            return Ok(ConsensusUpdate::default());
        }
        self.verify_full_network_vertex(&consensus, &vertex)?;
        let (block, committed_subdags) =
            consensus.mysticeti.import_block(&vertex.mysticeti_block)?;
        let expected_author = consensus
            .authorities
            .iter()
            .position(|authority| authority == &vertex.author)
            .ok_or_else(|| anyhow::anyhow!("Unknown Mysticeti vertex author"))?;
        anyhow::ensure!(
            block.author().index() == expected_author,
            "Mysticeti author index mismatch"
        );
        anyhow::ensure!(block.round() == vertex.round, "Mysticeti round mismatch");
        anyhow::ensure!(
            mysticeti_reference_to_vertex_id(block.reference()) == vertex.id,
            "Mysticeti block digest mismatch"
        );
        let parents = block
            .includes()
            .iter()
            .filter(|reference| reference.round > 0)
            .map(mysticeti_reference_to_vertex_id)
            .collect::<Vec<_>>();
        anyhow::ensure!(parents == vertex.parents, "Mysticeti parent set mismatch");
        if vertex.transactions.is_empty() {
            anyhow::ensure!(
                block.transactions().is_empty(),
                "Empty Kanari vertex carries an unexpected Mysticeti transaction commitment"
            );
        } else {
            let expected = signed_tx_batch_to_mysticeti_transaction(
                vertex.transactions.as_ref(),
                vertex.timestamp,
            );
            anyhow::ensure!(
                block.transactions().len() == 1,
                "Mysticeti adapter block has invalid payload count"
            );
            anyhow::ensure!(
                block.transactions()[0].as_bytes() == expected.as_bytes(),
                "Mysticeti adapter transaction commitment mismatch"
            );
        }
        info!(
            "[DAG v2 SYNC] Accepted authenticated Mysticeti vertex {} round {} txs {}",
            hex::encode(vertex.id),
            vertex.round,
            vertex.transactions.len()
        );
        consensus.add_vertex(vertex)?;
        drop(consensus);
        self.persist_consensus_state()?;
        self.process_committed_subdags(committed_subdags)
    }
}

impl BlockchainEngine {
    fn execute_system_prologue_to_state_for_dag_v2(
        &self,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp_ms: u64,
    ) -> Result<()> {
        let runtime = &self.runtime_pool[0];
        let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
        let clock_id = runtime.ensure_system_clock(&mut state_write)?;
        let changeset = runtime.execute_clock_consensus_commit_prologue(clock_id, timestamp_ms)?;
        state_write.apply_changeset(&changeset)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../tests/unit/produce_dag_vertex_tests.rs"]
mod tests;
