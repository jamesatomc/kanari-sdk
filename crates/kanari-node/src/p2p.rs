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
use std::{io::Write, time::Duration};
use tokio::sync::mpsc;
use tracing::{info, warn};

// Add compression support
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::io::Read;

/// P2P message types
#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub enum P2PMessage {
    NewTransaction(String), // Serialized transaction
    NewBlock(String),       // Serialized FULL block with transactions
    NewDagVertex(String),   // Serialized DAG vertex for multi-node sync
    BlockRequest(u64, u64), // (height, timestamp) - timestamp makes it unique
    BlockResponse(String),  // Full block data response with transactions
    PeerInfo(PeerInfoMsg),
    // Add compressed message types for large data
    CompressedBlock(Vec<u8>),         // Compressed full block data (gzip)
    CompressedDagVertex(Vec<u8>),     // Compressed DAG vertex (gzip)
    CompressedBlockResponse(Vec<u8>), // Compressed block response with transactions (gzip)
}

#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub struct PeerInfoMsg {
    pub height: u64,
    pub peer_id: String,
    pub timestamp: u64, // Add timestamp to make messages unique
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
    pub blocks: IdentTopic,
    pub transactions: IdentTopic,
    pub peers: IdentTopic,
    pub dag_vertices: IdentTopic,
}

impl P2PNetwork {
    pub fn new(keypair: Keypair, listen_port: u16, enable_relay_server: bool) -> Result<Self> {
        let local_peer_id = PeerId::from(keypair.public());
        info!("Local peer id: {}", local_peer_id);

        // 🟢 1. สร้าง QUIC Transport (เร็วที่สุด ไม่ติด Head-of-line blocking)
        let quic_transport =
            libp2p::quic::tokio::Transport::new(libp2p::quic::Config::new(&keypair));

        // 🟢 2. สร้าง TCP Transport เป็น Fallback (สำหรับโหนดที่บล็อก UDP)
        let tcp_transport = tcp::tokio::Transport::new(tcp::Config::default().nodelay(true))
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&keypair)?)
            .multiplex(yamux::Config::default());

        // 🟢 3. รวมทั้งคู่เข้าด้วยกัน โดยให้ QUIC เป็นความสำคัญหลัก
        let transport = libp2p::core::transport::OrTransport::new(quic_transport, tcp_transport)
            .map(|either_output, _| match either_output {
                // 🟢 เปลี่ยนจาก EitherOutput::First เป็น Either::Left
                libp2p::futures::future::Either::Left((peer_id, muxer)) => {
                    (peer_id, libp2p::core::muxing::StreamMuxerBox::new(muxer))
                }
                // 🟢 เปลี่ยนจาก EitherOutput::Second เป็น Either::Right
                libp2p::futures::future::Either::Right((peer_id, muxer)) => {
                    (peer_id, libp2p::core::muxing::StreamMuxerBox::new(muxer))
                }
            })
            .boxed();

        // Create Gossipsub behavior
        let message_id_fn = |message: &gossipsub::Message| {
            // Use deterministic Blake3 hash for message ID instead of DefaultHasher
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
            .max_transmit_size(1_000_000) // Increase max message size to 1MB for block data
            .do_px() // Enable peer exchange for better discovery
            // Add flood publishing for critical messages (blocks, vertices)
            .flood_publish(true)
            .build()
            .map_err(|e| anyhow::anyhow!("Gossipsub config error: {}", e))?;

        let mut gossipsub = gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create gossipsub: {}", e))?;

        // Create topics
        let blocks_topic = IdentTopic::new("kanari/blocks");
        let tx_topic = IdentTopic::new("kanari/transactions");
        let peers_topic = IdentTopic::new("kanari/peers");
        let dag_vertices_topic = IdentTopic::new("kanari/dag_vertices");

        // Subscribe to topics
        gossipsub.subscribe(&blocks_topic)?;
        gossipsub.subscribe(&tx_topic)?;
        gossipsub.subscribe(&peers_topic)?;
        gossipsub.subscribe(&dag_vertices_topic)?;

        // Create mDNS for local peer discovery
        let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

        // Create Kademlia DHT for peer discovery
        let mut kademlia = kad::Behaviour::new(local_peer_id, MemoryStore::new(local_peer_id));

        // Bootstrap Kademlia
        kademlia.set_mode(Some(kad::Mode::Server));

        // Create DCUtR for hole punching (works without relay client for direct connections)
        let dcutr = dcutr::Behaviour::new(local_peer_id);

        // Create Identify protocol for peer information exchange
        let identify = identify::Behaviour::new(identify::Config::new(
            "/kanari/1.0.0".to_string(),
            keypair.public(),
        ));

        // Create Ping for connection keep-alive
        let ping = ping::Behaviour::new(ping::Config::new());

        // Create relay server (will only accept relay requests if configured properly)
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

        // Create behavior
        let behaviour = KanariBehaviour {
            gossipsub,
            mdns,
            kademlia,
            dcutr,
            identify,
            ping,
            relay,
        };

        // Create swarm
        let mut swarm = Swarm::new(
            transport,
            behaviour,
            local_peer_id,
            libp2p::swarm::Config::with_tokio_executor()
                .with_idle_connection_timeout(Duration::from_secs(60)),
        );

        // 🟢 Listen on both TCP and QUIC (UDP)
        let listen_addr_tcp = format!("/ip4/0.0.0.0/tcp/{}", listen_port);
        let listen_addr_quic = format!("/ip4/0.0.0.0/udp/{}/quic-v1", listen_port);

        swarm.listen_on(listen_addr_tcp.parse()?)?;
        swarm.listen_on(listen_addr_quic.parse()?)?;

        Ok(Self {
            swarm,
            topics: P2PTopics {
                blocks: blocks_topic,
                transactions: tx_topic,
                peers: peers_topic,
                dag_vertices: dag_vertices_topic,
            },
        })
    }

    pub fn publish_message(&mut self, msg: P2PMessage) -> Result<()> {
        let topic = match &msg {
            P2PMessage::NewBlock(_)
            | P2PMessage::BlockResponse(_)
            | P2PMessage::BlockRequest(_, _) => &self.topics.blocks,
            P2PMessage::NewTransaction(_) => &self.topics.transactions,
            P2PMessage::PeerInfo(_) => &self.topics.peers,
            P2PMessage::NewDagVertex(_) => &self.topics.dag_vertices,
            P2PMessage::CompressedBlock(_) => &self.topics.blocks,
            P2PMessage::CompressedDagVertex(_) => &self.topics.dag_vertices,
            P2PMessage::CompressedBlockResponse(_) => &self.topics.blocks,
        };

        // Log message publication for debugging
        match &msg {
            P2PMessage::PeerInfo(info) => {
                info!(
                    "[P2P] Publishing PeerInfo: height={}, peer_id={}",
                    info.height, info.peer_id
                );
            }
            P2PMessage::NewBlock(data) => {
                info!("[P2P] Publishing NewBlock (size: {})", data.len());
            }
            P2PMessage::NewDagVertex(data) => {
                info!("[P2P] Publishing NewDagVertex (size: {})", data.len());
            }
            P2PMessage::BlockRequest(h, t) => {
                info!("[P2P] Publishing BlockRequest: height={}, ts={}", h, t);
            }
            P2PMessage::BlockResponse(data) => {
                info!("[P2P] Publishing BlockResponse (size: {})", data.len());
            }
            _ => {
                tracing::debug!("[P2P] Publishing message: {:?}", msg);
            }
        }

        // Handle compression for large messages
        let final_msg = match msg {
            P2PMessage::NewBlock(ref data) if data.len() > 100_000 => {
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(data.as_bytes())?;
                let compressed_data = encoder.finish()?;
                info!(
                    "[P2P] Compressed block from {} to {} bytes",
                    data.len(),
                    compressed_data.len()
                );
                P2PMessage::CompressedBlock(compressed_data)
            }
            P2PMessage::NewDagVertex(ref data) if data.len() > 100_000 => {
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(data.as_bytes())?;
                let compressed_data = encoder.finish()?;
                info!(
                    "[P2P] Compressed DAG vertex from {} to {} bytes",
                    data.len(),
                    compressed_data.len()
                );
                P2PMessage::CompressedDagVertex(compressed_data)
            }
            P2PMessage::BlockResponse(ref data) if data.len() > 100_000 => {
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(data.as_bytes())?;
                let compressed_data = encoder.finish()?;
                info!(
                    "[P2P] Compressed BlockResponse from {} to {} bytes",
                    data.len(),
                    compressed_data.len()
                );
                P2PMessage::CompressedBlockResponse(compressed_data)
            }
            _ => msg,
        };

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
                let err_str = e.to_string();
                // Handle duplicate messages gracefully
                if err_str.contains("Duplicate") || err_str.contains("duplicate") {
                    // Duplicate is not an error, just skip silently
                    return Ok(());
                }
                if err_str.contains("InsufficientPeers") {
                    tracing::debug!("No peers subscribed to topic yet");
                    return Ok(());
                }

                // Log warning but don't fail
                warn!("Publish warning: {}", e);
                Ok(())
            }
        }
    }
}

/// Decompress a compressed block message
pub fn decompress_block(compressed_data: Vec<u8>) -> Result<String> {
    let mut decoder = GzDecoder::new(&compressed_data[..]);
    let mut decompressed = String::new();
    decoder.read_to_string(&mut decompressed)?;
    Ok(decompressed)
}

/// Decompress a compressed DAG vertex message
pub fn decompress_dag_vertex(compressed_data: Vec<u8>) -> Result<String> {
    let mut decoder = GzDecoder::new(&compressed_data[..]);
    let mut decompressed = String::new();
    decoder.read_to_string(&mut decompressed)?;
    Ok(decompressed)
}

pub struct P2PEventHandler {
    pub network: P2PNetwork,
    pub message_tx: mpsc::UnboundedSender<P2PMessage>,
    pub outgoing_rx: Option<mpsc::UnboundedReceiver<P2PMessage>>,
    pub peer_store: Option<std::sync::Arc<tokio::sync::Mutex<crate::peer_store::PeerStore>>>,
}

impl P2PEventHandler {
    pub fn new(network: P2PNetwork, message_tx: mpsc::UnboundedSender<P2PMessage>) -> Self {
        Self {
            network,
            message_tx,
            outgoing_rx: None,
            peer_store: None,
        }
    }

    pub fn with_outgoing(mut self, outgoing_rx: mpsc::UnboundedReceiver<P2PMessage>) -> Self {
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
                        // Log message reception for debugging
                        match &msg {
                            P2PMessage::PeerInfo(info) => {
                                info!(
                                    "[P2P] Received PeerInfo from {}: height={}, peer_id={}",
                                    propagation_source, info.height, info.peer_id
                                );
                            }
                            P2PMessage::NewBlock(data) => {
                                info!(
                                    "[P2P] Received NewBlock from {} (size: {})",
                                    propagation_source,
                                    data.len()
                                );
                            }
                            P2PMessage::NewDagVertex(data) => {
                                info!(
                                    "[P2P] Received NewDagVertex from {} (size: {})",
                                    propagation_source,
                                    data.len()
                                );
                            }
                            P2PMessage::CompressedBlock(compressed_data) => {
                                info!(
                                    "[P2P] Received CompressedBlock from {} (size: {})",
                                    propagation_source,
                                    compressed_data.len()
                                );
                                // Decompress and convert to NewBlock
                                match decompress_block(compressed_data.to_vec()) {
                                    Ok(block_data) => {
                                        let new_msg = P2PMessage::NewBlock(block_data);
                                        if let Err(e) = self.message_tx.send(new_msg) {
                                            warn!(
                                                "[P2P] Failed to forward decompressed block: {}",
                                                e
                                            );
                                        }
                                        return; // Already sent, don't send original msg
                                    }
                                    Err(e) => {
                                        warn!("[P2P] Failed to decompress block: {}", e);
                                        return;
                                    }
                                }
                            }
                            P2PMessage::CompressedDagVertex(compressed_data) => {
                                info!(
                                    "[P2P] Received CompressedDagVertex from {} (size: {})",
                                    propagation_source,
                                    compressed_data.len()
                                );
                                // Decompress and convert to NewDagVertex
                                match decompress_dag_vertex(compressed_data.to_vec()) {
                                    Ok(vertex_data) => {
                                        let new_msg = P2PMessage::NewDagVertex(vertex_data);
                                        if let Err(e) = self.message_tx.send(new_msg) {
                                            warn!(
                                                "[P2P] Failed to forward decompressed vertex: {}",
                                                e
                                            );
                                        }
                                        return; // Already sent, don't send original msg
                                    }
                                    Err(e) => {
                                        warn!("[P2P] Failed to decompress DAG vertex: {}", e);
                                        return;
                                    }
                                }
                            }
                            P2PMessage::BlockRequest(h, t) => {
                                info!(
                                    "[P2P] Received BlockRequest from {}: height={}, ts={}",
                                    propagation_source, h, t
                                );
                            }
                            P2PMessage::BlockResponse(data) => {
                                info!(
                                    "[P2P] Received BlockResponse from {} (size: {})",
                                    propagation_source,
                                    data.len()
                                );
                            }
                            _ => {
                                info!(
                                    "[P2P] Received message {:?} from {}",
                                    msg, propagation_source
                                );
                            }
                        }

                        if let Err(e) = self.message_tx.send(msg) {
                            warn!("[P2P] Failed to forward P2P message: {}", e);
                        }
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
