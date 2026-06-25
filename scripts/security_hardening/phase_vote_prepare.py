from .common import read, write


def apply():
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    marker = '''    pub fn consensus(&self) -> Arc<RwLock<CoreDagConsensus>> {
        self.consensus.clone()
    }
'''
    methods = '''    fn checkpoint_quorum(&self) -> usize {
        self.authority_public_keys.len().saturating_mul(2) / 3 + 1
    }

    fn enqueue_committed_subdags(&self, subdags: Vec<Vec<VertexId>>) {
        let consensus = self.consensus.read().unwrap_or_else(|error| error.into_inner());
        let staged: HashSet<VertexId> = self
            .staged_checkpoints
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .flat_map(|draft| draft.checkpoint.vertices.iter().copied())
            .collect();
        let mut queue = self.committed_anchors.write().unwrap_or_else(|error| error.into_inner());
        for vertices in subdags {
            let Some(anchor) = vertices.first().copied() else { continue; };
            let finalized = consensus
                .checkpoints
                .iter()
                .any(|checkpoint| checkpoint.vertices.first() == Some(&anchor));
            let queued = queue.iter().any(|(known, _)| *known == anchor);
            if !finalized && !queued && !staged.contains(&anchor) {
                queue.push_back((anchor, vertices));
            }
        }
    }

    fn prepare_next_committed_anchor(&self) -> Result<Option<CheckpointVote>> {
        if !self
            .staged_checkpoints
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
        {
            return Ok(None);
        }
        loop {
            let Some((anchor, mut support)) = self
                .committed_anchors
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front()
            else {
                return Ok(None);
            };
            support.sort();
            support.dedup();
            support.retain(|id| *id != anchor);
            support.insert(0, anchor);
            let vertex = {
                let consensus = self.consensus.read().unwrap_or_else(|error| error.into_inner());
                consensus
                    .vertices
                    .iter()
                    .find(|vertex| vertex.id == anchor)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!(
                        "Committed Mysticeti anchor {} has no Kanari vertex",
                        hex::encode(anchor)
                    ))?
            };
            if vertex.transactions.is_empty() {
                continue;
            }
            let (sequence, previous_hash, previous_timestamp) = {
                let chain = self.engine.blockchain.read().unwrap_or_else(|error| error.into_inner());
                (
                    chain.height().saturating_add(1),
                    chain.latest_checkpoint().hash()?,
                    chain.latest_checkpoint().timestamp,
                )
            };
            anyhow::ensure!(vertex.timestamp > previous_timestamp, "Committed anchor timestamp is not monotonic");
            let mut checkpoint = Checkpoint::new(
                sequence,
                support,
                vertex.transactions.clone(),
                Vec::new(),
                vertex.timestamp,
                previous_hash,
            );
            let (root, verified_state, to_execute, receipts) =
                self.engine.prepare_checkpoint_state(&checkpoint)?;
            anyhow::ensure!(
                root == vertex.metadata.state_root,
                "Committed anchor root differs from deterministic execution"
            );
            checkpoint.state_root = root;
            let vote = CheckpointVote::new(
                checkpoint.clone(),
                0,
                vertex.round,
                self.authority_id.clone(),
                &self.local_signing_key,
                &self.authority_public_keys,
            )?;
            let checkpoint_id = vote.checkpoint_id;
            self.staged_checkpoints
                .write()
                .unwrap_or_else(|error| error.into_inner())
                .insert(checkpoint_id, StagedCheckpoint {
                    checkpoint,
                    verified_state,
                    to_execute,
                    receipts,
                    validate_supply: true,
                    epoch: 0,
                    round: vertex.round,
                });
            return Ok(Some(vote));
        }
    }

'''
    if marker not in text:
        raise RuntimeError("DagEngine method insertion point not found")
    write(path, text.replace(marker, methods + marker, 1))
