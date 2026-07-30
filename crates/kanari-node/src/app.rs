// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use kanari_core::{
    BlockchainEngine, decode_hex_exact, read_json_file, write_file_atomically,
    write_json_pretty_atomically,
};
use kanari_rpc_api::{CanonicalStateSnapshotResponse, CompareCanonicalStateSnapshotRequest};
use kanari_rpc_server::start_server_with_transaction_broadcaster;
use kanari_types::address::Address as KanariAddress;
use kanari_types::gas_coin::GasModule;
use libp2p::identity::Keypair;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use zeroize::Zeroizing;

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

use crate::NetworkMode;
use crate::config::NodeRuntimeConfig;
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
    path.to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid data directory path: {}", path.display()))
}

fn short_value(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    if value.len() <= 48 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..24], &value[value.len() - 16..])
    }
}

fn divergence_snapshot_dump_dir() -> Option<PathBuf> {
    std::env::var("KANARI_DIVERGENCE_SNAPSHOT_DIR")
        .ok()
        .map(PathBuf::from)
}

fn divergence_reference_snapshot_path() -> Option<PathBuf> {
    std::env::var("KANARI_DIVERGENCE_REFERENCE_SNAPSHOT")
        .ok()
        .map(PathBuf::from)
}

fn sanitize_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}

fn write_snapshot_dump(path: &Path, snapshot: &CanonicalStateSnapshotResponse) {
    if let Err(error) = write_json_pretty_atomically(path, snapshot) {
        tracing::warn!(
            "Failed to write startup divergence snapshot {}: {}",
            path.display(),
            error
        );
    } else {
        tracing::info!("Wrote startup divergence snapshot to {}", path.display());
    }
}

fn emit_startup_divergence_diagnostics(engine: &Arc<BlockchainEngine>, local_peer_id: &str) {
    let snapshot = match engine.canonical_state_snapshot_response(None, None) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::error!(
                peer_id = local_peer_id,
                "Failed to read startup canonical state snapshot: {error:#}"
            );
            return;
        }
    };

    tracing::info!(
        height = snapshot.height,
        state_root = snapshot.state_root,
        entries = snapshot.entry_count,
        peer_id = local_peer_id,
        "Startup canonical state snapshot ready"
    );

    if let Some(reference_path) = divergence_reference_snapshot_path() {
        match read_json_file::<CanonicalStateSnapshotResponse>(&reference_path).ok() {
            Some(reference) => {
                let diff = match engine.compare_canonical_state_snapshot(
                    &CompareCanonicalStateSnapshotRequest {
                        entries: reference.entries.clone(),
                    },
                ) {
                    Ok(diff) => diff,
                    Err(error) => {
                        tracing::error!(reference_file = %reference_path.display(), "Failed to compare startup canonical state snapshot: {error:#}");
                        return;
                    }
                };
                if let Some(first_divergence) = diff.first_divergence {
                    tracing::warn!(
                        reference_file = %reference_path.display(),
                        reference_height = reference.height,
                        reference_state_root = reference.state_root,
                        first_divergence,
                        "Startup divergence detected against reference snapshot"
                    );
                } else {
                    tracing::info!(
                        reference_file = %reference_path.display(),
                        reference_height = reference.height,
                        reference_state_root = reference.state_root,
                        "Startup snapshot matches reference"
                    );
                }
            }
            None => tracing::warn!(
                reference_file = %reference_path.display(),
                "Failed to load startup divergence reference snapshot"
            ),
        }
    }

    if let Some(dir) = divergence_snapshot_dump_dir() {
        let filename = format!(
            "startup-h{}-{}.json",
            snapshot.height,
            sanitize_path_component(local_peer_id)
        );
        write_snapshot_dump(&dir.join(filename), &snapshot);
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
            std::env::set_var("KANARI_DAG_WAL_DIR", dir_str);
        }
    }

    Ok(())
}

pub fn create_engine(
    data_dir: &Option<std::path::PathBuf>,
    network: &NetworkMode,
) -> Result<BlockchainEngine> {
    create_engine_with_genesis(data_dir, network, None)
}

pub fn create_engine_required(
    data_dir: &std::path::Path,
    network: &NetworkMode,
) -> Result<BlockchainEngine> {
    configure_engine_environment(Some(data_dir), network)?;
    tracing::info!("Using required data directory: {}", data_dir.display());
    BlockchainEngine::new_dir_required(path_to_env_value(data_dir)?)
}

pub fn create_engine_with_genesis(
    data_dir: &Option<std::path::PathBuf>,
    network: &NetworkMode,
    genesis_path: Option<&std::path::Path>,
) -> Result<BlockchainEngine> {
    configure_engine_environment(data_dir.as_deref(), network)?;
    let engine = if let Some(dir) = data_dir {
        tracing::info!("Using data directory: {}", dir.display());
        let dir_str = path_to_env_value(dir)?;
        BlockchainEngine::new_dir(dir_str)?
    } else {
        BlockchainEngine::new()?
    };

    if let Some(path) = genesis_path {
        let manifest = BlockchainEngine::read_genesis_manifest(path)?;
        engine.validate_genesis_manifest(&manifest, network.as_str())?;
        tracing::info!(path = %path.display(), "Genesis manifest validated");
    }

    Ok(engine)
}

pub fn configure_consensus_signing_key(
    engine: &mut BlockchainEngine,
    private_key_path: &std::path::Path,
    public_keys_path: &std::path::Path,
) -> Result<()> {
    let key_file = Zeroizing::new(std::fs::read_to_string(private_key_path).map_err(|error| {
        anyhow::anyhow!(
            "Failed to read consensus private key file {}: {}",
            private_key_path.display(),
            error
        )
    })?);
    let trimmed = key_file.trim();
    let private_key_hex = if trimmed.starts_with('{') {
        let encrypted: kanari_crypto::EncryptedData =
            serde_json::from_str(trimmed).map_err(|error| {
                anyhow::anyhow!(
                    "Invalid encrypted consensus key file {}: {}",
                    private_key_path.display(),
                    error
                )
            })?;
        let password =
            Zeroizing::new(std::env::var("KANARI_CONSENSUS_KEY_PASSWORD").map_err(|_| {
                anyhow::anyhow!(
                    "KANARI_CONSENSUS_KEY_PASSWORD is required to decrypt {}",
                    private_key_path.display()
                )
            })?);
        Zeroizing::new(
            kanari_crypto::decrypt_string(&encrypted, &password)
                .map_err(|error| anyhow::anyhow!("Failed to decrypt consensus key: {error}"))?,
        )
    } else {
        if BlockchainEngine::network_name().eq_ignore_ascii_case("mainnet") {
            anyhow::bail!(
                "Mainnet refuses plaintext consensus key file {}; encrypt it by setting KANARI_CONSENSUS_KEY_PASSWORD when running consensus-keygen",
                private_key_path.display()
            );
        }
        tracing::warn!(
            path = %private_key_path.display(),
            "Using a plaintext consensus key for a non-mainnet network"
        );
        Zeroizing::new(trimmed.to_string())
    };
    let private_key = Zeroizing::new(decode_hex_exact(
        "consensus private key seed",
        &private_key_hex,
        32,
    )?);
    let private_key: [u8; 32] = private_key
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid consensus private key seed length"))?;
    let private_key = Zeroizing::new(private_key);
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&private_key);

    let public_key_hex_by_authority: BTreeMap<String, String> = read_json_file(public_keys_path)
        .with_context(|| {
            format!(
                "Invalid consensus public keys JSON {}",
                public_keys_path.display()
            )
        })?;

    let mut public_keys = BTreeMap::new();
    for (authority, key_hex) in public_key_hex_by_authority {
        public_keys.insert(
            authority,
            decode_hex_exact("consensus public key", &key_hex, 32)?,
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

fn node_secret_password() -> Option<Zeroizing<String>> {
    std::env::var("KANARI_NODE_IDENTITY_PASSWORD")
        .or_else(|_| std::env::var("KANARI_CONSENSUS_KEY_PASSWORD"))
        .ok()
        .filter(|password| !password.is_empty())
        .map(Zeroizing::new)
}

pub fn load_or_create_p2p_identity(data_dir: &std::path::Path, network: &str) -> Result<Keypair> {
    let path = data_dir.join("p2p-identity.key");
    let mainnet = network.eq_ignore_ascii_case("mainnet");

    if path.exists() {
        let stored = Zeroizing::new(std::fs::read(&path)?);
        let encoded = if stored.first() == Some(&b'{') {
            let encrypted: kanari_crypto::EncryptedData = serde_json::from_slice(&stored)
                .map_err(|error| anyhow::anyhow!("Invalid encrypted P2P identity: {error}"))?;
            let password = node_secret_password().ok_or_else(|| {
                anyhow::anyhow!(
                    "KANARI_NODE_IDENTITY_PASSWORD (or KANARI_CONSENSUS_KEY_PASSWORD) is required to decrypt {}",
                    path.display()
                )
            })?;
            Zeroizing::new(
                kanari_crypto::decrypt_data(&encrypted, password.as_str())
                    .map_err(|error| anyhow::anyhow!("Failed to decrypt P2P identity: {error}"))?,
            )
        } else {
            if mainnet {
                anyhow::bail!(
                    "Mainnet refuses plaintext P2P identity {}; remove it and set KANARI_NODE_IDENTITY_PASSWORD to generate an encrypted identity",
                    path.display()
                );
            }
            tracing::warn!(path = %path.display(), "Using plaintext P2P identity on a non-mainnet network");
            Zeroizing::new(stored.to_vec())
        };
        return Keypair::from_protobuf_encoding(&encoded)
            .map_err(|error| anyhow::anyhow!("Invalid P2P identity {}: {error}", path.display()));
    }

    let keypair = Keypair::generate_ed25519();
    let encoded = Zeroizing::new(
        keypair
            .to_protobuf_encoding()
            .map_err(|error| anyhow::anyhow!("Failed to encode P2P identity: {error}"))?,
    );
    let stored = match node_secret_password() {
        Some(password) => serde_json::to_vec_pretty(
            &kanari_crypto::encrypt_data(&encoded, password.as_str())
                .map_err(|error| anyhow::anyhow!("Failed to encrypt P2P identity: {error}"))?,
        )?,
        None if mainnet => anyhow::bail!(
            "Mainnet requires KANARI_NODE_IDENTITY_PASSWORD (or KANARI_CONSENSUS_KEY_PASSWORD) before generating its P2P identity"
        ),
        None => {
            tracing::warn!(path = %path.display(), "Writing plaintext P2P identity for a non-mainnet network");
            encoded.to_vec()
        }
    };
    write_file_atomically(&path, &stored)?;
    Ok(keypair)
}

fn queue_network_message(
    network_tx: &tokio::sync::mpsc::Sender<crate::p2p::QueuedP2PMessage>,
    msg: P2PMessage,
    failure_context: &str,
) -> bool {
    match network_tx.try_send(crate::p2p::QueuedP2PMessage::new(msg)) {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!("{}: {}", failure_context, e);
            false
        }
    }
}

fn serialize_and_queue_message<T: Serialize>(
    network_tx: &tokio::sync::mpsc::Sender<crate::p2p::QueuedP2PMessage>,
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

fn extract_failed_tx_hash(error_text: &str) -> Option<Vec<u8>> {
    let marker = "Execution failed for tx ";
    let start = error_text.find(marker)? + marker.len();
    let rest = &error_text[start..];
    let end = rest.find(' ')?;
    hex::decode(&rest[..end]).ok()
}

fn should_drop_invalid_pending_transaction(error_text: &str) -> bool {
    error_text.contains("cannot overlap with a mutable object input")
        || error_text.contains("Gas payment object")
        || error_text.contains("Insufficient native coin object balance for gas")
        || error_text.contains("does not exist")
        || error_text.contains("version mismatch")
        || error_text.contains("digest mismatch")
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

    let total_supply_str = GasModule::format_gas(stats.total_supply);

    let (genesis_root, size_bytes) = genesis_root_info(&engine);
    let dao_addr = KanariAddress::DAO_ADDRESS;
    let dev_addr = KanariAddress::DEV_ADDRESS;
    tracing::info!(
        network = %network,
        checkpoint = stats.height,
        txs = stats.total_transactions,
        owners = stats.total_owners,
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

    // Backpressure prevents compressed-message queues from growing without bound.
    let p2p_channel_capacity = NodeRuntimeConfig::p2p_channel_capacity();
    let (p2p_msg_tx, mut p2p_msg_rx) =
        tokio::sync::mpsc::channel::<crate::p2p::AuthenticatedP2PMessage>(p2p_channel_capacity);
    let (network_tx, network_rx) =
        tokio::sync::mpsc::channel::<crate::p2p::QueuedP2PMessage>(p2p_channel_capacity);

    let keypair = load_or_create_p2p_identity(&data_dir, &network)?;
    let peer_id = keypair.public().to_peer_id().to_string();
    tracing::info!(peer_id = %short_value(&peer_id), "Node peer identity ready");
    emit_startup_divergence_diagnostics(&engine, &peer_id);

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
            let max_concurrent_sync_messages = NodeRuntimeConfig::max_concurrent_sync_messages();
            let permits = Arc::new(tokio::sync::Semaphore::new(max_concurrent_sync_messages));
            while let Some(msg) = p2p_msg_rx.recv().await {
                let permit = match permits.clone().acquire_owned().await {
                    Ok(permit) => permit,
                    Err(_) => break,
                };
                let sync = sync_for_messages.clone();
                let task = tokio::spawn(async move {
                    let _permit = permit;
                    sync.handle_message(msg).await;
                });
                tokio::spawn(async move {
                    match task.await {
                        Ok(()) => {}
                        Err(e) if e.is_panic() => tracing::error!(
                            "[P2P] Sync message handler panicked; continuing: {}",
                            e
                        ),
                        Err(e) => tracing::error!(
                            "[P2P] Sync message handler task failed; continuing: {}",
                            e
                        ),
                    }
                });
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
                if let Err(error) = network_tx_for_rpc
                    .try_send(crate::p2p::QueuedP2PMessage::new(P2PMessage::NewTransaction(payload)))
                {
                    tracing::warn!(
                        error = %error,
                        "P2P transaction broadcast queue is full; transaction remains accepted locally"
                    );
                }
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
    let mut last_dag_attempt = Instant::now() - Duration::from_secs(1);
    let mut last_dag_rebroadcast = Instant::now() - Duration::from_secs(1);

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

        let dag_attempt_interval = if stats.pending_transactions > 0 && pending_gossip_ready {
            Duration::from_millis(100)
        } else {
            // Keep Mysticeti warm while idle without continuously signing,
            // serializing, and gossiping empty blocks. Pending work still
            // switches back to the low-latency cadence immediately.
            Duration::from_secs(2)
        };
        let should_produce_pending =
            pending_gossip_ready && last_dag_attempt.elapsed() >= dag_attempt_interval;

        if should_produce_pending {
            last_dag_attempt = Instant::now();
            match engine.produce_checkpoint() {
                Ok(block_info) => {
                    did_work = true;

                    if block_info.tx_count == 0 {
                        tracing::debug!("DAG idle vertex (round #{}) produced", block_info.round);
                    } else {
                        tracing::info!(
                            "DAG Vertex (Round #{}) produced: {} txs ({} executed, {} failed)",
                            block_info.round,
                            block_info.tx_count,
                            block_info.executed,
                            block_info.failed
                        );
                    }

                    if let Some(vertex) = block_info.vertex {
                        if let Some(vertex_len) = serialize_and_queue_message(
                            &network_tx,
                            &vertex,
                            P2PMessage::NewDagVertex,
                            "Failed to serialize DAG vertex for broadcast",
                            "Failed to queue DAG vertex broadcast",
                        ) {
                            if block_info.tx_count == 0 {
                                tracing::debug!(
                                    "Broadcasting idle DAG vertex {} (round {}, {} bytes)",
                                    block_info.vertex_id,
                                    block_info.round,
                                    vertex_len
                                );
                            } else {
                                tracing::info!(
                                    "Broadcasting DAG vertex {} (round {}) to network ({} bytes)",
                                    block_info.vertex_id,
                                    block_info.round,
                                    vertex_len
                                );
                            }
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
                        match engine.get_checkpoint_sync(current_height) {
                            Ok(Some(checkpoint_sync)) => {
                                serialize_and_queue_message(
                                    &network_tx,
                                    &checkpoint_sync,
                                    P2PMessage::NewCheckpoint,
                                    "Failed to serialize checkpoint for broadcast",
                                    "Failed to queue checkpoint broadcast",
                                );
                            }
                            Ok(None) => tracing::warn!(
                                checkpoint = current_height,
                                "Produced checkpoint is missing from the local checkpoint index; not broadcasting"
                            ),
                            Err(error) => tracing::error!(
                                checkpoint = current_height,
                                error = %error,
                                "Failed to prepare locally produced checkpoint for broadcast"
                            ),
                        }
                    }
                }
                Err(e) => {
                    let error_text = format!("{:#}", e);
                    if error_text.contains("DAG_WAITING")
                        || error_text.contains("SYNC_WAITING")
                        || error_text.contains("Not enough parents for quorum")
                    {
                        tracing::debug!("Checkpoint production waiting: {}", error_text);
                        if stats.pending_transactions > 0 {
                            idle_delay = Duration::from_millis(50);
                        }
                        if last_dag_rebroadcast.elapsed() >= Duration::from_secs(1) {
                            last_dag_rebroadcast = Instant::now();
                            sync_manager
                                .broadcast_latest_dag_vertices(4, "while waiting for quorum");
                            sync_manager.request_dag_vertices_for_quorum();
                        }
                    } else if should_drop_invalid_pending_transaction(&error_text) {
                        let failed_hash = extract_failed_tx_hash(&error_text).or_else(|| {
                            // Older/nested error wrappers may omit the tx hash. The
                            // producer executes the conflict-free snapshot in canonical
                            // order, so its first transaction is the one that failed.
                            engine
                                .pending_conflict_free_transactions_snapshot()
                                .first()
                                .map(|tx| tx.transaction_hash().to_vec())
                        });
                        if let Some(tx_hash) = failed_hash {
                            let removed = engine.remove_pending_transactions_by_hashes(
                                std::slice::from_ref(&tx_hash),
                            );
                            if !removed.is_empty() {
                                for tx in &removed {
                                    tracing::warn!(
                                        tx_hash = %hex::encode(tx.signed_tx.transaction_hash()),
                                        sender = %tx.signed_tx.transaction.sender_address(),
                                        nonce = tx.signed_tx.transaction.nonce(),
                                        "Dropped invalid pending transaction after deterministic execution failure"
                                    );
                                }
                                tracing::info!(
                                    "Released object/gas reservations for invalid pending transaction"
                                );
                                idle_delay = Duration::from_millis(10);
                            } else {
                                tracing::error!("Checkpoint production failed: {:#}", e);
                            }
                        } else {
                            tracing::error!("Checkpoint production failed: {:#}", e);
                        }
                    } else if !error_text.contains("DAG not ready") {
                        tracing::error!("Checkpoint production failed: {:#}", e);
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

#[cfg(test)]
#[path = "../tests/unit/app_tests.rs"]
mod security_tests;
