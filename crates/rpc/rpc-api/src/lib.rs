use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use futures::FutureExt;
use jsonrpc_core::IoHandler;
use jsonrpc_http_server::{ServerBuilder, AccessControlAllowOrigin, DomainsValidation};
use metadata::{get_file, upload_file};
use network::NetworkConfig;
mod metadata;
mod stake;
mod get_block;
mod accounts;
mod move_api; // Add the new module

use stake::{get_staking_info, get_staking_stats, stake_tokens, unstake_tokens};
use get_block::{
    get_all_blocks, 
    get_account_details, 
    search_transactions, 
    get_transaction_by_id, 
    get_transaction_status, 
    get_gas_fee_info
};
use accounts::{get_blockchain_status, get_wallets, list_accounts, transfer_tokens};
use move_api::{
    list_modules,
    get_module,
    get_module_transactions,
    execute_function,
    deploy_module,
    get_vm_state,
    estimate_gas
};

// Format function locally since panorama::utils is not available
fn format_kari_amount(ka_amount: u64) -> String {
    const KA_PER_KARI: u64 = 1_000_000_000;
    
    // Calculate whole and fractional parts
    let whole_kari = ka_amount / KA_PER_KARI;
    let fractional_ka = ka_amount % KA_PER_KARI;
    
    // Format with thousands separators and 9 decimal places
    let whole_formatted = format!("{}", whole_kari)
        .chars()
        .rev()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(",")
        .chars()
        .rev()
        .collect::<String>();
    
    format!("{}.{:09}", whole_formatted, fractional_ka)
}

/// Starts the RPC server for file operations
pub async fn start_rpc_server(network_config: NetworkConfig) -> Result<(), tokio::io::Error> {
    let mut io = IoHandler::new();

    // Load kanari config to keep track of node connection details
    let mut kanari_config = match common::load_kanari_config() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Warning: Failed to load kanari configuration: {}", e);
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
        }
    };

    // Add file operations
    io.add_method("upload_file", |params| {
        futures::future::ready(upload_file(params)).boxed()
    });

    io.add_method("get_file", |params| {
        futures::future::ready(get_file(params)).boxed()
    });

    // Add blockchain operations
    io.add_method("blockchain_status", |params| {
        futures::future::ready(get_blockchain_status(params)).boxed()
    });
    
    
    io.add_method("list_accounts", |params| {
        futures::future::ready(list_accounts(params)).boxed()
    });
    
    io.add_method("transfer", |params| {
        futures::future::ready(transfer_tokens(params)).boxed()
    });
    
    io.add_method("get_wallets", |params| {
        futures::future::ready(get_wallets(params)).boxed()
    });

    // Use methods from get_block.rs
    io.add_method("get_all_blocks", |params| {
        futures::future::ready(get_all_blocks(params)).boxed()
    });
    
    io.add_method("get_account_details", |params| {
        futures::future::ready(get_account_details(params)).boxed()
    });
    
    io.add_method("search_transactions", |params| {
        futures::future::ready(search_transactions(params)).boxed()
    });

    io.add_method("get_transaction_by_id", |params| {
        futures::future::ready(get_transaction_by_id(params)).boxed()
    });

    io.add_method("get_transaction_status", |params| {
        futures::future::ready(get_transaction_status(params)).boxed()
    });

    io.add_method("get_gas_fee_info", |params| {
        futures::future::ready(get_gas_fee_info(params)).boxed()
    });

    // Add staking operations
    io.add_method("stake_tokens", |params| {
        futures::future::ready(stake_tokens(params)).boxed()
    });

    io.add_method("unstake_tokens", |params| {
        futures::future::ready(unstake_tokens(params)).boxed()
    });

    io.add_method("get_staking_info", |params| {
        futures::future::ready(get_staking_info(params)).boxed()
    });

    io.add_method("get_staking_stats", |params| {
        futures::future::ready(get_staking_stats(params)).boxed()
    });

    // Add enhanced Move VM operations
    io.add_method("list_modules", |params| {
        futures::future::ready(list_modules(params)).boxed()
    });

    io.add_method("get_module", |params| {
        futures::future::ready(get_module(params)).boxed()
    });

    io.add_method("get_module_transactions", |params| {
        futures::future::ready(get_module_transactions(params)).boxed()
    });

    io.add_method("execute_function", |params| {
        futures::future::ready(execute_function(params)).boxed()
    });

    io.add_method("deploy_module", |params| {
        futures::future::ready(deploy_module(params)).boxed()
    });

    io.add_method("get_vm_state", |params| {
        futures::future::ready(get_vm_state(params)).boxed()
    });

    io.add_method("estimate_gas", |params| {
        futures::future::ready(estimate_gas(params)).boxed()
    });

    // Configure socket address - bind only to localhost if in localhost_only mode
    let bind_addr = if network_config.localhost_only {
        println!("Localhost-only mode: Binding to 127.0.0.1");
        SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), // Bind only to localhost
            network_config.port
        )
    } else {
        SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), // Bind to all interfaces
            network_config.port
        )
    };

    // Create CORS settings for production
    let allowed_origins = vec![
        AccessControlAllowOrigin::Any, // Allow all origins for testing
    ];

    // Start HTTP server
    match ServerBuilder::new(io)
        .cors(DomainsValidation::AllowOnly(allowed_origins))
        .threads(4) // Increase thread count for better performance
        .max_request_body_size(10 * 1024 * 1024) // 10MB max request size
        .health_api(("health", "ready")) // Add health check endpoints
        .start_http(&bind_addr)
    {
        Ok(server) => {
            // Display basic server information
            if network_config.localhost_only {
                println!("Running in LOCALHOST-ONLY mode");
                println!("HTTP server running on http://127.0.0.1:{}", network_config.port);
                println!("Note: The node will not connect to or accept connections from other nodes");
            } else {
                println!("HTTP server running on http://{}:{}", network_config.node_address, network_config.port);
                
                // Update kanari config with the actual running port
                if let Some(kanari_mapping) = kanari_config.as_mapping_mut() {
                    // Extract active_env as a String to avoid immutable borrow persisting
                    let active_env = kanari_mapping.get("active_env")
                                     .and_then(|v| v.as_str())
                                     .unwrap_or("local")
                                     .to_string();
                                     
                    if let Some(envs) = kanari_mapping.get_mut("envs").and_then(|v| v.as_sequence_mut()) {
                        for env in envs {
                            if let Some(alias) = env.get("alias").and_then(|v| v.as_str()) {
                                if alias == active_env {
                                    // Update RPC URL with the new port
                                    env["rpc"] = serde_yaml::Value::String(format!("http://127.0.0.1:{}", network_config.port));
                                    break;
                                }
                            }
                        }
                    }
                    
                    // Save the updated kanari config
                    if let Err(e) = common::save_kanari_config(&kanari_config) {
                        eprintln!("Warning: Failed to save updated kanari config: {}", e);
                    }
                }
                
                if !network_config.peers.is_empty() {
                    println!("Connected to peers:");
                    for peer in &network_config.peers {
                        println!("  - {}", peer);
                    }
                }
            }
            
            // Create a channel for shutdown coordination
            let (shutdown_complete_tx, _shutdown_complete_rx) = tokio::sync::oneshot::channel();
            
            // Spawn a task to wait for the server in the background
            tokio::spawn(async move {
                server.wait();
                let _ = shutdown_complete_tx.send(());
            });
            
            // Return immediately, allowing the server to run in the background
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to start HTTP server: {}", e);
            Err(e.into())
        }
    }
}