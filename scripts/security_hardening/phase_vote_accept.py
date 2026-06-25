from .common import read, write


def apply():
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    marker = '''    pub fn consensus(&self) -> Arc<RwLock<CoreDagConsensus>> {
        self.consensus.clone()
    }
'''
    methods = '''    fn accept_checkpoint_vote(&self, vote: CheckpointVote) -> Result<ConsensusUpdate> {
        anyhow::ensure!(
            self.authority_public_keys.contains_key(&vote.authority),
            "Checkpoint vote is from a non-committee authority"
        );
        let checkpoint_id = vote.checkpoint_id;
        {
            let staged = self
                .staged_checkpoints
                .read()
                .unwrap_or_else(|error| error.into_inner());
            let expected = staged
                .get(&checkpoint_id)
                .ok_or_else(|| anyhow::anyhow!("Checkpoint vote has no locally committed draft"))?;
            anyhow::ensure!(
                expected.epoch == vote.epoch && expected.round == vote.round,
                "Checkpoint vote epoch or round mismatch"
            );
            vote.verify_for_checkpoint(&expected.checkpoint, &self.authority_public_keys)?;
        }
        let vote_count = {
            let mut all_votes = self
                .checkpoint_votes
                .write()
                .unwrap_or_else(|error| error.into_inner());
            let votes = all_votes.entry(checkpoint_id).or_default();
            if let Some(existing) = votes.get(&vote.authority) {
                anyhow::ensure!(existing == &vote.signature, "Conflicting checkpoint vote");
            } else {
                votes.insert(vote.authority.clone(), vote.signature.clone());
            }
            votes.len()
        };
        if vote_count < self.checkpoint_quorum() {
            return Ok(ConsensusUpdate::default());
        }

        let signatures = self
            .checkpoint_votes
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&checkpoint_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(authority, signature)| crate::consensus::CheckpointAuthoritySignature {
                authority,
                signature,
            })
            .collect::<Vec<_>>();
        let mut staged = self
            .staged_checkpoints
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&checkpoint_id)
            .ok_or_else(|| anyhow::anyhow!("Certified checkpoint draft disappeared"))?;
        staged.checkpoint.certificate = Some(CheckpointCertificate {
            epoch: staged.epoch,
            round: staged.round,
            committee_digest: Checkpoint::committee_digest(&self.authority_public_keys)?,
            signatures,
        });
        staged
            .checkpoint
            .verify_certificate(&self.authority_public_keys, self.authority_public_keys.len())?;
        self.engine.apply_prepared_checkpoint(
            staged.checkpoint.clone(),
            staged.verified_state,
            staged.to_execute,
            staged.receipts,
            staged.validate_supply,
        )?;
        {
            let mut consensus = self.consensus.write().unwrap_or_else(|error| error.into_inner());
            consensus.record_checkpoint(staged.checkpoint.clone());
        }
        self.persist_consensus_state()?;
        let mut update = ConsensusUpdate {
            checkpoint_votes: Vec::new(),
            finalized_checkpoints: vec![staged.checkpoint],
        };
        if let Some(next_vote) = self.prepare_next_committed_anchor()? {
            let next_update = self.accept_checkpoint_vote(next_vote.clone())?;
            update.checkpoint_votes.push(next_vote);
            update.checkpoint_votes.extend(next_update.checkpoint_votes);
            update.finalized_checkpoints.extend(next_update.finalized_checkpoints);
        }
        Ok(update)
    }

    fn process_committed_subdags(&self, subdags: Vec<Vec<VertexId>>) -> Result<ConsensusUpdate> {
        self.enqueue_committed_subdags(subdags);
        let Some(vote) = self.prepare_next_committed_anchor()? else {
            return Ok(ConsensusUpdate::default());
        };
        let mut update = self.accept_checkpoint_vote(vote.clone())?;
        update.checkpoint_votes.insert(0, vote);
        Ok(update)
    }

    pub fn submit_checkpoint_vote(&self, vote: CheckpointVote) -> Result<ConsensusUpdate> {
        self.accept_checkpoint_vote(vote)
    }

'''
    if marker not in text:
        raise RuntimeError("DagEngine vote method insertion point not found")
    write(path, text.replace(marker, methods + marker, 1))
