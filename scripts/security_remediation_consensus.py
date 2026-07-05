#!/usr/bin/env python3
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(path: str) -> str:
    return (ROOT / path).read_text(encoding="utf-8")


def save(path: str, text: str) -> None:
    (ROOT / path).write_text(text, encoding="utf-8")


def replace_required(text: str, old: str, new: str, label: str) -> str:
    if new in text:
        return text
    count = text.count(old)
    if count != 1:
        raise RuntimeError(f"{label}: expected one source pattern, found {count}")
    return text.replace(old, new, 1)


path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
s = load(path)
s = replace_required(
    s,
    "Checkpoint, ConsensusRuntimeProtocol, DagMetrics, DagProductionPolicy, DagVertex,\n    PersistentDagState, VertexId,",
    "compute_committee_digest, Checkpoint, CheckpointSignature, ConsensusRuntimeProtocol,\n    DagMetrics, DagProductionPolicy, DagVertex, PersistentDagState, VertexId,",
    "consensus imports",
)
old = """        self.stage_locally_produced_vertex(
            &vertex,
            next_checkpoint_sequence,
            verified_state,
            to_execute,
            validate_supply,
        )?;
        let checkpoint = self.finalize_staged_checkpoint(vertex.id)?;
        let checkpoint_info = Some(CheckpointInfo {
            sequence: checkpoint.sequence,
            vertex_count: checkpoint.vertices.len(),
            tx_count: checkpoint.transactions.len(),
        });"""
new = """        let proposal = self.stage_locally_produced_vertex(
            &vertex,
            next_checkpoint_sequence,
            verified_state,
            to_execute,
            validate_supply,
        )?;
        let checkpoint = if proposal
            .verify_certificate(&self.authorities, &self.authority_public_keys)
            .is_ok()
        {
            Some(self.finalize_staged_checkpoint(vertex.id)?)
        } else {
            info!(
                "[DAG v2] Checkpoint proposal {} is staged with {}/{} signatures",
                proposal.sequence,
                proposal
                    .certificate
                    .as_ref()
                    .map(|certificate| certificate.signatures.len())
                    .unwrap_or(0),
                crate::consensus::checkpoint_quorum_size(self.authorities.len())
            );
            None
        };
        let checkpoint_info = checkpoint.as_ref().map(|checkpoint| CheckpointInfo {
            sequence: checkpoint.sequence,
            vertex_count: checkpoint.vertices.len(),
            tx_count: checkpoint.transactions.len(),
        });"""
s = replace_required(s, old, new, "stage/finalize quorum gate")
old = """        let checkpoint = Checkpoint::new(
            checkpoint_sequence,
            vec![vertex.id],
            vertex.transactions.clone(),
            vertex.metadata.state_root.clone(),
            vertex.timestamp,
            prev_hash,
        );"""
new = """        let committee_digest =
            compute_committee_digest(&self.authorities, &self.authority_public_keys)?;
        let mut checkpoint = Checkpoint::new(
            checkpoint_sequence,
            vec![vertex.id],
            vertex.transactions.clone(),
            vertex.metadata.state_root.clone(),
            vertex.timestamp,
            prev_hash,
        )
        .with_consensus_context(
            vertex.chain_id.clone(),
            0,
            committee_digest,
            vertex.round,
        );
        use ed25519_dalek::Signer;
        let local_signature = self
            .local_signing_key
            .sign(&checkpoint.proposal_digest()?)
            .to_bytes()
            .to_vec();
        checkpoint.add_signature(self.authority_id.clone(), local_signature);"""
s = replace_required(s, old, new, "checkpoint consensus context")
old = """    fn finalize_staged_checkpoint(&self, vertex_id: VertexId) -> Result<Checkpoint> {
        let staged = self
            .staged_checkpoints
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&vertex_id)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing staged checkpoint for vertex {}",
                    hex::encode(vertex_id)
                )
            })?;

        self.engine.apply_prepared_checkpoint(
            staged.checkpoint.clone(),
            staged.verified_state,
            staged.to_execute,
            staged.validate_supply,
        )?;"""
new = """    fn finalize_staged_checkpoint(&self, vertex_id: VertexId) -> Result<Checkpoint> {
        {
            let staged = self
                .staged_checkpoints
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let proposal = staged.get(&vertex_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "Missing staged checkpoint for vertex {}",
                    hex::encode(vertex_id)
                )
            })?;
            proposal
                .checkpoint
                .verify_certificate(&self.authorities, &self.authority_public_keys)?;
        }

        let staged = self
            .staged_checkpoints
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&vertex_id)
            .ok_or_else(|| anyhow::anyhow!("Staged checkpoint disappeared"))?;

        self.engine.apply_prepared_checkpoint(
            staged.checkpoint.clone(),
            staged.verified_state,
            staged.to_execute,
            staged.validate_supply,
        )?;"""
s = replace_required(s, old, new, "finalize certificate gate")
marker = "    pub fn consensus(&self) -> Arc<RwLock<CoreDagConsensus>> {"
if "pub fn sign_checkpoint_proposal(" not in s:
    methods = """    pub fn pending_checkpoint(&self, vertex_id: VertexId) -> Option<Checkpoint> {
        self.staged_checkpoints
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&vertex_id)
            .map(|staged| staged.checkpoint.clone())
    }

    pub fn sign_checkpoint_proposal(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<CheckpointSignature> {
        let expected_committee =
            compute_committee_digest(&self.authorities, &self.authority_public_keys)?;
        if checkpoint.committee_digest != expected_committee {
            anyhow::bail!("Checkpoint proposal committee mismatch");
        }
        let (computed_root, _, _) = self.engine.prepare_checkpoint_state(checkpoint)?;
        if computed_root != checkpoint.state_root {
            anyhow::bail!("Checkpoint proposal state root mismatch");
        }
        use ed25519_dalek::Signer;
        Ok(CheckpointSignature {
            authority: self.authority_id.clone(),
            signature: self
                .local_signing_key
                .sign(&checkpoint.proposal_digest()?)
                .to_bytes()
                .to_vec(),
        })
    }

    pub fn add_checkpoint_vote(
        &self,
        vertex_id: VertexId,
        vote: CheckpointSignature,
    ) -> Result<Option<Checkpoint>> {
        {
            let mut staged = self
                .staged_checkpoints
                .write()
                .unwrap_or_else(|e| e.into_inner());
            let proposal = staged.get_mut(&vertex_id).ok_or_else(|| {
                anyhow::anyhow!("Unknown staged checkpoint {}", hex::encode(vertex_id))
            })?;
            proposal
                .checkpoint
                .add_signature(vote.authority, vote.signature);
            if proposal
                .checkpoint
                .verify_certificate(&self.authorities, &self.authority_public_keys)
                .is_err()
            {
                return Ok(None);
            }
        }
        self.finalize_staged_checkpoint(vertex_id).map(Some)
    }

"""
    if marker not in s:
        raise RuntimeError("consensus methods insertion marker missing")
    s = s.replace(marker, methods + marker, 1)
save(path, s)

# Reject uncertified checkpoints in every apply/sync path.
path = "crates/kanari-core/src/engine/apply_checkpoint.rs"
s = load(path)
s = replace_required(
    s,
    "    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {\n        Self::ensure_non_empty_committed_checkpoint(&checkpoint)?;",
    "    pub fn apply_checkpoint(&self, checkpoint: Checkpoint) -> Result<()> {\n        checkpoint.verify_certificate(&self.authorities, &self.consensus_public_keys)?;\n        Self::ensure_non_empty_committed_checkpoint(&checkpoint)?;",
    "apply checkpoint certificate verification",
)
save(path, s)

path = "crates/kanari-core/src/engine/queries.rs"
s = load(path)
s = replace_required(
    s,
    "        let checkpoint_to_apply = checkpoint.clone();",
    "        checkpoint.verify_certificate(&self.authorities, &self.consensus_public_keys)?;\n\n        let checkpoint_to_apply = checkpoint.clone();",
    "sync certificate verification",
)
save(path, s)

print("quorum checkpoint remediation applied or already present")
