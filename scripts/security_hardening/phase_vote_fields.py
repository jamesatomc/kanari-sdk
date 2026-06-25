from .common import read, write


def apply():
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    text = text.replace(
        "use std::collections::{BTreeMap, BTreeSet, HashSet};",
        "use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};",
        1,
    )
    text = text.replace(
        "    Checkpoint, ConsensusRuntimeProtocol, DagMetrics, DagProductionPolicy, DagVertex,\n    PersistentDagState, VertexId,\n};",
        "    Checkpoint, CheckpointCertificate, CheckpointVote, ConsensusRuntimeProtocol,\n    DagMetrics, DagProductionPolicy, DagVertex, PersistentDagState, VertexId,\n};",
        1,
    )
    text = text.replace(
        "    pub checkpoint: Option<CheckpointInfo>,\n    pub vertex: Option<DagVertex>,",
        "    pub checkpoint: Option<CheckpointInfo>,\n    pub checkpoint_votes: Vec<CheckpointVote>,\n    pub vertex: Option<DagVertex>,",
        1,
    )
    text = text.replace(
        "    validate_supply: bool,\n}",
        "    validate_supply: bool,\n    epoch: u64,\n    round: u64,\n}\n\n#[derive(Debug, Clone, Default)]\npub struct ConsensusUpdate {\n    pub checkpoint_votes: Vec<CheckpointVote>,\n    pub finalized_checkpoints: Vec<Checkpoint>,\n}",
        1,
    )
    text = text.replace(
        "    staged_checkpoints: Arc<RwLock<BTreeMap<VertexId, StagedCheckpoint>>>,\n}",
        "    staged_checkpoints: Arc<RwLock<BTreeMap<VertexId, StagedCheckpoint>>>,\n    checkpoint_votes: Arc<RwLock<BTreeMap<VertexId, BTreeMap<String, Vec<u8>>>>>,\n    committed_anchors: Arc<RwLock<VecDeque<(VertexId, Vec<VertexId>)>>>,\n}",
        1,
    )
    text = text.replace(
        "            staged_checkpoints: Arc::new(RwLock::new(BTreeMap::new())),\n        };",
        "            staged_checkpoints: Arc::new(RwLock::new(BTreeMap::new())),\n            checkpoint_votes: Arc::new(RwLock::new(BTreeMap::new())),\n            committed_anchors: Arc::new(RwLock::new(VecDeque::new())),\n        };",
        1,
    )
    write(path, text)
