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
    write(path, text)
