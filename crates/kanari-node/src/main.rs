// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

// Main entry point for Kanari blockchain node
use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use kanari_crypto::keys::{CurveType, KANARI_KEY_PREFIX, generate_keypair};
use kanari_types::error::{KanariError, KanariUnwrapExt};
use tracing::info;

mod app;
mod indexer;
mod p2p;
mod peer_store;
mod sync;
use app::{configure_consensus_signing_key, create_engine, default_data_dir, run_node};

#[derive(Clone, Debug, ValueEnum)]
pub(crate) enum NetworkMode {
    Mainnet,
    Testnet,
    Devnet,
}

impl NetworkMode {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
            Self::Devnet => "devnet",
        }
    }
}

fn print_boot_banner(
    node_label: &str,
    network: &NetworkMode,
    p2p_port: u16,
    rpc_port: u16,
    rpc_host: &str,
    data_dir: &std::path::Path,
) {
    info!(
        node = node_label,
        network = network.as_str(),
        p2p_port,
        rpc_port,
        rpc_host,
        data_dir = %data_dir.display(),
        "Starting Kanari node"
    );
    info!("Initializing Kanari engine and Move runtime");
}

/// Kanari node command-line interface
#[derive(Parser)]
#[command(name = "kanari-node", about = "Kanari run server")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// start the node
    Start {
        /// Network mode for runtime and production safety defaults
        #[arg(long, value_enum, default_value = "testnet")]
        network: NetworkMode,
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
        /// Local Ed25519 consensus private key seed as 32-byte hex
        #[arg(long)]
        consensus_private_key_hex: String,
        /// JSON file mapping authority IDs to Ed25519 consensus public keys as hex
        #[arg(long)]
        consensus_public_keys: std::path::PathBuf,
        /// Bootstrap peer multiaddr to connect to (can be specified multiple times)
        #[arg(long, value_name = "MULTIADDR")]
        bootstrap: Option<Vec<String>>,
    },
    /// Run a local-only node
    Local,
    /// Generate Ed25519 consensus keys for local multi-node setup
    ConsensusKeygen {
        /// Number of authorities/nodes to generate
        #[arg(long)]
        node_count: usize,
        /// Output directory for node private seeds and public key map
        #[arg(long)]
        output_dir: std::path::PathBuf,
        /// Overwrite existing key files
        #[arg(long, default_value = "false")]
        force: bool,
    },
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?)
}

fn validate_start_authority_config(
    authority_id: &Option<String>,
    authorities: &Option<Vec<String>>,
) -> Result<()> {
    match (authority_id.as_ref(), authorities.as_ref()) {
        (Some(_), Some(authorities)) if !authorities.is_empty() => Ok(()),
        (Some(_), Some(_)) => {
            anyhow::bail!("DAG multi-node start requires a non-empty --authorities list")
        }
        (None, None) => anyhow::bail!(
            "kanari-node start requires --authority-id and --authorities for deterministic multi-node DAG startup; use the Local command for local-only mode"
        ),
        _ => anyhow::bail!(
            "kanari-node start requires both --authority-id and --authorities together"
        ),
    }
}

fn write_consensus_key_files(
    node_count: usize,
    output_dir: &std::path::Path,
    force: bool,
) -> Result<()> {
    if node_count == 0 {
        anyhow::bail!("--node-count must be at least 1");
    }

    std::fs::create_dir_all(output_dir)?;
    let public_keys_path = output_dir.join("consensus-public-keys.json");
    if public_keys_path.exists() && !force {
        anyhow::bail!(
            "{} already exists; pass --force to overwrite consensus keys",
            public_keys_path.display()
        );
    }

    let mut public_keys = BTreeMap::new();
    for node_id in 1..=node_count {
        let keypair =
            generate_keypair(CurveType::Ed25519).map_err(|e| KanariError::OperationFailed {
                context: "Failed to generate consensus key",
                details: e.to_string(),
            })?;
        let private_seed = keypair
            .private_key
            .strip_prefix(KANARI_KEY_PREFIX)
            .require("Generated private key has unexpected format")?
            .to_string();
        let authority = format!("0x{}", node_id);
        let private_key_path =
            output_dir.join(format!("node{}-consensus-private-key.hex", node_id));
        if private_key_path.exists() && !force {
            anyhow::bail!(
                "{} already exists; pass --force to overwrite consensus keys",
                private_key_path.display()
            );
        }

        std::fs::write(private_key_path, private_seed)?;
        public_keys.insert(authority, keypair.public_key);
    }

    std::fs::write(
        public_keys_path,
        serde_json::to_string_pretty(&public_keys)?,
    )?;

    tracing::info!(
        "Generated {} consensus key(s) in {}",
        node_count,
        output_dir.display()
    );
    Ok(())
}

fn main() -> Result<()> {
    // Initialize tracing subscriber first so all commands have log output
    tracing_subscriber::fmt()
        .with_ansi(true)
        .with_level(true)
        .with_target(true)
        .compact()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::ConsensusKeygen {
            node_count,
            output_dir,
            force,
        } => write_consensus_key_files(node_count, &output_dir, force),
        Commands::Start {
            network,
            p2p_port,
            rpc_port,
            rpc_host,
            data_dir,
            relay_server,
            authority_id,
            authorities,
            consensus_private_key_hex,
            consensus_public_keys,
            bootstrap,
        } => {
            validate_start_authority_config(&authority_id, &authorities)?;
            let data_dir_path = data_dir.clone().unwrap_or_else(default_data_dir);
            let node_label = authority_id
                .as_deref()
                .map(|id| format!("Kanari Node {}", id))
                .unwrap_or_else(|| "Kanari Node".to_string());
            print_boot_banner(
                &node_label,
                &network,
                p2p_port,
                rpc_port,
                &rpc_host,
                &data_dir_path,
            );
            let mut engine = create_engine(&data_dir, &network)?;
            info!("Engine initialized. Configuring authority and consensus keys");

            let id = authority_id.invariant("validated authority_id must exist");
            let auths = authorities.invariant("validated authorities must exist");
            tracing::info!(
                "Configuring Authority ID: {} with {} authorities",
                id,
                auths.len()
            );
            engine.set_authorities(id, auths);
            configure_consensus_signing_key(
                &mut engine,
                &consensus_private_key_hex,
                &consensus_public_keys,
            )?;
            info!("Consensus keys configured. Entering node runtime");

            runtime()?.block_on(run_node(
                std::sync::Arc::new(engine),
                network.as_str().to_string(),
                p2p_port,
                rpc_port,
                rpc_host,
                data_dir_path,
                relay_server,
                bootstrap,
            ))
        }
        Commands::Local => {
            tracing::info!("Starting local node: RPC on 127.0.0.1:6767 (P2P disabled)");
            let data_dir_path = std::path::PathBuf::from("./.kanari-local");
            // Ensure data directory exists
            std::fs::create_dir_all(&data_dir_path)?;
            let data_dir = Some(data_dir_path.clone());
            print_boot_banner(
                "Kanari Local Node",
                &NetworkMode::Devnet,
                0,
                6767,
                "127.0.0.1",
                &data_dir_path,
            );
            let engine = create_engine(&data_dir, &NetworkMode::Devnet)?;
            info!("Engine initialized. Starting local RPC node");
            runtime()?.block_on(run_node(
                std::sync::Arc::new(engine),
                NetworkMode::Devnet.as_str().to_string(),
                0,
                6767,
                "127.0.0.1".to_string(),
                data_dir_path,
                false,
                None,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_start_authority_config;

    #[test]
    fn start_requires_both_authority_fields() {
        assert!(validate_start_authority_config(&None, &None).is_err());
        assert!(validate_start_authority_config(&Some("0x1".to_string()), &None).is_err());
        assert!(validate_start_authority_config(&None, &Some(vec!["0x1".to_string()])).is_err());
    }

    #[test]
    fn start_rejects_empty_authority_list() {
        assert!(validate_start_authority_config(&Some("0x1".to_string()), &Some(vec![])).is_err());
    }

    #[test]
    fn start_accepts_complete_authority_config() {
        assert!(
            validate_start_authority_config(
                &Some("0x1".to_string()),
                &Some(vec!["0x1".to_string(), "0x2".to_string()]),
            )
            .is_ok()
        );
    }
}
