// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::p2p::{P2PMessage, PeerInfoMsg, decompress_block};
use centauri::consensus::DagVertex;
use kanari_core::{BlockchainEngine, FullBlockData, engine::DagEngine};
use kanari_types::transaction::SignedTransaction;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Handles block and transaction synchronization between peers
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub struct SyncManager {
    engine: Arc<BlockchainEngine>,
    network_tx: mpsc::UnboundedSender<P2PMessage>,
    local_peer_id: String,
    /// Optional indexer for blockchain data indexing
    indexer: Option<Arc<Mutex<kanari_indexer::Indexer>>>,
    /// Buffer for blocks that arrived out of order (height -> block)
    block_buffer: Mutex<std::collections::BTreeMap<u64, FullBlockData>>,
    /// Highest height seen in the network
    max_peer_height: AtomicU64,
    /// Maximum number of blocks to keep in buffer to prevent memory exhaustion
    max_buffer_size: usize,
}

impl SyncManager {
    pub fn new(
        engine: Arc<BlockchainEngine>,
        network_tx: mpsc::UnboundedSender<P2PMessage>,
        local_peer_id: String,
        indexer: Option<Arc<Mutex<kanari_indexer::Indexer>>>,
    ) -> Self {
        Self {
            engine,
            network_tx,
            local_peer_id,
            indexer,
            block_buffer: Mutex::new(std::collections::BTreeMap::new()),
            max_peer_height: AtomicU64::new(0),
            max_buffer_size: 1000, // Limit buffer to 1000 blocks for 200-node networks
        }
    }

    /// Start periodic sync tasks
    pub async fn start(self: Arc<Self>) {
        let sync = self.clone();
        tokio::spawn(async move {
            loop {
                sync.check_sync_status().await;
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
    }

    /// Check if we are behind and need to sync
    async fn check_sync_status(&self) {
        let stats = self.engine.get_stats();
        let max_seen = self.max_peer_height.load(Ordering::Relaxed);

        // Fallback to P2P sync if we are behind
        if stats.height < max_seen {
            info!(
                "[SYNC] Behind network P2P (current: {}, max seen: {}). Requesting via P2P...",
                stats.height, max_seen
            );
            self.request_blocks(stats.height + 1, max_seen).await;
        }
    }

    /// Process incoming P2P messages
    pub async fn handle_message(&self, msg: P2PMessage) {
        match msg {
            P2PMessage::NewTransaction(tx_data) => {
                info!("[P2P] Received NewTransaction");
                self.handle_new_transaction(tx_data).await;
            }
            P2PMessage::NewBlock(block_data) => {
                info!("[P2P] Received NewBlock");
                self.handle_new_block(block_data).await;
            }
            P2PMessage::NewDagVertex(vertex_data) => {
                info!("[P2P] Received NewDagVertex");
                self.handle_new_dag_vertex(vertex_data).await;
            }
            P2PMessage::BlockRequest(height, _timestamp) => {
                info!("[P2P] Received BlockRequest for height {}", height);
                self.handle_block_request(height).await;
            }
            P2PMessage::BlockResponse(block_data) => {
                info!("[P2P] Received BlockResponse");
                self.handle_block_response(block_data).await;
            }
            P2PMessage::PeerInfo(peer_info) => {
                info!("[P2P] Received PeerInfo from {}", peer_info.peer_id);
                self.handle_peer_info(peer_info).await;
            }
            // Handle compressed messages (should be decompressed before reaching here)
            P2PMessage::CompressedBlock(_) | P2PMessage::CompressedDagVertex(_) => {
                warn!(
                    "[P2P] Received compressed message in sync manager - should be decompressed already"
                );
            }
            P2PMessage::CompressedBlockResponse(compressed_data) => {
                if let Ok(data) = decompress_block(compressed_data.to_vec()) {
                    self.handle_block_response(data).await;
                }
            }
        }
    }

    async fn handle_new_transaction(&self, tx_data: String) {
        match serde_json::from_str::<SignedTransaction>(&tx_data) {
            Ok(signed_tx) => match self.engine.submit_transaction(signed_tx.clone()) {
                Ok(tx_hash) => {
                    info!(
                        "Received transaction from network: 0x{}",
                        hex::encode(tx_hash)
                    );
                }
                Err(e) => {
                    warn!("Failed to submit transaction from network: {}", e);
                }
            },
            Err(e) => {
                error!("Failed to deserialize transaction: {}", e);
            }
        }
    }

    async fn handle_new_block(&self, block_data: String) {
        match serde_json::from_str::<FullBlockData>(&block_data) {
            Ok(block) => {
                let stats = self.engine.get_stats();
                info!(
                    "[SYNC] Processing NewBlock #{} from network (current height: {}, txs: {})",
                    block.height, stats.height, block.tx_count
                );

                let block_height = block.height;
                if block_height > stats.height {
                    // Buffer the block
                    {
                        let mut buffer = self
                            .block_buffer
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        // Check if buffer is full
                        if buffer.len() >= self.max_buffer_size {
                            warn!(
                                "[SYNC] Block buffer full (max: {}). Dropping block #{}",
                                self.max_buffer_size, block_height
                            );
                            return;
                        }
                        buffer.insert(block_height, block);
                        info!(
                            "[SYNC] Buffered NewBlock #{}. Buffer size: {}/{}",
                            block_height,
                            buffer.len(),
                            self.max_buffer_size
                        );
                    }

                    // Try to apply consecutive blocks from the buffer
                    self.try_apply_buffered_blocks().await;

                    // If we're still behind, request missing blocks
                    let new_stats = self.engine.get_stats();
                    let latest_buffered = {
                        let buffer = self
                            .block_buffer
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        buffer.keys().last().cloned().unwrap_or(0)
                    };

                    if latest_buffered > new_stats.height + 1 {
                        info!(
                            "[SYNC] Gap detected after NewBlock #{}. Current: {}, buffered: {}. Requesting missing blocks...",
                            block_height, new_stats.height, latest_buffered
                        );
                        self.request_blocks(new_stats.height + 1, latest_buffered - 1)
                            .await;
                    }
                } else {
                    info!(
                        "[SYNC] Received old NewBlock #{} (current: {}) - ignoring",
                        block.height, stats.height
                    );
                }
            }
            Err(e) => {
                error!("Failed to deserialize new block: {}", e);
            }
        }
    }

    /// Try to apply buffered blocks in sequence
    async fn try_apply_buffered_blocks(&self) {
        loop {
            let stats = self.engine.get_stats();
            let next_height = stats.height + 1;

            let next_block = {
                let mut buffer = self
                    .block_buffer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                buffer.remove(&next_height)
            };

            if let Some(block) = next_block {
                info!(
                    "[SYNC] Found block #{} in buffer, attempting to apply. (current height: {})",
                    block.height, stats.height
                );
                match self.engine.sync_full_block_from_data(&block) {
                    Ok(_) => {
                        info!(
                            "[SYNC] Successfully synced block #{} with {} transactions",
                            block.height, block.tx_count
                        );

                        // Index the block if indexer is available
                        if let Some(ref indexer) = self.indexer {
                            match self.index_block_with_indexer(indexer, &block) {
                                Ok(_) => {}
                                Err(e) => {
                                    error!(
                                        "[INDEXER] Failed to index block #{}: {}",
                                        block.height, e
                                    );
                                }
                            }
                        }

                        // Broadcast our new height
                        self.broadcast_peer_info().await;
                    }
                    Err(e) => {
                        error!("[SYNC] Failed to sync block #{}: {}.", block.height, e);
                        break;
                    }
                }
            } else {
                // No more consecutive blocks in buffer.
                // Check if we are still behind the network and should request more.
                let stats = self.engine.get_stats();
                let max_seen = self.max_peer_height.load(Ordering::Relaxed);
                if stats.height < max_seen {
                    info!(
                        "[SYNC] Applied all buffered blocks but still behind network (current: {}, max seen: {}).",
                        stats.height, max_seen
                    );

                    // Check buffer for gaps
                    {
                        let buffer = self
                            .block_buffer
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if !buffer.is_empty() {
                            let buffered_heights: Vec<_> = buffer.keys().collect();
                            info!(
                                "[SYNC] Buffered blocks: {:?}. Next required: {}",
                                buffered_heights,
                                stats.height + 1
                            );
                        }
                    }

                    self.request_blocks(stats.height + 1, max_seen).await;
                }
                break;
            }
        }

        // Clean up old blocks from buffer
        {
            let stats = self.engine.get_stats();
            let mut buffer = self
                .block_buffer
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let initial_len = buffer.len();
            buffer.retain(|&h, _| h > stats.height);
            if buffer.len() < initial_len {
                info!(
                    "[SYNC] Cleaned up {} old blocks from buffer. Current size: {}",
                    initial_len - buffer.len(),
                    buffer.len()
                );
            }
        }
    }

    async fn handle_new_dag_vertex(&self, vertex_data: String) {
        // DAG vertices are serialized as centauri::consensus::DagVertex
        // NOT as DagBlockInfo which is just metadata

        match serde_json::from_str::<DagVertex>(&vertex_data) {
            Ok(vertex) => {
                let stats = self.engine.get_stats();
                info!(
                    "Received DAG vertex {} (round {}) from network with {} transactions",
                    hex::encode(vertex.id),
                    vertex.round,
                    vertex.transactions.len()
                );

                // Add vertex to local DAG consensus
                // Auto-initialize DAG engine if not already initialized
                if let Some(dag_engine_arc) = self.engine.get_dag_engine() {
                    // Check and initialize with write lock to prevent TOCTOU race
                    {
                        let mut guard = dag_engine_arc
                            .write()
                            .unwrap_or_else(|e| e.into_inner());

                        // Only initialize if still None after acquiring write lock
                        if guard.is_none() {
                            info!(
                                "[DAG SYNC] Auto-initializing DAG engine for vertex round {}",
                                vertex.round
                            );

                            // Initialize DAG engine with same authorities as the network
                            let authority_id = self.engine.get_authority_id();
                            let authorities = self.engine.get_authorities();

                            match DagEngine::new(self.engine.clone(), authority_id, authorities) {
                                Ok(engine) => {
                                    *guard = Some(engine);
                                    info!("[DAG SYNC] DAG engine initialized successfully");
                                }
                                Err(e) => {
                                    error!("[DAG SYNC] Failed to initialize DAG engine: {}", e);
                                }
                            }
                        }
                        // Write lock is released here when guard goes out of scope
                    }

                    // Get the engine with a read lock for processing
                    let dag_engine_opt = {
                        let guard = dag_engine_arc
                            .read()
                            .unwrap_or_else(|e| e.into_inner());
                        guard.as_ref().cloned()
                    };

                    if let Some(dag_engine) = dag_engine_opt {
                        match dag_engine.add_network_vertex(vertex.clone()) {
                            Ok(_) => {
                                info!(
                                    "Successfully added DAG vertex {} to local consensus",
                                    hex::encode(vertex.id)
                                );
                                // Check if height changed to broadcast new info
                                let new_stats = self.engine.get_stats();
                                if new_stats.height > stats.height {
                                    info!("New height reached via DAG: {}", new_stats.height);
                                    self.broadcast_peer_info().await;
                                }
                            }
                            Err(e) => {
                                warn!("Failed to add DAG vertex to consensus: {}", e);
                            }
                        }
                    } else {
                        warn!("DAG engine not initialized, cannot process vertex");
                    }
                } else {
                    warn!("DAG mode not enabled, ignoring vertex");
                }
            }
            Err(e) => {
                error!("Failed to deserialize DAG vertex: {}", e);
            }
        }
    }

    async fn handle_block_request(&self, height: u64) {
        info!("[SYNC] Received block request for height {}", height);
        if let Some(full_block_data) = self.engine.get_full_block(height) {
            info!(
                "[SYNC] Found block #{} with {} txs, sending response",
                height, full_block_data.tx_count
            );
            if let Ok(data_str) = serde_json::to_string(&full_block_data) {
                let msg = P2PMessage::BlockResponse(data_str);
                if let Err(e) = self.network_tx.send(msg) {
                    error!("[SYNC] Failed to send block response: {}", e);
                }
            }
        } else {
            warn!(
                "[SYNC] Block #{} not found in our engine for request",
                height
            );
        }
    }

    async fn handle_block_response(&self, block_data: String) {
        match serde_json::from_str::<FullBlockData>(&block_data) {
            Ok(block) => {
                let stats = self.engine.get_stats();

                info!(
                    "[SYNC] Processing block response #{} (current height: {}, txs: {}, from network)",
                    block.height, stats.height, block.tx_count
                );

                if block.height > stats.height {
                    // Buffer the block with size limit check
                    let buffer_len = {
                        let mut buffer = self
                            .block_buffer
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        // Check if buffer is full
                        if buffer.len() >= self.max_buffer_size {
                            warn!(
                                "[SYNC] Block buffer full (max: {}). Dropping block #{}",
                                self.max_buffer_size, block.height
                            );
                            return;
                        }
                        buffer.insert(block.height, block.clone());
                        buffer.len()
                    };

                    info!(
                        "[SYNC] Buffered block #{}. Buffer size: {}/{}",
                        block.height, buffer_len, self.max_buffer_size
                    );

                    // Try to apply consecutive blocks
                    self.try_apply_buffered_blocks().await;
                } else {
                    info!(
                        "[SYNC] Received old block response #{} (current: {}) - ignoring",
                        block.height, stats.height
                    );
                }
            }
            Err(e) => {
                error!("[SYNC] Failed to deserialize block response: {}", e);
            }
        }
    }

    async fn handle_peer_info(&self, peer_info: PeerInfoMsg) {
        let stats = self.engine.get_stats();
        info!(
            "[SYNC] Received PeerInfo from {}: height={}, our_height={}",
            peer_info.peer_id, peer_info.height, stats.height
        );

        // Update max seen height
        let current_max = self.max_peer_height.load(Ordering::Relaxed);
        if peer_info.height > current_max {
            info!(
                "[SYNC] Updating max_peer_height from {} to {}",
                current_max, peer_info.height
            );
            self.max_peer_height
                .store(peer_info.height, Ordering::Relaxed);
        }

        if peer_info.height > stats.height {
            info!(
                "[SYNC] Peer {} is ahead at height {} (current: {})",
                peer_info.peer_id, peer_info.height, stats.height
            );
            // Request missing blocks starting from the next block we need
            // If we are at 0 (genesis only), start from 1.
            // If we are at N, start from N+1.
            let start_height = stats.height + 1;

            // Only request if start_height <= peer_height
            if start_height <= peer_info.height {
                info!(
                    "[SYNC] Requesting blocks from {} to {} from peer {}",
                    start_height, peer_info.height, peer_info.peer_id
                );
                self.request_blocks(start_height, peer_info.height).await;
            } else {
                warn!(
                    "[SYNC] Peer {} has height {} but we need {} (stats.height: {}). Not requesting.",
                    peer_info.peer_id, peer_info.height, start_height, stats.height
                );
            }
        } else {
            info!(
                "[SYNC] Peer {} is at height {} (current: {}). We are synced or ahead.",
                peer_info.peer_id, peer_info.height, stats.height
            );
        }
    }

    async fn request_blocks(&self, from: u64, to: u64) {
        if from > to {
            return;
        }

        let stats = self.engine.get_stats();
        info!(
            "[SYNC] Requesting blocks from {} to {} (our current height: {})",
            from, to, stats.height
        );
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Limit the number of blocks requested at once to avoid network congestion
        // Increased from 50 to 200 for better performance in large networks (200+ nodes)
        let max_request = 200;
        let actual_to = to.min(from + max_request);

        for height in from..=actual_to {
            // Check if we already have this block in buffer before requesting
            {
                let buffer = self
                    .block_buffer
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if buffer.contains_key(&height) {
                    continue;
                }
            }

            let msg = P2PMessage::BlockRequest(height, timestamp);
            if let Err(e) = self.network_tx.send(msg) {
                error!(
                    "[SYNC] Failed to send block request for height {}: {}",
                    height, e
                );
                break;
            }
        }
        info!(
            "[SYNC] Sent {} block requests starting from {}",
            actual_to - from + 1,
            from
        );
    }

    /// Broadcast local chain height to peers
    pub async fn broadcast_peer_info(&self) {
        let stats = self.engine.get_stats();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let msg = P2PMessage::PeerInfo(PeerInfoMsg {
            height: stats.height,
            peer_id: self.local_peer_id.clone(),
            timestamp,
        });

        if let Err(e) = self.network_tx.send(msg) {
            error!("Failed to broadcast peer info: {}", e);
        }
    }

    /// Index a block using the indexer (helper method)
    fn index_block_with_indexer(
        &self,
        indexer: &Arc<Mutex<kanari_indexer::Indexer>>,
        full_block: &FullBlockData,
    ) -> anyhow::Result<()> {
        use kanari_types::block::{Block, BlockHeader};
        use kanari_types::event::Event;
        use smt::compute_merkle_root;

        // Convert FullBlockData to kanari-types Block
        let tx_hashes: Vec<Vec<u8>> = full_block.transactions.iter().map(|tx| tx.hash()).collect();

        let merkle_root = compute_merkle_root(&tx_hashes);

        // Create events from transaction results (if available)
        // For now, we'll create empty events - in production this would come from execution results
        let events: Vec<Event> = vec![];

        let header = BlockHeader::new(
            full_block.height,
            hex::decode(&full_block.prev_hash).unwrap_or_default(),
            hex::decode(&full_block.state_root).unwrap_or_default(),
            merkle_root,
            full_block.tx_count,
            full_block.timestamp,
        );

        let block = Block {
            header,
            transactions: full_block.transactions.clone(),
            events,
        };

        // Index the block
        let idx = indexer
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to acquire indexer lock: {}", e))?;

        idx.index_block(&block)?;

        Ok(())
    }
}
