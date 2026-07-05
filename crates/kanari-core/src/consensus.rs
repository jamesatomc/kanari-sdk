// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use kanari_crypto::hash_data_blake3;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::transaction::SignedTransaction;
use mysticeti_consensus::protocol::Protocol as MysticetiProtocol;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub type VertexId = [u8; 32];
pub type Round = u64;
pub type AuthorityId = String;
pub type TransactionBatch = Arc<[SignedTransaction]>;

pub const CHECKPOINT_PROTOCOL_VERSION: u64 = 3;

fn logical_tx_hash(tx: &SignedTransaction) -> Vec<u8> {
    tx.transaction_hash().to_vec()
}

fn vertex_id_from_hash_bytes(bytes: &[u8]) -> VertexId {
    let mut id = [0u8; 32];
    id.copy_from_slice(&bytes[..32]);
    id
}

pub fn checkpoint_quorum_size(authority_count: usize) -> usize {
    if authority_count == 0 {
        0
    } else {
        (authority_count * 2) / 3 + 1
    }
}

pub fn compute_committee_digest(
    authorities: &[AuthorityId],
    public_keys: &BTreeMap<AuthorityId, Vec<u8>>,
) -> Result<Vec<u8>> {
    let mut members = authorities
        .iter()
        .map(|authority| {
            let key = public_keys
                .get(authority)
                .with_context(|| format!("Missing consensus public key for {authority}"))?;
            Ok((authority.clone(), key.clone()))
        })
        .collect::<Result<Vec<_>>>()?;
    members.sort_by(|a, b| a.0.cmp(&b.0));
    members.dedup_by(|a, b| a.0 == b.0);
    if members.len() != authorities.len() {
        anyhow::bail!("Consensus authority set contains duplicates");
    }
    Ok(hash_data_blake3(&bcs::to_bytes(&(
        b"kanari:committee:v1".as_slice(),
        members,
    ))?))
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
        .invariant("DagVertex::new failed")
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
        };
        vertex.id = vertex.compute_hash()?;
        Ok(vertex)
    }

    pub fn compute_hash(&self) -> Result<VertexId> {
        let tx_hashes: Vec<Vec<u8>> = self.transactions.iter().map(logical_tx_hash).collect();
        let bytes = bcs::to_bytes(&(
            b"kanari:dag-vertex:v2".as_slice(),
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

    pub fn signing_digest(&self) -> Result<VertexId> {
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

    pub fn verify(&self) -> Result<()> {
        if self.id != self.compute_hash()? {
            anyhow::bail!("Vertex hash mismatch");
        }
        if self.transactions.len() != self.metadata.tx_count {
            anyhow::bail!("Transaction count mismatch");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointSignature {
    pub authority: AuthorityId,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointCertificate {
    pub epoch: u64,
    pub committee_digest: Vec<u8>,
    pub signatures: Vec<CheckpointSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub sequence: u64,
    pub vertices: Vec<VertexId>,
    pub transactions: TransactionBatch,
    pub state_root: Vec<u8>,
    pub timestamp: u64,
    pub prev_checkpoint_hash: Vec<u8>,
    #[serde(default)]
    pub chain_id: String,
    #[serde(default = "default_checkpoint_protocol_version")]
    pub protocol_version: u64,
    #[serde(default)]
    pub epoch: u64,
    #[serde(default)]
    pub committee_digest: Vec<u8>,
    #[serde(default)]
    pub commit_round: Round,
    #[serde(default)]
    pub certificate: Option<CheckpointCertificate>,
}

fn default_checkpoint_protocol_version() -> u64 {
    CHECKPOINT_PROTOCOL_VERSION
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
            chain_id: String::new(),
            protocol_version: CHECKPOINT_PROTOCOL_VERSION,
            epoch: 0,
            committee_digest: Vec::new(),
            commit_round: 0,
            certificate: None,
        }
    }

    pub fn with_consensus_context(
        mut self,
        chain_id: String,
        epoch: u64,
        committee_digest: Vec<u8>,
        commit_round: Round,
    ) -> Self {
        self.chain_id = chain_id;
        self.epoch = epoch;
        self.committee_digest = committee_digest;
        self.commit_round = commit_round;
        self
    }

    pub fn proposal_digest(&self) -> Result<Vec<u8>> {
        let tx_hashes: Vec<Vec<u8>> = self.transactions.iter().map(logical_tx_hash).collect();
        Ok(hash_data_blake3(&bcs::to_bytes(&(
            b"kanari:checkpoint-proposal:v3".as_slice(),
            self.protocol_version,
            &self.chain_id,
            self.epoch,
            &self.committee_digest,
            self.commit_round,
            self.sequence,
            &self.vertices,
            &tx_hashes,
            &self.state_root,
            self.timestamp,
            &self.prev_checkpoint_hash,
        ))?))
    }

    pub fn add_signature(&mut self, authority: AuthorityId, signature: Vec<u8>) {
        let certificate = self
            .certificate
            .get_or_insert_with(|| CheckpointCertificate {
                epoch: self.epoch,
                committee_digest: self.committee_digest.clone(),
                signatures: Vec::new(),
            });
        certificate
            .signatures
            .retain(|existing| existing.authority != authority);
        certificate
            .signatures
            .push(CheckpointSignature { authority, signature });
        certificate
            .signatures
            .sort_by(|a, b| a.authority.cmp(&b.authority));
    }

    pub fn verify_certificate(
        &self,
        authorities: &[AuthorityId],
        public_keys: &BTreeMap<AuthorityId, Vec<u8>>,
    ) -> Result<()> {
        if self.sequence == 0 {
            return Ok(());
        }
        if self.protocol_version != CHECKPOINT_PROTOCOL_VERSION {
            anyhow::bail!(
                "Unsupported checkpoint protocol version {}",
                self.protocol_version
            );
        }
        if self.chain_id.is_empty() {
            anyhow::bail!("Checkpoint chain_id is empty");
        }
        let expected_committee = compute_committee_digest(authorities, public_keys)?;
        if self.committee_digest != expected_committee {
            anyhow::bail!("Checkpoint committee digest does not match active committee");
        }
        let certificate = self
            .certificate
            .as_ref()
            .context("Checkpoint is missing a quorum certificate")?;
        if certificate.epoch != self.epoch
            || certificate.committee_digest != self.committee_digest
        {
            anyhow::bail!("Checkpoint certificate context mismatch");
        }

        let authority_set: BTreeSet<_> = authorities.iter().cloned().collect();
        if authority_set.len() != authorities.len() {
            anyhow::bail!("Active authority set contains duplicates");
        }
        let digest = self.proposal_digest()?;
        let mut verified = BTreeSet::new();
        for vote in &certificate.signatures {
            if !authority_set.contains(&vote.authority) {
                anyhow::bail!("Checkpoint vote from non-committee authority {}", vote.authority);
            }
            if !verified.insert(vote.authority.clone()) {
                anyhow::bail!("Duplicate checkpoint vote from {}", vote.authority);
            }
            let public_key = public_keys
                .get(&vote.authority)
                .with_context(|| format!("Missing public key for {}", vote.authority))?;
            let key_bytes: [u8; 32] = public_key
                .as_slice()
                .try_into()
                .context("Invalid Ed25519 consensus public key length")?;
            let verifying_key = VerifyingKey::from_bytes(&key_bytes)
                .context("Invalid Ed25519 consensus public key")?;
            let signature = Signature::from_slice(&vote.signature)
                .context("Invalid checkpoint signature length")?;
            verifying_key
                .verify(&digest, &signature)
                .with_context(|| format!("Invalid checkpoint signature from {}", vote.authority))?;
        }

        let quorum = checkpoint_quorum_size(authorities.len());
        if verified.len() < quorum {
            anyhow::bail!(
                "Checkpoint certificate has {} valid signatures; quorum is {}",
                verified.len(),
                quorum
            );
        }
        Ok(())
    }

    pub fn hash(&self) -> Result<Vec<u8>> {
        let proposal = self.proposal_digest()?;
        let certificate = self.certificate.clone().map(|mut certificate| {
            certificate
                .signatures
                .sort_by(|a, b| a.authority.cmp(&b.authority));
            certificate
        });
        Ok(hash_data_blake3(&bcs::to_bytes(&(
            b"kanari:certified-checkpoint:v3".as_slice(),
            proposal,
            certificate,
        ))?))
    }

    pub fn genesis() -> Self {
        Self {
            sequence: 0,
            vertices: Vec::new(),
            transactions: Vec::new().into(),
            state_root: smt::default_hashes()[0].to_vec(),
            timestamp: 0,
            prev_checkpoint_hash: vec![0u8; 32],
            chain_id: "genesis".to_string(),
            protocol_version: CHECKPOINT_PROTOCOL_VERSION,
            epoch: 0,
            committee_digest: Vec::new(),
            commit_round: 0,
            certificate: None,
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
