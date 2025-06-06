use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use log::{info, warn, debug};
use mona_types::storage::block::Block;
use mona_types::Transaction;
use tokio::sync::mpsc;
use serde::{Serialize, Deserialize};
use consensus_pos::Blake3Algorithm;
use mona_types::storage::BlockchainError;
use std::net::ToSocketAddrs;

use super::{send_message_to_peer, ACTIVE_CONNECTIONS, NodeMessage};

/// Network statistics tracker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    pub blocks_received: u64,
    pub blocks_sent: u64,
    pub transactions_received: u64,
    pub transactions_sent: u64,
    pub peers_connected: usize,
    pub bytes_received: u64,
    pub bytes_sent: u64,
    pub last_update: u64,
}

impl Default for NetworkStats {
    fn default() -> Self {
        Self {
            blocks_received: 0,
            blocks_sent: 0,
            transactions_received: 0,
            transactions_sent: 0,
            peers_connected: 0,
            bytes_received: 0,
            bytes_sent: 0,
            last_update: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

// Global statistics
lazy_static::lazy_static! {
    pub static ref NETWORK_STATS: Arc<Mutex<NetworkStats>> = Arc::new(Mutex::new(NetworkStats::default()));
}

// Domain resolution helper - Resolves domain names to IP addresses
pub fn resolve_domain(addr: &str) -> Result<String, BlockchainError> {
    debug!("Resolving address: {}", addr);
    
    // Check if address contains a port
    let (host, port) = if addr.contains(':') {
        let parts: Vec<&str> = addr.split(':').collect();
        if parts.len() != 2 {
            return Err(BlockchainError::Network(format!("Invalid address format: {}", addr)));
        }
        (parts[0], parts[1])
    } else {
        // Default to P2P port if no port specified
        (addr, "51303")
    };
    
    // If it's already an IP address, just return it
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(addr.to_string());
    }
    
    // Attempt DNS resolution
    let socket_addr = format!("{}:{}", host, port);
    info!("Attempting DNS resolution for {}", socket_addr);
    
    match socket_addr.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(addr) = addrs.next() {
                let resolved = format!("{}:{}", addr.ip(), addr.port());
                info!("Resolved {} to {}", socket_addr, resolved);
                Ok(resolved)
            } else {
                Err(BlockchainError::Network(format!("Could not resolve domain: {}", host)))
            }
        },
        Err(e) => {
            Err(BlockchainError::Network(format!("DNS resolution failed for {}: {}", host, e)))
        }
    }
}

// Broadcast a transaction to all peers
pub fn broadcast_transaction(transaction: &Transaction) -> Result<(), BlockchainError> {
    let transaction_id = transaction.transaction_id.clone();
    
    // Create transaction announcement with just the ID
    // Peers will request the full transaction if needed
    let announcement = NodeMessage::TransactionAnnounce {
        transaction_ids: vec![transaction_id.clone()],
    };
    
    // Get list of connected peers
    let peer_ids = {
        let connections = ACTIVE_CONNECTIONS.read().unwrap();
        connections.keys().cloned().collect::<Vec<_>>()
    };
    
    // Exit early if no peers
    if peer_ids.is_empty() {
        debug!("No peers to broadcast transaction {}", transaction_id);
        return Ok(());
    }
    
    info!("Broadcasting transaction {} to {} peers", transaction_id, peer_ids.len());
    
    // Broadcast to all peers
    let mut success_count = 0;
    for peer_id in &peer_ids {
        if let Err(e) = send_message_to_peer(peer_id, &announcement) {
            warn!("Failed to announce transaction to peer {}: {}", peer_id, e);
        } else {
            success_count += 1;
            debug!("Transaction {} announced to peer {}", transaction_id, peer_id);
        }
    }
    
    // Update statistics
    if success_count > 0 {
        let mut stats = NETWORK_STATS.lock().unwrap();
        // Increments by the number of peers the transaction was successfully announced to.
        stats.transactions_sent += success_count as u64;
        stats.last_update = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }
    
    Ok(())
}

// Broadcast a block to all peers
pub fn broadcast_block(block: &Block<Blake3Algorithm>, status_tx: &mpsc::Sender<String>) -> Result<(), BlockchainError> {
    // Create block announcement
    let announcement = NodeMessage::BlockAnnounce {
        block_index: block.index as u64, // Convert u32 to u64
        block_hash: block.hash.clone(),
    };
    
    // Get list of connected peers
    let peer_ids = {
        let connections = ACTIVE_CONNECTIONS.read().unwrap();
        connections.keys().cloned().collect::<Vec<_>>()
    };
    
    // Exit early if no peers
    if peer_ids.is_empty() {
        debug!("No peers to broadcast block {}", block.index);
        return Ok(());
    }
    
    info!("Broadcasting block {} to {} peers", block.index, peer_ids.len());
    
    // Broadcast to all peers
    let mut success_count = 0;
    for peer_id in &peer_ids {
        if let Err(e) = send_message_to_peer(peer_id, &announcement) {
            warn!("Failed to announce block to peer {}: {}", peer_id, e);
        } else {
            success_count += 1;
            debug!("Block {} announced to peer {}", block.index, peer_id);
        }
    }
    
    // Update statistics
    if success_count > 0 {
        let mut stats = NETWORK_STATS.lock().unwrap();
        stats.blocks_sent += 1;
        stats.last_update = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
    }
    
    // Notify UI about the block broadcast
    let broadcast_status = serde_json::json!({
        "event": "block_broadcast",
        "block_index": block.index,
        "block_hash": block.hash,
        "peer_count": peer_ids.len(),
        "success_count": success_count
    }).to_string();
    
    let _ = status_tx.blocking_send(broadcast_status);
    
    Ok(())
}

// Get the latest network statistics
pub fn get_network_statistics() -> NetworkStats {
    let mut stats = NETWORK_STATS.lock().unwrap();
    
    // Update peer count
    stats.peers_connected = super::get_peer_count();
    
    // Update timestamp
    stats.last_update = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    stats.clone()
}

// Check if we're connected to a specific peer
pub fn is_connected_to_peer(peer_id: &str) -> bool {
    let connections = ACTIVE_CONNECTIONS.read().unwrap();
    connections.contains_key(peer_id)
}

// Get a list of node IDs we're connected to
pub fn get_connected_peers() -> Vec<String> {
    let connections = ACTIVE_CONNECTIONS.read().unwrap();
    connections.keys().cloned().collect()
}

// Sync blockchain with a specific peer
pub fn request_sync_from_peer(peer_id: &str) -> Result<(), BlockchainError> {
    info!("Requesting blockchain sync from peer {}", peer_id);
    
    // Send a custom sync request message
    // (This could be expanded in the protocol)
    
    Ok(())
}

// Connect to a peer using domain name or IP address
pub fn connect_to_peer_by_name(address: &str) -> Result<String, BlockchainError> {
    // First, try to resolve the domain name if needed
    let resolved_addr = match resolve_domain(address) {
        Ok(addr) => addr,
        Err(e) => {
            warn!("Failed to resolve address {}: {}", address, e);
            return Err(e);
        }
    };
    
    // Get node configuration for connection attempt
    let config = super::NODE_CONFIG.read().unwrap().clone();
    
    // Attempt connection with resolved address
    info!("Connecting to peer at {} (resolved from {})", resolved_addr, address);
    
    match super::connect_to_peer(&resolved_addr, &config) {
        Ok(()) => {
            info!("Successfully connected to peer at {} ({})", address, resolved_addr);
            Ok(resolved_addr)
        },
        Err(e) => {
            warn!("Failed to connect to peer at {} ({}): {}", address, resolved_addr, e);
            Err(e)
        }
    }
}
