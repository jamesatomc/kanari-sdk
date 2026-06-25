from .common import read, write


def apply():
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    marker = '''    pub fn metrics(&self) -> &DagMetrics {
        &self.metrics
    }
'''
    addition = marker + '''
    fn needs_progress(&self) -> bool {
        self.current_round > self.last_checkpoint_round
    }
'''
    if marker not in text:
        raise RuntimeError("CoreDagConsensus metrics marker not found")
    text = text.replace(marker, addition, 1)
    marker = '''    pub fn latest_own_vertices(&self, limit: usize) -> Vec<DagVertex> {
        let consensus = self.consensus.read().unwrap_or_else(|e| e.into_inner());
        consensus.latest_vertices_by_authority(&self.authority_id, limit)
    }
'''
    addition = marker + '''
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
'''
    if marker not in text:
        raise RuntimeError("DagEngine progress marker not found")
    write(path, text.replace(marker, addition, 1))

    path = "crates/kanari-core/src/engine.rs"
    text = read(path)
    text = text.replace(
        '''    pub fn produce_checkpoint(&self) -> Result<CheckpointProductionInfo> {
        let dag_engine = self.dag_engine_instance()?;
        let has_pending_transactions = self.pending_transaction_len() > 0;

        if !has_pending_transactions {
            anyhow::bail!("No new transactions to checkpoint");
        }

        dag_engine.produce_vertex()
    }''',
        '''    pub fn produce_checkpoint(&self) -> Result<CheckpointProductionInfo> {
        self.dag_engine_instance()?.produce_vertex()
    }

    pub fn dag_needs_progress(&self) -> Result<bool> {
        Ok(self.dag_engine_instance()?.needs_progress())
    }''',
        1,
    )
    write(path, text)
