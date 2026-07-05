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


# Production result exposes staged proposal for network voting.
path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
s = load(path)
s = replace_required(
    s,
    "    pub checkpoint: Option<CheckpointInfo>,\n    pub vertex: Option<DagVertex>,",
    "    pub checkpoint: Option<CheckpointInfo>,\n    pub checkpoint_proposal: Option<Checkpoint>,\n    pub vertex: Option<DagVertex>,",
    "production proposal field",
)
s = s.replace(
    "checkpoint: None,\n                vertex:",
    "checkpoint: None,\n                checkpoint_proposal: None,\n                vertex:",
)
s = replace_required(
    s,
    "            checkpoint: checkpoint_info,\n            vertex: Some(vertex),",
    "            checkpoint: checkpoint_info,\n            checkpoint_proposal: Some(proposal),\n            vertex: Some(vertex),",
    "production proposal return",
)
save(path, s)

# Engine wrappers for validator voting and proposer vote collection.
path = "crates/kanari-core/src/engine.rs"
s = load(path)
s = s.replace(
    "Checkpoint, DagMetrics, DagProductionPolicy, DagVertex, PersistentDagState,",
    "Checkpoint, CheckpointSignature, DagMetrics, DagProductionPolicy, DagVertex, PersistentDagState,",
)
marker = "    pub fn produce_checkpoint(&self) -> Result<CheckpointProductionInfo> {"
if "pub fn sign_checkpoint_proposal(" not in s:
    methods = """    pub fn sign_checkpoint_proposal(
        &self,
        checkpoint: &Checkpoint,
    ) -> Result<CheckpointSignature> {
        self.dag_engine_instance()?.sign_checkpoint_proposal(checkpoint)
    }

    pub fn add_checkpoint_vote(
        &self,
        vertex_id: [u8; 32],
        vote: CheckpointSignature,
    ) -> Result<Option<Checkpoint>> {
        self.dag_engine_instance()?.add_checkpoint_vote(vertex_id, vote)
    }

"""
    if marker not in s:
        raise RuntimeError("engine checkpoint wrapper marker missing")
    s = s.replace(marker, methods + marker, 1)
save(path, s)

# P2P wire messages.
path = "crates/kanari-node/src/p2p.rs"
s = load(path)
s = replace_required(
    s,
    "    NewDagVertex(String),   // Serialized DAG vertex for multi-node sync",
    "    NewDagVertex(String),   // Serialized DAG vertex for multi-node sync\n    CheckpointProposal(CheckpointProposalMsg),\n    CheckpointVote(CheckpointVoteMsg),",
    "checkpoint vote variants",
)
if "pub struct CheckpointProposalMsg" not in s:
    marker = "#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]\npub struct PeerInfoMsg"
    structs = """#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct CheckpointProposalMsg {
    pub vertex_id: String,
    pub proposer_peer_id: String,
    pub checkpoint_data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct CheckpointVoteMsg {
    pub vertex_id: String,
    pub proposer_peer_id: String,
    pub voter_peer_id: String,
    pub vote_data: String,
}

"""
    if marker not in s:
        raise RuntimeError("checkpoint vote struct marker missing")
    s = s.replace(marker, structs + marker, 1)

# The node hardening transformer may run before or after this transformer.
# Extend the real claimed-identity match only when it already exists; otherwise the
# node transformer below will create it with proposal/vote cases included by fixups.
if "fn claimed_identity_matches" in s and "P2PMessage::CheckpointProposal(message)" not in s:
    old = """            P2PMessage::PeerInfo(info) => info.peer_id == source,
            P2PMessage::DagVertexRebroadcast(message) => message.sender_peer_id == source,"""
    new = """            P2PMessage::PeerInfo(info) => info.peer_id == source,
            P2PMessage::CheckpointProposal(message) => message.proposer_peer_id == source,
            P2PMessage::CheckpointVote(message) => message.voter_peer_id == source,
            P2PMessage::DagVertexRebroadcast(message) => message.sender_peer_id == source,"""
    if old not in s:
        raise RuntimeError("claimed identity match extension marker missing")
    s = s.replace(old, new, 1)
save(path, s)

# Sync manager signs verified proposals and collects votes only for local proposals.
path = "crates/kanari-node/src/sync.rs"
s = load(path)
s = s.replace(
    "CheckpointRequestMsg, CheckpointResponseMsg, DagVertexMsg, DagVertexRequestMsg,\n    DagVertexResponseMsg, P2PMessage, PeerInfoMsg,",
    "CheckpointProposalMsg, CheckpointRequestMsg, CheckpointResponseMsg, CheckpointVoteMsg,\n    DagVertexMsg, DagVertexRequestMsg, DagVertexResponseMsg, P2PMessage, PeerInfoMsg,",
)
s = s.replace(
    "use kanari_core::{BlockchainEngine, CheckpointSyncData, DagVertex};",
    "use kanari_core::{BlockchainEngine, Checkpoint, CheckpointSignature, CheckpointSyncData, DagVertex};",
)
old = """            P2PMessage::NewDagVertex(vertex_data) => {
                info!("[P2P] Received NewDagVertex");
                self.handle_new_dag_vertex(vertex_data).await;
            }"""
new = """            P2PMessage::NewDagVertex(vertex_data) => {
                info!("[P2P] Received NewDagVertex");
                self.handle_new_dag_vertex(vertex_data).await;
            }
            P2PMessage::CheckpointProposal(proposal) => {
                self.handle_checkpoint_proposal(proposal).await;
            }
            P2PMessage::CheckpointVote(vote) => {
                self.handle_checkpoint_vote(vote).await;
            }"""
s = replace_required(s, old, new, "sync checkpoint vote match")
marker = "    fn parse_message<T: DeserializeOwned>(data: &str, context: &str) -> Option<T> {"
if "async fn handle_checkpoint_proposal(" not in s:
    methods = """    async fn handle_checkpoint_proposal(&self, proposal: CheckpointProposalMsg) {
        if proposal.proposer_peer_id == self.local_peer_id {
            return;
        }
        let Some(checkpoint) = Self::parse_message::<Checkpoint>(
            &proposal.checkpoint_data,
            "checkpoint proposal",
        ) else {
            return;
        };
        let vote = match self.engine.sign_checkpoint_proposal(&checkpoint) {
            Ok(vote) => vote,
            Err(error) => {
                warn!("Rejected checkpoint proposal {}: {}", checkpoint.sequence, error);
                return;
            }
        };
        let vote_data = match serde_json::to_string(&vote) {
            Ok(data) => data,
            Err(error) => {
                warn!("Failed to serialize checkpoint vote: {}", error);
                return;
            }
        };
        self.send_network_message(
            P2PMessage::CheckpointVote(CheckpointVoteMsg {
                vertex_id: proposal.vertex_id,
                proposer_peer_id: proposal.proposer_peer_id,
                voter_peer_id: self.local_peer_id.clone(),
                vote_data,
            }),
            "Failed to queue checkpoint vote",
        );
    }

    async fn handle_checkpoint_vote(&self, vote_message: CheckpointVoteMsg) {
        if vote_message.proposer_peer_id != self.local_peer_id {
            return;
        }
        let Some(vote) = Self::parse_message::<CheckpointSignature>(
            &vote_message.vote_data,
            "checkpoint vote",
        ) else {
            return;
        };
        let raw_vertex = match hex::decode(&vote_message.vertex_id) {
            Ok(bytes) if bytes.len() == 32 => bytes,
            _ => {
                warn!("Invalid checkpoint vote vertex id");
                return;
            }
        };
        let mut vertex_id = [0u8; 32];
        vertex_id.copy_from_slice(&raw_vertex);
        match self.engine.add_checkpoint_vote(vertex_id, vote) {
            Ok(Some(checkpoint)) => {
                let sync_data = CheckpointSyncData { checkpoint };
                match serde_json::to_string(&sync_data) {
                    Ok(data) => {
                        self.send_network_message(
                            P2PMessage::NewCheckpoint(data),
                            "Failed to queue certified checkpoint",
                        );
                    }
                    Err(error) => warn!("Failed to serialize certified checkpoint: {}", error),
                }
            }
            Ok(None) => {}
            Err(error) => warn!("Rejected checkpoint vote: {}", error),
        }
    }

"""
    if marker not in s:
        raise RuntimeError("sync vote method insertion marker missing")
    s = s.replace(marker, methods + marker, 1)
save(path, s)

# Node broadcasts staged proposal after local deterministic execution.
path = "crates/kanari-node/src/app.rs"
s = load(path)
s = s.replace(
    "AuthenticatedP2PMessage, P2PEventHandler, P2PMessage, P2PNetwork,",
    "AuthenticatedP2PMessage, CheckpointProposalMsg, P2PEventHandler, P2PMessage, P2PNetwork,",
)
marker = "                    if let Some(vertex) = block_info.vertex {"
if "block_info.checkpoint_proposal.as_ref()" not in s:
    broadcast = """                    if let Some(proposal) = block_info.checkpoint_proposal.as_ref() {
                        match serde_json::to_string(proposal) {
                            Ok(checkpoint_data) => {
                                queue_network_message(
                                    &network_tx,
                                    P2PMessage::CheckpointProposal(CheckpointProposalMsg {
                                        vertex_id: block_info.vertex_id.clone(),
                                        proposer_peer_id: peer_id.clone(),
                                        checkpoint_data,
                                    }),
                                    "Failed to queue checkpoint proposal",
                                );
                            }
                            Err(error) => tracing::warn!(
                                "Failed to serialize checkpoint proposal: {}",
                                error
                            ),
                        }
                    }

"""
    if marker not in s:
        raise RuntimeError("app proposal broadcast marker missing")
    s = s.replace(marker, broadcast + marker, 1)
save(path, s)

print("checkpoint quorum vote transport v2 applied or already present")
