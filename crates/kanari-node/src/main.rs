// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

// Main entry point for Kanari blockchain node
use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use kanari_core::{decode_hex_exact, write_file_atomically, write_json_pretty_atomically};
use kanari_crypto::keys::{CurveType, KANARI_KEY_PREFIX, generate_keypair};
use kanari_types::error::{KanariError, KanariUnwrapExt};
use tracing::info;
use zeroize::Zeroizing;

mod app;
mod config;
mod indexer;
mod p2p;
mod peer_store;
mod sync;
mod validator_backup;
use app::{
    configure_consensus_signing_key, create_engine, create_engine_required,
    create_engine_with_genesis, default_data_dir, load_or_create_p2p_identity, run_node,
};
use validator_backup::{export_validator_backup, import_validator_backup};

fn write_cli_line(args: std::fmt::Arguments<'_>) -> Result<()> {
    use std::io::Write;

    writeln!(std::io::stdout(), "{args}")?;
    Ok(())
}

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
        /// File containing an encrypted (recommended) or development-only plaintext consensus key
        #[arg(long)]
        consensus_private_key_file: std::path::PathBuf,
        /// JSON file mapping authority IDs to Ed25519 consensus public keys as hex
        #[arg(long)]
        consensus_public_keys: std::path::PathBuf,
        /// Bootstrap peer multiaddr to connect to (can be specified multiple times)
        #[arg(long, value_name = "MULTIADDR")]
        bootstrap: Option<Vec<String>>,
        /// Genesis manifest shared by the network; validated before startup
        #[arg(long)]
        genesis: Option<std::path::PathBuf>,
    },
    /// Export the deterministic genesis identity for other nodes
    GenesisExport {
        #[arg(long, value_enum, default_value = "testnet")]
        network: NetworkMode,
        /// Existing node data directory to read the authoritative genesis from
        #[arg(long)]
        data_dir: Option<std::path::PathBuf>,
        #[arg(long)]
        output: std::path::PathBuf,
    },
    /// Export a committed state snapshot for a new node
    SnapshotExport {
        #[arg(long, value_enum, default_value = "testnet")]
        network: NetworkMode,
        #[arg(long)]
        data_dir: std::path::PathBuf,
        #[arg(long)]
        output: std::path::PathBuf,
        /// Allow exporting a legacy database whose checkpoint root differs from its current state root
        #[arg(long, default_value_t = false)]
        allow_state_root_migration: bool,
    },
    /// Import and verify a committed state snapshot into an empty data dir
    SnapshotImport {
        #[arg(long, value_enum, default_value = "testnet")]
        network: NetworkMode,
        #[arg(long)]
        snapshot: std::path::PathBuf,
        #[arg(long)]
        data_dir: std::path::PathBuf,
        /// Checkpoint hash obtained from a trusted channel (required outside devnet)
        #[arg(long)]
        expected_checkpoint_hash: Option<String>,
    },
    /// Export encrypted full-validator recovery data (state, WAL, identity, keys, and genesis)
    ValidatorBackupExport {
        #[arg(long, value_enum, default_value = "testnet")]
        network: NetworkMode,
        #[arg(long)]
        data_dir: std::path::PathBuf,
        #[arg(long)]
        consensus_private_key_file: std::path::PathBuf,
        #[arg(long)]
        consensus_public_keys: std::path::PathBuf,
        #[arg(long)]
        genesis: std::path::PathBuf,
        #[arg(long)]
        output: std::path::PathBuf,
    },
    /// Restore an encrypted full-validator backup into empty directories
    ValidatorBackupImport {
        #[arg(long, value_enum, default_value = "testnet")]
        network: NetworkMode,
        #[arg(long)]
        backup: std::path::PathBuf,
        #[arg(long)]
        data_dir: std::path::PathBuf,
        /// Empty directory receiving consensus keys and genesis
        #[arg(long)]
        recovery_dir: std::path::PathBuf,
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
    /// Encrypt an existing 32-byte consensus seed without changing the authority key
    ConsensusKeyEncrypt {
        #[arg(long)]
        input: std::path::PathBuf,
        #[arg(long)]
        output: std::path::PathBuf,
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
        let private_seed = Zeroizing::new(
            keypair
                .private_key
                .strip_prefix(KANARI_KEY_PREFIX)
                .require("Generated private key has unexpected format")?
                .to_string(),
        );
        let authority = format!("0x{}", node_id);
        let private_key_path =
            output_dir.join(format!("node{}-consensus-private-key.key", node_id));
        if private_key_path.exists() && !force {
            anyhow::bail!(
                "{} already exists; pass --force to overwrite consensus keys",
                private_key_path.display()
            );
        }

        let key_file = match std::env::var("KANARI_CONSENSUS_KEY_PASSWORD") {
            Ok(password) if !password.is_empty() => {
                let password = Zeroizing::new(password);
                let encrypted = kanari_crypto::encrypt_string(&private_seed, &password)
                    .map_err(|error| anyhow::anyhow!("Failed to encrypt consensus key: {error}"))?;
                serde_json::to_string_pretty(&encrypted)?
            }
            _ => {
                tracing::warn!(
                    path = %private_key_path.display(),
                    "Writing a plaintext development consensus key; set KANARI_CONSENSUS_KEY_PASSWORD to encrypt it"
                );
                private_seed.to_string()
            }
        };
        write_file_atomically(&private_key_path, key_file.as_bytes())?;
        public_keys.insert(authority, keypair.public_key);
    }

    write_json_pretty_atomically(&public_keys_path, &public_keys)?;

    tracing::info!(
        "Generated {} consensus key(s) in {}",
        node_count,
        output_dir.display()
    );
    Ok(())
}

fn encrypt_existing_consensus_key(
    input: &std::path::Path,
    output: &std::path::Path,
    force: bool,
) -> Result<()> {
    if output.exists() && !force {
        anyhow::bail!(
            "{} already exists; pass --force to overwrite it",
            output.display()
        );
    }
    let private_seed = Zeroizing::new(std::fs::read_to_string(input)?);
    let private_seed = Zeroizing::new(private_seed.trim().to_string());
    let _decoded = Zeroizing::new(decode_hex_exact(
        "consensus private key seed",
        &private_seed,
        32,
    )?);
    let password =
        Zeroizing::new(std::env::var("KANARI_CONSENSUS_KEY_PASSWORD").map_err(|_| {
            anyhow::anyhow!("KANARI_CONSENSUS_KEY_PASSWORD is required to encrypt a consensus key")
        })?);
    if password.len() < 12 {
        anyhow::bail!("KANARI_CONSENSUS_KEY_PASSWORD must contain at least 12 characters");
    }
    let encrypted = kanari_crypto::encrypt_string(&private_seed, &password)
        .map_err(|error| anyhow::anyhow!("Failed to encrypt consensus key: {error}"))?;
    write_json_pretty_atomically(output, &encrypted)?;
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
        Commands::ConsensusKeyEncrypt {
            input,
            output,
            force,
        } => {
            encrypt_existing_consensus_key(&input, &output, force)?;
            write_cli_line(format_args!(
                "Encrypted consensus key written to {}",
                output.display()
            ))
        }
        Commands::Start {
            network,
            p2p_port,
            rpc_port,
            rpc_host,
            data_dir,
            relay_server,
            authority_id,
            authorities,
            consensus_private_key_file,
            consensus_public_keys,
            bootstrap,
            genesis,
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
            let mut engine = create_engine_with_genesis(&data_dir, &network, genesis.as_deref())?;
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
                &consensus_private_key_file,
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
        Commands::GenesisExport {
            network,
            data_dir,
            output,
        } => {
            let engine = create_engine(&data_dir, &network)?;
            engine.write_genesis_manifest(&output, network.as_str())?;
            write_cli_line(format_args!(
                "Genesis manifest written to {}",
                output.display()
            ))
        }
        Commands::SnapshotExport {
            network,
            data_dir,
            output,
            allow_state_root_migration,
        } => {
            let engine = create_engine_required(&data_dir, &network)?;
            let snapshot = engine.export_state_snapshot_with_options(
                &output,
                network.as_str(),
                allow_state_root_migration,
            )?;
            write_cli_line(format_args!(
                "Snapshot written to {} at height {} (state root {})",
                output.display(),
                snapshot.checkpoint_height,
                snapshot.state_root
            ))
        }
        Commands::SnapshotImport {
            network,
            snapshot,
            data_dir,
            expected_checkpoint_hash,
        } => {
            let imported = match expected_checkpoint_hash.as_deref() {
                Some(expected) => kanari_core::BlockchainEngine::import_state_snapshot(
                    &snapshot,
                    &data_dir,
                    network.as_str(),
                    expected,
                )?,
                None if matches!(&network, NetworkMode::Devnet) => {
                    tracing::warn!(
                        "Importing devnet snapshot without an externally pinned checkpoint hash"
                    );
                    kanari_core::BlockchainEngine::import_trusted_state_snapshot(
                        &snapshot,
                        &data_dir,
                        network.as_str(),
                    )?
                }
                None => anyhow::bail!(
                    "--expected-checkpoint-hash is required for testnet/mainnet snapshot import"
                ),
            };
            write_cli_line(format_args!(
                "Snapshot imported into {} at height {} (state root {})",
                data_dir.display(),
                imported.checkpoint_height,
                imported.state_root
            ))
        }
        Commands::ValidatorBackupExport {
            network,
            data_dir,
            consensus_private_key_file,
            consensus_public_keys,
            genesis,
            output,
        } => {
            let engine = create_engine_required(&data_dir, &network)?;
            drop(load_or_create_p2p_identity(&data_dir, network.as_str())?);
            let summary = export_validator_backup(
                &engine,
                network.as_str(),
                &data_dir,
                &consensus_private_key_file,
                &consensus_public_keys,
                &genesis,
                &output,
            )?;
            write_cli_line(format_args!(
                "Encrypted validator backup written to {} at height {} (state root {}, {} recovery files)",
                output.display(),
                summary.checkpoint_height,
                summary.state_root,
                summary.included_files
            ))
        }
        Commands::ValidatorBackupImport {
            network,
            backup,
            data_dir,
            recovery_dir,
        } => {
            let summary =
                import_validator_backup(&backup, network.as_str(), &data_dir, &recovery_dir)?;
            write_cli_line(format_args!(
                "Validator backup restored into {} at height {} (state root {}, {} recovery files in {})",
                data_dir.display(),
                summary.checkpoint_height,
                summary.state_root,
                summary.included_files,
                recovery_dir.display()
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
            let mut engine = create_engine(&data_dir, &NetworkMode::Devnet)?;
            let consensus_dir = data_dir_path.join("consensus-keys");
            let private_key_path = consensus_dir.join("node1-consensus-private-key.key");
            let public_keys_path = consensus_dir.join("consensus-public-keys.json");
            match (private_key_path.exists(), public_keys_path.exists()) {
                (false, false) => {
                    write_consensus_key_files(1, &consensus_dir, false)?;
                    tracing::info!(
                        path = %consensus_dir.display(),
                        "Generated persistent single-validator consensus key for local mode"
                    );
                }
                (true, true) => {}
                _ => anyhow::bail!(
                    "Local consensus key set is incomplete in {}; remove the incomplete consensus-keys directory and retry",
                    consensus_dir.display()
                ),
            }
            engine.set_authorities("0x1".to_string(), vec!["0x1".to_string()]);
            configure_consensus_signing_key(&mut engine, &private_key_path, &public_keys_path)?;
            info!("Engine initialized. Local consensus key configured; starting RPC node");
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
#[path = "../tests/unit/main_tests.rs"]
mod tests;
