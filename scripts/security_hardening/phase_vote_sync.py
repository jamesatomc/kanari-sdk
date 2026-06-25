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
    marker = '''    async fn handle_new_dag_vertex(&self, vertex_data: String) {'''
    helpers = '''    fn broadcast_consensus_update(&self, update: ConsensusUpdate) {
        for vote in update.checkpoint_votes {
            match serde_json::to_string(&vote) {
                Ok(data) => {
                    self.send_network_message(
                        P2PMessage::CheckpointVote(data),
                        "[CONSENSUS] Failed to queue checkpoint vote",
                    );
                }
                Err(error) => warn!("[CONSENSUS] Failed to serialize checkpoint vote: {}", error),
            }
        }
        for checkpoint in update.finalized_checkpoints {
            if let Some(checkpoint_data) = self.engine.get_checkpoint_sync(checkpoint.sequence) {
                match serde_json::to_string(&checkpoint_data) {
                    Ok(data) => {
                        self.send_network_message(
                            P2PMessage::NewCheckpoint(data),
                            "[CONSENSUS] Failed to queue certified checkpoint",
                        );
                    }
                    Err(error) => warn!("[CONSENSUS] Failed to serialize certified checkpoint: {}", error),
                }
            }
        }
    }

    async fn handle_checkpoint_vote(&self, vote_data: String) {
        let Some(vote) = Self::parse_message::<CheckpointVote>(&vote_data, "checkpoint vote") else {
            return;
        };
        match self.engine.submit_checkpoint_vote(vote) {
            Ok(update) => self.broadcast_consensus_update(update),
            Err(error) => warn!("[CONSENSUS] Rejected checkpoint vote: {}", error),
        }
    }

'''
    if marker not in text:
        raise RuntimeError("DAG vertex handler marker not found")
    text = text.replace(marker, helpers + marker, 1)
    text = text.replace(
        '''            match self.engine.add_network_dag_vertex(vertex.clone()) {
                Ok(()) => {
                    info!(
                        "Successfully added DAG vertex {} to local consensus",
                        hex::encode(vertex.id)
                    );
                    self.retry_buffered_dag_vertices();
                }''',
        '''            match self.engine.add_network_dag_vertex(vertex.clone()) {
                Ok(update) => {
                    info!(
                        "Successfully added DAG vertex {} to local consensus",
                        hex::encode(vertex.id)
                    );
                    self.broadcast_consensus_update(update);
                    self.retry_buffered_dag_vertices();
                }''',
        1,
    )
    text = text.replace(
        '''            match self.engine.add_network_dag_vertex(vertex.clone()) {
                Ok(()) => {
                    info!(
                        "[DAG SYNC] Applied buffered DAG vertex {} (round {})",
                        vertex_id, vertex.round
                    );
                }''',
        '''            match self.engine.add_network_dag_vertex(vertex.clone()) {
                Ok(update) => {
                    info!(
                        "[DAG SYNC] Applied buffered DAG vertex {} (round {})",
                        vertex_id, vertex.round
                    );
                    self.broadcast_consensus_update(update);
                }''',
        1,
    )
    write(path, text)
