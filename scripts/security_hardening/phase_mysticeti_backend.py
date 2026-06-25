from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    old_imports = '''    block::{
        BlockReference as MysticetiBlockReference, RoundNumber as MysticetiRound,
        transaction::Transaction as MysticetiTransaction,
    },
    committee::Committee as MysticetiCommittee,
    context::TokioCtx as MysticetiTokioCtx,
    core::{Core as MysticetiCore, block_handler::RealBlockHandler as MysticetiBlockHandler},
    crypto::CryptoEngine as MysticetiCryptoEngine,
    metrics::Metrics as MysticetiMetrics,
    storage::Storage as MysticetiStorage,
};'''
    new_imports = '''    block::{
        Block as MysticetiBlock, BlockReference as MysticetiBlockReference,
        RoundNumber as MysticetiRound, transaction::Transaction as MysticetiTransaction,
    },
    committee::Committee as MysticetiCommittee,
    consensus::Linearizer as MysticetiLinearizer,
    context::TokioCtx as MysticetiTokioCtx,
    core::{Core as MysticetiCore, block_handler::RealBlockHandler as MysticetiBlockHandler},
    crypto::{AsBytes as MysticetiAsBytes, CryptoEngine as MysticetiCryptoEngine},
    data::Data as MysticetiData,
    metrics::Metrics as MysticetiMetrics,
    storage::Storage as MysticetiStorage,
};'''
    if old_imports not in text:
        raise RuntimeError("Mysticeti imports not found")
    text = text.replace(old_imports, new_imports, 1)

    text = text.replace(
        '''    transaction_sender: mpsc::Sender<Vec<MysticetiTransaction>>,
    protocol: MysticetiRuntimeProtocol,
}''',
        '''    transaction_sender: mpsc::Sender<Vec<MysticetiTransaction>>,
    protocol: MysticetiRuntimeProtocol,
    linearizer: MysticetiLinearizer,
}''',
        1,
    )
    text = text.replace(
        '''            transaction_sender,
            protocol,
        }''',
        '''            transaction_sender,
            protocol,
            linearizer: MysticetiLinearizer::new(),
        }''',
        1,
    )
    old_advance = '''    fn try_advance(&mut self) {
        let committed = self.core.try_commit();
        if !committed.is_empty() {
            log::debug!("Mysticeti committed {} leader block(s)", committed.len());
        }
    }'''
    new_advance = '''    fn try_advance(&mut self) -> Vec<Vec<VertexId>> {
        let leaders = self.core.try_commit();
        if leaders.is_empty() {
            return Vec::new();
        }
        let subdags = self
            .linearizer
            .handle_commit(self.core.block_reader(), leaders);
        let committed_vertices = subdags
            .iter()
            .map(|subdag| {
                let anchor = mysticeti_reference_to_vertex_id(&subdag.anchor);
                let mut vertices = vec![anchor];
                vertices.extend(
                    subdag
                        .blocks
                        .iter()
                        .filter(|block| block.round() > 0 && block.reference() != &subdag.anchor)
                        .map(|block| mysticeti_reference_to_vertex_id(block.reference())),
                );
                vertices
            })
            .collect::<Vec<_>>();
        self.core.handle_committed_subdag(subdags);
        committed_vertices
    }'''
    if old_advance not in text:
        raise RuntimeError("Mysticeti advance method not found")
    text = text.replace(old_advance, new_advance, 1)

    old_summary = '''        let reference = *block.reference();
        Ok(Some(MysticetiBlockSummary {
            vertex_id: mysticeti_reference_to_vertex_id(&reference),
            round: block.round(),
            parents: block
                .includes()
                .iter()
                .map(mysticeti_reference_to_vertex_id)
                .collect(),
        }))'''
    new_summary = '''        let reference = *block.reference();
        let serialized_block = block.serialized_bytes().to_vec();
        let committed_subdags = self.try_advance();
        Ok(Some(MysticetiBlockSummary {
            vertex_id: mysticeti_reference_to_vertex_id(&reference),
            round: block.round(),
            parents: block
                .includes()
                .iter()
                .filter(|reference| reference.round > 0)
                .map(mysticeti_reference_to_vertex_id)
                .collect(),
            serialized_block,
            committed_subdags,
        }))'''
    if old_summary not in text:
        raise RuntimeError("Mysticeti proposal summary not found")
    text = text.replace(old_summary, new_summary, 1)

    marker = '''    fn propose_block(
        &mut self,
        transactions: &[SignedTransaction],
        timestamp_ms: u64,
    ) -> Result<Option<MysticetiBlockSummary>> {'''
    import_method = '''    fn import_block(
        &mut self,
        serialized_block: &[u8],
    ) -> Result<(MysticetiData<MysticetiBlock>, Vec<Vec<VertexId>>)> {
        anyhow::ensure!(!serialized_block.is_empty(), "missing serialized Mysticeti block");
        let block = MysticetiData::<MysticetiBlock>::from_bytes(
            minibytes::Bytes::from(serialized_block.to_vec()),
        )?;
        block
            .verify(
                &self._committee,
                self.core.quorum_threshold(),
                &self.core.verifier(),
            )
            .map_err(|error| anyhow::anyhow!("Mysticeti block verification failed: {error:?}"))?;
        let mut processed = self.core.add_blocks(vec![block]);
        anyhow::ensure!(processed.len() == 1, "Missing parent or rejected Mysticeti block");
        let imported = processed.remove(0);
        let committed = self.try_advance();
        Ok((imported, committed))
    }

'''
    if marker not in text:
        raise RuntimeError("Mysticeti propose marker not found")
    text = text.replace(marker, import_method + marker, 1)

    text = text.replace(
        '''struct MysticetiBlockSummary {
    vertex_id: VertexId,
    round: MysticetiRound,
    parents: Vec<VertexId>,
}''',
        '''struct MysticetiBlockSummary {
    vertex_id: VertexId,
    round: MysticetiRound,
    parents: Vec<VertexId>,
    serialized_block: Vec<u8>,
    committed_subdags: Vec<Vec<VertexId>>,
}''',
        1,
    )
    text = text.replace("        self.mysticeti.try_advance();", "        let _ = self.mysticeti.try_advance();", 1)
    write(path, text)
