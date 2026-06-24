from __future__ import annotations

from .common import read, write


def apply() -> None:
    path = "crates/kanari-core/src/engine/bootstrap.rs"
    text = read(path)
    text = text.replace(
        '''    fn init_with_options(
        persistent_store: Option<Arc<PersistentStore>>,
        enable_in_memory_smt: bool,
    ) -> Result<Self> {
        tracing::info!("Loading blockchain checkpoints");''',
        '''    fn init_with_options(
        persistent_store: Option<Arc<PersistentStore>>,
        enable_in_memory_smt: bool,
    ) -> Result<Self> {
        if let Some(store) = &persistent_store {
            if let Some(checkpoint) = store
                .load::<Checkpoint>(b"pending_checkpoint_commit")
                .context("Failed to inspect checkpoint commit journal")?
            {
                anyhow::bail!(
                    "unclean checkpoint commit detected at sequence {}; refusing startup until recovery is performed",
                    checkpoint.sequence
                );
            }
        }
        tracing::info!("Loading blockchain checkpoints");''',
        1,
    )
    write(path, text)

    path = "crates/kanari-node/src/p2p.rs"
    text = read(path)
    text = text.replace(
        "const MAX_DECOMPRESSED_PAYLOAD_SIZE: usize = 8 * 1024 * 1024;",
        "const MAX_DECOMPRESSED_PAYLOAD_SIZE: usize = 2 * 1024 * 1024;",
        1,
    )
    text = text.replace(".max_transmit_size(1_000_000)", ".max_transmit_size(512_000)", 1)
    write(path, text)

    path = "crates/kanari-node/src/sync.rs"
    text = read(path)
    text = text.replace(
        '''    /// Maximum number of checkpoints to keep in buffer to prevent memory exhaustion
    max_buffer_size: usize,
    max_dag_vertex_buffer_size: usize,''',
        '''    /// Maximum number and encoded bytes of checkpoints retained before verification.
    max_buffer_size: usize,
    max_buffer_bytes: usize,
    buffered_checkpoint_bytes: Mutex<usize>,
    max_dag_vertex_buffer_size: usize,''',
        1,
    )
    text = text.replace(
        '''            max_buffer_size: 1000, // Limit buffer to 1000 checkpoints for 200-node networks
            max_dag_vertex_buffer_size: 2048,''',
        '''            max_buffer_size: 64,
            max_buffer_bytes: 32 * 1024 * 1024,
            buffered_checkpoint_bytes: Mutex::new(0),
            max_dag_vertex_buffer_size: 512,''',
        1,
    )
    text = text.replace(
        '''        let sequence = checkpoint.checkpoint.sequence;
        let mut buffer = self.checkpoint_buffer_guard();
        let candidate_count: usize = buffer.values().map(VecDeque::len).sum();
        if candidate_count >= self.max_buffer_size {''',
        '''        let sequence = checkpoint.checkpoint.sequence;
        let current_height = self.engine.get_stats().height;
        if sequence > current_height.saturating_add(MAX_CHECKPOINTS_PER_REQUEST) {
            warn!("[SYNC] Dropping checkpoint #{} too far ahead of local height {}", sequence, current_height);
            return None;
        }
        let encoded_size = bcs::to_bytes(&checkpoint)
            .map(|bytes| bytes.len())
            .unwrap_or(self.max_buffer_bytes);
        let mut byte_count = self
            .buffered_checkpoint_bytes
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut buffer = self.checkpoint_buffer_guard();
        let candidate_count: usize = buffer.values().map(VecDeque::len).sum();
        if candidate_count >= self.max_buffer_size
            || byte_count.saturating_add(encoded_size) > self.max_buffer_bytes
        {''',
        1,
    )
    text = text.replace(
        '''        candidates.push_back(BufferedCheckpointCandidate {
            checkpoint,
            source_peer_id: source_peer_id.map(str::to_owned),
        });
        let candidate_count = candidate_count + 1;''',
        '''        candidates.push_back(BufferedCheckpointCandidate {
            checkpoint,
            source_peer_id: source_peer_id.map(str::to_owned),
        });
        *byte_count = byte_count.saturating_add(encoded_size);
        let candidate_count = candidate_count + 1;''',
        1,
    )
    text = text.replace(
        '''        let next_candidate = buffer
            .get_mut(&next_sequence)
            .and_then(|candidates| candidates.pop_front());''',
        '''        let next_candidate = buffer
            .get_mut(&next_sequence)
            .and_then(|candidates| candidates.pop_front());
        if let Some(candidate) = &next_candidate {
            let removed = bcs::to_bytes(&candidate.checkpoint)
                .map(|bytes| bytes.len())
                .unwrap_or(0);
            let mut byte_count = self
                .buffered_checkpoint_bytes
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *byte_count = byte_count.saturating_sub(removed);
        }''',
        1,
    )
    write(path, text)
