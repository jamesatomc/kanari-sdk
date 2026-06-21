// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use libp2p::{
    PeerId, Swarm, Transport,
    core::upgrade,
    dcutr,
    futures::StreamExt,
    gossipsub::{self, IdentTopic, MessageAuthenticity, ValidationMode},
    identify,
    identity::Keypair,
    kad::{self, store::MemoryStore},
    mdns, noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    time::Duration,
};
use tokio::sync::mpsc;
use tracing::{info, warn};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

const LARGE_MESSAGE_COMPRESSION_THRESHOLD: usize = 100_000;
const MAX_COMPRESSED_P2P_PAYLOAD: usize = 1_000_000;
const MAX_DECOMPRESSED_P2P_PAYLOAD: usize = 8 * 1024 * 1024;

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
    pub current_round: u64,
    pub target_round: u64,
    pub missing_authorities: Vec<String>,
    pub requester_vertex_data: Vec<String>,
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
pub struct KanariBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
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
            .validation_mode(ValidationMode::Permissive)
            .message_id_fn(message_id_fn)
            // Parameters optimized for large networks (100-1000 nodes)
            .mesh_n_low(6) // Maintain at least 6 peers in mesh
            .mesh_n(12) // Target 12 peers in mesh
            .mesh_n_high(24) // Allow up to 24 peers in mesh
            .gossip_factor(0.25) // Gossip 25% of known messages to mesh peers
            .heartbeat_initial_delay(Duration::from_millis(100))
            .max_transmit_size(1_000_000) // Increase max message size to 1MB for checkpoint payloads
            .do_px() // Enable peer exchange for better discovery
            // Add flood publishing for critical messages (checkpoints, vertices)
            .flood_publish(true)
            .build()
            .map_err(|e| anyhow::anyhow!("Gossipsub config error: {}", e))?;

        let mut gossipsub = gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create gossipsub: {}", e))?;

        let checkpoints_topic = IdentTopic::new("kanari/checkpoints");
        let tx_topic = IdentTopic::new("kanari/transactions");
        let peers_topic = IdentTopic::new("kanari/peers");
        let dag_vertices_topic = IdentTopic::new("kanari/dag_vertices");

        gossipsub.subscribe(&checkpoints_topic)?;
        gossipsub.subscribe(&tx_topic)?;
        gossipsub.subscribe(&peers_topic)?;
        gossipsub.subscribe(&dag_vertices_topic)?;

        let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

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
            mdns,
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
            libp2p::swarm::Config::with_tokio_executor()
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
            | P2PMessage::CompressedTargetedCheckpointResponse(_) => &self.topics.checkpoints,
            P2PMessage::NewTransaction(_) => &self.topics.transactions,
            P2PMessage::PeerInfo(_) => &self.topics.peers,
            P2PMessage::NewDagVertex(_)
            | P2PMessage::DagVertexRebroadcast(_)
            | P2PMessage::DagVertexRequest(_)
            | P2PMessage::DagVertexResponse(_)
            | P2PMessage::CompressedDagVertex(_) => &self.topics.dag_vertices,
        }
    }

    fn log_published_message(msg: &P2PMessage) {
        match msg {
            P2PMessage::PeerInfo(info) => {
                info!(
                    "[P2P] Publishing PeerInfo: height={}, peer_id={}",
                    info.height, info.peer_id
                );
            }
            P2PMessage::NewCheckpoint(data) => {
                info!("[P2P] Publishing NewCheckpoint (size: {})", data.len());
            }
            P2PMessage::NewDagVertex(data) => {
                info!("[P2P] Publishing NewDagVertex (size: {})", data.len());
            }
            P2PMessage::DagVertexRebroadcast(msg) => {
                info!(
                    "[P2P] Publishing DagVertexRebroadcast: sender={}, nonce={}, size={}",
                    msg.sender_peer_id,
                    msg.nonce,
                    msg.vertex_data.len()
                );
            }
            P2PMessage::DagVertexRequest(req) => {
                info!(
                    "[P2P] Publishing DagVertexRequest: requester={}, parent_round={}, target_round={}, missing={:?}, limit={}",
                    req.requester_peer_id,
                    req.parent_round,
                    req.target_round,
                    req.missing_authorities,
                    req.limit
                );
            }
            P2PMessage::DagVertexResponse(resp) => {
                info!(
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
        let topic = self.message_topic(&msg).clone();
        Self::log_published_message(&msg);
        let final_msg = Self::compress_large_message(msg)?;

        let config = bincode::config::standard();
        let data = bincode::encode_to_vec(&final_msg, config)
            .map_err(|e| anyhow::anyhow!("Failed to encode message: {}", e))?;

        // Publish and handle duplicate gracefully
        match self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic.clone(), data)
        {
            Ok(_) => Ok(()),
            Err(e) => {
                if is_duplicate_publish_error(&e) {
                    return Ok(());
                }
                if is_insufficient_peers_error(&e) {
                    tracing::debug!("No peers subscribed to topic yet");
                    return Ok(());
                }

                warn!("Publish warning: {}", e);
                Ok(())
            }
        }
    }
}

/// Decompress a compressed UTF-8 P2P payload with strict input/output bounds.
pub fn decompress_payload(compressed_data: Vec<u8>) -> Result<String> {
    if compressed_data.len() > MAX_COMPRESSED_P2P_PAYLOAD {
        anyhow::bail!(
            "Compressed P2P payload exceeds {} bytes",
            MAX_COMPRESSED_P2P_PAYLOAD
        );
    }

    let decoder = GzDecoder::new(&compressed_data[..]);
    let mut limited = decoder.take((MAX_DECOMPRESSED_P2P_PAYLOAD + 1) as u64);
    let mut decompressed = Vec::new();
    limited.read_to_end(&mut decompressed)?;

    if decompressed.len() > MAX_DECOMPRESSED_P2P_PAYLOAD {
        anyhow::bail!(
            "Decompressed P2P payload exceeds {} bytes",
            MAX_DECOMPRESSED_P2P_PAYLOAD
        );
    }

    String::from_utf8(decompressed)
        .map_err(|e| anyhow::anyhow!("Decompressed P2P payload is not valid UTF-8: {}", e))
}

pub struct P2PEventHandler {
    pub network: P2PNetwork,
    pub message_tx: mpsc::Sender<P2PMessage>,
    pub outgoing_rx: Option<mpsc::Receiver<P2PMessage>>,
    pub peer_store: Option<std::sync::Arc<tokio::sync::Mutex<crate::peer_store::PeerStore>>>,
    message_forwarding_closed: bool,
}

impl P2PEventHandler {
    pub fn new(network: P2PNetwork, message_tx: mpsc::Sender<P2PMessage>) -> Self {
        Self {
            network,
            message_tx,
            outgoing_rx: None,
            peer_store: None,
            message_forwarding_closed: false,
        }
    }

    pub fn with_outgoing(mut self, outgoing_rx: mpsc::Receiver<P2PMessage>) -> Self {
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

    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                // Handle swarm events
                Some(event) = self.network.swarm.next() => {
                    self.handle_event(event).await;
                }
                // Handle outgoing messages to publish
                Some(msg) = async {
                    match &mut self.outgoing_rx {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending().await,
                    }
                } => {
                    if let Err(e) = self.network.publish_message(msg) {
                        warn!("Failed to publish outgoing message: {}", e);
                    }
                }
                else => break,
            }
        }
    }

    fn forward_message(&mut self, msg: P2PMessage, context: &str) -> bool {
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

        match self.message_tx.try_send(msg) {
            Ok(_) => true,
            Err(e) => {
                warn!(
                    "{}: {}; suppressing further incoming P2P forwards",
                    context, e
                );
                self.message_forwarding_closed = true;
                false
            }
        }
    }

    fn forward_decompressed_message(
        &mut self,
        compressed_data: &[u8],
        decompress: fn(Vec<u8>) -> Result<String>,
        make_message: fn(String) -> P2PMessage,
        failure_context: &str,
        send_context: &str,
    ) -> bool {
        match decompress(compressed_data.to_vec()) {
            Ok(data) => self.forward_message(make_message(data), send_context),
            Err(e) => {
                warn!("{}: {}", failure_context, e);
                false
            }
        }
    }

    fn forward_targeted_checkpoint_response(
        &mut self,
        resp: &CompressedCheckpointResponseMsg,
        send_context: &str,
    ) -> bool {
        match decompress_payload(resp.compressed_checkpoint_data.clone()) {
            Ok(checkpoint_data) => self.forward_message(
                P2PMessage::TargetedCheckpointResponse(CheckpointResponseMsg {
                    sequence: resp.sequence,
                    request_timestamp: resp.request_timestamp,
                    requester_peer_id: resp.requester_peer_id.clone(),
                    responder_peer_id: resp.responder_peer_id.clone(),
                    checkpoint_data,
                }),
                send_context,
            ),
            Err(e) => {
                warn!(
                    "[P2P] Failed to decompress targeted checkpoint response: {}",
                    e
                );
                false
            }
        }
    }

    fn claimed_source_peer_id(msg: &P2PMessage) -> Option<&str> {
        match msg {
            P2PMessage::PeerInfo(info) => Some(&info.peer_id),
            P2PMessage::DagVertexRebroadcast(msg) => Some(&msg.sender_peer_id),
            P2PMessage::DagVertexRequest(req) => Some(&req.requester_peer_id),
            P2PMessage::DagVertexResponse(resp) => Some(&resp.responder_peer_id),
            P2PMessage::TargetedCheckpointRequest(req) => Some(&req.requester_peer_id),
            P2PMessage::TargetedCheckpointResponse(resp) => Some(&resp.responder_peer_id),
            P2PMessage::CompressedTargetedCheckpointResponse(resp) => Some(&resp.responder_peer_id),
            _ => None,
        }
    }

    fn log_received_message(source: &PeerId, msg: &P2PMessage) {
        match msg {
            P2PMessage::PeerInfo(info) => {
                info!(
                    "[P2P] Received PeerInfo from {}: height={}, peer_id={}",
                    source, info.height, info.peer_id
                );
            }
            P2PMessage::NewCheckpoint(data) => {
                info!(
                    "[P2P] Received NewCheckpoint from {} (size: {})",
                    source,
                    data.len()
                );
            }
            P2PMessage::NewDagVertex(data) => {
                info!(
                    "[P2P] Received NewDagVertex from {} (size: {})",
                    source,
                    data.len()
                );
            }
            P2PMessage::DagVertexRebroadcast(msg) => {
                info!(
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
                info!(
                    "[P2P] Received DagVertexRequest from {}: requester={}, parent_round={}, target_round={}, missing={:?}",
                    source,
                    req.requester_peer_id,
                    req.parent_round,
                    req.target_round,
                    req.missing_authorities
                );
            }
            P2PMessage::DagVertexResponse(resp) => {
                info!(
                    "[P2P] Received DagVertexResponse from {}: responder={}, requester={}, parent_round={}, vertices={}",
                    source,
                    resp.responder_peer_id,
                    resp.requester_peer_id,
                    resp.parent_round,
                    resp.vertex_data.len()
                );
            }
            _ => {
                info!("[P2P] Received message {:?} from {}", msg, source);
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
                let config = bincode::config::standard();
                match bincode::decode_from_slice::<P2PMessage, _>(&message.data, config) {
                    Ok((msg, _)) => {
                        if let Some(claimed_peer_id) = Self::claimed_source_peer_id(&msg)
                            && claimed_peer_id != propagation_source.to_string()
                        {
                            warn!(
                                "[P2P] Dropping message with source identity mismatch: transport={}, claimed={}",
                                propagation_source, claimed_peer_id
                            );
                            return;
                        }

                        Self::log_received_message(&propagation_source, &msg);

                        match &msg {
                            P2PMessage::CompressedCheckpoint(compressed_data) => {
                                self.forward_decompressed_message(
                                    compressed_data,
                                    decompress_payload,
                                    P2PMessage::NewCheckpoint,
                                    "[P2P] Failed to decompress checkpoint",
                                    "[P2P] Failed to forward decompressed checkpoint",
                                );
                                return;
                            }
                            P2PMessage::CompressedDagVertex(compressed_data) => {
                                self.forward_decompressed_message(
                                    compressed_data,
                                    decompress_payload,
                                    P2PMessage::NewDagVertex,
                                    "[P2P] Failed to decompress DAG vertex",
                                    "[P2P] Failed to forward decompressed vertex",
                                );
                                return;
                            }
                            P2PMessage::CompressedCheckpointResponse(compressed_data) => {
                                self.forward_decompressed_message(
                                    compressed_data,
                                    decompress_payload,
                                    P2PMessage::CheckpointResponse,
                                    "[P2P] Failed to decompress checkpoint response",
                                    "[P2P] Failed to forward decompressed checkpoint response",
                                );
                                return;
                            }
                            P2PMessage::CompressedTargetedCheckpointResponse(resp) => {
                                self.forward_targeted_checkpoint_response(
                                    resp,
                                    "[P2P] Failed to forward decompressed targeted checkpoint response",
                                );
                                return;
                            }
                            _ => {}
                        }

                        self.forward_message(msg, "[P2P] Failed to forward P2P message");
                    }
                    Err(e) => {
                        warn!("[P2P] Failed to decode P2P message: {}", e);
                    }
                }
            }
            SwarmEvent::Behaviour(KanariBehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                for (peer_id, multiaddr) in peers {
                    info!("Discovered peer: {} at {}", peer_id, multiaddr);
                    self.network
                        .swarm
                        .behaviour_mut()
                        .gossipsub
                        .add_explicit_peer(&peer_id);
                    self.network
                        .swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, multiaddr.clone());

                    // Explicitly dial discovered peer to ensure connection
                    if let Err(e) = self.network.swarm.dial(multiaddr.clone()) {
                        warn!("Failed to dial discovered peer {}: {}", peer_id, e);
                    } else {
                        // Add to peer store if available
                        if let Some(store_arc) = &self.peer_store {
                            let mut store = store_arc.lock().await;
                            store.add_peer(peer_id, vec![multiaddr.clone()]);
                            let _ = store.save();
                        }
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
                            && let Ok(addr) = addr_str.parse::<libp2p::Multiaddr>()
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
            SwarmEvent::Behaviour(KanariBehaviourEvent::Mdns(mdns::Event::Expired(peers))) => {
                for (peer_id, _) in peers {
                    info!("Peer expired: {}", peer_id);
                    self.network
                        .swarm
                        .behaviour_mut()
                        .gossipsub
                        .remove_explicit_peer(&peer_id);
                }
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                endpoint,
                num_established,
                ..
            } => {
                info!(
                    "Connection established with {} at {} (total: {})",
                    peer_id,
                    endpoint.get_remote_address(),
                    num_established
                );

                // Save peer to persistent store
                if let Some(peer_store) = &self.peer_store {
                    let mut store = peer_store.lock().await;
                    let addresses = vec![endpoint.get_remote_address().clone()];
                    store.add_peer(peer_id, addresses);

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
                // Add identified addresses to Kademlia
                for addr in info.listen_addrs {
                    self.network
                        .swarm
                        .behaviour_mut()
                        .kademlia
                        .add_address(&peer_id, addr);
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
