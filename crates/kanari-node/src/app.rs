// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use kanari_core::BlockchainEngine;
use kanari_rpc_server::start_server_with_transaction_broadcaster;
use kanari_types::address::Address as KanariAddress;
use kanari_types::error::{KanariError, KanariUnwrapExt};
use kanari_types::kanari::KanariModule;
use libp2p::identity::Keypair;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::time::sleep;

use crate::NetworkMode;
use crate::indexer::NodeIndexer;
use crate::p2p::{P2PEventHandler, P2PMessage, P2PNetwork};
use crate::peer_store::PeerStore;
use crate::sync::SyncManager;

pub fn default_data_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".kanari")
        .join("kanari-db")
}

fn env_write_guard() -> &'static Mutex<()> {
    static ENV_WRITE_GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    ENV_WRITE_GUARD.get_or_init(|| Mutex::new(()))
}

fn path_to_env_value(path: &std::path::Path) -> Result<&str> {
    path.to_str().require("Invalid data directory path")
}

fn short_value(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    if value.len() <= 48 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..24], &value[value.len() - 16..])
    }
}

fn configure_engine_environment(
    data_dir: Option<&std::path::Path>,
    network: &NetworkMode,
) -> Result<()> {
    let _env_guard = env_write_guard().lock().unwrap_or_else(|e| e.into_inner());

    // Safety: this runs during CLI startup before the Tokio runtime is created
    // and before any background worker threads are spawned.
    unsafe {
        std::env::set_var("KANARI_NETWORK", network.as_str());
    }

    if let Some(dir) = data_dir {
        let dir_str = path_to_env_value(dir)?;
        // Safety: guarded by the same startup-only contract described above.
        unsafe {
            std::env::set_var("KANARI_STATE_DB", dir_str);
            std::env::set_var("KANARI_MOVE_VM_DB", dir_str);
        }
    }

    Ok(())
}

pub fn create_engine(
    data_dir: &Option<std::path::PathBuf>,
    network: &NetworkMode,
) -> Result<BlockchainEngine> {
    configure_engine_environment(data_dir.as_deref(), network)?;
    let engine = if let Some(dir) = data_dir {
        tracing::info!("Using data directory: {}", dir.display());
        let dir_str = path_to_env_value(dir)?;
        BlockchainEngine::new_dir(dir_str)?
    } else {
        BlockchainEngine::new()?
    };
    let (executed, removed) = engine.recover_pending_object_transactions()?;
    if executed > 0 || removed > 0 {
        tracing::info!(
            executed,
            removed,
            "Recovered durable object transaction pool"
        );
    }
    Ok(engine)
}

fn normalize_authority_id(authority_id: String) -> String {
    if authority_id.starts_with("0x") {
        authority_id
    } else {
        format!("0x{}", authority_id)
    }
}

fn decode_hex_bytes(label: &str, value: &str, expected_len: usize) -> Result<Vec<u8>> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(trimmed).map_err(|e| KanariError::OperationFailed {
        context: "invalid hex input",
        details: format!("{}: {}", label, e),
    })?;
    if bytes.len() != expected_len {
        anyhow::bail!(
            "Invalid {} length: expected {} bytes, got {}",
            label,
            expected_len,
            bytes.len()
        );
    }
    Ok(bytes)
}

pub fn configure_consensus_signing_key(
    engine: &mut BlockchainEngine,
    private_key_hex: &str,
    public_keys_path: &std::path::Path,
) -> Result<()> {
    let private_key = decode_hex_bytes("consensus private key seed", private_key_hex, 32)?;
    let private_key: [u8; 32] = private_key.try_into().map_err(|_| {
        KanariError::InvariantViolation("Invalid consensus private key seed length")
    })?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&private_key);

    let public_keys_json =
        std::fs::read_to_string(public_keys_path).map_err(|e| KanariError::OperationFailed {
            context: "failed to read consensus public keys file",
            details: format!("{}: {}", public_keys_path.display(), e),
        })?;
    let public_key_hex_by_authority: BTreeMap<String, String> =
        serde_json::from_str(&public_keys_json).map_err(|e| KanariError::OperationFailed {
            context: "invalid consensus public keys JSON",
            details: format!("{}: {}", public_keys_path.display(), e),
        })?;
    let mut public_keys = BTreeMap::new();
    for (authority, key_hex) in public_key_hex_by_authority {
        public_keys.insert(
            normalize_authority_id(authority),
            decode_hex_bytes("consensus public key", &key_hex, 32)?,
        );
    }

    engine.set_consensus_signing_key(signing_key, public_keys)
}

fn log_shutdown() {
    tracing::info!("Shutdown signal received. Cleaning up and exiting...");
}

fn genesis_root_info(engine: &BlockchainEngine) -> (String, usize) {
    match engine.get_block(0) {
        Some(block) => {
            let size = block.hash.len() / 2;
            (block.hash, size)
        }
        None => ("unknown".to_string(), 0),
    }
}

fn queue_network_message(
    network_tx: &tokio::sync::mpsc::UnboundedSender<P2PMessage>,
    msg: P2PMessage,
    failure_context: &str,
) -> bool {
    match network_tx.send(msg) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("{}: {}", failure_context, e);
            false
        }
    }
}

fn serialize_and_queue_message<T: Serialize>(
    network_tx: &tokio::sync::mpsc::UnboundedSender<P2PMessage>,
    value: &T,
    wrap: impl FnOnce(String) -> P2PMessage,
    serialize_context: &str,
    send_context: &str,
) -> Option<usize> {
    match serde_json::to_string(value) {
        Ok(payload) => {
            let payload_len = payload.len();
            queue_network_message(network_tx, wrap(payload), send_context);
            Some(payload_len)
        }
        Err(e) => {
            tracing::error!("{}: {}", serialize_context, e);
            None
        }
    }
}

fn detect_local_ip() -> Option<String> {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|socket| {
            if socket.connect("8.8.8.8:80").is_ok() {
                socket.local_addr().ok().map(|a| a.ip().to_string())
            } else {
                None
            }
        })
}

fn current_chain_height(engine: &Arc<BlockchainEngine>) -> u64 {
    engine
        .blockchain
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .height()
}

pub async fn run_node(
    engine: Arc<BlockchainEngine>,
    network: String,
    p2p_port: u16,
    rpc_port: u16,
    rpc_host: String,
    data_dir: std::path::PathBuf,
    relay_server: bool,
    bootstrap_peers: Option<Vec<String>>,
) -> Result<()> {
    engine.validate_runtime_health()?;
    let runtime_guards = engine.runtime_guard_config();
    let stats = engine.get_stats();

    let total_supply_str = KanariModule::format_kanari(stats.total_supply);

    let (genesis_root, size_bytes) = genesis_root_info(&engine);
    let dao_addr = KanariAddress::DAO_ADDRESS;
    let dev_addr = KanariAddress::DEV_ADDRESS;
    tracing::info!(
        network = %network,
        checkpoint = stats.height,
        txs = stats.total_transactions,
        accounts = stats.total_accounts,
        supply = %total_supply_str,
        "Kanari blockchain node starting"
    );
    tracing::info!(
        strict_persistence = runtime_guards.strict_persistence_required,
        strict_checkpoint_roots = runtime_guards.strict_checkpoint_roots,
        fail_fast_supply = runtime_guards.fail_fast_supply_enabled,
        "Runtime guards"
    );
    tracing::info!(
        root = %format!("0x{}", short_value(&genesis_root)),
        size_bytes,
        dao = %short_value(dao_addr),
        sequencer = %short_value(dev_addr),
        "System addresses"
    );

    let (p2p_msg_tx, mut p2p_msg_rx) = tokio::sync::mpsc::unbounded_channel::<P2PMessage>();
    let (network_tx, network_rx) = tokio::sync::mpsc::unbounded_channel::<P2PMessage>();

    let keypair = Keypair::generate_ed25519();
    let peer_id = keypair.public().to_peer_id().to_string();
    tracing::info!(peer_id = %short_value(&peer_id), "Node peer identity ready");

    let node_indexer = match NodeIndexer::new(data_dir.clone()) {
        Ok(idx) => {
            tracing::info!("Blockchain indexer initialized successfully");
            Some(idx)
        }
        Err(e) => {
            tracing::warn!(
                "Failed to initialize indexer: {}. Indexing will be disabled.",
                e
            );
            None
        }
    };

    let sync_manager = Arc::new(SyncManager::new(
        engine.clone(),
        network_tx.clone(),
        peer_id.clone(),
        node_indexer.as_ref().map(|idx| idx.indexer().clone()),
    ));
    sync_manager.clone().start().await;

    if p2p_port > 0 {
        let peer_store_path = PeerStore::default_path(&data_dir.display().to_string());
        let mut peer_store = PeerStore::load(peer_store_path.clone()).unwrap_or_else(|e| {
            tracing::warn!("Failed to load peer store: {}, creating new one", e);
            PeerStore::new(peer_store_path)
        });
        peer_store.cleanup_old_peers(7 * 24 * 60 * 60);

        let mut p2p_network = P2PNetwork::new(keypair, p2p_port, relay_server)?;
        tracing::info!(p2p_port, "P2P network initialized");
        if relay_server {
            tracing::info!(
                "Relay server mode: ENABLED - This node will help relay traffic for NAT'd peers"
            );
        }

        if let Some(bootstrap_list) = bootstrap_peers {
            for bootstrap_addr in bootstrap_list {
                match bootstrap_addr.parse::<libp2p::Multiaddr>() {
                    Ok(addr) => {
                        tracing::info!("Connecting to bootstrap peer: {}", addr);
                        if let Err(e) = p2p_network.swarm.dial(addr.clone()) {
                            tracing::warn!("Failed to dial bootstrap peer {}: {}", addr, e);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Invalid bootstrap multiaddr {}: {}", bootstrap_addr, e);
                    }
                }
            }
        }

        let peer_store_arc = Arc::new(tokio::sync::Mutex::new(peer_store));
        let mut event_handler = P2PEventHandler::new(p2p_network, p2p_msg_tx)
            .with_outgoing(network_rx)
            .with_peer_store(peer_store_arc.clone());
        tokio::spawn(async move {
            event_handler.run().await;
        });

        let sync_for_messages = sync_manager.clone();
        tokio::spawn(async move {
            while let Some(msg) = p2p_msg_rx.recv().await {
                let sync = sync_for_messages.clone();
                match tokio::spawn(async move {
                    sync.handle_message(msg).await;
                })
                .await
                {
                    Ok(()) => {}
                    Err(e) if e.is_panic() => {
                        tracing::error!(
                            "[P2P] Sync message handler panicked; continuing to process incoming messages: {}",
                            e
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "[P2P] Sync message handler task failed; continuing to process incoming messages: {}",
                            e
                        );
                    }
                }
            }
            tracing::warn!(
                "[P2P] Incoming P2P message receiver closed; network gossip will no longer reach sync manager"
            );
        });

        let sync_for_broadcast = sync_manager.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(500)).await;
            loop {
                sync_for_broadcast.broadcast_peer_info().await;
                sleep(Duration::from_secs(1)).await;
            }
        });
    } else {
        tracing::info!("P2P disabled; running in local-only mode");
    }

    let bind_addr = format!("{}:{}", rpc_host, rpc_port);

    let display_ip = if rpc_host == "0.0.0.0" {
        detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        rpc_host.clone()
    };
    tracing::info!(listen = %bind_addr, public_url = %format!("http://{}:{}", display_ip, rpc_port), "Starting JSON-RPC HTTP server");

    let engine_for_rpc = engine.clone();
    let bind_addr_clone = bind_addr.clone();
    let network_tx_for_rpc = network_tx.clone();
    tokio::spawn(async move {
        if let Err(e) = start_server_with_transaction_broadcaster(
            engine_for_rpc,
            &bind_addr_clone,
            move |signed_tx| {
                let payload = serde_json::to_string(&signed_tx)?;
                network_tx_for_rpc
                    .send(P2PMessage::NewTransaction(payload))
                    .map_err(|e| KanariError::OperationFailed {
                        context: "failed to queue transaction broadcast",
                        details: e.to_string(),
                    })?;
                Ok(())
            },
        )
        .await
        {
            tracing::error!("RPC server error: {}", e);
        }
    });

    sleep(Duration::from_millis(500)).await;
    let ready_stats = engine.get_stats();
    tracing::info!(
        listen = %bind_addr,
        public_url = %format!("http://{}:{}", display_ip, rpc_port),
        env = %network,
        checkpoint = ready_stats.height,
        pending = ready_stats.pending_transactions,
        "Kanari RPC service ready"
    );

    let mut last_stats_log = Instant::now() - Duration::from_secs(2);
    let mut last_pending_count = ready_stats.pending_transactions;
    let mut pending_gossip_ready_at: Option<Instant> = None;

    loop {
        let stats = engine.get_stats();
        if last_stats_log.elapsed() >= Duration::from_secs(2) {
            last_stats_log = Instant::now();
            tracing::info!(
                height = stats.height,
                txs = stats.total_transactions,
                pending = stats.pending_transactions,
                "Node status"
            );

            if let Some(ref node_idx) = node_indexer
                && stats.height > 0
                && stats.height.is_multiple_of(10)
            {
                match node_idx.get_stats() {
                    Ok(idx_stats) => tracing::info!("[INDEXER] {}", idx_stats),
                    Err(e) => tracing::warn!("[INDEXER] Failed to get stats: {}", e),
                }
            }
        }

        let mut did_work = false;
        let mut idle_delay = Duration::from_millis(50);

        if stats.pending_transactions > last_pending_count {
            pending_gossip_ready_at = Some(Instant::now() + Duration::from_millis(150));
        } else if stats.pending_transactions == 0 {
            pending_gossip_ready_at = None;
        }
        last_pending_count = stats.pending_transactions;

        let pending_gossip_ready = match pending_gossip_ready_at {
            Some(ready_at) => Instant::now() >= ready_at,
            None => true,
        };

        if stats.pending_transactions > 0 && !pending_gossip_ready {
            idle_delay = Duration::from_millis(10);
        }

        let object_pending = engine.pending_object_transaction_len().unwrap_or_else(|error| {
            tracing::warn!("Failed to read object pending count: {}", error);
            0
        });
        if object_pending > 0 {
            match engine.recover_pending_object_transactions() {
                Ok((executed, removed)) => {
                    if executed > 0 || removed > 0 {
                        did_work = true;
                        tracing::info!(executed, removed, "Processed pending object transactions");
                    }
                }
                Err(error) => tracing::error!("Object transaction recovery failed: {}", error),
            }
        }
        let legacy_pending = engine.pending_transaction_len();
        let should_produce_pending = legacy_pending > 0 && pending_gossip_ready;

        if should_produce_pending {
            match engine.produce_checkpoint() {
                Ok(block_info) => {
                    did_work = true;

                    tracing::info!(
                        "DAG Vertex (Round #{}) produced: {} txs ({} executed, {} failed)",
                        block_info.round,
                        block_info.tx_count,
                        block_info.executed,
                        block_info.failed
                    );

                    if let Some(vertex) = block_info.vertex {
                        if let Some(vertex_len) = serialize_and_queue_message(
                            &network_tx,
                            &vertex,
                            P2PMessage::NewDagVertex,
                            "Failed to serialize DAG vertex for broadcast",
                            "Failed to queue DAG vertex broadcast",
                        ) {
                            tracing::info!(
                                "Broadcasting DAG vertex {} (round {}) to network ({} bytes)",
                                block_info.vertex_id,
                                block_info.round,
                                vertex_len
                            );
                            tracing::info!("DAG vertex queued for broadcast successfully");
                        }
                    } else {
                        tracing::warn!("No vertex in block_info to broadcast");
                    }

                    if let Some(ref node_idx) = node_indexer {
                        let current_height = current_chain_height(&engine);
                        if let Some(full_block_data) = engine.get_full_block(current_height) {
                            let block = BlockchainEngine::block_from_full_data(&full_block_data);

                            match node_idx.index_block(&block) {
                                Ok(_) => {
                                    if current_height.is_multiple_of(100) {
                                        tracing::info!(
                                            "[INDEXER] Indexed locally produced block #{}",
                                            current_height
                                        );
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "[INDEXER] Failed to index locally produced block #{}: {}",
                                        current_height,
                                        e
                                    );
                                }
                            }
                        }
                    }

                    if block_info.checkpoint.is_some() {
                        let current_height = current_chain_height(&engine);
                        if let Some(checkpoint_sync) = engine.get_checkpoint_sync(current_height) {
                            serialize_and_queue_message(
                                &network_tx,
                                &checkpoint_sync,
                                P2PMessage::NewCheckpoint,
                                "Failed to serialize checkpoint for broadcast",
                                "Failed to queue checkpoint broadcast",
                            );
                        }
                    }
                }
                Err(e) => {
                    let error_text = e.to_string();
                    if error_text.contains("DAG_WAITING")
                        || error_text.contains("SYNC_WAITING")
                        || error_text.contains("Not enough parents for quorum")
                    {
                        tracing::info!("Checkpoint production waiting: {}", error_text);
                        if stats.pending_transactions > 0 {
                            idle_delay = Duration::from_millis(50);
                        }
                        sync_manager.broadcast_latest_dag_vertices(16, "while waiting for quorum");
                        sync_manager.request_dag_vertices_for_quorum().await;
                    } else if !error_text.contains("DAG not ready") {
                        tracing::error!("Checkpoint production failed: {}", e);
                    }
                }
            }
        }

        if !did_work {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    log_shutdown();
                    break;
                }
                _ = sleep(idle_delay) => {}
            }
        } else if tokio::time::timeout(Duration::from_millis(1), tokio::signal::ctrl_c())
            .await
            .is_ok()
        {
            log_shutdown();
            break;
        }
    }

    tracing::info!("Node shutdown complete.");
    Ok(())
}
