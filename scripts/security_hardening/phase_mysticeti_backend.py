from __future__ import annotations

import re
from .common import read, write


def apply() -> None:
    path = "crates/kanari-core/src/engine/produce_dag_vertex.rs"
    text = read(path)
    text = text.replace(
        '''    block::Transaction as MysticetiTransaction,
    committee::Committee as MysticetiCommittee,
    config::{Authority as MysticetiAuthority, Parameters as MysticetiParameters},
    context::Context as MysticetiContext,
    core::Core as MysticetiCore,
    crypto::CryptoEngine as MysticetiCryptoEngine,
    metrics::Metrics as MysticetiMetrics,
    storage::Storage as MysticetiStorage,
};''',
        '''    block::{Block as MysticetiBlock, Transaction as MysticetiTransaction},
    committee::Committee as MysticetiCommittee,
    config::{Authority as MysticetiAuthority, Parameters as MysticetiParameters},
    consensus::Linearizer as MysticetiLinearizer,
    context::Context as MysticetiContext,
    core::Core as MysticetiCore,
    crypto::{AsBytes as MysticetiAsBytes, CryptoEngine as MysticetiCryptoEngine},
    data::Data as MysticetiData,
    metrics::Metrics as MysticetiMetrics,
    storage::Storage as MysticetiStorage,
};''',
        1,
    )
    text = text.replace(
        '''struct MysticetiBlockSummary {
    vertex_id: VertexId,
    round: u64,
    parents: Vec<VertexId>,
}''',
        '''struct MysticetiBlockSummary {
    vertex_id: VertexId,
    round: u64,
    parents: Vec<VertexId>,
    serialized_block: Vec<u8>,
    committed_subdags: Vec<Vec<VertexId>>,
}''',
        1,
    )
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

    old = '''    fn try_advance(&mut self) {
        let committed = self.core.try_commit();
        if !committed.is_empty() {
            debug!(
                "[MYSTICETI] committed {} leader block(s) up to round {}",
                committed.len(),
                committed.last().map(|block| block.round()).unwrap_or(0)
            );
        }
    }'''
    new = '''    fn try_advance(&mut self) -> Vec<Vec<VertexId>> {
        let committed_leaders = self.core.try_commit();
        if committed_leaders.is_empty() {
            return Vec::new();
        }
        let committed_subdags = self
            .linearizer
            .handle_commit(self.core.block_reader(), committed_leaders);
        let summaries = committed_subdags
            .iter()
            .map(|subdag| {
                subdag
                    .blocks
                    .iter()
                    .filter(|block| block.round() > 0)
                    .map(|block| {
                        let mut id = [0u8; 32];
                        id.copy_from_slice(block.reference().digest.as_ref());
                        id
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|vertices| !vertices.is_empty())
            .collect::<Vec<_>>();
        let last_round = committed_subdags
            .last()
            .map(|subdag| subdag.anchor.round)
            .unwrap_or(0);
        self.core.handle_committed_subdag(committed_subdags);
        debug!(
            "[MYSTICETI] committed {} sub-DAG(s) up to round {}",
            summaries.len(),
            last_round
        );
        summaries
    }'''
    if old not in text:
        raise RuntimeError("Mysticeti try_advance block not found")
    text = text.replace(old, new, 1)

    old = '''        let summary = MysticetiBlockSummary {
            vertex_id: digest_to_vertex_id(block.reference().digest.as_ref()),
            round: block.round(),
            parents: block
                .includes()
                .iter()
                .map(|reference| digest_to_vertex_id(reference.digest.as_ref()))
                .collect(),
        };
        self.try_advance();
        Ok(Some(summary))'''
    new = '''        let serialized_block = block.serialized_bytes().to_vec();
        let summary = MysticetiBlockSummary {
            vertex_id: digest_to_vertex_id(block.reference().digest.as_ref()),
            round: block.round(),
            parents: block
                .includes()
                .iter()
                .map(|reference| digest_to_vertex_id(reference.digest.as_ref()))
                .collect(),
            serialized_block,
            committed_subdags: self.try_advance(),
        };
        Ok(Some(summary))'''
    if old not in text:
        raise RuntimeError("Mysticeti proposal summary block not found")
    text = text.replace(old, new, 1)

    marker = '''    fn propose_block(
        &mut self,
        transactions: &[SignedTransaction],
        timestamp_ms: u64,
    ) -> Result<Option<MysticetiBlockSummary>> {'''
    if marker not in text:
        raise RuntimeError("Mysticeti propose method marker not found")
    import_method = '''    fn import_block(
        &mut self,
        serialized_block: &[u8],
    ) -> Result<(MysticetiData<MysticetiBlock>, Vec<Vec<VertexId>>)> {
        anyhow::ensure!(!serialized_block.is_empty(), "missing serialized Mysticeti block");
        let block = MysticetiData::<MysticetiBlock>::from_bytes(
            minibytes::Bytes::from(serialized_block.to_vec()),
        )?;
        block.verify(
            &self._committee,
            self.core.quorum_threshold(),
            &self.core.verifier(),
        )?;
        let processed = self.core.add_blocks(vec![block]);
        anyhow::ensure!(processed.len() == 1, "Mysticeti block is missing parents or was rejected");
        let imported = processed.into_iter().next().expect("one imported block");
        let committed = self.try_advance();
        Ok((imported, committed))
    }

'''
    text = text.replace(marker, import_method + marker, 1)
    text = text.replace("        self.mysticeti.try_advance();", "        let _ = self.mysticeti.try_advance();", 1)
    write(path, text)
