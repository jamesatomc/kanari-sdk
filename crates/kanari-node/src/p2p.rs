// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use futures::StreamExt;
use libp2p_core::{Multiaddr, PeerId, Transport, upgrade};
use libp2p_dcutr as dcutr;
use libp2p_gossipsub as gossipsub;
use libp2p_gossipsub::{IdentTopic, MessageAuthenticity, ValidationMode};
use libp2p_identify as identify;
use libp2p_identity::Keypair;
use libp2p_kad::{self as kad, store::MemoryStore};
use libp2p_noise as noise;
use libp2p_ping as ping;
use libp2p_relay as relay;
use libp2p_swarm::{NetworkBehaviour, Swarm, SwarmEvent};
use libp2p_tcp as tcp;
use libp2p_yamux as yamux;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::{Read, Write},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::config::NodeRuntimeConfig;

const LARGE_MESSAGE_COMPRESSION_THRESHOLD: usize = 100_000;
const MAX_DECOMPRESSED_PAYLOAD_SIZE: usize = 32 * 1024 * 1024;
const MAX_GOSSIP_MESSAGE_SIZE: usize = 4 * 1024 * 1024;
const P2P_CHUNK_SIZE: usize = 512 * 1024;
const MAX_CHUNKED_PAYLOAD_SIZE: usize = 32 * 1024 * 1024;
const MAX_CHUNKS_PER_PAYLOAD: usize = 64;
const CHUNK_TTL: Duration = Duration::from_secs(30);

/// P2P message types
#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub enum P2PMessage {
    NewTransaction(String), // Serialized transaction
    NewCheckpoint(String),  // Serialized committed checkpoint sync payload
    NewDagVertex(String),   // Serialized DAG vertex for multi-node sync
    DagVertexRebroadcast(DagVertexMsg),
    DagVertexRequest(DagVertexRequestMsg),
    DagVertexResponse(DagVertexResponseMsg),
    CheckpointRequest(u64, u64), // (sequence, timestamp) - timestamp makes it unique
    CheckpointResponse(String),  // Serialized checkpoint sync payload
    TargetedCheckpointRequest(CheckpointRequestMsg),
    TargetedCheckpointResponse(CheckpointResponseMsg),
    PeerInfo(PeerInfoMsg),
    CompressedCheckpoint(Vec<u8>), // Compressed checkpoint sync payload (gzip)
    CompressedDagVertex(Vec<u8>),  // Compressed DAG vertex (gzip)
    CompressedCheckpointResponse(Vec<u8>), // Compressed checkpoint response (gzip)
    CompressedTargetedCheckpointResponse(CompressedCheckpointResponseMsg),
    Chunk(P2PMessageChunk),
}

/// Internal envelope binding a decoded gossip payload to the libp2p identity
/// that signed the original message. This is never serialized on the wire.
#[derive(Debug)]
pub struct AuthenticatedP2PMessage {
    pub source: PeerId,
    pub message: P2PMessage,
}

/// Internal envelope for locally queued outbound P2P messages.
///
/// This is intentionally not part of the serialized P2P protocol; it lets the
/// node measure local app/sync/RPC -> P2P publisher queue latency without
/// changing wire compatibility.
#[derive(Debug)]
pub struct QueuedP2PMessage {
    pub message: P2PMessage,
    pub enqueued_at: Instant,
}

impl QueuedP2PMessage {
    pub fn new(message: P2PMessage) -> Self {
        Self {
            message,
            enqueued_at: Instant::now(),
        }
    }
}

/// Keep state-recovery control traffic ahead of best-effort gossip when the
/// bounded outbound queue has accumulated a backlog.  A recovering validator
/// must be able to request and receive checkpoints even while transaction and
/// rebroadcast traffic is intentionally delayed by a chaos campaign.
fn outbound_priority(message: &P2PMessage) -> u8 {
    match message {
        P2PMessage::NewCheckpoint(_)
        | P2PMessage::CheckpointRequest(_, _)
        | P2PMessage::CheckpointResponse(_)
        | P2PMessage::TargetedCheckpointRequest(_)
        | P2PMessage::TargetedCheckpointResponse(_)
        | P2PMessage::CompressedCheckpoint(_)
        | P2PMessage::CompressedCheckpointResponse(_)
        | P2PMessage::CompressedTargetedCheckpointResponse(_)
        | P2PMessage::Chunk(P2PMessageChunk {
            topic: P2PTopicKind::Checkpoint,
            ..
        }) => 0,
        P2PMessage::DagVertexRequest(_)
        | P2PMessage::DagVertexResponse(_)
        | P2PMessage::NewDagVertex(_) => 1,
        P2PMessage::NewTransaction(_) => 2,
        P2PMessage::DagVertexRebroadcast(_)
        | P2PMessage::PeerInfo(_)
        | P2PMessage::CompressedDagVertex(_)
        | P2PMessage::Chunk(_) => 3,
    }
}

fn is_recovery_control_message(message: &P2PMessage) -> bool {
    matches!(
        message,
        P2PMessage::NewCheckpoint(_)
            | P2PMessage::CheckpointRequest(_, _)
            | P2PMessage::CheckpointResponse(_)
            | P2PMessage::TargetedCheckpointRequest(_)
            | P2PMessage::TargetedCheckpointResponse(_)
            | P2PMessage::CompressedCheckpoint(_)
            | P2PMessage::CompressedCheckpointResponse(_)
            | P2PMessage::CompressedTargetedCheckpointResponse(_)
            | P2PMessage::DagVertexRequest(_)
            | P2PMessage::DagVertexResponse(_)
            | P2PMessage::Chunk(P2PMessageChunk {
                topic: P2PTopicKind::Checkpoint,
                ..
            })
    )
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, bincode::Encode, bincode::Decode,
)]
pub enum P2PTopicKind {
    Checkpoint,
    Transaction,
    Peer,
    DagVertex,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct P2PMessageChunk {
    pub transfer_id: [u8; 32],
    pub topic: P2PTopicKind,
    pub index: u16,
    pub total: u16,
    pub data: Vec<u8>,
}

struct ChunkAssembly {
    created_at: Instant,
    topic: P2PTopicKind,
    chunks: Vec<Option<Vec<u8>>>,
    received: usize,
    bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct PeerInfoMsg {
    pub height: u64,
    pub peer_id: String,
    pub timestamp: u64,
    pub latest_checkpoint_hash: String,
    pub latest_state_root: String,
    pub total_transactions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct DagVertexMsg {
    pub vertex_data: String,
    pub nonce: u64,
    pub sender_peer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct DagVertexRequestMsg {
    pub requester_peer_id: String,
    pub parent_round: u64,
    pub timestamp: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct DagVertexResponseMsg {
    pub requester_peer_id: String,
    pub responder_peer_id: String,
    pub request_timestamp: u64,
    pub parent_round: u64,
    pub vertex_data: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct CheckpointRequestMsg {
    pub sequence: u64,
    pub timestamp: u64,
    pub requester_peer_id: String,
    pub responder_peer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct CheckpointResponseMsg {
    pub sequence: u64,
    pub request_timestamp: u64,
    pub requester_peer_id: String,
    pub responder_peer_id: String,
    pub checkpoint_data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct CompressedCheckpointResponseMsg {
    pub sequence: u64,
    pub request_timestamp: u64,
    pub requester_peer_id: String,
    pub responder_peer_id: String,
    pub compressed_checkpoint_data: Vec<u8>,
}

/// Network behavior combining multiple protocols
#[derive(NetworkBehaviour)]
#[behaviour(prelude = "libp2p_swarm::derive_prelude")]
pub struct KanariBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub dcutr: dcutr::Behaviour,
    pub identify: identify::Behaviour,
    pub ping: ping::Behaviour,
    pub relay: relay::Behaviour,
}

pub struct P2PNetwork {
    pub swarm: Swarm<KanariBehaviour>,
    pub topics: P2PTopics,
}

pub struct P2PTopics {
    pub checkpoints: IdentTopic,
    pub transactions: IdentTopic,
    pub peers: IdentTopic,
    pub dag_vertices: IdentTopic,
}

fn topic_namespace() -> String {
    std::env::var("KANARI_P2P_NAMESPACE")
        .or_else(|_| std::env::var("KANARI_NETWORK"))
        .unwrap_or_else(|_| "devnet".to_string())
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn namespaced_topic(namespace: &str, topic: &str) -> IdentTopic {
    IdentTopic::new(format!("kanari/{namespace}/{topic}"))
}

fn gzip_string(data: &str) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data.as_bytes())?;
    Ok(encoder.finish()?)
}

fn is_duplicate_publish_error(err: &gossipsub::PublishError) -> bool {
    let err_str = err.to_string();
    err_str.contains("Duplicate") || err_str.contains("duplicate")
}

fn is_insufficient_peers_error(err: &gossipsub::PublishError) -> bool {
    err.to_string().contains("InsufficientPeers")
}

impl P2PNetwork {
    pub fn new(keypair: Keypair, listen_port: u16, enable_relay_server: bool) -> Result<Self> {
        let local_peer_id = PeerId::from(keypair.public());
        info!("Local peer id: {}", local_peer_id);

        let transport = tcp::tokio::Transport::default()
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&keypair)?)
            .multiplex(yamux::Config::default())
            .boxed();

        let message_id_fn = |message: &gossipsub::Message| {
            let hash = kanari_crypto::hash_data_blake3(&message.data);
            gossipsub::MessageId::from(hex::encode(hash))
        };

        // Configure Gossipsub for large networks (200+ nodes)
        // Reference: https://github.com/libp2p/specs/blob/master/pubsub/gossipsub/gossipsub-v1.1.md#recommended-parameters
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(1))
            .validation_mode(ValidationMode::Strict)
            .message_id_fn(message_id_fn)
            // Parameters optimized for large networks (100-1000 nodes)
            .mesh_n_low(6) // Maintain at least 6 peers in mesh
            .mesh_n(12) // Target 12 peers in mesh
            .mesh_n_high(24) // Allow up to 24 peers in mesh
            .gossip_factor(0.25) // Gossip 25% of known messages to mesh peers
            .heartbeat_initial_delay(Duration::from_millis(100))
            .max_transmit_size(MAX_GOSSIP_MESSAGE_SIZE)
            .do_px() // Enable peer exchange for better discovery
            // Mesh propagation already provides redundancy. Flood publishing
            // multiplies large DAG/checkpoint messages by every connected peer
            // and can exhaust libp2p's per-peer queues under sustained load.
            .flood_publish(false)
            .build()
            .map_err(|e| anyhow::anyhow!("Gossipsub config error: {}", e))?;

        let mut gossipsub = gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create gossipsub: {}", e))?;

        let namespace = topic_namespace();
        info!(namespace = %namespace, "Using P2P topic namespace");
        let checkpoints_topic = namespaced_topic(&namespace, "checkpoints");
        let tx_topic = namespaced_topic(&namespace, "transactions");
        let peers_topic = namespaced_topic(&namespace, "peers");
        let dag_vertices_topic = namespaced_topic(&namespace, "dag_vertices");

        gossipsub.subscribe(&checkpoints_topic)?;
        gossipsub.subscribe(&tx_topic)?;
        gossipsub.subscribe(&peers_topic)?;
        gossipsub.subscribe(&dag_vertices_topic)?;

        let mut kademlia = kad::Behaviour::new(local_peer_id, MemoryStore::new(local_peer_id));

        kademlia.set_mode(Some(kad::Mode::Server));

        let dcutr = dcutr::Behaviour::new(local_peer_id);

        let identify = identify::Behaviour::new(identify::Config::new(
            "/kanari/1.0.0".to_string(),
            keypair.public(),
        ));

        let ping = ping::Behaviour::new(ping::Config::new());

        let relay_config = if enable_relay_server {
            relay::Config::default()
        } else {
            // Limit relay to essentially disable it without Option<T>
            relay::Config {
                max_reservations: 0,
                max_circuits: 0,
                ..Default::default()
            }
        };
        let relay = relay::Behaviour::new(local_peer_id, relay_config);

        let behaviour = KanariBehaviour {
            gossipsub,
            kademlia,
            dcutr,
            identify,
            ping,
            relay,
        };

        let mut swarm = Swarm::new(
            transport,
            behaviour,
            local_peer_id,
            libp2p_swarm::Config::with_tokio_executor()
                .with_idle_connection_timeout(Duration::from_secs(60)),
        );

        // Listen on all interfaces
        let listen_addr = format!("/ip4/0.0.0.0/tcp/{}", listen_port);
        swarm.listen_on(listen_addr.parse()?)?;

        Ok(Self {
            swarm,
            topics: P2PTopics {
                checkpoints: checkpoints_topic,
                transactions: tx_topic,
                peers: peers_topic,
                dag_vertices: dag_vertices_topic,
            },
        })
    }

    fn message_topic(&self, msg: &P2PMessage) -> &IdentTopic {
        match msg {
            P2PMessage::NewCheckpoint(_)
            | P2PMessage::CheckpointResponse(_)
            | P2PMessage::CheckpointRequest(_, _)
            | P2PMessage::TargetedCheckpointRequest(_)
            | P2PMessage::TargetedCheckpointResponse(_)
            | P2PMessage::CompressedCheckpoint(_)
            | P2PMessage::CompressedCheckpointResponse(_)
            | P2PMessage::CompressedTargetedCheckpointResponse(_)
            | P2PMessage::Chunk(P2PMessageChunk {
                topic: P2PTopicKind::Checkpoint,
                ..
            }) => &self.topics.checkpoints,
            P2PMessage::NewTransaction(_)
            | P2PMessage::Chunk(P2PMessageChunk {
                topic: P2PTopicKind::Transaction,
                ..
            }) => &self.topics.transactions,
            P2PMessage::PeerInfo(_)
            | P2PMessage::Chunk(P2PMessageChunk {
                topic: P2PTopicKind::Peer,
                ..
            }) => &self.topics.peers,
            P2PMessage::NewDagVertex(_)
            | P2PMessage::DagVertexRebroadcast(_)
            | P2PMessage::DagVertexRequest(_)
            | P2PMessage::DagVertexResponse(_)
            | P2PMessage::CompressedDagVertex(_)
            | P2PMessage::Chunk(P2PMessageChunk {
                topic: P2PTopicKind::DagVertex,
                ..
            }) => &self.topics.dag_vertices,
        }
    }

    fn message_topic_kind(msg: &P2PMessage) -> P2PTopicKind {
        match msg {
            P2PMessage::NewCheckpoint(_)
            | P2PMessage::CheckpointResponse(_)
            | P2PMessage::CheckpointRequest(_, _)
            | P2PMessage::TargetedCheckpointRequest(_)
            | P2PMessage::TargetedCheckpointResponse(_)
            | P2PMessage::CompressedCheckpoint(_)
            | P2PMessage::CompressedCheckpointResponse(_)
            | P2PMessage::CompressedTargetedCheckpointResponse(_) => P2PTopicKind::Checkpoint,
            P2PMessage::NewTransaction(_) => P2PTopicKind::Transaction,
            P2PMessage::PeerInfo(_) => P2PTopicKind::Peer,
            P2PMessage::NewDagVertex(_)
            | P2PMessage::DagVertexRebroadcast(_)
            | P2PMessage::DagVertexRequest(_)
            | P2PMessage::DagVertexResponse(_)
            | P2PMessage::CompressedDagVertex(_) => P2PTopicKind::DagVertex,
            P2PMessage::Chunk(chunk) => chunk.topic,
        }
    }

    fn log_published_message(msg: &P2PMessage) {
        match msg {
            P2PMessage::PeerInfo(info) => {
                debug!(
                    "[P2P] Publishing PeerInfo: height={}, peer_id={}",
                    info.height, info.peer_id
                );
            }
            P2PMessage::NewCheckpoint(data) => {
                debug!("[P2P] Publishing NewCheckpoint (size: {})", data.len());
            }
            P2PMessage::NewTransaction(data) => {
                debug!("[P2P] Publishing NewTransaction (size: {})", data.len());
            }
            P2PMessage::NewDagVertex(data) => {
                debug!("[P2P] Publishing NewDagVertex (size: {})", data.len());
            }
            P2PMessage::DagVertexRebroadcast(msg) => {
                debug!(
                    "[P2P] Publishing DagVertexRebroadcast: sender={}, nonce={}, size={}",
                    msg.sender_peer_id,
                    msg.nonce,
                    msg.vertex_data.len()
                );
            }
            P2PMessage::DagVertexRequest(req) => {
                debug!(
                    "[P2P] Publishing DagVertexRequest: requester={}, parent_round={}, limit={}",
                    req.requester_peer_id, req.parent_round, req.limit
                );
            }
            P2PMessage::DagVertexResponse(resp) => {
                debug!(
                    "[P2P] Publishing DagVertexResponse: responder={}, requester={}, parent_round={}, vertices={}",
                    resp.responder_peer_id,
                    resp.requester_peer_id,
                    resp.parent_round,
                    resp.vertex_data.len()
                );
            }
            P2PMessage::CheckpointRequest(seq, t) => {
                info!(
                    "[P2P] Publishing CheckpointRequest: sequence={}, ts={}",
                    seq, t
                );
            }
            P2PMessage::TargetedCheckpointRequest(req) => {
                info!(
                    "[P2P] Publishing TargetedCheckpointRequest: sequence={}, responder={}, requester={}, ts={}",
                    req.sequence, req.responder_peer_id, req.requester_peer_id, req.timestamp
                );
            }
            P2PMessage::CheckpointResponse(data) => {
                info!("[P2P] Publishing CheckpointResponse (size: {})", data.len());
            }
            P2PMessage::TargetedCheckpointResponse(resp) => {
                info!(
                    "[P2P] Publishing TargetedCheckpointResponse: sequence={}, requester={}, request_ts={}, size={}",
                    resp.sequence,
                    resp.requester_peer_id,
                    resp.request_timestamp,
                    resp.checkpoint_data.len()
                );
            }
            _ => {
                tracing::debug!("[P2P] Publishing message: {:?}", msg);
            }
        }
    }

    fn compress_large_message(msg: P2PMessage) -> Result<P2PMessage> {
        match msg {
            P2PMessage::NewCheckpoint(data) if data.len() > LARGE_MESSAGE_COMPRESSION_THRESHOLD => {
                let compressed_data = gzip_string(&data)?;
                info!(
                    "[P2P] Compressed checkpoint from {} to {} bytes",
                    data.len(),
                    compressed_data.len()
                );
                Ok(P2PMessage::CompressedCheckpoint(compressed_data))
            }
            P2PMessage::NewDagVertex(data) if data.len() > LARGE_MESSAGE_COMPRESSION_THRESHOLD => {
                let compressed_data = gzip_string(&data)?;
                info!(
                    "[P2P] Compressed DAG vertex from {} to {} bytes",
                    data.len(),
                    compressed_data.len()
                );
                Ok(P2PMessage::CompressedDagVertex(compressed_data))
            }
            P2PMessage::DagVertexRebroadcast(msg)
                if msg.vertex_data.len() > LARGE_MESSAGE_COMPRESSION_THRESHOLD =>
            {
                let compressed_data = gzip_string(&msg.vertex_data)?;
                info!(
                    "[P2P] Compressed rebroadcast DAG vertex from {} to {} bytes",
                    msg.vertex_data.len(),
                    compressed_data.len()
                );
                Ok(P2PMessage::CompressedDagVertex(compressed_data))
            }
            P2PMessage::CheckpointResponse(data)
                if data.len() > LARGE_MESSAGE_COMPRESSION_THRESHOLD =>
            {
                let compressed_data = gzip_string(&data)?;
                info!(
                    "[P2P] Compressed CheckpointResponse from {} to {} bytes",
                    data.len(),
                    compressed_data.len()
                );
                Ok(P2PMessage::CompressedCheckpointResponse(compressed_data))
            }
            P2PMessage::TargetedCheckpointResponse(resp)
                if resp.checkpoint_data.len() > LARGE_MESSAGE_COMPRESSION_THRESHOLD =>
            {
                let compressed_checkpoint_data = gzip_string(&resp.checkpoint_data)?;
                info!(
                    "[P2P] Compressed TargetedCheckpointResponse from {} to {} bytes",
                    resp.checkpoint_data.len(),
                    compressed_checkpoint_data.len()
                );
                Ok(P2PMessage::CompressedTargetedCheckpointResponse(
                    CompressedCheckpointResponseMsg {
                        sequence: resp.sequence,
                        request_timestamp: resp.request_timestamp,
                        requester_peer_id: resp.requester_peer_id,
                        responder_peer_id: resp.responder_peer_id,
                        compressed_checkpoint_data,
                    },
                ))
            }
            other => Ok(other),
        }
    }

    pub fn publish_message(&mut self, msg: P2PMessage) -> Result<()> {
        Self::log_published_message(&msg);
        let final_msg = Self::compress_large_message(msg)?;
        let config = bincode::config::standard();
        let data = bincode::encode_to_vec(&final_msg, config)
            .map_err(|e| anyhow::anyhow!("Failed to encode message: {}", e))?;

        if data.len() > MAX_GOSSIP_MESSAGE_SIZE {
            anyhow::ensure!(
                data.len() <= MAX_CHUNKED_PAYLOAD_SIZE,
                "Encoded P2P payload exceeds {} byte chunked-message limit",
                MAX_CHUNKED_PAYLOAD_SIZE
            );
            let total = data.len().div_ceil(P2P_CHUNK_SIZE);
            anyhow::ensure!(
                total <= MAX_CHUNKS_PER_PAYLOAD,
                "Encoded P2P payload requires too many chunks"
            );
            let digest = kanari_crypto::hash_data_blake3(&data);
            let mut transfer_id = [0u8; 32];
            transfer_id.copy_from_slice(&digest);
            let topic_kind = Self::message_topic_kind(&final_msg);
            for (index, bytes) in data.chunks(P2P_CHUNK_SIZE).enumerate() {
                let chunk = P2PMessage::Chunk(P2PMessageChunk {
                    transfer_id,
                    topic: topic_kind,
                    index: index as u16,
                    total: total as u16,
                    data: bytes.to_vec(),
                });
                let topic = self.message_topic(&chunk).clone();
                let encoded = bincode::encode_to_vec(&chunk, config)
                    .map_err(|e| anyhow::anyhow!("Failed to encode P2P chunk: {e}"))?;
                self.publish_encoded(topic, encoded, !is_recovery_control_message(&chunk))?;
            }
            return Ok(());
        }

        let topic = self.message_topic(&final_msg).clone();
        self.publish_encoded(topic, data, !is_recovery_control_message(&final_msg))
    }

    fn publish_encoded(
        &mut self,
        topic: IdentTopic,
        data: Vec<u8>,
        apply_synthetic_chaos: bool,
    ) -> Result<()> {
        if apply_synthetic_chaos {
            maybe_apply_chaos_publish_delay();
        }

        // Fault injection targets best-effort gossip. Recovery control traffic
        // must remain reliable enough to heal the deliberately delayed gossip.
        let duplicate_publishes = if apply_synthetic_chaos {
            chaos_duplicate_publish_count()
        } else {
            0
        };
        for _ in 0..duplicate_publishes {
            match self
                .swarm
                .behaviour_mut()
                .gossipsub
                .publish(topic.clone(), data.clone())
            {
                Ok(_) => {}
                Err(e) if is_duplicate_publish_error(&e) || is_insufficient_peers_error(&e) => {}
                Err(e) => {
                    tracing::debug!("Chaos duplicate P2P publish failed: {e}");
                }
            }
        }

        match self.swarm.behaviour_mut().gossipsub.publish(topic, data) {
            Ok(_) => Ok(()),
            Err(e) => {
                if is_duplicate_publish_error(&e) {
                    return Ok(());
                }
                if is_insufficient_peers_error(&e) {
                    tracing::debug!("No peers subscribed to topic yet");
                    return Ok(());
                }

                Err(anyhow::anyhow!("Failed to publish P2P message: {e}"))
            }
        }
    }
}

fn chaos_env_u64(name: &str, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.min(max))
        .unwrap_or(0)
}

fn maybe_apply_chaos_publish_delay() {
    let delay_ms = chaos_env_u64("KANARI_CHAOS_P2P_PUBLISH_DELAY_MS", 30_000);
    if delay_ms > 0 {
        std::thread::sleep(Duration::from_millis(delay_ms));
    }
}

fn chaos_duplicate_publish_count() -> usize {
    chaos_env_u64("KANARI_CHAOS_P2P_DUPLICATE_PUBLISHES", 8) as usize
}

fn chaos_reorder_best_effort_enabled() -> bool {
    std::env::var("KANARI_CHAOS_P2P_REORDER_BEST_EFFORT")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

/// Decompress a compressed UTF-8 P2P payload.
pub fn decompress_payload(compressed_data: Vec<u8>) -> Result<String> {
    let decoder = GzDecoder::new(&compressed_data[..]);
    let mut limited = decoder.take((MAX_DECOMPRESSED_PAYLOAD_SIZE + 1) as u64);
    let mut decompressed = Vec::new();
    limited.read_to_end(&mut decompressed)?;
    if decompressed.len() > MAX_DECOMPRESSED_PAYLOAD_SIZE {
        anyhow::bail!(
            "Decompressed P2P payload exceeds {} byte limit",
            MAX_DECOMPRESSED_PAYLOAD_SIZE
        );
    }
    String::from_utf8(decompressed)
        .map_err(|error| anyhow::anyhow!("Decompressed P2P payload is not UTF-8: {error}"))
}

pub struct P2PEventHandler {
    pub network: P2PNetwork,
    pub message_tx: mpsc::Sender<AuthenticatedP2PMessage>,
    pub outgoing_rx: Option<mpsc::Receiver<QueuedP2PMessage>>,
    pub peer_store: Option<std::sync::Arc<tokio::sync::Mutex<crate::peer_store::PeerStore>>>,
    message_forwarding_closed: bool,
    chunk_assemblies: HashMap<(PeerId, [u8; 32]), ChunkAssembly>,
    decompression_permits: std::sync::Arc<Semaphore>,
}

impl P2PEventHandler {
    pub fn new(network: P2PNetwork, message_tx: mpsc::Sender<AuthenticatedP2PMessage>) -> Self {
        Self {
            network,
            message_tx,
            outgoing_rx: None,
            peer_store: None,
            message_forwarding_closed: false,
            chunk_assemblies: HashMap::new(),
            decompression_permits: std::sync::Arc::new(Semaphore::new(
                NodeRuntimeConfig::p2p_max_concurrent_decompressions(),
            )),
        }
    }

    pub fn with_outgoing(mut self, outgoing_rx: mpsc::Receiver<QueuedP2PMessage>) -> Self {
        self.outgoing_rx = Some(outgoing_rx);
        self
    }

    pub fn with_peer_store(
        mut self,
        peer_store: std::sync::Arc<tokio::sync::Mutex<crate::peer_store::PeerStore>>,
    ) -> Self {
        self.peer_store = Some(peer_store);
        self
    }

    /// Re-dial persisted peers after a process restart.
    ///
    /// A restarted node cannot rely on the remote side retrying at exactly the
    /// instant its listener returns.  In particular, a four-node committee can
    /// otherwise leave the returning node disconnected forever: the original
    /// reconnect attempt happens while the process is still down, and all
    /// subsequent gossip/sync requests have no subscribers.  The peer store is
    /// advisory only; invalid or stale entries are ignored and normal swarm
    /// connection handling remains the authority for live peers.
    async fn dial_saved_peers(&mut self) {
        let peers = match &self.peer_store {
            Some(store) => store
                .lock()
                .await
                .peers
                .iter()
                .flat_map(|(peer_id, peer)| {
                    peer.addresses
                        .iter()
                        .cloned()
                        .map(move |address| (peer_id.clone(), address))
                })
                .collect::<Vec<_>>(),
            None => return,
        };

        for (stored_peer_id, address) in peers {
            if let Ok(peer_id) = stored_peer_id.parse::<PeerId>() {
                // A listen-address observed through an inbound connection can
                // otherwise make a node try to dial itself after a restart.
                if peer_id == *self.network.swarm.local_peer_id()
                    || self.network.swarm.is_connected(&peer_id)
                {
                    continue;
                }
            }
            match address.parse::<Multiaddr>() {
                Ok(address) => match self.network.swarm.dial(address.clone()) {
                    Ok(()) => debug!(%address, "Dialing persisted P2P peer"),
                    Err(error) => debug!(%address, %error, "Could not dial persisted P2P peer"),
                },
                Err(error) => {
                    warn!(%address, %error, "Ignoring invalid persisted P2P peer address")
                }
            }
        }
    }

    pub async fn run(&mut self) {
        // Do this before waiting for swarm events so a node returning from a
        // crash actively rejoins even if all remote reconnect attempts already
        // happened during its downtime.
        self.dial_saved_peers().await;
        let mut saved_peer_retry = tokio::time::interval(Duration::from_secs(5));
        // The first immediate tick would duplicate the startup dialing above.
        saved_peer_retry.tick().await;

        loop {
            tokio::select! {
                // Retrying is intentionally bounded by the small persisted peer
                // set and lets a node recover from a restart race without
                // depending on best-effort gossipsub publication.
                _ = saved_peer_retry.tick() => {
                    self.dial_saved_peers().await;
                }
                // Handle swarm events
                Some(event) = self.network.swarm.next() => {
                    self.handle_event(event).await;
                }
                // Handle outgoing messages to publish
                Some(queued) = async {
                    match &mut self.outgoing_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    // Drain a small bounded batch and publish recovery control
                    // messages first. This avoids a stale rebroadcast flood
                    // starving checkpoint catch-up after a node restart.
                    let mut batch = vec![queued];
                    if let Some(rx) = self.outgoing_rx.as_mut() {
                        for _ in 0..63 {
                            match rx.try_recv() {
                                Ok(message) => batch.push(message),
                                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => break,
                            }
                        }
                    }
                    batch.sort_by_key(|queued| outbound_priority(&queued.message));
                    // Test-only fault injection: recovery and DAG repair traffic
                    // keep their priority, while best-effort transaction and
                    // rebroadcast gossip is published in reverse local order.
                    if chaos_reorder_best_effort_enabled() {
                        let first_best_effort = batch
                            .iter()
                            .position(|queued| outbound_priority(&queued.message) > 1)
                            .unwrap_or(batch.len());
                        batch[first_best_effort..].reverse();
                    }
                    for queued in batch {
                        let queued_ms = queued.enqueued_at.elapsed().as_millis();
                        debug!(
                            p2p_outbound_queue_latency_ms = queued_ms,
                            p2p_outbound_priority = outbound_priority(&queued.message),
                            "P2P outbound queue latency"
                        );
                        if let Err(e) = self.network.publish_message(queued.message) {
                            warn!("Failed to publish outgoing message: {}", e);
                        }
                    }
                }
                else => break,
            }
        }
    }

    fn forward_message(&mut self, source: PeerId, msg: P2PMessage, context: &str) -> bool {
        if self.message_forwarding_closed || self.message_tx.is_closed() {
            if !self.message_forwarding_closed {
                warn!(
                    "{}: receiver channel is closed; suppressing further incoming P2P forwards",
                    context
                );
                self.message_forwarding_closed = true;
            }
            return false;
        }

        match self.message_tx.try_send(AuthenticatedP2PMessage {
            source,
            message: msg,
        }) {
            Ok(_) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                debug!("{}: incoming P2P queue is full; dropping message", context);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!(
                    "{}: receiver closed; suppressing further incoming P2P forwards",
                    context
                );
                self.message_forwarding_closed = true;
                false
            }
        }
    }

    fn spawn_decompressed_message<F>(
        &self,
        source: PeerId,
        compressed_data: Vec<u8>,
        make_message: F,
        failure_context: &'static str,
    ) where
        F: FnOnce(String) -> P2PMessage + Send + 'static,
    {
        let Ok(permit) = self.decompression_permits.clone().try_acquire_owned() else {
            debug!("{failure_context}: decompression pool is full; dropping message");
            return;
        };
        let message_tx = self.message_tx.clone();
        tokio::spawn(async move {
            let result =
                tokio::task::spawn_blocking(move || decompress_payload(compressed_data)).await;
            match result {
                Ok(Ok(data)) => {
                    let _ = message_tx.try_send(AuthenticatedP2PMessage {
                        source,
                        message: make_message(data),
                    });
                }
                Ok(Err(error)) => warn!("{failure_context}: {error}"),
                Err(error) => warn!("{failure_context}: worker failed: {error}"),
            }
            drop(permit);
        });
    }

    fn log_received_message(source: &PeerId, msg: &P2PMessage) {
        match msg {
            P2PMessage::PeerInfo(info) => {
                debug!(
                    "[P2P] Received PeerInfo from {}: height={}, peer_id={}",
                    source, info.height, info.peer_id
                );
            }
            P2PMessage::NewCheckpoint(data) => {
                debug!(
                    "[P2P] Received NewCheckpoint from {} (size: {})",
                    source,
                    data.len()
                );
            }
            P2PMessage::NewDagVertex(data) => {
                debug!(
                    "[P2P] Received NewDagVertex from {} (size: {})",
                    source,
                    data.len()
                );
            }
            P2PMessage::DagVertexRebroadcast(msg) => {
                debug!(
                    "[P2P] Received DagVertexRebroadcast from {}: sender={}, nonce={}, size={}",
                    source,
                    msg.sender_peer_id,
                    msg.nonce,
                    msg.vertex_data.len()
                );
            }
            P2PMessage::CompressedCheckpoint(compressed_data) => {
                info!(
                    "[P2P] Received CompressedCheckpoint from {} (size: {})",
                    source,
                    compressed_data.len()
                );
            }
            P2PMessage::CheckpointRequest(seq, t) => {
                info!(
                    "[P2P] Received CheckpointRequest from {}: sequence={}, ts={}",
                    source, seq, t
                );
            }
            P2PMessage::TargetedCheckpointRequest(req) => {
                info!(
                    "[P2P] Received TargetedCheckpointRequest from {}: sequence={}, responder={}, requester={}, ts={}",
                    source,
                    req.sequence,
                    req.responder_peer_id,
                    req.requester_peer_id,
                    req.timestamp
                );
            }
            P2PMessage::CheckpointResponse(data) => {
                info!(
                    "[P2P] Received CheckpointResponse from {} (size: {})",
                    source,
                    data.len()
                );
            }
            P2PMessage::TargetedCheckpointResponse(resp) => {
                info!(
                    "[P2P] Received TargetedCheckpointResponse from {}: sequence={}, requester={}, request_ts={}, size={}",
                    source,
                    resp.sequence,
                    resp.requester_peer_id,
                    resp.request_timestamp,
                    resp.checkpoint_data.len()
                );
            }
            P2PMessage::CompressedCheckpointResponse(compressed_data) => {
                info!(
                    "[P2P] Received CompressedCheckpointResponse from {} (size: {})",
                    source,
                    compressed_data.len()
                );
            }
            P2PMessage::CompressedTargetedCheckpointResponse(resp) => {
                info!(
                    "[P2P] Received CompressedTargetedCheckpointResponse from {}: sequence={}, requester={}, request_ts={}, size={}",
                    source,
                    resp.sequence,
                    resp.requester_peer_id,
                    resp.request_timestamp,
                    resp.compressed_checkpoint_data.len()
                );
            }
            P2PMessage::CompressedDagVertex(compressed_data) => {
                info!(
                    "[P2P] Received CompressedDagVertex from {} (size: {})",
                    source,
                    compressed_data.len()
                );
            }
            P2PMessage::DagVertexRequest(req) => {
                debug!(
                    "[P2P] Received DagVertexRequest from {}: requester={}, parent_round={}",
                    source, req.requester_peer_id, req.parent_round
                );
            }
            P2PMessage::DagVertexResponse(resp) => {
                debug!(
                    "[P2P] Received DagVertexResponse from {}: responder={}, requester={}, parent_round={}, vertices={}",
                    source,
                    resp.responder_peer_id,
                    resp.requester_peer_id,
                    resp.parent_round,
                    resp.vertex_data.len()
                );
            }
            P2PMessage::NewTransaction(data) => {
                debug!(
                    "[P2P] Received NewTransaction from {} (size: {})",
                    source,
                    data.len()
                );
            }
            P2PMessage::Chunk(chunk) => {
                debug!(
                    "[P2P] Received {:?} chunk from {} ({}/{}, size: {})",
                    chunk.topic,
                    source,
                    chunk.index.saturating_add(1),
                    chunk.total,
                    chunk.data.len()
                );
            }
        }
    }

    fn process_decoded_message(&mut self, source: PeerId, msg: P2PMessage) {
        Self::log_received_message(&source, &msg);
        match msg {
            P2PMessage::CompressedCheckpoint(compressed_data) => {
                self.spawn_decompressed_message(
                    source,
                    compressed_data,
                    P2PMessage::NewCheckpoint,
                    "[P2P] Failed to decompress checkpoint",
                );
            }
            P2PMessage::CompressedDagVertex(compressed_data) => {
                self.spawn_decompressed_message(
                    source,
                    compressed_data,
                    P2PMessage::NewDagVertex,
                    "[P2P] Failed to decompress DAG vertex",
                );
            }
            P2PMessage::CompressedCheckpointResponse(compressed_data) => {
                self.spawn_decompressed_message(
                    source,
                    compressed_data,
                    P2PMessage::CheckpointResponse,
                    "[P2P] Failed to decompress checkpoint response",
                );
            }
            P2PMessage::CompressedTargetedCheckpointResponse(resp) => {
                let sequence = resp.sequence;
                let request_timestamp = resp.request_timestamp;
                let requester_peer_id = resp.requester_peer_id;
                let responder_peer_id = resp.responder_peer_id;
                self.spawn_decompressed_message(
                    source,
                    resp.compressed_checkpoint_data,
                    move |checkpoint_data| {
                        P2PMessage::TargetedCheckpointResponse(CheckpointResponseMsg {
                            sequence,
                            request_timestamp,
                            requester_peer_id,
                            responder_peer_id,
                            checkpoint_data,
                        })
                    },
                    "[P2P] Failed to decompress targeted checkpoint response",
                );
            }
            P2PMessage::Chunk(_) => warn!("[P2P] Nested chunk message rejected"),
            message => {
                self.forward_message(source, message, "[P2P] Failed to forward P2P message");
            }
        }
    }

    fn accept_chunk(&mut self, source: PeerId, chunk: P2PMessageChunk) -> Option<P2PMessage> {
        self.chunk_assemblies
            .retain(|_, assembly| assembly.created_at.elapsed() <= CHUNK_TTL);
        let total = usize::from(chunk.total);
        let index = usize::from(chunk.index);
        if total == 0
            || total > MAX_CHUNKS_PER_PAYLOAD
            || index >= total
            || chunk.data.len() > P2P_CHUNK_SIZE
        {
            warn!("[P2P] Rejected malformed chunked payload");
            return None;
        }
        let key = (source, chunk.transfer_id);
        let max_inflight = NodeRuntimeConfig::p2p_max_inflight_chunked_payloads();
        if !self.chunk_assemblies.contains_key(&key) && self.chunk_assemblies.len() >= max_inflight
        {
            warn!("[P2P] Chunk assembly limit reached; dropping new transfer");
            return None;
        }
        let max_inflight_per_peer = NodeRuntimeConfig::p2p_max_inflight_chunked_payloads_per_peer();
        if !self.chunk_assemblies.contains_key(&key)
            && self
                .chunk_assemblies
                .keys()
                .filter(|(peer, _)| peer == &source)
                .count()
                >= max_inflight_per_peer
        {
            warn!("[P2P] Per-peer chunk assembly limit reached; dropping new transfer");
            return None;
        }
        let (reject, complete) = {
            let assembly = self
                .chunk_assemblies
                .entry(key)
                .or_insert_with(|| ChunkAssembly {
                    created_at: Instant::now(),
                    topic: chunk.topic,
                    chunks: vec![None; total],
                    received: 0,
                    bytes: 0,
                });
            if assembly.chunks.len() != total || assembly.topic != chunk.topic {
                (true, false)
            } else {
                if assembly.chunks[index].is_none() {
                    assembly.bytes = assembly.bytes.saturating_add(chunk.data.len());
                    if assembly.bytes <= MAX_CHUNKED_PAYLOAD_SIZE {
                        assembly.chunks[index] = Some(chunk.data);
                        assembly.received += 1;
                    }
                }
                (
                    assembly.bytes > MAX_CHUNKED_PAYLOAD_SIZE,
                    assembly.received == total,
                )
            }
        };
        if reject {
            self.chunk_assemblies.remove(&key);
            return None;
        }
        if !complete {
            return None;
        }

        let completed = self.chunk_assemblies.remove(&key)?;
        let mut encoded = Vec::with_capacity(completed.bytes);
        for bytes in completed.chunks.into_iter().flatten() {
            encoded.extend_from_slice(&bytes);
        }
        if kanari_crypto::hash_data_blake3(&encoded).as_slice() != chunk.transfer_id.as_slice() {
            warn!("[P2P] Reassembled chunk digest mismatch");
            return None;
        }
        match bincode::decode_from_slice::<P2PMessage, _>(&encoded, bincode::config::standard()) {
            Ok((message, consumed))
                if consumed == encoded.len()
                    && !matches!(message, P2PMessage::Chunk(_))
                    && P2PNetwork::message_topic_kind(&message) == completed.topic =>
            {
                Some(message)
            }
            Ok(_) => {
                warn!("[P2P] Reassembled message failed framing or topic validation");
                None
            }
            Err(error) => {
                warn!("[P2P] Failed to decode reassembled message: {error}");
                None
            }
        }
    }

    async fn handle_event(&mut self, event: SwarmEvent<KanariBehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("Listening on {}", address);
            }
            SwarmEvent::Behaviour(KanariBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                propagation_source,
                message_id: _,
                message,
            })) => {
                let authenticated_source = message.source.unwrap_or(propagation_source);
                let config = bincode::config::standard();
                match bincode::decode_from_slice::<P2PMessage, _>(&message.data, config) {
                    Ok((msg, _)) => {
                        if let P2PMessage::Chunk(chunk) = msg {
                            if let Some(reassembled) =
                                self.accept_chunk(authenticated_source, chunk)
                            {
                                self.process_decoded_message(authenticated_source, reassembled);
                            }
                        } else {
                            self.process_decoded_message(authenticated_source, msg);
                        }
                    }
                    Err(e) => {
                        warn!("[P2P] Failed to decode P2P message: {}", e);
                    }
                }
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                endpoint: _,
                num_established,
                cause,
                ..
            } => {
                info!(
                    "Connection to {} closed (cause: {:?}, remaining: {})",
                    peer_id, cause, num_established
                );
                if num_established == 0 {
                    // If no connections left to this peer, try to reconnect if it's in our peer store
                    if let Some(store_arc) = &self.peer_store {
                        let store = store_arc.lock().await;
                        if let Some(peer_info) = store.peers.get(&peer_id.to_string())
                            && let Some(addr_str) = peer_info.addresses.first()
                            && let Ok(addr) = addr_str.parse::<Multiaddr>()
                        {
                            info!("Attempting to reconnect to {} at {}...", peer_id, addr);
                            let _ = self.network.swarm.dial(addr);
                        }
                    }
                }
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                if let Some(pid) = peer_id {
                    warn!("Failed to connect to {}: {}", pid, error);
                } else {
                    warn!("Outgoing connection error: {}", error);
                }
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
                ..
            } => {
                // Bootstrap/static peers may never arrive through mDNS. Treat
                // every live connection as an explicit peer so the 4-node
                // committee does not depend on a mesh tuned for 6+ peers.
                self.network
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .add_explicit_peer(&peer_id);
                info!(
                    "Connection established with {} at {} (total: {})",
                    peer_id,
                    endpoint.get_remote_address(),
                    num_established
                );

                // Record liveness here, but do not persist the remote endpoint
                // as a dial target. For an inbound connection that endpoint is
                // often the peer's ephemeral source port, not its listener.
                // Persisting it causes a restart to redial stale ports and can
                // destabilize a small recovery mesh.
                if let Some(peer_store) = &self.peer_store {
                    let mut store = peer_store.lock().await;
                    store.add_peer(peer_id, Vec::new());

                    // Save to disk (async, ignore errors)
                    if let Err(e) = store.save() {
                        warn!("Failed to save peer store: {}", e);
                    }
                }
            }
            SwarmEvent::Behaviour(KanariBehaviourEvent::Dcutr(dcutr::Event {
                remote_peer_id,
                result,
            })) => match result {
                Ok(_) => {
                    info!("DCUtR hole punching succeeded with {}", remote_peer_id);
                }
                Err(e) => {
                    warn!(
                        "DCUtR hole punching failed with {}: {:?}",
                        remote_peer_id, e
                    );
                }
            },
            SwarmEvent::Behaviour(KanariBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
                info!(
                    "Identified peer {}: protocol {}, agent {}",
                    peer_id, info.protocol_version, info.agent_version
                );
                // Identify supplies the peer's advertised listener addresses.
                // Unlike the remote endpoint of an inbound connection, these
                // are safe to persist and redial after a process restart.
                let listen_addrs = info.listen_addrs;
                for addr in &listen_addrs {
                    self.network
                        .swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, addr.clone());
                }
                if let Some(peer_store) = &self.peer_store {
                    let mut store = peer_store.lock().await;
                    store.add_peer(peer_id, listen_addrs);
                    if let Err(error) = store.save() {
                        warn!(%error, "Failed to save identified peer addresses");
                    }
                }
            }
            SwarmEvent::Behaviour(KanariBehaviourEvent::Relay(
                relay::Event::ReservationReqAccepted { src_peer_id, .. },
            )) => {
                info!("Relay: Accepted reservation request from {}", src_peer_id);
            }
            SwarmEvent::Behaviour(KanariBehaviourEvent::Relay(
                relay::Event::CircuitReqAccepted {
                    src_peer_id,
                    dst_peer_id,
                },
            )) => {
                info!(
                    "Relay: Accepted circuit from {} to {}",
                    src_peer_id, dst_peer_id
                );
            }
            SwarmEvent::Behaviour(KanariBehaviourEvent::Relay(relay::Event::CircuitClosed {
                src_peer_id,
                dst_peer_id,
                ..
            })) => {
                info!(
                    "Relay: Circuit closed between {} and {}",
                    src_peer_id, dst_peer_id
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/p2p_tests.rs"]
mod tests;
