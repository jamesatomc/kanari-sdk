// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;

type CommitVertexSelection = (Arc<DagVertex>, Vec<VertexId>, Vec<SignedTransaction>);

pub(crate) const CANONICAL_CHECKPOINT_TIMESTAMP_EPOCH_MS: u64 = 4_102_444_800_000;

pub(crate) fn canonical_checkpoint_timestamp(sequence: u64) -> u64 {
    CANONICAL_CHECKPOINT_TIMESTAMP_EPOCH_MS.saturating_add(sequence)
}

fn canonical_checkpoint_tx_key(tx: &SignedTransaction) -> (String, u64, Vec<u8>) {
    (
        tx.transaction.sender_address().to_ascii_lowercase(),
        tx.transaction.sequence_number(),
        logical_tx_hash(tx),
    )
}

fn sort_checkpoint_transactions(transactions: &mut [SignedTransaction]) {
    transactions.sort_by_key(canonical_checkpoint_tx_key);
}

impl DagStore {
    fn same_checkpoint_payload_except_state_root(
        existing: &Checkpoint,
        candidate: &Checkpoint,
    ) -> bool {
        let existing_tx_hashes: Vec<_> =
            existing.transactions.iter().map(logical_tx_hash).collect();
        let candidate_tx_hashes: Vec<_> =
            candidate.transactions.iter().map(logical_tx_hash).collect();

        existing.sequence == candidate.sequence
            && existing.vertices == candidate.vertices
            && existing_tx_hashes == candidate_tx_hashes
            && existing.timestamp == candidate.timestamp
            && existing.prev_checkpoint_hash == candidate.prev_checkpoint_hash
    }

    fn validate_checkpoint_payload(&self, checkpoint: &Checkpoint) -> Result<()> {
        if checkpoint.vertices.len() == 1 && self.executed_tx_hashes.is_empty() {
            let vertex_id = checkpoint.vertices[0];
            let vertex = self.vertices.get(&vertex_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "Checkpoint references missing vertex {}",
                    hex::encode(vertex_id)
                )
            })?;

            let mut expected_transactions = vertex.transactions.clone();
            sort_checkpoint_transactions(&mut expected_transactions);
            let same_transactions = expected_transactions.len() == checkpoint.transactions.len()
                && expected_transactions
                    .iter()
                    .zip(checkpoint.transactions.iter())
                    .all(|(expected, actual)| {
                        expected.transaction_hash() == actual.transaction_hash()
                    });

            if !same_transactions {
                anyhow::bail!("Checkpoint transactions do not match referenced DAG vertices");
            }

            return Ok(());
        }

        let mut seen_vertices = HashSet::new();
        let mut seen_tx_hashes = HashSet::new();
        let mut expected_transactions = Vec::new();
        for vertex_id in &checkpoint.vertices {
            if !seen_vertices.insert(*vertex_id) {
                anyhow::bail!(
                    "Checkpoint contains duplicate vertex {}",
                    hex::encode(vertex_id)
                );
            }

            let vertex = self.vertices.get(vertex_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "Checkpoint references missing vertex {}",
                    hex::encode(vertex_id)
                )
            })?;

            for tx in &vertex.transactions {
                let tx_hash = logical_tx_hash(tx);
                if self.executed_tx_hashes.contains(&tx_hash) {
                    continue;
                }
                if seen_tx_hashes.insert(tx_hash.clone()) {
                    expected_transactions.push(tx.clone());
                }
            }
        }

        sort_checkpoint_transactions(&mut expected_transactions);
        let expected_tx_hashes: Vec<Vec<u8>> =
            expected_transactions.iter().map(logical_tx_hash).collect();
        let actual_tx_hashes: Vec<Vec<u8>> = checkpoint
            .transactions
            .iter()
            .map(logical_tx_hash)
            .collect();
        if actual_tx_hashes != expected_tx_hashes {
            anyhow::bail!("Checkpoint transactions do not match referenced DAG vertices");
        }

        Ok(())
    }

    pub fn should_create_checkpoint(&self) -> bool {
        let rounds_since_last = self
            .current_round
            .saturating_sub(self.last_checkpoint_round);
        let pending_count = self.pending_vertices.len();

        if rounds_since_last >= self.checkpoint_config.max_rounds {
            return true;
        }

        if rounds_since_last < self.checkpoint_config.min_rounds {
            return false;
        }

        if pending_count >= self.checkpoint_config.max_vertices {
            return true;
        }

        pending_count >= self.checkpoint_config.min_vertices
    }

    pub fn get_checkpoint_config(&self) -> &CheckpointConfig {
        &self.checkpoint_config
    }

    pub fn is_vertex_checkpointed(&self, id: &VertexId) -> bool {
        self.vertex_checkpoint_map.contains_key(id)
    }

    pub fn set_checkpoint_config(&mut self, config: CheckpointConfig) -> Result<()> {
        config.validate()?;
        self.checkpoint_config = config;
        Ok(())
    }

    pub fn latest_checkpoint(&self) -> Checkpoint {
        self.checkpoints
            .back()
            .cloned()
            .unwrap_or_else(Checkpoint::genesis)
    }

    pub fn last_checkpoint_round(&self) -> Round {
        self.last_checkpoint_round
    }

    pub fn get_checkpoint(&self, sequence: u64) -> Option<&Checkpoint> {
        self.checkpoints.iter().find(|cp| cp.sequence == sequence)
    }

    pub fn add_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<()> {
        let latest = self.latest_checkpoint();
        let expected_seq = latest.sequence + 1;

        if checkpoint.sequence != expected_seq {
            if checkpoint.sequence == latest.sequence {
                let checkpoint_hash = checkpoint.hash()?;
                let latest_hash = latest.hash()?;
                if checkpoint_hash == latest_hash {
                    return Ok(());
                }
                if Self::same_checkpoint_payload_except_state_root(&latest, &checkpoint) {
                    if let Some(latest_checkpoint) = self.checkpoints.back_mut() {
                        *latest_checkpoint = checkpoint;
                    }
                    return Ok(());
                }
            }
            anyhow::bail!(
                "Invalid checkpoint sequence: expected {}, got {}",
                expected_seq,
                checkpoint.sequence
            );
        }

        let prev_hash = latest.hash()?;
        if checkpoint.prev_checkpoint_hash != prev_hash {
            anyhow::bail!("Invalid previous checkpoint hash");
        }

        self.validate_checkpoint_payload(&checkpoint)?;

        for tx in &checkpoint.transactions {
            self.executed_tx_hashes.insert(logical_tx_hash(tx));
        }

        for vertex_id in &checkpoint.vertices {
            self.vertex_checkpoint_map
                .insert(*vertex_id, checkpoint.sequence);
        }

        let checkpoint_vertices_set: HashSet<_> = checkpoint.vertices.iter().collect();
        self.pending_vertices
            .retain(|id| !checkpoint_vertices_set.contains(id));

        self.last_checkpoint_round = self.current_round;
        self.checkpoints.push_back(checkpoint.clone());

        if checkpoint.sequence > 10 {
            let cutoff_seq = checkpoint.sequence.saturating_sub(10);
            let vertices_to_remove: Vec<VertexId> = self
                .vertex_checkpoint_map
                .iter()
                .filter(|&(_, &seq)| seq <= cutoff_seq)
                .map(|(id, _)| *id)
                .collect();

            for vertex_id in vertices_to_remove {
                self.remove_vertex_fully(&vertex_id);
                self.vertex_checkpoint_map.remove(&vertex_id);
            }
        }

        const MAX_RETAIN_ROUNDS: u64 = 100;
        if self.current_round > MAX_RETAIN_ROUNDS {
            let cutoff_round = self.current_round.saturating_sub(MAX_RETAIN_ROUNDS);
            let orphaned_vertices: Vec<VertexId> = self
                .vertices
                .iter()
                .filter(|(_, v)| v.round < cutoff_round)
                .map(|(id, _)| *id)
                .collect();

            for id in orphaned_vertices {
                self.remove_vertex_fully(&id);
            }
        }

        const TX_RETENTION_WINDOW: usize = 10_000;
        if self.checkpoints.len() > TX_RETENTION_WINDOW
            && let Some(old_checkpoint) = self.checkpoints.pop_front()
        {
            for tx in &old_checkpoint.transactions {
                self.executed_tx_hashes.remove(&logical_tx_hash(tx));
            }
        }

        Ok(())
    }

    pub fn get_checkpoint_stats(&self) -> CheckpointStats {
        CheckpointStats {
            pending_vertices: self.pending_vertices.len(),
            rounds_since_last: self
                .current_round
                .saturating_sub(self.last_checkpoint_round),
            total_checkpoints: self.checkpoints.len(),
            should_checkpoint: self.should_create_checkpoint(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CheckpointStats {
    pub pending_vertices: usize,
    pub rounds_since_last: u64,
    pub total_checkpoints: usize,
    pub should_checkpoint: bool,
}

impl DagConsensus {
    pub(crate) fn mysticeti_commit_leaders(&self, commit_round: Round) -> Vec<AuthorityId> {
        let mut authorities: Vec<_> = self.committee.validators.keys().cloned().collect();
        authorities.sort();
        if authorities.is_empty() {
            return Vec::new();
        }

        let leader_count = self.protocol.leader_count.get().min(authorities.len());
        let start = (commit_round as usize) % authorities.len();
        (0..leader_count)
            .map(|offset| authorities[(start + offset) % authorities.len()].clone())
            .collect()
    }

    pub(crate) fn select_commit_vertex(
        &self,
        commit_round: u64,
        preferred_leader_id: &AuthorityId,
    ) -> Result<Option<CommitVertexSelection>> {
        let quorum = self.quorum_threshold();

        let Some(leader_vertex) = self
            .store
            .get_vertices_in_round(commit_round)
            .into_iter()
            .filter(|vertex| vertex.author == *preferred_leader_id)
            .min_by_key(|vertex| vertex.id)
        else {
            tracing::debug!(
                "[Consensus] Waiting for leader {} in round {}",
                preferred_leader_id,
                commit_round
            );
            return Ok(None);
        };

        let decision_round = commit_round + self.protocol.decision_depth();
        let decision_round_vertices = self.store.get_vertices_in_round(decision_round);
        let trusted_support_count = decision_round_vertices
            .iter()
            .filter(|decision_vertex| {
                self.vertex_reaches_target(decision_vertex.id, leader_vertex.id)
            })
            .map(|next_vertex| &next_vertex.author)
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|auth| self.store.is_authority_trusted(auth))
            .count();

        if trusted_support_count < quorum {
            tracing::debug!(
                "[Consensus] Waiting for Mysticeti decision quorum on leader {} in round {} via decision round {}: support {}/{}",
                preferred_leader_id,
                commit_round,
                decision_round,
                trusted_support_count,
                quorum
            );
            return Ok(None);
        }

        let vertices_to_commit = self.collect_vertices_to_commit(leader_vertex.id)?;
        let transactions = self.collect_checkpoint_transactions(&vertices_to_commit);

        Ok(Some((leader_vertex, vertices_to_commit, transactions)))
    }

    fn vertex_reaches_target(&self, root: VertexId, target: VertexId) -> bool {
        let mut stack = vec![root];
        let mut visited = HashSet::new();

        while let Some(vertex_id) = stack.pop() {
            if vertex_id == target {
                return true;
            }
            if !visited.insert(vertex_id) {
                continue;
            }

            if let Some(vertex) = self.store.get_vertex(&vertex_id) {
                stack.extend(vertex.parents.iter().copied());
            }
        }

        false
    }

    fn build_checkpoint_for_commit_selections(
        &self,
        commit_round: Round,
        selections: Vec<(AuthorityId, CommitVertexSelection)>,
    ) -> Result<Checkpoint> {
        let mut seen_vertices = HashSet::new();
        let mut vertices_to_commit = Vec::new();
        let mut committed_leaders = Vec::new();

        for (leader_id, (_commit_vertex, leader_vertices, _)) in selections {
            committed_leaders.push(leader_id);
            for vertex_id in leader_vertices {
                if seen_vertices.insert(vertex_id) {
                    vertices_to_commit.push(vertex_id);
                }
            }
        }

        let all_transactions = self.collect_checkpoint_transactions(&vertices_to_commit);
        let latest = self.store.latest_checkpoint();
        let prev_hash = latest.hash()?;
        let sequence = latest.sequence + 1;
        let timestamp = canonical_checkpoint_timestamp(sequence);
        let state_root = self.checkpoint_state_root(&vertices_to_commit, &all_transactions)?;
        let checkpoint = Checkpoint::new(
            sequence,
            vertices_to_commit,
            all_transactions,
            state_root,
            timestamp,
            prev_hash,
        );

        if tracing::enabled!(tracing::Level::INFO) {
            let tx_hashes: Vec<Vec<u8>> = checkpoint
                .transactions
                .iter()
                .map(logical_tx_hash)
                .collect();
            let tx_digest = hash_data_blake3(&bcs::to_bytes(&tx_hashes)?);
            tracing::info!(
                "[Consensus] Built checkpoint seq={} commit_round={} leaders={:?} vertices={} txs={} tx_digest={} prev={} provisional_root={}",
                checkpoint.sequence,
                commit_round,
                committed_leaders,
                checkpoint.vertices.len(),
                checkpoint.transactions.len(),
                hex::encode(tx_digest),
                hex::encode(&checkpoint.prev_checkpoint_hash),
                hex::encode(&checkpoint.state_root)
            );
        }

        Ok(checkpoint)
    }

    pub(crate) fn collect_checkpoint_transactions(
        &self,
        vertices_to_commit: &[VertexId],
    ) -> Vec<SignedTransaction> {
        let mut seen_tx_hashes = HashSet::new();
        let mut all_transactions = Vec::new();

        for vertex_id in vertices_to_commit {
            if let Some(vertex) = self.store.get_vertex(vertex_id) {
                for tx in &vertex.transactions {
                    let tx_hash = logical_tx_hash(tx);
                    if self.store.executed_tx_hashes.contains(&tx_hash) {
                        continue;
                    }
                    if seen_tx_hashes.insert(tx_hash) {
                        all_transactions.push(tx.clone());
                    }
                }
            }
        }

        sort_checkpoint_transactions(&mut all_transactions);
        all_transactions
    }

    pub(crate) fn checkpoint_state_root(
        &self,
        _vertices_to_commit: &[VertexId],
        checkpoint_transactions: &[SignedTransaction],
    ) -> Result<Vec<u8>> {
        // DAG vertices carry speculative execution roots that may depend on a broader
        // parent ancestry than the transaction set that will ultimately be committed in
        // this checkpoint. The canonical checkpoint root must therefore be derived by the
        // execution engine when it replays `checkpoint.transactions` against the canonical
        // pre-checkpoint state.
        //
        // Until that canonical root is computed, consensus should advertise only the
        // latest finalized checkpoint root as a provisional placeholder instead of
        // reusing a vertex-local speculative root.
        
        // Compute the canonical state root by replaying checkpoint transactions
        // against the latest checkpoint state
        let latest_checkpoint = self.store.latest_checkpoint();
        let state_snapshot = self.execution_engine.state_read().clone();
        let state_arc = Arc::new(RwLock::new(state_snapshot));
        
        // Filter out already executed transactions
        let to_execute: Vec<SignedTransaction> = checkpoint_transactions
            .iter()
            .filter(|tx| !self.store.executed_tx_hashes.contains(&logical_tx_hash(tx)))
            .cloned()
            .collect();
        
        // Apply system prologue if there are transactions to execute
        if !to_execute.is_empty() {
            let timestamp = canonical_checkpoint_timestamp(latest_checkpoint.sequence + 1);
            self.execution_engine.apply_system_prologue_to_state(
                &state_arc,
                timestamp,
                false,
            )?;
            
            // Execute transactions without persisting objects
            self.execution_engine.execute_tx_waves_strict_serial(
                to_execute,
                &state_arc,
                Some(timestamp),
                false,
            )?;
        }
        
        // Compute the new state root
        let verified_state = state_arc.read().unwrap_or_else(|e| e.into_inner()).clone();
        let computed_root = verified_state.compute_state_root();
        Ok(computed_root)
    }

    pub fn try_commit(&mut self) -> Result<Option<Checkpoint>> {
        let current_round = self.store.current_round();
        tracing::debug!(
            "[DAG Consensus] try_commit: current_round = {}",
            current_round
        );

        let decision_depth = self.protocol.decision_depth();
        if current_round <= decision_depth {
            return Ok(None);
        }

        let mut start_round = self.store.last_checkpoint_round() + 1;
        let max_commit_round = current_round.saturating_sub(decision_depth);

        if start_round > max_commit_round {
            return Ok(None);
        }

        tracing::info!(
            "[DAG Consensus] Catching up on missed rounds ({} to {})",
            start_round,
            max_commit_round
        );

        while start_round <= max_commit_round {
            let commit_round = start_round;

            let leader_ids = self.mysticeti_commit_leaders(commit_round);
            if leader_ids.is_empty() {
                tracing::warn!(
                    "[DAG Consensus] Empty committee at round {}, skipping",
                    commit_round
                );
                start_round += 1;
                continue;
            }

            let mut selections = Vec::new();

            for leader_id in leader_ids {
                let Some((commit_vertex, vertices_to_commit, all_transactions)) =
                    self.select_commit_vertex(commit_round, &leader_id)?
                else {
                    continue;
                };

                let total_authorities = self.committee.validators.len();
                let quorum = self.quorum_threshold();
                let support_count = self
                    .store
                    .get_vertices_in_round(commit_round + self.protocol.decision_depth())
                    .iter()
                    .filter(|v| self.vertex_reaches_target(v.id, commit_vertex.id))
                    .map(|v| &v.author)
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .filter(|auth| self.store.is_authority_trusted(auth))
                    .count();

                tracing::info!(
                    "[Consensus] Quorum reached! Support: {} / {} (threshold: {}), commit_author={}, txs={}",
                    support_count,
                    total_authorities,
                    quorum,
                    commit_vertex.author,
                    all_transactions.len()
                );

                selections.push((
                    leader_id,
                    (commit_vertex, vertices_to_commit, all_transactions),
                ));
            }

            if !selections.is_empty() {
                return Ok(Some(self.build_checkpoint_for_commit_selections(
                    commit_round,
                    selections,
                )?));
            }

            start_round += 1;
        }

        Ok(None)
    }

    pub fn add_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<()> {
        self.store.add_checkpoint(checkpoint)
    }

    pub fn latest_checkpoint(&self) -> Checkpoint {
        self.store.latest_checkpoint()
    }
}
