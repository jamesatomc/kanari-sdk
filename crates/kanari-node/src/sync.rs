// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::{
    config::NodeRuntimeConfig,
    p2p::{
        AuthenticatedP2PMessage, CheckpointRequestMsg, CheckpointResponseMsg, DagVertexMsg,
        DagVertexRequestMsg, DagVertexResponseMsg, P2PMessage, PeerInfoMsg, QueuedP2PMessage,
    },
};
use kanari_core::{BlockchainEngine, CheckpointSyncData, DagVertex};
use kanari_types::transaction::SignedTransaction;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use std::time::Duration;

const REQUEST_RETRY_COOLDOWN_MS: u64 = 2_000;
const DAG_VERTEX_REQUEST_RETRY_COOLDOWN_MS: u64 = 1_000;
const MAX_CHECKPOINTS_PER_REQUEST: u64 = 200;
const MAX_TRACKED_PEERS: usize = 2_048;
const MAX_PENDING_REQUEST_TRACKING: usize = 4_096;
const DAG_REBROADCAST_COOLDOWN_MS: u64 = 5_000;
const MAX_TRACKED_DAG_REBROADCASTS: usize = 4_096;

#[derive(Clone)]
struct BufferedCheckpointCandidate {
    checkpoint: CheckpointSyncData,
    source_peer_id: Option<String>,
}

#[derive(Debug, Clone)]
struct DivergentPeerInfo {
    height: u64,
    latest_checkpoint_hash: String,
    latest_state_root: String,
}

pub struct SyncManager {
    engine: Arc<BlockchainEngine>,
    network_tx: mpsc::Sender<QueuedP2PMessage>,
    local_peer_id: String,
    /// Optional indexer for blockchain data indexing
    indexer: Option<Arc<Mutex<kanari_indexer::Indexer>>>,
    /// Buffer for checkpoints that arrived out of order (sequence -> candidate checkpoints)
    checkpoint_buffer: Mutex<BTreeMap<u64, VecDeque<BufferedCheckpointCandidate>>>,
    /// Last advertised height by peer id.
    peer_heights: Mutex<BTreeMap<String, u64>>,
    /// Peers that advertised a conflicting state root and should not be used for sync.
    divergent_peers: Mutex<BTreeMap<String, DivergentPeerInfo>>,
    /// Last request timestamp per checkpoint sequence to avoid request spam while still retrying fast.
    pending_checkpoint_requests: Mutex<BTreeMap<u64, u64>>,
    /// Last request timestamp per DAG parent round to avoid request storms while catching up.
    pending_dag_vertex_requests: Mutex<BTreeMap<u64, u64>>,
    /// DAG vertices that arrived before their parents. Gossip delivery is unordered,
    /// so retry these after each successful vertex import.
    dag_vertex_buffer: Mutex<VecDeque<DagVertex>>,
    /// Last rebroadcast attempt per vertex. Record attempts (not only success)
    /// so a full outbound queue cannot trigger a retry storm every node tick.
    dag_vertex_rebroadcasts: Mutex<BTreeMap<[u8; 32], u64>>,
    /// Maximum number of checkpoints to keep in buffer to prevent memory exhaustion
    max_buffer_size: usize,
    max_dag_vertex_buffer_size: usize,
}

impl SyncManager {
    fn local_checkpoint_identity(&self, sequence: u64) -> Option<(String, String)> {
        self.engine
            .get_block(sequence)
            .map(|block| (block.hash, block.state_root))
    }

    fn divergence_kind(
        local_checkpoint_hash: &str,
        local_state_root: &str,
        peer_info: &PeerInfoMsg,
    ) -> Option<&'static str> {
        let state_root_mismatch = peer_info.latest_state_root != local_state_root;
        if state_root_mismatch {
            if peer_info.latest_checkpoint_hash != local_checkpoint_hash {
                Some("checkpoint-history-and-state-root")
            } else {
                Some("state-root")
            }
        } else {
            None
        }
    }

    fn checkpoint_buffer_guard(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<u64, VecDeque<BufferedCheckpointCandidate>>> {
        self.checkpoint_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn peer_heights_guard(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, u64>> {
        self.peer_heights.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn divergent_peers_guard(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<String, DivergentPeerInfo>> {
        self.divergent_peers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn pending_checkpoint_requests_guard(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, u64>> {
        self.pending_checkpoint_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn pending_dag_vertex_requests_guard(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, u64>> {
        self.pending_dag_vertex_requests
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn dag_vertex_buffer_guard(&self) -> std::sync::MutexGuard<'_, VecDeque<DagVertex>> {
        self.dag_vertex_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn should_rebroadcast_dag_vertex(&self, id: [u8; 32], now: u64) -> bool {
        let mut attempts = self
            .dag_vertex_rebroadcasts
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if attempts
            .get(&id)
            .is_some_and(|last| now.saturating_sub(*last) < DAG_REBROADCAST_COOLDOWN_MS)
        {
            return false;
        }
        if attempts.len() >= MAX_TRACKED_DAG_REBROADCASTS {
            let cutoff = now.saturating_sub(DAG_REBROADCAST_COOLDOWN_MS);
            attempts.retain(|_, timestamp| *timestamp >= cutoff);
            if attempts.len() >= MAX_TRACKED_DAG_REBROADCASTS
                && let Some(oldest) = attempts
                    .iter()
                    .min_by_key(|(_, timestamp)| **timestamp)
                    .map(|(id, _)| *id)
            {
                attempts.remove(&oldest);
            }
        }
        attempts.insert(id, now);
        true
    }

    pub fn new(
        engine: Arc<BlockchainEngine>,
        network_tx: mpsc::Sender<QueuedP2PMessage>,
        local_peer_id: String,
        indexer: Option<Arc<Mutex<kanari_indexer::Indexer>>>,
    ) -> Self {
        Self {
            engine,
            network_tx,
            local_peer_id,
            indexer,
            checkpoint_buffer: Mutex::new(BTreeMap::new()),
            peer_heights: Mutex::new(BTreeMap::new()),
            divergent_peers: Mutex::new(BTreeMap::new()),
            pending_checkpoint_requests: Mutex::new(BTreeMap::new()),
            pending_dag_vertex_requests: Mutex::new(BTreeMap::new()),
            dag_vertex_buffer: Mutex::new(VecDeque::new()),
            dag_vertex_rebroadcasts: Mutex::new(BTreeMap::new()),
            max_buffer_size: 1000, // Limit buffer to 1000 checkpoints for 200-node networks
            max_dag_vertex_buffer_size: 2048,
        }
    }

    /// Start periodic sync tasks
    pub async fn start(self: Arc<Self>) {
        let sync = self.clone();
        tokio::spawn(async move {
            loop {
                sync.check_sync_status().await;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
    }

    /// Check if we are behind and need to sync
    async fn check_sync_status(&self) {
        let stats = self.engine.get_stats();
        let max_seen = self.max_eligible_peer_height();

        // Fallback to P2P sync if we are behind
        if stats.height < max_seen {
            let target_peer = self.best_peer_for_height(stats.height + 1);
            debug!(
                "[SYNC] Behind network P2P (current: {}, max seen: {}, target: {:?}). Requesting checkpoints via P2P...",
                stats.height, max_seen, target_peer
            );
            self.request_checkpoints(stats.height + 1, max_seen, target_peer.as_deref())
                .await;
        }
    }

    /// Process incoming P2P messages
    pub async fn handle_message(&self, received: AuthenticatedP2PMessage) {
        let authenticated_peer_id = received.source.to_string();
        let msg = received.message;
        match msg {
            P2PMessage::NewTransaction(tx_data) => {
                info!("[P2P] Received NewTransaction");
                self.handle_new_transaction(tx_data).await;
            }
            P2PMessage::NewCheckpoint(checkpoint_data) => {
                info!("[P2P] Received NewCheckpoint");
                self.handle_new_checkpoint(checkpoint_data).await;
            }
            P2PMessage::NewDagVertex(vertex_data) => {
                debug!("[P2P] Received NewDagVertex");
                self.handle_new_dag_vertex(vertex_data);
            }
            P2PMessage::DagVertexRebroadcast(msg) => {
                if msg.sender_peer_id != authenticated_peer_id {
                    warn!("[P2P] Rejected DAG rebroadcast with spoofed sender identity");
                    return;
                }
                if msg.sender_peer_id == self.local_peer_id {
                    return;
                }
                debug!(
                    "[P2P] Received DagVertexRebroadcast from {}",
                    msg.sender_peer_id
                );
                self.handle_new_dag_vertex(msg.vertex_data);
            }
            P2PMessage::DagVertexRequest(req) => {
                if req.requester_peer_id != authenticated_peer_id {
                    warn!("[P2P] Rejected DAG request with spoofed requester identity");
                    return;
                }
                if req.requester_peer_id == self.local_peer_id {
                    return;
                }
                info!(
                    "[P2P] Received DagVertexRequest for parent round {} from {}",
                    req.parent_round, req.requester_peer_id
                );
                self.handle_dag_vertex_request(req);
            }
            P2PMessage::DagVertexResponse(resp) => {
                if resp.responder_peer_id != authenticated_peer_id {
                    warn!("[P2P] Rejected DAG response with spoofed responder identity");
                    return;
                }
                if resp.requester_peer_id != self.local_peer_id {
                    return;
                }
                info!(
                    "[P2P] Received DagVertexResponse from {} with {} vertices",
                    resp.responder_peer_id,
                    resp.vertex_data.len()
                );
                self.handle_dag_vertex_response(resp);
            }
            P2PMessage::CheckpointRequest(sequence, timestamp) => {
                info!("[P2P] Received CheckpointRequest for sequence {}", sequence);
                self.handle_checkpoint_request(sequence, timestamp, None, None)
                    .await;
            }
            P2PMessage::TargetedCheckpointRequest(req) => {
                if req.requester_peer_id != authenticated_peer_id {
                    warn!("[P2P] Rejected checkpoint request with spoofed requester identity");
                    return;
                }
                if req.requester_peer_id == self.local_peer_id {
                    return;
                }
                if req.responder_peer_id != self.local_peer_id {
                    return;
                }
                info!(
                    "[P2P] Received targeted CheckpointRequest for sequence {} from {}",
                    req.sequence, req.requester_peer_id
                );
                self.handle_checkpoint_request(
                    req.sequence,
                    req.timestamp,
                    Some(req.requester_peer_id.as_str()),
                    Some(req.responder_peer_id.as_str()),
                )
                .await;
            }
            P2PMessage::CheckpointResponse(checkpoint_data) => {
                info!("[P2P] Received CheckpointResponse");
                self.handle_checkpoint_response(checkpoint_data, None).await;
            }
            P2PMessage::TargetedCheckpointResponse(resp) => {
                if resp.responder_peer_id != authenticated_peer_id {
                    warn!("[P2P] Rejected checkpoint response with spoofed responder identity");
                    return;
                }
                if resp.requester_peer_id != self.local_peer_id {
                    return;
                }
                info!(
                    "[P2P] Received targeted CheckpointResponse for sequence {} from {}",
                    resp.sequence, resp.responder_peer_id
                );
                self.handle_checkpoint_response(
                    resp.checkpoint_data,
                    Some(&resp.responder_peer_id),
                )
                .await;
            }
            P2PMessage::PeerInfo(peer_info) => {
                if peer_info.peer_id != authenticated_peer_id {
                    warn!("[P2P] Rejected PeerInfo with spoofed peer identity");
                    return;
                }
                debug!("[P2P] Received PeerInfo from {}", peer_info.peer_id);
                self.handle_peer_info(peer_info).await;
            }
            P2PMessage::CompressedCheckpoint(_)
            | P2PMessage::CompressedDagVertex(_)
            | P2PMessage::CompressedCheckpointResponse(_)
            | P2PMessage::CompressedTargetedCheckpointResponse(_)
            | P2PMessage::Chunk(_) => {
                warn!(
                    "[P2P] Received compressed message in sync manager; P2P event handling should decompress before forwarding"
                );
            }
        }
    }

    fn parse_message<T: DeserializeOwned>(data: &str, context: &str) -> Option<T> {
        match serde_json::from_str(data) {
            Ok(value) => Some(value),
            Err(e) => {
                error!("Failed to deserialize {}: {}", context, e);
                None
            }
        }
    }

    fn send_network_message(&self, msg: P2PMessage, context: &str) -> bool {
        match self.network_tx.try_send(QueuedP2PMessage::new(msg)) {
            Ok(_) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                debug!(
                    "{}: outbound queue is full; bounded retry will handle it",
                    context
                );
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                error!("{}: outbound network task is closed", context);
                false
            }
        }
    }

    fn serialize_dag_vertices(vertices: Vec<DagVertex>, context: &str) -> Vec<String> {
        let mut vertex_data = Vec::with_capacity(vertices.len());
        for vertex in vertices {
            match serde_json::to_string(&vertex) {
                Ok(data) => vertex_data.push(data),
                Err(e) => warn!(
                    "{}: failed to serialize vertex {}: {}",
                    context,
                    hex::encode(vertex.id),
                    e
                ),
            }
        }
        vertex_data
    }

    fn queue_dag_vertex_request(&self, parent_round: u64, timestamp: u64, context: &str) -> bool {
        if !self.should_request_dag_vertices_for_round(parent_round, timestamp) {
            return false;
        }

        self.send_network_message(
            P2PMessage::DagVertexRequest(DagVertexRequestMsg {
                requester_peer_id: self.local_peer_id.clone(),
                parent_round,
                timestamp,
                limit: NodeRuntimeConfig::dag_vertices_per_response() as u64,
            }),
            context,
        )
    }

    pub fn broadcast_latest_dag_vertices(&self, limit: usize, reason: &str) {
        let vertices = match self.engine.latest_own_dag_vertices(limit) {
            Ok(vertices) => vertices,
            Err(e) => {
                warn!("Failed to load latest DAG vertices for rebroadcast: {}", e);
                return;
            }
        };

        if vertices.is_empty() {
            return;
        }

        let now = Self::current_timestamp();

        for vertex in vertices {
            if !self.should_rebroadcast_dag_vertex(vertex.id, now) {
                continue;
            }
            let Ok(vertex_data) = serde_json::to_string(&vertex) else {
                warn!(
                    "Failed to serialize DAG vertex {} for rebroadcast",
                    hex::encode(vertex.id)
                );
                continue;
            };

            let msg = P2PMessage::DagVertexRebroadcast(DagVertexMsg {
                vertex_data,
                nonce: now ^ vertex.round,
                sender_peer_id: self.local_peer_id.clone(),
            });
            if self.send_network_message(msg, "Failed to queue DAG vertex rebroadcast") {
                info!(
                    "Rebroadcasting DAG vertex {} (round {}) {}",
                    hex::encode(vertex.id),
                    vertex.round,
                    reason
                );
            }
        }
    }

    pub fn request_dag_vertices_for_quorum(&self) {
        let policy = match self.engine.dag_production_policy() {
            Ok(policy) => policy,
            Err(e) => {
                warn!(
                    "[DAG SYNC] Failed to read production policy for vertex request: {}",
                    e
                );
                return;
            }
        };

        if !policy.should_wait_for_current_round_quorum() {
            return;
        }

        let timestamp = Self::current_timestamp();
        if self.queue_dag_vertex_request(
            policy.parent_round,
            timestamp,
            "[DAG SYNC] Failed to queue DAG vertex request",
        ) {
            info!(
                "[DAG SYNC] Requested missing vertices for round {} from authorities {:?}",
                policy.parent_round, policy.missing_parent_authors
            );
        }
    }

    fn handle_dag_vertex_request(&self, request: DagVertexRequestMsg) {
        let limit =
            (request.limit as usize).clamp(1, NodeRuntimeConfig::dag_vertices_per_response());
        let vertices = match self
            .engine
            .dag_vertices_through_round_for_sync(request.parent_round, limit)
        {
            Ok(vertices) => vertices,
            Err(e) => {
                warn!(
                    "[DAG SYNC] Failed to load vertices for request response: {}",
                    e
                );
                return;
            }
        };

        if vertices.is_empty() {
            return;
        }

        let vertex_data = Self::serialize_dag_vertices(vertices, "[DAG SYNC] Preparing response");

        if vertex_data.is_empty() {
            return;
        }

        let response = DagVertexResponseMsg {
            requester_peer_id: request.requester_peer_id,
            responder_peer_id: self.local_peer_id.clone(),
            request_timestamp: request.timestamp,
            parent_round: request.parent_round,
            vertex_data,
        };

        self.send_network_message(
            P2PMessage::DagVertexResponse(response),
            "[DAG SYNC] Failed to queue DAG vertex response",
        );
    }

    fn handle_dag_vertex_response(&self, response: DagVertexResponseMsg) {
        let mut vertices = response
            .vertex_data
            .into_iter()
            .filter_map(|data| Self::parse_message::<DagVertex>(&data, "DAG vertex response"))
            .collect::<Vec<_>>();
        vertices.sort_by(|left, right| {
            left.round
                .cmp(&right.round)
                .then_with(|| left.author.cmp(&right.author))
                .then_with(|| left.id.cmp(&right.id))
        });
        for vertex in vertices {
            self.handle_dag_vertex(vertex);
        }
        self.request_missing_buffered_dag_parents();
    }

    fn request_missing_buffered_dag_parents(&self) {
        let buffered = self
            .dag_vertex_buffer_guard()
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let missing_rounds = match self.engine.dag_missing_parent_rounds_for_sync(&buffered) {
            Ok(rounds) => rounds,
            Err(error) => {
                debug!("[DAG SYNC] Cannot inspect buffered parent gaps: {}", error);
                return;
            }
        };
        let Some(parent_round) = missing_rounds.into_iter().max() else {
            return;
        };

        let timestamp = Self::current_timestamp();
        if self.queue_dag_vertex_request(
            parent_round,
            timestamp,
            "[DAG SYNC] Failed to queue ancestry repair request",
        ) {
            debug!(
                "[DAG SYNC] Requested next missing checkpoint ancestry page through round {}",
                parent_round
            );
        }
    }

    fn current_timestamp() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn buffer_checkpoint(
        &self,
        checkpoint: CheckpointSyncData,
        source_peer_id: Option<&str>,
        label: &str,
    ) -> Option<usize> {
        let sequence = checkpoint.checkpoint.sequence;
        let mut buffer = self.checkpoint_buffer_guard();
        let candidate_count: usize = buffer.values().map(VecDeque::len).sum();
        if candidate_count >= self.max_buffer_size {
            warn!(
                "[SYNC] Checkpoint buffer full (max: {}). Dropping checkpoint #{}",
                self.max_buffer_size, sequence
            );
            return None;
        }

        let candidates = buffer.entry(sequence).or_default();
        if candidates.iter().any(|candidate| {
            candidate.checkpoint.checkpoint.hash().ok() == checkpoint.checkpoint.hash().ok()
        }) {
            info!(
                "[SYNC] Duplicate {} #{} already buffered, ignoring",
                label, sequence
            );
            return Some(candidate_count);
        }

        candidates.push_back(BufferedCheckpointCandidate {
            checkpoint,
            source_peer_id: source_peer_id.map(str::to_owned),
        });
        let candidate_count = candidate_count + 1;
        info!(
            "[SYNC] Buffered {} #{}. Candidates for sequence: {}, total buffered: {}/{}",
            label,
            sequence,
            candidates.len(),
            candidate_count,
            self.max_buffer_size
        );
        Some(candidate_count)
    }

    fn latest_buffered_sequence(&self) -> u64 {
        let buffer = self.checkpoint_buffer_guard();
        buffer.keys().last().copied().unwrap_or(0)
    }

    fn pop_next_buffer_candidate(&self, next_sequence: u64) -> Option<BufferedCheckpointCandidate> {
        let mut buffer = self.checkpoint_buffer_guard();
        let next_candidate = buffer
            .get_mut(&next_sequence)
            .and_then(|candidates| candidates.pop_front());
        let should_remove = buffer
            .get(&next_sequence)
            .map(|candidates| candidates.is_empty())
            .unwrap_or(false);
        if should_remove {
            buffer.remove(&next_sequence);
        }
        next_candidate
    }

    fn best_peer_for_height(&self, height: u64) -> Option<String> {
        self.best_peer_for_height_excluding(height, None)
    }

    fn best_peer_for_height_excluding(
        &self,
        height: u64,
        excluded_peer_id: Option<&str>,
    ) -> Option<String> {
        let peers = self.peer_heights_guard();
        let divergent_peers = self.divergent_peers_guard();
        peers
            .iter()
            .filter(|(peer_id, peer_height)| {
                peer_id.as_str() != self.local_peer_id
                    && **peer_height >= height
                    && !divergent_peers.contains_key(peer_id.as_str())
            })
            .filter(|(peer_id, _)| excluded_peer_id != Some(peer_id.as_str()))
            .max_by_key(|(_, peer_height)| *peer_height)
            .map(|(peer_id, _)| peer_id.clone())
    }

    fn max_eligible_peer_height(&self) -> u64 {
        let peers = self.peer_heights_guard();
        let divergent_peers = self.divergent_peers_guard();
        peers
            .iter()
            .filter(|(peer_id, _)| !divergent_peers.contains_key(peer_id.as_str()))
            .map(|(_, height)| *height)
            .max()
            .unwrap_or(0)
    }

    fn record_peer_height(&self, peer_id: String, height: u64) {
        let mut peers = self.peer_heights_guard();
        if !peers.contains_key(&peer_id)
            && peers.len() >= MAX_TRACKED_PEERS
            && let Some(oldest) = peers.keys().next().cloned()
        {
            peers.remove(&oldest);
        }
        peers.insert(peer_id, height);
    }

    fn mark_peer_divergent(&self, peer_info: &PeerInfoMsg) {
        {
            let mut divergent = self.divergent_peers_guard();
            if !divergent.contains_key(&peer_info.peer_id)
                && divergent.len() >= MAX_TRACKED_PEERS
                && let Some(oldest) = divergent.keys().next().cloned()
            {
                divergent.remove(&oldest);
            }
            divergent.insert(
                peer_info.peer_id.clone(),
                DivergentPeerInfo {
                    height: peer_info.height,
                    latest_checkpoint_hash: peer_info.latest_checkpoint_hash.clone(),
                    latest_state_root: peer_info.latest_state_root.clone(),
                },
            );
        }

        self.peer_heights_guard().remove(&peer_info.peer_id);
    }

    fn clear_peer_divergence_if_recovered(
        &self,
        peer_info: &PeerInfoMsg,
        local_height: u64,
        local_checkpoint_hash: &str,
        local_state_root: &str,
    ) {
        let recorded = self
            .divergent_peers_guard()
            .get(&peer_info.peer_id)
            .cloned();
        let exact_tip_match = peer_info.height == local_height
            && peer_info.latest_checkpoint_hash == local_checkpoint_hash
            && peer_info.latest_state_root == local_state_root;
        let advertised_root_is_canonical = peer_info.height <= local_height
            && self
                .engine
                .get_block(peer_info.height)
                .is_some_and(|block| block.state_root == peer_info.latest_state_root);
        let recorded_root_is_canonical = recorded.as_ref().is_some_and(|divergence| {
            divergence.height <= local_height
                && self
                    .engine
                    .get_block(divergence.height)
                    .is_some_and(|block| block.state_root == divergence.latest_state_root)
        });
        if !exact_tip_match && !advertised_root_is_canonical && !recorded_root_is_canonical {
            return;
        }

        let mut divergent = self.divergent_peers_guard();
        if divergent.remove(&peer_info.peer_id).is_some() {
            info!(
                "[SYNC] Peer {} now matches local checkpoint/state again. Clearing divergence quarantine.",
                peer_info.peer_id
            );
        }
    }

    fn is_peer_divergent(&self, peer_id: &str) -> bool {
        self.divergent_peers_guard().contains_key(peer_id)
    }

    fn should_request_checkpoint_sequence(&self, sequence: u64, now: u64) -> bool {
        let mut pending = self.pending_checkpoint_requests_guard();
        match pending.get(&sequence).copied() {
            Some(last_requested)
                if now.saturating_sub(last_requested) < REQUEST_RETRY_COOLDOWN_MS =>
            {
                false
            }
            _ => {
                if pending.len() >= MAX_PENDING_REQUEST_TRACKING
                    && let Some(oldest) = pending.keys().next().copied()
                {
                    pending.remove(&oldest);
                }
                pending.insert(sequence, now);
                true
            }
        }
    }

    fn should_request_dag_vertices_for_round(&self, parent_round: u64, now: u64) -> bool {
        let mut pending = self.pending_dag_vertex_requests_guard();
        match pending.get(&parent_round).copied() {
            Some(last_requested)
                if now.saturating_sub(last_requested) < DAG_VERTEX_REQUEST_RETRY_COOLDOWN_MS =>
            {
                false
            }
            _ => {
                if pending.len() >= MAX_PENDING_REQUEST_TRACKING
                    && let Some(oldest) = pending.keys().next().copied()
                {
                    pending.remove(&oldest);
                }
                pending.insert(parent_round, now);
                true
            }
        }
    }

    fn clear_pending_checkpoint_requests_up_to(&self, sequence: u64) {
        let mut pending = self.pending_checkpoint_requests_guard();
        pending.retain(|pending_sequence, _| *pending_sequence > sequence);
    }

    fn should_buffer_dag_vertex_error(error_text: &str) -> bool {
        let normalized = error_text.to_ascii_lowercase();
        normalized.contains("parent vertex not found")
            || normalized.contains("missing parent")
            || normalized.contains("missing parents")
            || normalized.contains("not enough parents for quorum")
            || normalized.contains("dag_waiting")
            || normalized.contains("sync_waiting")
    }

    fn buffer_dag_vertex(&self, vertex: DagVertex, reason: &str) {
        let vertex_id = vertex.id;
        let mut buffer = self.dag_vertex_buffer_guard();

        if buffer.iter().any(|buffered| buffered.id == vertex_id) {
            return;
        }

        if buffer.len() >= self.max_dag_vertex_buffer_size
            && let Some(evicted) = buffer.pop_front()
        {
            warn!(
                "[DAG SYNC] Evicting buffered DAG vertex {} (round {}) due to buffer limit {}",
                hex::encode(evicted.id),
                evicted.round,
                self.max_dag_vertex_buffer_size
            );
        }

        info!(
            "[DAG SYNC] Buffering DAG vertex {} (round {}) for retry: {}",
            hex::encode(vertex_id),
            vertex.round,
            reason
        );
        buffer.push_back(vertex);
    }

    fn retry_buffered_dag_vertices(&self) {
        let retry_count = self.dag_vertex_buffer_guard().len();
        if retry_count == 0 {
            return;
        }

        for _ in 0..retry_count {
            let Some(vertex) = self.dag_vertex_buffer_guard().pop_front() else {
                break;
            };

            let vertex_id = hex::encode(vertex.id);
            match self.engine.add_network_dag_vertex(vertex.clone()) {
                Ok(()) => {
                    info!(
                        "[DAG SYNC] Applied buffered DAG vertex {} (round {})",
                        vertex_id, vertex.round
                    );
                }
                Err(e) => {
                    let error_text = e.to_string();
                    if Self::should_buffer_dag_vertex_error(&error_text) {
                        self.buffer_dag_vertex(vertex, &error_text);
                    } else {
                        warn!(
                            "[DAG SYNC] Dropping buffered DAG vertex {} (round {}): {}",
                            vertex_id, vertex.round, error_text
                        );
                    }
                }
            }
        }
    }

    async fn request_missing_checkpoints_from_peer(
        &self,
        peer_id: &str,
        peer_height: u64,
        local_height: u64,
    ) {
        let start_height = local_height + 1;
        if start_height > peer_height {
            warn!(
                "[SYNC] Peer {} has height {} but we need checkpoint {} (stats.height: {}). Not requesting.",
                peer_id, peer_height, start_height, local_height
            );
            return;
        }

        info!(
            "[SYNC] Requesting checkpoints from {} to {} from peer {}",
            start_height, peer_height, peer_id
        );
        self.request_checkpoints(start_height, peer_height, Some(peer_id))
            .await;
    }

    async fn process_incoming_checkpoint(
        &self,
        checkpoint_data: CheckpointSyncData,
        source_peer_id: Option<&str>,
        received_label: &str,
        buffered_label: &str,
        check_for_gap: bool,
    ) {
        for vertex in checkpoint_data.dag_vertices.iter().cloned() {
            if let Err(error) = self.engine.add_network_dag_vertex(vertex.clone()) {
                let error_text = error.to_string();
                if Self::should_buffer_dag_vertex_error(&error_text) {
                    self.buffer_dag_vertex(vertex, &error_text);
                } else if !error_text.contains("duplicate") {
                    warn!("[SYNC] Rejected checkpoint DAG evidence: {}", error_text);
                    return;
                }
            }
        }
        self.retry_buffered_dag_vertices();
        self.request_missing_buffered_dag_parents();
        let stats = self.engine.get_stats();
        let checkpoint = &checkpoint_data.checkpoint;
        info!(
            "[SYNC] Processing {} #{} (current height: {}, txs: {})",
            received_label,
            checkpoint.sequence,
            stats.height,
            checkpoint.transactions.len()
        );

        if checkpoint.sequence <= stats.height {
            let local_matches = match self.engine.checkpoint_hash(checkpoint.sequence) {
                Ok(Some(local_hash)) => match checkpoint.hash() {
                    Ok(remote_hash) => local_hash == remote_hash,
                    Err(error) => {
                        warn!(
                            "Failed to hash checkpoint during sync comparison: {}",
                            error
                        );
                        false
                    }
                },
                Ok(None) => false,
                Err(error) => {
                    warn!(
                        "Failed to load local checkpoint during sync comparison: {}",
                        error
                    );
                    false
                }
            };
            if !local_matches {
                warn!(
                    "[SYNC] Rejected conflicting checkpoint #{} after DAG verification",
                    checkpoint.sequence
                );
                return;
            }
            debug!(
                "[SYNC] Received old {} #{} (current: {}) - ignoring",
                received_label, checkpoint.sequence, stats.height
            );
            return;
        }

        let sequence = checkpoint.sequence;
        if self
            .buffer_checkpoint(checkpoint_data, source_peer_id, buffered_label)
            .is_none()
        {
            return;
        }

        self.try_apply_buffered_checkpoints().await;

        if check_for_gap {
            let new_stats = self.engine.get_stats();
            let latest_buffered = self.latest_buffered_sequence();
            if latest_buffered > new_stats.height + 1 {
                info!(
                    "[SYNC] Gap detected after {} #{}. Current: {}, buffered: {}. Requesting missing checkpoints...",
                    received_label, sequence, new_stats.height, latest_buffered
                );
                let target_peer = self.best_peer_for_height(new_stats.height + 1);
                self.request_checkpoints(
                    new_stats.height + 1,
                    latest_buffered - 1,
                    target_peer.as_deref(),
                )
                .await;
            }
        }
    }

    async fn handle_new_transaction(&self, tx_data: String) {
        if let Some(signed_tx) = Self::parse_message::<SignedTransaction>(&tx_data, "transaction") {
            match self
                .engine
                .submit_transactions_batch(vec![signed_tx.clone()])
            {
                Ok(tx_hashes) => {
                    debug!(
                        "Received transaction from network: 0x{}",
                        hex::encode(&tx_hashes[0])
                    );
                }
                Err(e) => {
                    warn!("Failed to submit transaction from network: {}", e);
                }
            }
        }
    }

    async fn handle_new_checkpoint(&self, checkpoint_data: String) {
        if let Some(checkpoint) =
            Self::parse_message::<CheckpointSyncData>(&checkpoint_data, "new checkpoint")
        {
            self.process_incoming_checkpoint(
                checkpoint,
                None,
                "NewCheckpoint",
                "NewCheckpoint",
                true,
            )
            .await;
        }
    }

    fn log_buffered_gap(&self, current_height: u64) {
        let buffer = self.checkpoint_buffer_guard();
        if !buffer.is_empty() {
            let buffered_heights: Vec<_> = buffer.keys().collect();
            info!(
                "[SYNC] Buffered checkpoints: {:?}. Next required: {}",
                buffered_heights,
                current_height + 1
            );
        }
    }

    fn index_checkpoint_if_needed(&self, sequence: u64) {
        if let Some(ref indexer) = self.indexer
            && let Some(materialized_block_view) = self.engine.get_full_block(sequence)
            && let Err(e) = self.index_checkpoint_with_indexer(indexer, &materialized_block_view)
        {
            error!("[INDEXER] Failed to index checkpoint #{}: {}", sequence, e);
        }
    }

    /// Try to apply buffered checkpoints in sequence
    async fn try_apply_buffered_checkpoints(&self) {
        loop {
            let stats = self.engine.get_stats();
            let next_sequence = stats.height + 1;
            let next_candidate = self.pop_next_buffer_candidate(next_sequence);

            if let Some(candidate) = next_candidate {
                let checkpoint = candidate.checkpoint;
                info!(
                    "[SYNC] Found checkpoint #{} in buffer, attempting to apply. (current height: {})",
                    checkpoint.checkpoint.sequence, stats.height
                );
                match self.engine.sync_checkpoint_from_data(&checkpoint) {
                    Ok(_) => {
                        info!(
                            "[SYNC] Successfully synced checkpoint #{} with {} transactions",
                            checkpoint.checkpoint.sequence,
                            checkpoint.checkpoint.transactions.len()
                        );
                        self.index_checkpoint_if_needed(checkpoint.checkpoint.sequence);

                        // Broadcast our new height
                        self.broadcast_peer_info().await;
                        self.clear_pending_checkpoint_requests_up_to(
                            checkpoint.checkpoint.sequence,
                        );
                    }
                    Err(e) => {
                        let has_more_candidates = {
                            let buffer = self.checkpoint_buffer_guard();
                            buffer
                                .get(&checkpoint.checkpoint.sequence)
                                .map(|candidates| !candidates.is_empty())
                                .unwrap_or(false)
                        };

                        if has_more_candidates {
                            warn!(
                                "[SYNC] Failed to sync checkpoint #{} candidate: {}. Trying next candidate.",
                                checkpoint.checkpoint.sequence, e
                            );
                            continue;
                        }

                        warn!(
                            "[SYNC] Failed to sync checkpoint #{} candidate: {}. Requesting replacement.",
                            checkpoint.checkpoint.sequence, e
                        );
                        let target_peer = self
                            .best_peer_for_height_excluding(
                                checkpoint.checkpoint.sequence,
                                candidate.source_peer_id.as_deref(),
                            )
                            .or_else(|| self.best_peer_for_height(checkpoint.checkpoint.sequence));
                        self.request_checkpoints(
                            checkpoint.checkpoint.sequence,
                            checkpoint.checkpoint.sequence,
                            target_peer.as_deref(),
                        )
                        .await;
                        break;
                    }
                }
            } else {
                // No more consecutive checkpoints in buffer.
                // Check if we are still behind the network and should request more.
                let stats = self.engine.get_stats();
                let max_seen = self.max_eligible_peer_height();
                if stats.height < max_seen {
                    info!(
                        "[SYNC] Applied all buffered checkpoints but still behind network (current: {}, max seen: {}).",
                        stats.height, max_seen
                    );

                    self.log_buffered_gap(stats.height);

                    let target_peer = self.best_peer_for_height(stats.height + 1);
                    self.request_checkpoints(stats.height + 1, max_seen, target_peer.as_deref())
                        .await;
                }
                break;
            }
        }

        // Clean up old checkpoints from buffer
        {
            let stats = self.engine.get_stats();
            let mut buffer = self.checkpoint_buffer_guard();
            let initial_len = buffer.len();
            buffer.retain(|&h, _| h > stats.height);
            if buffer.len() < initial_len {
                info!(
                    "[SYNC] Cleaned up {} old checkpoints from buffer. Current size: {}",
                    initial_len - buffer.len(),
                    buffer.len()
                );
            }
        }
    }

    fn handle_new_dag_vertex(&self, vertex_data: String) {
        // DAG vertices are serialized as kanari_core::DagVertex,
        // not as the higher-level block metadata wrapper.

        if let Some(vertex) = Self::parse_message::<DagVertex>(&vertex_data, "DAG vertex") {
            self.handle_dag_vertex(vertex);
        }
    }

    fn handle_dag_vertex(&self, vertex: DagVertex) {
        debug!(
            "Received DAG vertex {} (round {}) from network with {} transactions",
            hex::encode(vertex.id),
            vertex.round,
            vertex.transactions.len()
        );

        match self.engine.add_network_dag_vertex(vertex.clone()) {
            Ok(()) => {
                debug!(
                    "Successfully added DAG vertex {} to local consensus",
                    hex::encode(vertex.id)
                );
                self.retry_buffered_dag_vertices();
            }
            Err(e) => {
                let error_text = e.to_string();
                if Self::should_buffer_dag_vertex_error(&error_text) {
                    self.buffer_dag_vertex(vertex, &error_text);
                } else {
                    warn!("Failed to add DAG vertex to consensus: {}", error_text);
                }
            }
        }
    }

    async fn handle_checkpoint_request(
        &self,
        sequence: u64,
        request_timestamp: u64,
        requester_peer_id: Option<&str>,
        responder_peer_id: Option<&str>,
    ) {
        info!(
            "[SYNC] Received checkpoint request for sequence {}",
            sequence
        );
        match self.engine.get_checkpoint_sync(sequence) {
            Ok(Some(checkpoint_sync)) => {
                info!(
                    "[SYNC] Found checkpoint #{} with {} txs, sending response",
                    sequence,
                    checkpoint_sync.checkpoint.transactions.len()
                );
                match serde_json::to_string(&checkpoint_sync) {
                    Ok(data_str) => {
                        let msg = if let (Some(requester_peer_id), Some(responder_peer_id)) =
                            (requester_peer_id, responder_peer_id)
                        {
                            P2PMessage::TargetedCheckpointResponse(CheckpointResponseMsg {
                                sequence,
                                request_timestamp,
                                requester_peer_id: requester_peer_id.to_string(),
                                responder_peer_id: responder_peer_id.to_string(),
                                checkpoint_data: data_str,
                            })
                        } else {
                            P2PMessage::CheckpointResponse(data_str)
                        };
                        self.send_network_message(msg, "[SYNC] Failed to send checkpoint response");
                    }
                    Err(error) => warn!(
                        "[SYNC] Failed to serialize checkpoint #{} response: {}",
                        sequence, error
                    ),
                }
            }
            Ok(None) => warn!(
                "[SYNC] Checkpoint #{} not found in our engine for request",
                sequence
            ),
            Err(error) => warn!("[SYNC] Unable to serve checkpoint #{}: {}", sequence, error),
        }
    }

    async fn handle_checkpoint_response(
        &self,
        checkpoint_data: String,
        source_peer_id: Option<&str>,
    ) {
        if let Some(checkpoint) =
            Self::parse_message::<CheckpointSyncData>(&checkpoint_data, "checkpoint response")
        {
            self.process_incoming_checkpoint(
                checkpoint,
                source_peer_id,
                "checkpoint response",
                "checkpoint",
                false,
            )
            .await;
        }
    }

    async fn handle_peer_info(&self, peer_info: PeerInfoMsg) {
        let stats = self.engine.get_stats();
        let (local_checkpoint_hash, local_state_root) = self
            .local_checkpoint_identity(stats.height)
            .unwrap_or_else(|| {
                (
                    self.engine.latest_checkpoint_hash_hex(),
                    self.engine.latest_checkpoint_state_root_hex(),
                )
            });
        debug!(
            "[SYNC] Received PeerInfo from {}: height={}, txs={}, our_height={}, our_txs={}",
            peer_info.peer_id,
            peer_info.height,
            peer_info.total_transactions,
            stats.height,
            stats.total_transactions
        );

        if peer_info.peer_id != self.local_peer_id {
            self.clear_peer_divergence_if_recovered(
                &peer_info,
                stats.height,
                &local_checkpoint_hash,
                &local_state_root,
            );
        }

        if peer_info.height == stats.height {
            if let Some(kind) =
                Self::divergence_kind(&local_checkpoint_hash, &local_state_root, &peer_info)
            {
                warn!(
                    "[SYNC] Diverged state detected with peer {} at height {} (kind={}). local_checkpoint_hash={}, peer_checkpoint_hash={}, local_state_root={}, peer_state_root={}, local_txs={}, peer_txs={}",
                    peer_info.peer_id,
                    stats.height,
                    kind,
                    local_checkpoint_hash,
                    peer_info.latest_checkpoint_hash,
                    local_state_root,
                    peer_info.latest_state_root,
                    stats.total_transactions,
                    peer_info.total_transactions
                );
                self.mark_peer_divergent(&peer_info);
            } else if peer_info.peer_id != self.local_peer_id {
                self.record_peer_height(peer_info.peer_id.clone(), peer_info.height);
            }
        } else if peer_info.peer_id != self.local_peer_id
            && !self.is_peer_divergent(&peer_info.peer_id)
        {
            self.record_peer_height(peer_info.peer_id.clone(), peer_info.height);
        }

        if let Some(divergence) = self
            .divergent_peers_guard()
            .get(&peer_info.peer_id)
            .cloned()
        {
            warn!(
                "[SYNC] Peer {} remains quarantined due to divergent history at height {} (peer_checkpoint={}, peer_state_root={}). Local checkpoint={}, local_state_root={}.",
                peer_info.peer_id,
                divergence.height,
                divergence.latest_checkpoint_hash,
                divergence.latest_state_root,
                local_checkpoint_hash,
                local_state_root
            );
            return;
        }

        if peer_info.height > stats.height {
            info!(
                "[SYNC] Peer {} is ahead at height {} (current: {})",
                peer_info.peer_id, peer_info.height, stats.height
            );
            self.request_missing_checkpoints_from_peer(
                &peer_info.peer_id,
                peer_info.height,
                stats.height,
            )
            .await;
        } else {
            info!(
                "[SYNC] Peer {} is at height {} (current: {}). We are synced or ahead.",
                peer_info.peer_id, peer_info.height, stats.height
            );
        }
    }

    async fn request_checkpoints(&self, from: u64, to: u64, target_peer_id: Option<&str>) {
        if from > to {
            return;
        }

        let timestamp = Self::current_timestamp();

        // Limit the number of checkpoints requested at once to avoid network congestion.
        let actual_to = to.min(from + MAX_CHECKPOINTS_PER_REQUEST);
        let mut sent = 0u64;

        for sequence in from..=actual_to {
            // Check if we already have this checkpoint in buffer before requesting
            {
                let buffer = self.checkpoint_buffer_guard();
                if buffer.contains_key(&sequence) {
                    continue;
                }
            }

            if !self.should_request_checkpoint_sequence(sequence, timestamp) {
                continue;
            }

            let msg = if let Some(target_peer_id) = target_peer_id {
                P2PMessage::TargetedCheckpointRequest(CheckpointRequestMsg {
                    sequence,
                    timestamp,
                    requester_peer_id: self.local_peer_id.clone(),
                    responder_peer_id: target_peer_id.to_string(),
                })
            } else {
                P2PMessage::CheckpointRequest(sequence, timestamp)
            };

            if !self.send_network_message(
                msg,
                &format!(
                    "[SYNC] Failed to send checkpoint request for sequence {}",
                    sequence
                ),
            ) {
                break;
            }
            sent += 1;
        }
        if sent > 0 {
            info!(
                "[SYNC] Requested checkpoints {}..={} from {:?}",
                from, actual_to, target_peer_id
            );
        } else {
            debug!("[SYNC] Checkpoint request {} is still in cooldown", from);
        }
    }

    /// Broadcast local chain height to peers
    pub async fn broadcast_peer_info(&self) {
        let stats = self.engine.get_stats();
        let timestamp = Self::current_timestamp();

        let msg = P2PMessage::PeerInfo(PeerInfoMsg {
            height: stats.height,
            peer_id: self.local_peer_id.clone(),
            timestamp,
            latest_checkpoint_hash: self.engine.latest_checkpoint_hash_hex(),
            latest_state_root: self.engine.latest_checkpoint_state_root_hex(),
            total_transactions: stats.total_transactions,
        });

        self.send_network_message(msg, "Failed to broadcast peer info");
        self.broadcast_latest_dag_vertices(4, "during peer info broadcast");
    }

    /// Index a committed checkpoint using the indexer via its materialized block view.
    fn index_checkpoint_with_indexer(
        &self,
        indexer: &Arc<Mutex<kanari_indexer::Indexer>>,
        materialized_block_view: &kanari_core::FullBlockData,
    ) -> anyhow::Result<()> {
        let materialized_block = BlockchainEngine::block_from_full_data(materialized_block_view);
        let idx = indexer
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to acquire indexer lock: {}", e))?;
        idx.index_block(&materialized_block)?;

        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/sync_tests.rs"]
mod tests;
