from .common import read, write


def apply():
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    text = text.replace(
        '''        let expected_parent_round = vertex.round - 1;
        let mut parent_authors = HashSet::new();''',
        '''        let mut parent_authors = HashSet::new();''',
        1,
    )
    text = text.replace(
        '''            if parent.round != expected_parent_round {
                anyhow::bail!(
                    "Invalid parent round for DAG vertex {}: expected {}, got {}",
                    hex::encode(vertex.id),
                    expected_parent_round,
                    parent.round
                );
            }''',
        '''            if parent.round >= vertex.round {
                anyhow::bail!(
                    "Invalid parent round for DAG vertex {}: parent {} is not earlier than round {}",
                    hex::encode(vertex.id),
                    parent.round,
                    vertex.round
                );
            }''',
        1,
    )
    old = '''    pub fn add_network_vertex(&self, vertex: DagVertex) -> Result<()> {
        let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
        if consensus.known_vertex(&vertex.id) {
            return Ok(());
        }
        self.verify_full_network_vertex(&consensus, &vertex)?;
        info!(
            "[DAG v2 SYNC] Accepted network vertex {} round {} txs {}",
            hex::encode(vertex.id),
            vertex.round,
            vertex.transactions.len()
        );
        consensus.add_vertex(vertex)?;
        drop(consensus);
        self.persist_consensus_state()
    }'''
    new = '''    pub fn add_network_vertex(&self, vertex: DagVertex) -> Result<ConsensusUpdate> {
        let mut consensus = self.consensus.write().unwrap_or_else(|e| e.into_inner());
        if consensus.known_vertex(&vertex.id) {
            return Ok(ConsensusUpdate::default());
        }
        self.verify_full_network_vertex(&consensus, &vertex)?;
        let (block, committed_subdags) = consensus
            .mysticeti
            .import_block(&vertex.mysticeti_block)?;
        let expected_author = consensus
            .authorities
            .iter()
            .position(|authority| authority == &vertex.author)
            .ok_or_else(|| anyhow::anyhow!("Unknown Mysticeti vertex author"))?;
        anyhow::ensure!(block.author().index() == expected_author, "Mysticeti author index mismatch");
        anyhow::ensure!(block.round() == vertex.round, "Mysticeti round mismatch");
        anyhow::ensure!(
            mysticeti_reference_to_vertex_id(block.reference()) == vertex.id,
            "Mysticeti block digest mismatch"
        );
        let parents = block
            .includes()
            .iter()
            .filter(|reference| reference.round > 0)
            .map(mysticeti_reference_to_vertex_id)
            .collect::<Vec<_>>();
        anyhow::ensure!(parents == vertex.parents, "Mysticeti parent set mismatch");
        if vertex.transactions.is_empty() {
            anyhow::ensure!(
                block.transactions().is_empty(),
                "Empty Kanari vertex carries an unexpected Mysticeti transaction commitment"
            );
        } else {
            let expected = signed_tx_batch_to_mysticeti_transaction(
                vertex.transactions.as_ref(),
                vertex.timestamp,
            );
            anyhow::ensure!(block.transactions().len() == 1, "Mysticeti adapter block has invalid payload count");
            anyhow::ensure!(
                block.transactions()[0].as_bytes() == expected.as_bytes(),
                "Mysticeti adapter transaction commitment mismatch"
            );
        }
        info!(
            "[DAG v2 SYNC] Accepted authenticated Mysticeti vertex {} round {} txs {}",
            hex::encode(vertex.id),
            vertex.round,
            vertex.transactions.len()
        );
        consensus.add_vertex(vertex)?;
        drop(consensus);
        self.persist_consensus_state()?;
        self.process_committed_subdags(committed_subdags)
    }'''
    if old not in text:
        raise RuntimeError("network DAG vertex method not found")
    text = text.replace(old, new, 1)
    write(path, text)

    path = "crates/kanari-core/src/engine.rs"
    text = read(path)
    text = text.replace(
        '''    pub fn add_network_dag_vertex(&self, vertex: DagVertex) -> Result<()> {
        self.dag_engine_instance()?.add_network_vertex(vertex)
    }''',
        '''    pub fn add_network_dag_vertex(
        &self,
        vertex: DagVertex,
    ) -> Result<crate::engine::produce_dag_vertex::ConsensusUpdate> {
        self.dag_engine_instance()?.add_network_vertex(vertex)
    }

    pub fn submit_checkpoint_vote(
        &self,
        vote: crate::consensus::CheckpointVote,
    ) -> Result<crate::engine::produce_dag_vertex::ConsensusUpdate> {
        self.dag_engine_instance()?.submit_checkpoint_vote(vote)
    }''',
        1,
    )
    write(path, text)
