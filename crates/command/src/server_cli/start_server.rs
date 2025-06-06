use colored::Colorize;
use network::NetworkConfig;
use panorama::simulation::run_blockchain;

use common::{load_kanari_config, save_kanari_config, ensure_network_config, load_config};
use mona_types::storage::{load_blockchain, save_blockchain};
use mona_blockchain::chain_id::CHAIN_ID;
use rpc_api::start_rpc_server;


use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

use mona_crypto::{check_wallet_exists, list_wallet_files};
use std::sync::{Arc, Mutex};
use serde_yaml::Value;
// Removed: use std::path::Path; // No longer needed for wallet checks here
use std::io::{self, Write};
use std::error::Error; // For Box<dyn Error>

pub async fn start_server(
    peers: Vec<String>,
    port: Option<u16>,
    localhost_only: bool,
    selected_wallet: Option<String>,
    use_tls: bool,
) -> Result<(), Box<dyn Error>> {
    // Check if any wallet exists first
    if !check_wallet_exists() {
        println!("{}", "No wallet found!".red());
        println!("Please create a wallet first using:");
        println!("{}", "kari keytool generate".green());
        return Err(Box::from("No wallet found. Please create a wallet first."));
    }

    // Get or create network configuration using unified function
    let network_config = ensure_network_config(CHAIN_ID, port, localhost_only, use_tls, peers)?;

    // Also load kanari configuration for wallet management
    let mut kanari_config = match load_kanari_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Failed to load kanari configuration: {}", e);
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        }
    };

    // Load legacy config for backward compatibility
    let config = match load_config() {
        Ok(cfg) => cfg,
        Err(_) => serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    };

    // Get the local IP address earlier for consistent use in configurations
    let determined_local_ip = match panorama::node::get_local_ip() {
        Some(ip) => ip,
        None => {
            eprintln!("Warning: Could not determine local IP address. Defaulting to 127.0.0.1 for configuration purposes.");
            "127.0.0.1".to_string()
        }
    };

    // Display configuration being used
    println!("Using network configuration:");
    println!("  Port: {}", network_config.port);
    println!("  Localhost only: {}", network_config.localhost_only);
    println!("  Use TLS: {}", network_config.use_tls);
    if !network_config.peers.is_empty() {
        println!("  Peers: {:?}", network_config.peers);
    }

    // Try to load blockchain with better error handling
    if let Err(e) = load_blockchain() {
        eprintln!(
            "Warning: Failed to load blockchain: {}. A new blockchain will be created.",
            e
        );
    }

    let running = Arc::new(Mutex::new(true));

    // Address selection logic updated for selected_wallet
    let address = if let Some(wallet_name) = selected_wallet {
        let all_wallets = list_wallet_files().unwrap_or_else(|e| {
            eprintln!("Warning: Failed to list wallet files: {}", e);
            Vec::new()
        });

        if all_wallets.iter().any(|(addr, _)| addr == &wallet_name) {
            println!("Using selected wallet as address: {}", wallet_name.green());
            if let Some(kanari_mapping) = kanari_config.as_mapping_mut() {
                kanari_mapping.insert(
                    Value::String("active_address".to_string()),
                    Value::String(wallet_name.clone()),
                );
                if let Err(e) = save_kanari_config(&kanari_config) {
                    eprintln!("Warning: Failed to save updated kanari config: {}", e);
                }
            }
            wallet_name
        } else {
            println!("Selected wallet '{}' does not exist.", wallet_name.red());
            if let Some((first_wallet_addr, _)) = all_wallets.first() {
                println!("Falling back to existing wallet: {}", first_wallet_addr.green());
                if let Some(kanari_mapping) = kanari_config.as_mapping_mut() {
                    kanari_mapping.insert(
                        Value::String("active_address".to_string()),
                        Value::String(first_wallet_addr.clone()),
                    );
                    if let Err(e) = save_kanari_config(&kanari_config) {
                        eprintln!("Warning: Failed to save updated kanari config: {}", e);
                    }
                }
                first_wallet_addr.clone()
            } else {
                println!("{}", "No other wallets found!".red());
                println!("Please create a wallet or ensure the selected wallet exists.");
                return Err(Box::from("Selected wallet not found and no fallback wallets available."));
            }
        }
    } else {
        // Original logic if no specific wallet is selected
        let all_wallets = list_wallet_files().unwrap_or_else(|e| {
            eprintln!("Warning: Failed to list wallet files: {}", e);
            Vec::new()
        });

        if let Some(kanari_mapping) = kanari_config.as_mapping_mut() { // Changed to as_mapping_mut for potential updates
            if let Some(active_addr_str) = kanari_mapping.get("active_address").and_then(|v| v.as_str()) {
                if all_wallets.iter().any(|(addr, _)| addr == active_addr_str) {
                    active_addr_str.to_string()
                } else {
                    println!("Active address wallet '{}' not found in keystore.", active_addr_str.red());
                    if let Some((first_wallet_addr, _)) = all_wallets.first() {
                        println!("Using existing wallet: {}", first_wallet_addr.green());
                        kanari_mapping.insert(
                            Value::String("active_address".to_string()),
                            Value::String(first_wallet_addr.clone()),
                        );
                        if let Err(e) = save_kanari_config(&kanari_config) {
                            eprintln!("Warning: Failed to save updated kanari config: {}", e);
                        }
                        first_wallet_addr.clone()
                    } else {
                        println!("{}", "No valid wallets found!".red());
                        return Err(Box::from("No valid wallets found after checking active_address."));
                    }
                }
            } else {
                // No active_address in kanari_config, try config.yaml or list_wallet_files
                match config.get("address").and_then(|v| v.as_str()) {
                    Some(address_str_from_config) => {
                        if all_wallets.iter().any(|(addr, _)| addr == address_str_from_config) {
                            // Update kanari_config with this address
                            kanari_mapping.insert(
                                Value::String("active_address".to_string()),
                                Value::String(address_str_from_config.to_string()),
                            );
                            if let Err(e) = save_kanari_config(&kanari_config) {
                                eprintln!("Warning: Failed to save updated kanari config: {}", e);
                            }
                            address_str_from_config.to_string()
                        } else {
                            println!("Config address wallet '{}' not found in keystore.", address_str_from_config.red());
                            if let Some((first_wallet_addr, _)) = all_wallets.first() {
                                println!("Using existing wallet: {}", first_wallet_addr.green());
                                kanari_mapping.insert(
                                    Value::String("active_address".to_string()),
                                    Value::String(first_wallet_addr.clone()),
                                );
                                if let Err(e) = save_kanari_config(&kanari_config) {
                                    eprintln!("Warning: Failed to save updated kanari config: {}", e);
                                }
                                first_wallet_addr.clone()
                            } else {
                                println!("{}", "No valid wallets found!".red());
                                return Err(Box::from("No valid wallets found after checking config.address."));
                            }
                        }
                    }
                    None => {
                        // No address in config.yaml, use first from list_wallet_files
                        if let Some((first_wallet_addr, _)) = all_wallets.first() {
                            println!("Setting address to existing wallet: {}", first_wallet_addr.green());
                            kanari_mapping.insert(
                                Value::String("active_address".to_string()),
                                Value::String(first_wallet_addr.clone()),
                            );
                            if let Err(e) = save_kanari_config(&kanari_config) {
                                eprintln!("Warning: Failed to save updated kanari config: {}", e);
                            }
                            first_wallet_addr.clone()
                        } else {
                            println!("{}", "No wallets found!".red());
                            return Err(Box::from("No wallets found at all."));
                        }
                    }
                }
            }
        } else {
             // Fallback if kanari_config is not a mapping (e.g., corrupted or new)
             // This block might be less likely if kanari_config is initialized to a mapping earlier
            match config.get("address").and_then(|v| v.as_str()) {
                Some(address_str_from_config) => {
                     if all_wallets.iter().any(|(addr, _)| addr == address_str_from_config) {
                        address_str_from_config.to_string()
                    } else {
                        println!("Config address wallet '{}' not found (no kanari_config). Using existing wallet.", address_str_from_config.red());
                        if let Some((first_wallet_addr, _)) = all_wallets.first() {
                            println!("Using existing wallet: {}", first_wallet_addr.green());
                            first_wallet_addr.clone()
                        } else {
                            println!("{}", "No valid wallets found!".red());
                            return Err(Box::from("No valid wallets found (no kanari_config, config.address invalid)."));
                        }
                    }
                }
                None => {
                    if let Some((first_wallet_addr, _)) = all_wallets.first() {
                        println!("Setting address to existing wallet (no kanari_config, no config.address): {}", first_wallet_addr.green());
                        first_wallet_addr.clone()
                    } else {
                        println!("{}", "No wallets found!".red());
                        return Err(Box::from("No wallets found (no kanari_config, no config.address)."));
                    }
                }
            }
        }
    };

    // Make sure kanari.yaml has the active address
    if let Some(kanari_mapping) = kanari_config.as_mapping_mut() {
        kanari_mapping.insert(
            serde_yaml::Value::String("active_address".to_string()),
            serde_yaml::Value::String(address.clone()),
        );
        if let Err(e) = save_kanari_config(&kanari_config) {
            eprintln!("Warning: Failed to save updated kanari config: {}", e);
        }
    }

    if address.is_empty() {
        println!("Please generate an address first using the 'kari keytool' command.");
        return Err(Box::from("Address is empty. Please generate an address."));
    }

    println!("Using existing address: {}", address.green());
    *running.lock().unwrap() = true;
    println!("{}", "Starting blockchain...".green());

    // Create a channel for block status updates
    let (tx, mut rx) = mpsc::channel::<String>(100);

    // Create a oneshot channel for shutdown signaling
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<>();

    let running_clone = Arc::clone(&running);
    let address_clone = address.clone();

    // Spawn blockchain simulation task
    let blockchain_handle = tokio::spawn(async move {
        println!("Running blockchain simulation...");
        run_blockchain(running_clone, address_clone, tx);
    });

    // Use the local IP address determined earlier
    let local_ip = determined_local_ip;

    // Create RPC configuration from network config
    let rpc_config = NetworkConfig {
        node_address: if localhost_only {
            "127.0.0.1".to_string()
        } else {
            local_ip.clone()
        },
        port: network_config.port,
        peers: network_config.peers.clone(),
        chain_id: network_config.chain_id.clone(),
        max_connections: network_config.max_connections,
        api_enabled: network_config.api_enabled,
        localhost_only: network_config.localhost_only,
        use_tls: network_config.use_tls,
        trusted_peers: network_config.trusted_peers.clone(),
    };

    // Display IP address and port information for connecting nodes
    println!("{}", "Node network information:".bright_yellow());

    if localhost_only {
        println!("  LOCALHOST-ONLY MODE: Node is only accessible from this machine");
        println!("  RPC API:   127.0.0.1:{} (HTTP)", rpc_config.port);
        println!("  No external P2P connections will be allowed");
    } else {
        println!("  RPC API:   {}:{} (HTTP)", local_ip, rpc_config.port);
        println!("  P2P:       {}:51303", local_ip);

        // Display peer connection information if applicable
        if !rpc_config.peers.is_empty() {
            println!("{}", "Node will connect to the following peers:".bright_yellow());
            for peer in &rpc_config.peers {
                println!("  - {}", peer.green());
            }
        } else {
            println!(
                "{}",
                "Warning: No peers configured. Running in standalone mode.".yellow()
            );
            println!("  To connect peers, use: kari start --peer <IP:PORT>");
        }
    }

    // Start RPC server with shutdown signal
    let rpc_handle = tokio::spawn(async move {
        println!("Starting RPC server on port {}...", rpc_config.port);

        // Try multiple times to start the server in case of port issues
        let mut attempts = 0;
        const MAX_ATTEMPTS: usize = 3;
        let mut current_rpc_config = rpc_config.clone();

        while attempts < MAX_ATTEMPTS {
            match start_rpc_server(current_rpc_config.clone()).await {
                Ok(_) => {
                    println!("RPC server started successfully on port {}.", current_rpc_config.port);
                    // Keep this task alive until it's aborted
                    loop {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        tokio::task::yield_now().await;
                    }
                    // Unreachable, but keeps structure, loop above is infinite until abort
                }
                Err(e) => {
                    attempts += 1;
                    eprintln!(
                        "Failed to start RPC server (attempt {}/{}): {}",
                        attempts, MAX_ATTEMPTS, e
                    );

                    if attempts >= MAX_ATTEMPTS {
                        eprintln!(
                            "Failed to start RPC server after {} attempts. Exiting from RPC spawn.",
                            MAX_ATTEMPTS
                        );
                        // This exit is inside a tokio::spawn, it will kill the task, not the main process directly.
                        // To signal failure to the main function, a channel or other IPC would be needed.
                        // For now, we let the task die. The main function will continue.
                        // If RPC server is critical, main function should await this handle or use a channel.
                        // Given the original code's std::process::exit(1), this task failing should probably propagate.
                        // However, changing this requires more significant refactoring of how errors are handled from spawned tasks.
                        // For now, just print and let the task end. The main function will proceed to shutdown.
                        return; // Exit the spawned task
                    }
                    
                    // Try with a different port
                    current_rpc_config.port += 1; // Modify the cloned config for the next attempt
                    eprintln!("Trying with port {} instead...", current_rpc_config.port);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });

    // This task will handle the Enter keypress
    println!("{}", "Block status will be shown below. Press Enter to stop the node.".yellow());
    io::stdout().flush().unwrap();

    // Spawn a task to listen for Enter key
    tokio::spawn(async move {
        let mut input = String::new();
        match io::stdin().read_line(&mut input) {
            Ok(_) => {
                println!("Received shutdown request...");
                // Signal shutdown
                let _ = shutdown_tx.send(());
            }
            Err(e) => {
                println!("Error reading input: {}", e);
                let _ = shutdown_tx.send(());
            }
        }
    });

    // Display block status updates; break loop once shutdown is signaled
    loop {
        tokio::select! {
            Some(status) = rx.recv() => {
                println!("{}", status.bright_cyan());
            }
            _ = &mut shutdown_rx => {
                println!("Shutdown signal received. Stopping node...");
                *running.lock().unwrap() = false;
                break;
            }
        }
    }

    // Give some time for the blockchain to shutdown gracefully
    println!("{}", "Stopping blockchain...".red());
    sleep(Duration::from_secs(1)).await;

    // Ensure blockchain state is saved
    let _ = save_blockchain();

    // Wait for blockchain task to complete (with timeout)
    match tokio::time::timeout(Duration::from_secs(5), blockchain_handle).await {
        Ok(_) => println!("Blockchain stopped gracefully"),
        Err(_) => println!("Blockchain shutdown timed out, forcing exit"),
    }

    // Force exit (needed because RPC server might be hanging)
    println!("Node stopped. Exiting...");
    // Abort the RPC server task explicitly before exiting
    rpc_handle.abort();
    // std::process::exit(0); // Replaced by Ok(())
    Ok(())
}
