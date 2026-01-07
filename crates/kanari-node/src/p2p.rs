// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use libp2p::{
    PeerId, Swarm, Transport,
    core::upgrade,
    futures::StreamExt,
    gossipsub::{self, IdentTopic, MessageAuthenticity, ValidationMode},
    identity::Keypair,
    kad::{self, store::MemoryStore},
    mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
    relay, dcutr, autonat,
};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};

/// P2P message types
#[derive(Debug, Clone, Serialize, Deserialize, bincode::Encode, bincode::Decode)]
pub enum P2PMessage {
    NewTransaction(String), // Serialized transaction
    NewBlock(String),       // Serialized FULL block with transactions
    BlockRequest(u64, u64), // (height, timestamp) - timestamp makes it unique
    BlockResponse(String),  // Full block data response with transactions
    PeerInfo(PeerInfoMsg),
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
    pub relay_client: relay::client::Behaviour,
    pub dcutr: dcutr::Behaviour,
    pub autonat: autonat::Behaviour,
}

pub struct P2PNetwork {
    pub swarm: Swarm<KanariBehaviour>,
    pub topics: P2PTopics,
}

pub struct P2PTopics {
    pub blocks: IdentTopic,
    pub transactions: IdentTopic,
    pub peers: IdentTopic,
}

impl P2PNetwork {
    pub fn new(keypair: Keypair, listen_port: u16) -> Result<Self> {
        let local_peer_id = PeerId::from(keypair.public());
        info!("Local peer id: {}", local_peer_id);

        // Create base TCP transport
        let transport = tcp::tokio::Transport::default()
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&keypair)?)
            .multiplex(yamux::Config::default())
            .boxed();

        // Create relay client for NAT traversal (will be used by behaviour)
        let (_relay_transport, relay_client) = relay::client::new(local_peer_id);

        // Create Gossipsub behavior
        let message_id_fn = |message: &gossipsub::Message| {
            let mut hasher = DefaultHasher::new();
            message.data.hash(&mut hasher);
            gossipsub::MessageId::from(hasher.finish().to_string())
        };

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(1))
            .validation_mode(ValidationMode::Permissive)
            .message_id_fn(message_id_fn)
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

        // Subscribe to topics
        gossipsub.subscribe(&blocks_topic)?;
        gossipsub.subscribe(&tx_topic)?;
        gossipsub.subscribe(&peers_topic)?;

        // Create mDNS for local peer discovery
        let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

        // Create Kademlia DHT for peer discovery
        let mut kademlia = kad::Behaviour::new(local_peer_id, MemoryStore::new(local_peer_id));

        // Bootstrap Kademlia
        kademlia.set_mode(Some(kad::Mode::Server));

        // Create DCUtR (Direct Connection Upgrade through Relay) for hole punching
        let dcutr = dcutr::Behaviour::new(local_peer_id);

        // Create AutoNAT for detecting NAT status
        let autonat = autonat::Behaviour::new(
            local_peer_id,
            autonat::Config {
                only_global_ips: false,
                ..Default::default()
            },
        );

        // Create behavior
        let behaviour = KanariBehaviour {
            gossipsub,
            mdns,
            kademlia,
            relay_client,
            dcutr,
            autonat,
        };

        // Create swarm
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
                blocks: blocks_topic,
                transactions: tx_topic,
                peers: peers_topic,
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
        };

        let config = bincode::config::standard();
        let data = bincode::encode_to_vec(&msg, config)
            .map_err(|e| anyhow::anyhow!("Failed to encode message: {}", e))?;

        // Publish and handle duplicate gracefully
        match self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic.clone(), data)
        {
            Ok(_) => Ok(()),
            Err(gossipsub::PublishError::Duplicate) => {
                // Duplicate is not an error, just skip silently
                Ok(())
            }
            Err(e) => {
                // Log warning but don't fail - could be no peers yet (InsufficientPeers/NoPeersSubscribedToTopic)
                warn!("Publish warning: {}", e);
                Ok(())
            }
        }
    }
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
                propagation_source: _,
                message_id: _,
                message,
            })) => {
                let config = bincode::config::standard();
                if let Ok((msg, _)) =
                    bincode::decode_from_slice::<P2PMessage, _>(&message.data, config)
                    && let Err(e) = self.message_tx.send(msg)
                {
                    warn!("Failed to forward P2P message: {}", e);
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
                        .add_address(&peer_id, multiaddr);
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
            SwarmEvent::ConnectionClosed {
                peer_id,
                cause,
                num_established,
                ..
            } => {
                info!(
                    "Connection closed with {} (remaining: {}) - {:?}",
                    peer_id, num_established, cause
                );
            }
            // Handle relay events
            SwarmEvent::Behaviour(KanariBehaviourEvent::RelayClient(
                relay::client::Event::ReservationReqAccepted { relay_peer_id, .. },
            )) => {
                info!("Relay reservation accepted by {}", relay_peer_id);
            }
            // Handle DCUtR events (hole punching)
            SwarmEvent::Behaviour(KanariBehaviourEvent::Dcutr(event)) => {
                info!("DCUtR event: {:?}", event);
            }
            // Handle AutoNAT events
            SwarmEvent::Behaviour(KanariBehaviourEvent::Autonat(autonat::Event::StatusChanged {
                old,
                new,
            })) => {
                info!("NAT status changed from {:?} to {:?}", old, new);
            }
            _ => {}
        }
    }
}
