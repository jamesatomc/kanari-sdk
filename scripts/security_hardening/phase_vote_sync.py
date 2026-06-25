from .common import read, write


def apply():
    path = "crates/kanari-node/src/sync.rs"
    text = read(path)
    text = text.replace(
        "use kanari_core::{BlockchainEngine, CheckpointSyncData, DagVertex};",
        "use kanari_core::{BlockchainEngine, CheckpointSyncData, CheckpointVote, ConsensusUpdate, DagVertex};",
        1,
    )
    text = text.replace(
        '''            P2PMessage::NewDagVertex(vertex_data) => {
                info!("[P2P] Received NewDagVertex");
                self.handle_new_dag_vertex(vertex_data).await;
            }''',
        '''            P2PMessage::NewDagVertex(vertex_data) => {
                info!("[P2P] Received NewDagVertex");
                self.handle_new_dag_vertex(vertex_data).await;
            }
            P2PMessage::CheckpointVote(vote_data) => {
                info!("[P2P] Received CheckpointVote");
                self.handle_checkpoint_vote(vote_data).await;
            }''',
        1,
    )
    write(path, text)
