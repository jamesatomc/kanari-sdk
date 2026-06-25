from __future__ import annotations

import re
from .common import read, write


def apply() -> None:
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)

    freeze = '''        let authority_count = self
            .consensus
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .authorities
            .len();
        anyhow::ensure!(
            authority_count == 1 || cfg!(test),
            "multi-validator checkpoint production is frozen until authenticated Mysticeti remote blocks and committed sub-DAG certificates are wired end-to-end"
        );
'''
    text = text.replace(freeze, "", 1)
    text = text.replace(
        '''        if tx_count == 0 {
            anyhow::bail!("No new transactions to checkpoint");
        }
''',
        "",
        1,
    )
    text = text.replace(
        "let (state_root, executed, failed, verified_state, to_execute, receipts, validate_supply) = {",
        "let (state_root, executed, failed, _verified_state, _to_execute, _receipts, _validate_supply) = {",
        1,
    )

    old = '''        let (vertex_id, round, parents) = mysticeti_block
            .map(|block| (block.vertex_id, block.round, block.parents))
            .unwrap_or((
                policy.parent_ids.first().copied().unwrap_or([0u8; 32]),
                policy.target_round,
                policy.parent_ids,
            ));'''
    new = '''        let block = mysticeti_block.ok_or_else(|| {
            anyhow::anyhow!("DAG_WAITING: Mysticeti threshold clock is not ready")
        })?;
        let vertex_id = block.vertex_id;
        let round = block.round;
        let parents = block.parents;
        let serialized_block = block.serialized_block;
        let committed_subdags = block.committed_subdags;'''
    if old not in text:
        raise RuntimeError("Mysticeti proposal unpack block not found")
    text = text.replace(old, new, 1)
    text = text.replace(
        '''        vertex.id = vertex_id;
        let signing_digest = vertex.signing_digest()?;''',
        '''        vertex.bind_mysticeti_block(serialized_block, vertex_id)?;
        let signing_digest = vertex.signing_digest()?;''',
        1,
    )

    old = '''        self.stage_locally_produced_vertex(
            &vertex,
            verified_state,
            to_execute,
            receipts,
            validate_supply,
        )?;
        let checkpoint = self.finalize_staged_checkpoint(vertex.id)?;
        let checkpoint_info = Some(CheckpointInfo {
            sequence: checkpoint.sequence,
            vertex_count: checkpoint.vertices.len(),
            tx_count: checkpoint.transactions.len(),
        });'''
    new = '''        {
            let mut consensus = self.consensus.write().unwrap_or_else(|error| error.into_inner());
            consensus.add_vertex(vertex.clone())?;
        }
        let update = self.process_committed_subdags(committed_subdags)?;
        let checkpoint_info = update.finalized_checkpoints.last().map(|checkpoint| CheckpointInfo {
            sequence: checkpoint.sequence,
            vertex_count: checkpoint.vertices.len(),
            tx_count: checkpoint.transactions.len(),
        });'''
    if old not in text:
        raise RuntimeError("Immediate checkpoint finalization block not found")
    text = text.replace(old, new, 1)
    text = text.replace(
        '''            checkpoint: checkpoint_info,
            vertex: Some(vertex),''',
        '''            checkpoint: checkpoint_info,
            checkpoint_votes: update.checkpoint_votes,
            vertex: Some(vertex),''',
        1,
    )

    pattern = re.compile(
        r"    fn stage_locally_produced_vertex\(.*?\n"
        r"    pub fn consensus\(&self\) -> Arc<RwLock<CoreDagConsensus>> \{",
        re.S,
    )
    text, count = pattern.subn(
        "    pub fn consensus(&self) -> Arc<RwLock<CoreDagConsensus>> {",
        text,
        count=1,
    )
    if count != 1:
        raise RuntimeError("Legacy staged-checkpoint methods not found")
    write(path, text)
