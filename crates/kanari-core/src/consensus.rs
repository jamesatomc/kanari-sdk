// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use kanari_crypto::hash_data_blake3;
use kanari_types::transaction::SignedTransaction;
use mysticeti_consensus::protocol::Protocol as MysticetiProtocol;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub type VertexId = [u8; 32];
pub type Round = u64;
pub type AuthorityId = String;
pub type TransactionBatch = Arc<[SignedTransaction]>;

fn logical_tx_hash(tx: &SignedTransaction) -> Vec<u8> {
    tx.transaction_hash().to_vec()
}

fn vertex_id_from_hash_bytes(bytes: &[u8]) -> VertexId {
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes[..32]);
    id
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagVertex {
    pub id: VertexId,
    pub round: Round,
    pub author: AuthorityId,
    pub chain_id: String,
    pub parents: Vec<VertexId>,
    pub transactions: TransactionBatch,
    pub timestamp: u64,
    pub signature: Vec<u8>,
    pub metadata: VertexMetadata,
    #[serde(skip)]
    pub cached_serialized_data: Option<Vec<u8>>,
    #[serde(skip)]
    pub cached_hash: Option<Vec<u8>>,
    #[serde(skip)]
    pub cached_signing_digest: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VertexMetadata {
    pub tx_count: usize,
    pub total_gas_used: u64,
    pub state_root: Vec<u8>,
    pub is_checkpoint: bool,
    pub checkpoint_seq: Option<u64>,
}

impl DagVertex {
    fn compute_hash_uncached(&self) -> Result<VertexId> {
        let tx_hashes: Vec<Vec<u8>> = self.transactions.iter().map(logical_tx_hash).collect();
        let bytes = bcs::to_bytes(&(
            &self.chain_id,
            self.round,
            &self.author,
            &self.parents,
            tx_hashes,
            self.timestamp,
            &self.metadata.state_root,
        ))?;
        Ok(vertex_id_from_hash_bytes(&hash_data_blake3(&bytes)))
    }

    fn compute_signing_digest_uncached(&self) -> Result<VertexId> {
        let tx_hashes: Vec<Vec<u8>> = self.transactions.iter().map(logical_tx_hash).collect();
        let bytes = bcs::to_bytes(&(
            b"kanari:dag-vertex-signature:v1".as_slice(),
            &self.id,
            &self.chain_id,
            self.round,
            &self.author,
            &self.parents,
            tx_hashes,
            self.timestamp,
            self.metadata.tx_count,
            self.metadata.total_gas_used,
            &self.metadata.state_root,
            self.metadata.is_checkpoint,
            self.metadata.checkpoint_seq,
        ))?;
        Ok(vertex_id_from_hash_bytes(&hash_data_blake3(&bytes)))
    }

    pub fn new<T>(
        round: Round,
        author: AuthorityId,
        chain_id: String,
        parents: Vec<VertexId>,
        transactions: T,
        state_root: Vec<u8>,
        timestamp: u64,
    ) -> Self
    where
        T: Into<TransactionBatch>,
    {
        Self::try_new(
            round,
            author,
            chain_id,
            parents,
            transactions,
            state_root,
            timestamp,
        )
        .expect("DagVertex::new failed")
    }

    pub fn try_new<T>(
        round: Round,
        author: AuthorityId,
        chain_id: String,
        parents: Vec<VertexId>,
        transactions: T,
        state_root: Vec<u8>,
        timestamp: u64,
    ) -> Result<Self>
    where
        T: Into<TransactionBatch>,
    {
        let transactions = transactions.into();
        let metadata = VertexMetadata {
            tx_count: transactions.len(),
            total_gas_used: 0,
            state_root,
            is_checkpoint: false,
            checkpoint_seq: None,
        };
        let mut vertex = Self {
            id: [0u8; 32],
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
            cached_signing_digest: None,
        };
        let hash = vertex.compute_hash_uncached()?;
        vertex.cached_hash = Some(hash.to_vec());
        vertex.id = hash;
        Ok(vertex)
    }

    pub fn compute_hash(&self) -> Result<VertexId> {
        if let Some(hash) = &self.cached_hash {
            return Ok(vertex_id_from_hash_bytes(hash));
        }
        self.compute_hash_uncached()
    }

    /// Bind the externally assigned Mysticeti block id to the full Kanari vertex payload.
    pub fn signing_digest(&self) -> Result<VertexId> {
        if let Some(digest) = &self.cached_signing_digest {
            let computed = self.compute_signing_digest_uncached()?;
            if vertex_id_from_hash_bytes(digest) != computed {
                anyhow::bail!("Vertex signing digest mismatch");
            }
            return Ok(computed);
        }
        self.compute_signing_digest_uncached()
    }

    /// Verify only the vertex payload/cache consistency.
    /// This does not validate network-level invariants such as author membership,
    /// parent availability, or the signature against the authority key.
    pub fn verify_payload_consistency(&self) -> Result<()> {
        if let Some(cached_hash) = &self.cached_hash {
            let computed = self.compute_hash_uncached()?;
            if vertex_id_from_hash_bytes(cached_hash) != computed {
                anyhow::bail!("Vertex payload hash mismatch");
            }
        }
        if self.transactions.len() != self.metadata.tx_count {
            anyhow::bail!("Transaction count mismatch");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub sequence: u64,
    pub vertices: Vec<VertexId>,
    pub transactions: TransactionBatch,
    pub state_root: Vec<u8>,
    pub timestamp: u64,
    pub prev_checkpoint_hash: Vec<u8>,
}

impl Checkpoint {
    pub fn new<T>(
        sequence: u64,
        vertices: Vec<VertexId>,
        transactions: T,
        state_root: Vec<u8>,
        timestamp: u64,
        prev_checkpoint_hash: Vec<u8>,
    ) -> Self
    where
        T: Into<TransactionBatch>,
    {
        Self {
            sequence,
            vertices,
            transactions: transactions.into(),
            state_root,
            timestamp,
            prev_checkpoint_hash,
        }
    }

    pub fn hash(&self) -> Result<Vec<u8>> {
        let tx_hashes: Vec<Vec<u8>> = self.transactions.iter().map(logical_tx_hash).collect();
        let serialized = bcs::to_bytes(&(
            self.sequence,
            &tx_hashes,
            &self.state_root,
            &self.prev_checkpoint_hash,
        ))?;
        Ok(hash_data_blake3(&serialized))
    }

    pub fn genesis() -> Self {
        Self {
            sequence: 0,
            vertices: Vec::new(),
            transactions: Vec::new().into(),
            state_root: smt::default_hashes()[0].to_vec(),
            timestamp: 0,
            prev_checkpoint_hash: vec![0u8; 32],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentDagState {
    pub vertices: Vec<DagVertex>,
    pub checkpoints: Vec<Checkpoint>,
    pub current_round: Round,
    pub last_checkpoint_round: Round,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagProductionPolicy {
    pub current_round: Round,
    pub parent_round: Round,
    pub target_round: Round,
    pub parent_ids: Vec<VertexId>,
    pub parent_authors: Vec<AuthorityId>,
    pub missing_parent_authors: Vec<AuthorityId>,
    pub parent_author_count: usize,
    pub quorum_size: usize,
    pub local_has_vertex_in_current_round: bool,
    pub using_catch_up_round: bool,
}

#[cfg(test)]
#[path = "../tests/unit/consensus_tests.rs"]
mod tests;

impl DagProductionPolicy {
    pub fn should_wait_for_current_round_quorum(&self) -> bool {
        self.current_round > 0
            && self.parent_round == self.current_round
            && self.local_has_vertex_in_current_round
            && self.parent_author_count < self.quorum_size
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsensusRuntimeProtocol {
    pub protocol: String,
    pub wave_length: u64,
    pub direct_commit_quorum: u64,
    pub pipeline: bool,
    pub leader_wait: bool,
}

impl ConsensusRuntimeProtocol {
    pub fn from_mysticeti(protocol: &MysticetiProtocol) -> Self {
        Self {
            protocol: "mysticeti".to_string(),
            wave_length: protocol.wave_length,
            direct_commit_quorum: protocol.direct_commit_quorum,
            pipeline: protocol.pipeline,
            leader_wait: protocol.leader_wait,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DagMetrics {
    pub vertices_created: u64,
    pub checkpoints_created: u64,
}

impl DagMetrics {
    pub fn export_prometheus(&self) -> Result<String> {
        Ok(format!(
            "# HELP dag_vertices_created_total Total DAG vertices created\n# TYPE dag_vertices_created_total counter\ndag_vertices_created_total {}\n# HELP dag_checkpoints_created_total Total DAG checkpoints created\n# TYPE dag_checkpoints_created_total counter\ndag_checkpoints_created_total {}\n# HELP dag_active_vertices Active DAG vertices retained in memory\n# TYPE dag_active_vertices gauge\ndag_active_vertices {}\n",
            self.vertices_created, self.checkpoints_created, self.vertices_created
        ))
    }
}
