from .common import read, write


def apply():
    path = "crates/kanari-node/src/p2p.rs"
    text = read(path)
    text = text.replace(
        "    NewDagVertex(String),   // Serialized DAG vertex for multi-node sync",
        "    NewDagVertex(String),   // Serialized DAG vertex for multi-node sync\n    CheckpointVote(String), // Serialized vote for a committed checkpoint draft",
        1,
    )
    write(path, text)
