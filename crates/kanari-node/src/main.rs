// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Main entry point for Kanari blockchain node
use anyhow::Result;
use clap::{Parser, Subcommand};
use kanari_core::BlockchainEngine;
use kanari_crypto::wallet::list_wallet_files;
use kanari_indexer::KanariIndexer;
use kanari_rpc_server::start_server;
use kanari_types::address::Address as KanariAddress;
use kanari_types::kanari::KanariModule;
use libp2p::identity::Keypair;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

mod p2p;
mod peer_store;
mod sync;

use p2p::{P2PEventHandler, P2PMessage, P2PNetwork};
use peer_store::PeerStore;
use sync::SyncManager;

/// Kanari node command-line interface
#[derive(Parser)]
#[command(name = "kanari-node")]
#[command(about = "Kanari blockchain node", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// start the node
    Start {
        /// P2P listen port
        #[arg(long, default_value = "19000")]
        p2p_port: u16,

        /// RPC listen port
        #[arg(long, default_value = "19001")]
        rpc_port: u16,

        /// RPC listen host/IP (use 0.0.0.0 to bind all interfaces)
        #[arg(long, default_value = "0.0.0.0")]
        rpc_host: String,

        /// Data directory for blockchain and state storage
        #[arg(long)]
        data_dir: Option<std::path::PathBuf>,

        /// Run as relay server to help other nodes behind NAT
        #[arg(long, default_value = "false")]
        relay_server: bool,

        /// Authority ID for DAG consensus (e.g. 0x1)
        #[arg(long)]
        authority_id: Option<String>,

        /// List of authority IDs for DAG consensus (comma-separated)
        #[arg(long, value_delimiter = ',')]
        authorities: Option<Vec<String>>,

        /// Bootstrap peer multiaddr to connect to (can be specified multiple times)
        #[arg(long, value_name = "MULTIADDR")]
        bootstrap: Option<Vec<String>>,
    },
    /// Run a local-only node
    Local {},
    /// List wallet files
    ListWallets,
    /// Show blockchain statistics
    Stats,
    /// Get account info
    Account {
        /// Account address
        address: String,
    },
    /// Get block information by height
    Block {
        /// Block height
        height: u64,
    },
}

// Main entry point
// Initializes and runs the Kanari blockchain node
// Sets up P2P networking, RPC server, and blockchain engine
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::ListWallets => {
            let wallets = list_wallet_files()?;
            for (addr, selected) in wallets {
                tracing::info!("{}{}", addr, if selected { " (selected)" } else { "" });
            }
            return Ok(());
        }

        Commands::Stats => {
            let engine = BlockchainEngine::new()?;
            let stats = engine.get_stats();
            tracing::info!("Blockchain Statistics:");
            tracing::info!("  Height: {}", stats.height);
            tracing::info!("  Total Blocks: {}", stats.total_blocks);
            tracing::info!("  Total Transactions: {}", stats.total_transactions);
            tracing::info!("  Pending Transactions: {}", stats.pending_transactions);
            tracing::info!("  Total Accounts: {}", stats.total_accounts);
            tracing::info!("  Total Supply: {} Kanari", stats.total_supply);
            return Ok(());
        }

        Commands::Account { address } => {
            let engine = BlockchainEngine::new()?;
            match engine.get_account_info(&address) {
                Some(info) => {
                    tracing::info!("  Account: {}", info.address);
                    tracing::info!("  Balance: {}", info.balance);
                    tracing::info!("  Sequence: {}", info.sequence_number);
                    tracing::info!("  Modules: {}", info.modules.len());
                    for module in &info.modules {
                        tracing::info!("    - {}", module);
                    }
                }
                None => tracing::info!("Account not found: {}", address),
            }
            return Ok(());
        }

        Commands::Block { height } => {
            let engine = BlockchainEngine::new()?;
            match engine.get_block(height) {
                Some(block) => {
                    tracing::info!("  Block #{}", block.height);
                    tracing::info!("  Timestamp: {}", block.timestamp);
                    tracing::info!("  Hash: {}", block.hash);
                    tracing::info!("  Prev Hash: {}", block.prev_hash);
                    tracing::info!("  Transactions: {}", block.tx_count);
                }
                None => tracing::info!("Block not found: {}", height),
            }
            return Ok(());
        }

        Commands::Start {
            p2p_port,
            rpc_port,
            rpc_host,
            data_dir,
            relay_server,
            authority_id,
            authorities,
            bootstrap,
        } => {
            let data_dir_path = data_dir.clone().unwrap_or_else(|| {
                let home = std::env::var("HOME")
                    .or_else(|_| std::env::var("USERPROFILE"))
                    .unwrap_or_else(|_| ".".to_string());
                std::path::PathBuf::from(home)
                    .join(".kanari")
                    .join("kanari-db")
            });

            // If data_dir is provided, use it. Otherwise, use the default internal logic
            // which handles path resolution and directory creation correctly.
            let mut engine = if let Some(ref d) = data_dir {
                unsafe {
                    std::env::set_var("KANARI_STATE_DB", d);
                    // Also set MOVE_VM_DB to ensure consistency if fallback is triggered
                    std::env::set_var("KANARI_MOVE_VM_DB", d);
                }
                tracing::info!("Using data directory: {}", d.display());
                BlockchainEngine::new_dir(d.to_str().expect("Invalid data directory path"))?
            } else {
                // Use default persistent store (internally resolves to ~/.kanari/kanari-db/kanari_db)
                BlockchainEngine::new()?
            };

            // Configure authorities if provided
            if let (Some(id), Some(auths)) = (authority_id, authorities) {
                tracing::info!(
                    "Configuring node with Authority ID: {} and {} authorities",
                    id,
                    auths.len()
                );
                engine.set_authorities(id, auths);
            }

            let engine_arc = Arc::new(engine);

            run_node(
                engine_arc,
                p2p_port,
                rpc_port,
                rpc_host,
                data_dir_path,
                relay_server,
                bootstrap,
            )
            .await?;
            return Ok(());
        }
        Commands::Local {} => {
            // Run a local-only node: RPC bound to localhost, P2P disabled
            let p2p_port = 0;
            let rpc_port = 6767;
            let rpc_host = "127.0.0.1".to_string();

            let data_dir_path = std::path::PathBuf::from("./.kanari-local");
            tracing::info!(
                "Starting local node: RPC on {}:{} (P2P disabled)",
                rpc_host,
                rpc_port
            );

            let engine = BlockchainEngine::new()?;
            let engine_arc = Arc::new(engine);

            run_node(
                engine_arc,
                p2p_port,
                rpc_port,
                rpc_host,
                data_dir_path,
                false,
                None,
            )
            .await?;
            return Ok(());
        }
    }
}

fn detect_local_ip() -> Option<String> {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .ok()
        .and_then(|socket| {
            // connect to a public IP to determine the outbound interface
            if socket.connect("8.8.8.8:80").is_ok() {
                socket.local_addr().ok().map(|a| a.ip().to_string())
            } else {
                None
            }
        })
}

async fn run_node(
    engine: Arc<BlockchainEngine>,
    p2p_port: u16,
    rpc_port: u16,
    rpc_host: String,
    data_dir: std::path::PathBuf,
    relay_server: bool,
    bootstrap_peers: Option<Vec<String>>,
) -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let stats = engine.get_stats();

    tracing::info!("Kanari blockchain node starting");
    tracing::info!("Network: Testnet, Move VM: Enabled");
    tracing::info!("Initial blockchain height: {}", stats.height);
    let total_supply_str = KanariModule::format_kanari(stats.total_supply);
    tracing::info!(
        "Total accounts: {}, Total supply: {}",
        stats.total_accounts,
        total_supply_str
    );

    // Get genesis/root object state from block 0 if available
    let (genesis_root, size_bytes) = match engine.get_block(0) {
        Some(b) => {
            let hash = b.hash;
            let size = hash.len() / 2;
            (hash, size)
        }
        None => ("unknown".to_string(), 0),
    };
    let dao_addr = KanariAddress::DAO_ADDRESS;
    tracing::info!(
        "The latest Root object state root: 0x{}, size: {} bytes",
        genesis_root,
        size_bytes
    );
    tracing::info!("DAO address: ({})", dao_addr);

    // Get RPC server sequencer / dev address from kanari-types constants
    let dev_addr = KanariAddress::DEV_ADDRESS;
    tracing::info!("RPC Server sequencer address: ({})", dev_addr);

    // 🟢 1. สร้างและรัน SQLite Indexer เพื่อเก็บประวัติทั้งหมดลง Database แบบรวดเร็ว
    let db_path = data_dir.join("kanari_index.db");
    let db_path_str = db_path.to_str().unwrap().to_string(); // เตรียม path ไว้ให้ Reader ด้วย
    let (indexer, indexer_tx) = KanariIndexer::new(&db_path_str, 100_000);
    tokio::spawn(indexer.run());
    tracing::info!("SQLite Indexer started successfully at {:?}", db_path);

    // Create channels for P2P message handling (used even in local mode, messages will be dropped)
    let (p2p_msg_tx, mut p2p_msg_rx) = tokio::sync::mpsc::unbounded_channel::<P2PMessage>();
    let (network_tx, network_rx) = tokio::sync::mpsc::unbounded_channel::<P2PMessage>();

    // Initialize P2P network info
    let keypair = Keypair::generate_ed25519();
    let peer_id = keypair.public().to_peer_id().to_string();
    tracing::info!("Node Peer ID: {}", peer_id);

    // Create sync manager
    let sync_manager = Arc::new(SyncManager::new(
        engine.clone(),
        network_tx.clone(),
        peer_id.clone(),
    ));

    // Start sync manager tasks
    sync_manager.clone().start().await;

    if p2p_port > 0 {
        // Load or create peer store
        let peer_store_path = PeerStore::default_path(&data_dir.display().to_string());
        let mut peer_store = PeerStore::load(peer_store_path.clone()).unwrap_or_else(|e| {
            tracing::warn!("Failed to load peer store: {}, creating new one", e);
            PeerStore::new(peer_store_path)
        });

        // Clean up old peers (older than 7 days)
        peer_store.cleanup_old_peers(7 * 24 * 60 * 60);

        let mut p2p_network = P2PNetwork::new(keypair, p2p_port, relay_server)?;
        tracing::info!("P2P network initialized on port {}", p2p_port);
        if relay_server {
            tracing::info!(
                "Relay server mode: ENABLED - This node will help relay traffic for NAT'd peers"
            );
        }

        // Connect to bootstrap peers if provided
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

        // Wrap peer store in Arc<Mutex> for sharing
        let peer_store_arc = Arc::new(tokio::sync::Mutex::new(peer_store));

        // Start P2P event handler with both incoming and outgoing message channels + peer store
        let mut event_handler = P2PEventHandler::new(p2p_network, p2p_msg_tx)
            .with_outgoing(network_rx)
            .with_peer_store(peer_store_arc.clone());
        tokio::spawn(async move {
            event_handler.run().await;
        });

        // Handle P2P messages from network
        let sync_for_messages = sync_manager.clone();
        tokio::spawn(async move {
            while let Some(msg) = p2p_msg_rx.recv().await {
                sync_for_messages.handle_message(msg).await;
            }
        });

        // Broadcast peer info periodically
        let sync_for_broadcast = sync_manager.clone();
        tokio::spawn(async move {
            // Wait a bit for peer discovery to complete before first broadcast
            sleep(Duration::from_secs(3)).await;
            loop {
                sync_for_broadcast.broadcast_peer_info().await;
                // Broadcast more frequently (every 5s) to ensure new peers sync quickly
                sleep(Duration::from_secs(5)).await;
            }
        });
    } else {
        tracing::info!("Running in local-only mode: P2P disabled");
    }

    // Start RPC server in background with cloned Arc
    let bind_addr = format!("{}:{}", rpc_host, rpc_port);
    tracing::info!("Binding RPC server to {}", bind_addr);

    // If binding to all interfaces, try to detect a representative local IP for display
    let display_ip = if rpc_host == "0.0.0.0" {
        detect_local_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        rpc_host.clone()
    };
    tracing::info!("Starting RPC server on http://{}:{}", display_ip, rpc_port);

    let engine_for_rpc = engine.clone();
    let bind_addr_clone = bind_addr.clone();

    tokio::spawn(async move {
        // 🚀 2. สร้าง Reader เพื่ออ่าน DB (แบบ Read-Only จะได้ไม่ไปล็อกคิวการเขียนของโหนดหลัก)
        match kanari_indexer::IndexerReader::new(&db_path_str) {
            Ok(reader) => {
                let indexer_reader = Arc::new(reader);
                // โยน Reader เข้าไปใน RPC Server
                if let Err(e) = start_server(engine_for_rpc, indexer_reader, &bind_addr_clone).await
                {
                    tracing::error!("RPC server error: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to start IndexerReader for RPC: {}", e);
            }
        }
    });

    // Wait for server to start
    sleep(Duration::from_millis(500)).await;
    tracing::info!("RPC server ready");

    loop {
        let stats = engine.get_stats();
        let wallets = list_wallet_files().unwrap_or_default();
        tracing::info!(
            "Block height: {}, Transactions: {}, Pending: {}, Wallets: {}",
            stats.height,
            stats.total_transactions,
            stats.pending_transactions,
            wallets.len()
        );

        let mut did_work = false;

        // Only produce blocks when there are pending transactions
        // This avoids consensus validation issues with empty blocks in both local and networked modes
        if stats.pending_transactions > 0 {
            match engine.produce_block() {
                Ok(block_info) => {
                    did_work = true;

                    tracing::info!(
                        "DAG Vertex (Round #{}) produced: {} txs ({} executed, {} failed)",
                        block_info.round,
                        block_info.tx_count,
                        block_info.executed,
                        block_info.failed
                    );

                    // 🟢 3. ส่งใบเสร็จ (Effects) เข้าไปใน SQLite Indexer ทันที!
                    if !block_info.effects.is_empty() {
                        let tx = indexer_tx.clone();
                        let effects = block_info.effects.clone();
                        tokio::spawn(async move {
                            if let Err(e) = tx.send(effects).await {
                                tracing::error!("Failed to send effects to indexer: {}", e);
                            } else {
                                tracing::debug!("Successfully sent effects to SQLite Indexer");
                            }
                        });
                    }

                    // Broadcast the DAG vertex to other nodes
                    if let Some(vertex) = block_info.vertex {
                        match serde_json::to_string(&vertex) {
                            Ok(vertex_str) => {
                                tracing::info!(
                                    "Broadcasting DAG vertex {} (round {}) to network ({} bytes)",
                                    block_info.vertex_id,
                                    block_info.round,
                                    vertex_str.len()
                                );
                                let msg = P2PMessage::NewDagVertex(vertex_str);
                                if let Err(e) = network_tx.send(msg) {
                                    tracing::warn!("Failed to queue DAG vertex broadcast: {}", e);
                                } else {
                                    tracing::info!("DAG vertex queued for broadcast successfully");
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to serialize DAG vertex for broadcast: {}",
                                    e
                                );
                            }
                        }
                    } else {
                        tracing::warn!("No vertex in block_info to broadcast");
                    }

                    // If a checkpoint was created, broadcast the new blocks as well
                    if block_info.checkpoint.is_some() {
                        let current_height = engine.blockchain.read().unwrap().height();
                        if let Some(full_block_data) = engine.get_full_block(current_height)
                            && let Ok(block_str) = serde_json::to_string(&full_block_data) {
                                let msg = P2PMessage::NewBlock(block_str);
                                if let Err(e) = network_tx.send(msg) {
                                    tracing::warn!("Failed to queue block broadcast: {}", e);
                                }
                            }
                    }
                }
                Err(e) => {
                    // ถ้ายังไม่พร้อม (รอ Quorum) มันจะพ่น Error "DAG not ready"
                    // ซึ่งถือเป็นเรื่องปกติ ให้ปล่อยมันรอไป
                    if !e.to_string().contains("DAG not ready") {
                        tracing::error!("Block production failed: {}", e);
                    }
                }
            }
        }

        // ✅ 3. โลจิกควบคุมความเร็ว (Throttle Control)
        if !did_work {
            // ถ้าไม่มีงานทำ ให้หลับรอ (ประหยัด CPU)
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Shutdown signal received. Cleaning up and exiting...");
                    break;
                }
                _ = sleep(Duration::from_secs(1)) => {}
            }
        } else {
            // ถ้าเพิ่งมีงานทำเสร็จ ให้วนลูปทำงานรอบต่อไปทันทีโดยไม่ติด Sleep
            // แต่ยังคงเช็คปุ่มปิดโปรแกรม (Ctrl+C) แบบ Non-blocking (ให้เวลาแค่ 1ms)
            if tokio::time::timeout(Duration::from_millis(1), tokio::signal::ctrl_c())
                .await
                .is_ok()
            {
                tracing::info!("Shutdown signal received. Cleaning up and exiting...");
                break;
            }
        }
    }

    tracing::info!("Node shutdown complete.");
    Ok(())
}
