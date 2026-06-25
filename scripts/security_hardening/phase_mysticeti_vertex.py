from .common import read, write


def apply():
    path = "crates/kanari-core/src/consensus.rs"
    text = read(path)
    text = text.replace(
        "    pub signature: Vec<u8>,\n    pub metadata: VertexMetadata,",
        "    pub signature: Vec<u8>,\n    #[serde(default)]\n    pub mysticeti_block: Vec<u8>,\n    pub metadata: VertexMetadata,",
        1,
    )
    text = text.replace(
        "            &self.metadata.state_root,\n        ))?;",
        "            &self.metadata.state_root,\n            &self.mysticeti_block,\n        ))?;",
        1,
    )
    text = text.replace(
        "            self.metadata.checkpoint_seq,\n        ))?;",
        "            self.metadata.checkpoint_seq,\n            &self.mysticeti_block,\n        ))?;",
        1,
    )
    text = text.replace(
        "            signature: Vec::new(),\n            metadata,",
        "            signature: Vec::new(),\n            mysticeti_block: Vec::new(),\n            metadata,",
        1,
    )
    marker = '''    pub fn compute_hash(&self) -> Result<VertexId> {
        if let Some(hash) = &self.cached_hash {
            return Ok(vertex_id_from_hash_bytes(hash));
        }
        self.compute_hash_uncached()
    }
'''
    addition = marker + '''
    pub fn bind_mysticeti_block(&mut self, block: Vec<u8>, block_id: VertexId) -> Result<()> {
        self.mysticeti_block = block;
        self.cached_hash = Some(self.compute_hash_uncached()?.to_vec());
        self.id = block_id;
        self.cached_signing_digest = None;
        Ok(())
    }
'''
    if marker not in text:
        raise RuntimeError("DagVertex method marker not found")
    write(path, text.replace(marker, addition, 1))
