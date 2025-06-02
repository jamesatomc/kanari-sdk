// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use common::get_main_wallet;
use consensus_pos::Blake3Algorithm;
use log::{debug, error, info, warn};
use serde_json::{Value, json};
use sha3::Digest; // Add this import
use std::collections::VecDeque;
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc; // Add this import

use crate::node::{NodeConfig, get_peer_count, propagate_block, start_node, stop_node};
use crate::staking::{is_validator, load_staking_state, process_rewards};
use crate::transfer_tokens::transfer_tokens;
use crate::utils::{
    calculate_gas_fee, format_gas_fee_display, update_last_block_time,
    update_pending_transaction_count,
};
use mona_blockchain::block::{Block, Transaction};
use mona_blockchain::blockchain::{BALANCES, BLOCKCHAIN_DATA, normalize_address, save_blockchain};
use mona_types::address::Address;
use mona_types::kari::{
    KA_PER_KARI, KARI, NODE_STAKING_MINIMUM_KARI, POOL_ADDRESS, POOL_RESERVED_KA,
    POOL_RESERVED_KARI, TOTAL_SUPPLY_KA, TOTAL_SUPPLY_KARI, VALIDATOR_STAKING_MINIMUM_KARI,
};
use mona_vm::{convert_to_vm_transaction, execute_vm_transaction}; // Remove VM_STATE as it's unused

pub mod create_genesis_block;
use create_genesis_block::create_genesis_block;

// Function to parse and normalize address
fn parse_address(address: &str) -> Result<Address, String> {
    Address::from_str(address).map_err(|_| format!("Invalid address format: {}", address))
}

// Add pending transactions queue
lazy_static::lazy_static! {
    static ref PENDING_TRANSACTIONS: RwLock<VecDeque<Transaction>> = RwLock::new(VecDeque::new());
}

// Now use .write() or .read() instead of .lock()
pub fn add_pending_transaction(transaction: Transaction) -> bool {
    match PENDING_TRANSACTIONS.write() {
        Ok(mut queue) => {
            queue.push_back(transaction);
            // Update pending transaction count for gas calculation
            update_pending_transaction_count(queue.len());
            true
        }
        Err(_) => false,
    }
}

// Enhanced function to handle VM transactions
pub fn process_vm_transaction(
    module_id: &str,
    function: &str,
    args: Vec<Vec<u8>>,
    sender_address: &str,
    gas_budget: u64,
    password: Option<&str>,
    tx: &mpsc::Sender<String>,
) -> Result<Transaction, String> {
    // Create VM transaction
    let vm_tx = mona_vm::VMTransaction::new(
        sender_address.to_string(),
        module_id.to_string(),
        function.to_string(),
        args,
        gas_budget,
    );

    // Sign if password provided
    let vm_tx = if let Some(pwd) = password {
        // Create payload for signing
        let mut hasher = sha3::Sha3_256::new();
        hasher.update(sender_address.as_bytes());
        hasher.update(module_id.as_bytes());
        hasher.update(function.as_bytes());
        hasher.update(gas_budget.to_le_bytes());
        let payload = hasher.finalize();

        // Sign with wallet if available
        if let Some(wallet_addr) = get_main_wallet() {
            if let Ok(wallet) = mona_crypto::load_wallet(&wallet_addr, pwd) {
                if let Ok(signature) = wallet.sign(&payload, pwd) {
                    vm_tx.with_signature(signature, wallet_addr.to_string()) // Fix: convert to String
                } else {
                    vm_tx
                }
            } else {
                vm_tx
            }
        } else {
            vm_tx
        }
    } else {
        vm_tx
    };

    // Execute VM transaction
    match execute_vm_transaction(&vm_tx) {
        Ok(result) => {
            // Create blockchain transaction for VM call
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            let vm_data = format!("VM:{}:{}:{}", module_id, function, gas_budget);

            let blockchain_tx = Transaction {
                transaction_id: vm_tx.tx_id.clone(),
                sender: parse_address(sender_address).map_err(|e| e.to_string())?,
                receiver: parse_address(sender_address).map_err(|e| e.to_string())?, // VM calls are self-transactions
                amount: 1, // Set minimal amount of 1 KA for VM transactions
                gas_fee: result["gas_used"].as_u64().unwrap_or(gas_budget),
                timestamp,
                signature: vm_tx.signature.unwrap_or_default(),
                data: Some(vm_data.into_bytes()),
            };

            // Add to pending transactions
            if add_pending_transaction(blockchain_tx.clone()) {
                let tx_json = json!({
                    "event": "vm_transaction_created",
                    "transaction": {
                        "id": blockchain_tx.transaction_id,
                        "type": "VM_FUNCTION_CALL",
                        "module_id": module_id,
                        "function": function,
                        "sender": sender_address,
                        "gas_used": blockchain_tx.gas_fee,
                        "gas_display": format_gas_fee_display(blockchain_tx.gas_fee),
                        "timestamp": timestamp,
                        "result": result
                    },
                    "status": "pending"
                })
                .to_string();

                let _ = tx.try_send(tx_json);
                Ok(blockchain_tx)
            } else {
                Err("Failed to add VM transaction to blockchain".to_string())
            }
        }
        Err(e) => {
            let error_json = json!({
                "event": "vm_transaction_error",
                "error": e,
                "details": {
                    "module_id": module_id,
                    "function": function,
                    "sender": sender_address,
                    "gas_budget": gas_budget
                }
            })
            .to_string();

            let _ = tx.try_send(error_json);
            Err(e)
        }
    }
}

// Enhanced process_transfer to handle VM transactions
pub fn process_transfer(
    from_address: &str,
    to_address: &str,
    amount: u64,
    password: &str,
    priority_boost: Option<u64>,
    tx: &mpsc::Sender<String>,
) -> Result<Transaction, String> {
    // Check if this is a VM transaction (contains :: in the address)
    if to_address.contains("::") {
        // This is a VM function call
        let parts: Vec<&str> = to_address.split("::").collect();
        if parts.len() >= 2 {
            let module_id = format!("{}::{}", parts[0], parts[1]);
            let function = parts.get(2).unwrap_or(&"main").to_string();

            return process_vm_transaction(
                &module_id,
                &function,
                vec![], // No args for simple calls
                from_address,
                amount, // Use amount as gas budget
                Some(password),
                tx,
            );
        }
    }

    // Regular token transfer logic
    // Parse addresses
    let from = match normalize_address(from_address) {
        Ok(addr) => addr,
        Err(e) => return Err(format!("Invalid sender address: {}", e)),
    };

    let to = match normalize_address(to_address) {
        Ok(addr) => addr,
        Err(e) => return Err(format!("Invalid receiver address: {}", e)),
    };

    // Calculate gas fee dynamically
    let gas_fee = calculate_gas_fee(priority_boost);
    let gas_fee_display = format_gas_fee_display(gas_fee);

    // Execute transfer using string representation and password for signing
    match transfer_tokens(
        &from.to_hex_literal(),
        &to.to_hex_literal(),
        amount,
        password,
        gas_fee,
    ) {
        Ok(transaction) => {
            // Verify signature right after creation for better debugging
            let signature_status = if transaction.signature.is_empty() {
                "unsigned"
            } else {
                match crate::transfer_tokens::verify_transaction::verify_transaction(&transaction) {
                    Ok(true) => "valid",
                    Ok(false) => "invalid",
                    Err(e) => {
                        warn!("Error verifying signature: {}", e);
                        "unknown"
                    }
                }
            };

            // Add to pending transactions
            if add_pending_transaction(transaction.clone()) {
                // Notify about successful transaction submission
                let tx_json = json!({
                    "event": "transaction_created",
                    "transaction": {
                        "id": transaction.transaction_id,
                        "sender": transaction.sender.to_hex_literal(),
                        "receiver": transaction.receiver.to_hex_literal(),
                        "amount": amount,
                        "gas_fee": transaction.gas_fee,
                        "gas_fee_display": format_gas_fee_display(transaction.gas_fee),
                        "gas_collector": crate::utils::GAS_FEE_COLLECTOR,
                        "total_cost": crate::utils::calculate_total_transaction_cost(amount, transaction.gas_fee),
                        "timestamp": transaction.timestamp,
                        "signed": !transaction.signature.is_empty(),
                        "signature_status": signature_status
                    },
                    "status": "pending"
                }).to_string();

                let _ = tx.try_send(tx_json);

                // Force save blockchain state to ensure transaction persistence
                match mona_blockchain::blockchain::save_blockchain() {
                    Ok(_) => info!("Transaction recorded and blockchain state saved"),
                    Err(e) => warn!("Transaction recorded but failed to save state: {}", e),
                }

                // Return transaction
                Ok(transaction)
            } else {
                // Try to add the transaction directly to the next block
                match force_transaction_inclusion(&transaction) {
                    true => {
                        info!("Transaction bypassed queue and directly included in blockchain");
                        Ok(transaction)
                    }
                    false => Err("Failed to add transaction to blockchain".to_string()),
                }
            }
        }
        Err(e) => {
            // Update error JSON to use calculated gas fee
            let error_json = json!({
                "event": "transaction_error",
                "error": format!("{}", e),
                "details": {
                    "sender": from_address,
                    "receiver": to_address,
                    "amount": amount,
                    "gas_fee": gas_fee,
                    "gas_fee_display": gas_fee_display,
                    "total_cost": crate::utils::calculate_total_transaction_cost(amount, gas_fee)
                }
            })
            .to_string();

            let _ = tx.try_send(error_json);

            // Return error
            Err(format!("{}", e))
        }
    }
}

// Function to handle transactions when queue fails
fn force_transaction_inclusion(transaction: &Transaction) -> bool {
    // Get the current blockchain state
    let blocks = BLOCKCHAIN_DATA.iter();
    let last_block = match blocks.last() {
        Some(block) => block,
        None => {
            error!("Cannot find previous block");
            return false;
        }
    };

    // Create transaction list
    let mut transactions = Vec::new();
    transactions.push(transaction.clone());

    // Create a forced block with this transaction
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Create block data - properly serialize Address types via serde JSON
    let block_data = json!({
        "block_type": "forced_transaction",
        "timestamp": timestamp,
        "transactions": [{
            "id": transaction.transaction_id, // Include transaction ID
            "sender": transaction.sender.to_hex_literal(),
            "receiver": transaction.receiver.to_hex_literal(),
            "amount": transaction.amount,
            "gas_fee": transaction.gas_fee, // Fix: Use transaction.gas_fee instead of crate::utils::GAS_FEE_AMOUNT
            "timestamp": transaction.timestamp
        }]
    })
    .to_string()
    .into_bytes();

    // Create emergency block to include the transaction
    let emergency_block = Block::new(
        last_block.index + 1,
        block_data,
        last_block.hash.clone(),
        0,
        transactions,
        "system".to_string(), // Use system as the minter for forced blocks
        Blake3Algorithm::new(),
    );

    // Add block to chain
    BLOCKCHAIN_DATA.add_block(emergency_block);

    // Save the blockchain immediately
    match save_blockchain() {
        Ok(_) => {
            info!("Emergency transaction block created and saved");
            true
        }
        Err(e) => {
            error!("Failed to save emergency transaction block: {}", e);
            false
        }
    }
}

pub fn run_blockchain(running: Arc<Mutex<bool>>, address: String, tx: mpsc::Sender<String>) {
    // System password for automated transactions - could be set via config
    let _system_password = "kanari_system";

    let coin = KARI::default();

    info!("Initializing blockchain with {} coin", coin.name);
    info!(
        "Total supply: {} {} ({} {})",
        TOTAL_SUPPLY_KARI,
        coin.symbol,
        TOTAL_SUPPLY_KA,
        format!("{}A", coin.symbol)
    );

    // Parse address
    let node_address = match parse_address(&address) {
        Ok(addr) => addr,
        Err(e) => {
            error!("Invalid node address: {}", e);
            // Send error notification
            let error_json = json!({
                "event": "blockchain_error",
                "error": format!("Invalid node address: {}", e),
                "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
            }).to_string();
            let _ = tx.try_send(error_json);
            return;
        }
    };

    // Get string representation of address for display
    let normalized_address = node_address.to_hex_literal();
    debug!("Using normalized address: {}", normalized_address);

    // Initialize staking system
    if let Err(e) = load_staking_state() {
        warn!("Failed to load staking state: {}", e);
    } else {
        info!("Staking system initialized");
    }

    // Initialize node networking if multiple nodes are supported
    let node_config = NodeConfig {
        node_id: format!("node-{}", normalized_address[..8].to_string()),
        blockchain_address: normalized_address.clone(),
        listen_ip: "0.0.0.0".to_string(), // Listen on all interfaces, not just localhost
        listen_port: 51303,               // Use fixed default port instead of dynamic calculation
        discovery_nodes: vec![
            // List of discovery nodes for peer discovery
            "devnet.kanari.site:51303".to_string(),
            "testnet.kanari.site:51303".to_string(),
            "mainnet.kanari.site:51303".to_string(),
        ],
        max_peers: 50, // Increased max peers for better network connectivity
        is_validator: is_validator(&node_address), // Dynamically check if this node is a validator
        use_tls: false, // TLS disabled by default
        cert_path: Some(format!(
            "{}/certs/node.crt",
            common::get_kari_dir().display()
        )),
        key_path: Some(format!(
            "{}/certs/node.key",
            common::get_kari_dir().display()
        )),
    };

    // Log node network configuration
    info!(
        "Node network configuration: {}:{} (validator: {})",
        node_config.listen_ip, node_config.listen_port, node_config.is_validator
    );

    // Start node networking
    if let Err(e) = start_node(node_config, tx.clone()) {
        warn!("Failed to start node networking: {}", e);
    } else {
        info!("Node networking started");
    }

    // Send initial status including staking information
    let init_status = json!({
        "event": "blockchain_initializing",
        "coin": {
            "name": coin.name,
            "symbol": coin.symbol,
            "decimals": coin.decimals,
            "total_supply": coin.total_supply,
            "display_supply": TOTAL_SUPPLY_KARI
        },
        "node_address": normalized_address,
        "staking": {
            "validator_minimum": VALIDATOR_STAKING_MINIMUM_KARI,
            "node_minimum": NODE_STAKING_MINIMUM_KARI,
        },
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    })
    .to_string();
    let _ = tx.try_send(init_status);

    // Check if blockchain is already initialized
    if !BLOCKCHAIN_DATA.is_empty() {
        info!(
            "Blockchain already initialized with {} blocks",
            BLOCKCHAIN_DATA.len()
        );

        // Enhanced blockchain status
        let blocks = BLOCKCHAIN_DATA.iter();
        let last_block = blocks.last().unwrap();

        let status_json = json!({
            "event": "blockchain_loaded",
            "blocks": BLOCKCHAIN_DATA.len(),
            "last_block": {
                "index": last_block.index,
                "hash": last_block.hash,
                "timestamp": last_block.timestamp
            },
            "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
        })
        .to_string();

        let _ = tx.try_send(status_json);

        // Check balance using both original and normalized address for troubleshooting
        match mona_blockchain::blockchain::get_balance(&normalized_address) {
            Ok(balance) => {
                let balance_in_kari = balance as f64 / KA_PER_KARI as f64;
                info!(
                    "Address {} has {:.9} {} ({} {}A)",
                    normalized_address, balance_in_kari, coin.symbol, balance, coin.symbol
                );
            }
            Err(e) => warn!(
                "Failed to get balance for address {}: {}",
                normalized_address, e
            ),
        }

        // Debug: Also check with original address if they're different
        if normalized_address != address {
            match mona_blockchain::blockchain::get_balance(&address) {
                Ok(balance) => {
                    debug!(
                        "Original address {} has {} {}A",
                        address, balance, coin.symbol
                    );
                }
                Err(e) => debug!(
                    "Failed to get balance for original address {}: {}",
                    address, e
                ),
            }
        }
    } else {
        // Create genesis block with enhanced coin info as JSON
        let genesis_block = create_genesis_block(&node_address, &coin);
        BLOCKCHAIN_DATA.add_block(genesis_block.clone());

        // Enhanced genesis block info
        let genesis_json = json!({
            "event": "genesis_created",
            "block": {
                "index": genesis_block.index,
                "hash": genesis_block.hash,
                "timestamp": genesis_block.timestamp,
                "datetime": chrono::DateTime::<chrono::Utc>::from_timestamp(genesis_block.timestamp as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "Unknown time".to_string())
            },
            "coin": {
                "name": coin.name,
                "symbol": coin.symbol,
                "decimals": coin.decimals
            },
            "minter": normalized_address,
            "total_supply": {
                "amount": TOTAL_SUPPLY_KA,
                "display": TOTAL_SUPPLY_KARI,
                "symbol": coin.symbol
            }
        }).to_string();

        let _ = tx.try_send(genesis_json);

        // Update balances with normalized address
        {
            let mut balances = BALANCES.lock().unwrap();
            balances.insert(normalized_address.clone(), coin.total_supply);

            // Update pool balance if exists
            if let Ok(pool_addr) = normalize_address(POOL_ADDRESS) {
                let pool_addr_str = pool_addr.to_hex_literal();
                balances.insert(pool_addr_str.clone(), POOL_RESERVED_KA);
                // Decrease genesis address balance by the pool amount
                if let Some(balance) = balances.get_mut(&normalized_address) {
                    *balance -= POOL_RESERVED_KA;
                }
                info!(
                    "Reserved {} KARI for pool address: {}",
                    POOL_RESERVED_KARI, pool_addr_str
                );
            }

            // Debug: Output all balances
            debug!("Initial balances after genesis:");
            for (addr, bal) in balances.iter() {
                debug!("  {} => {}", addr, bal);
            }
        }

        match save_blockchain() {
            Ok(_) => info!("Genesis block created successfully"),
            Err(e) => error!("Failed to save blockchain: {}", e),
        }

        info!(
            "Total supply of {} {} ({} {}A) minted to: {}",
            TOTAL_SUPPLY_KARI, coin.symbol, TOTAL_SUPPLY_KA, coin.symbol, normalized_address
        );
    }

    // Start block production loop
    info!(
        "Starting block production with node address: {}",
        normalized_address
    );

    // Block production loop
    loop {
        if !*running.lock().unwrap() {
            info!("Blockchain simulation stopped");

            // Stop node networking
            if let Err(e) = stop_node() {
                warn!("Error stopping node: {}", e);
            }

            // Send shutdown notification
            let shutdown_json = json!({
                "event": "blockchain_stopped",
                "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
            }).to_string();
            let _ = tx.try_send(shutdown_json);
            break;
        }

        // Get the previous block
        let blocks = BLOCKCHAIN_DATA.iter();
        let prev_block = match blocks.last() {
            Some(block) => block,
            None => {
                error!("Cannot find previous block");
                break;
            }
        };

        // Get pending transactions for this block
        let transactions = {
            match PENDING_TRANSACTIONS.write() {
                Ok(mut queue) => {
                    let mut block_txs = Vec::new();

                    info!(
                        "Processing transaction queue with {} pending transactions",
                        queue.len()
                    );

                    while let Some(tx_item) = queue.pop_front() {
                        // Check if this is a VM transaction and execute it
                        if tx_item.is_vm_transaction() {
                            if let Some(vm_tx) = convert_to_vm_transaction(&tx_item) {
                                match execute_vm_transaction(&vm_tx) {
                                    Ok(result) => {
                                        info!(
                                            "VM transaction executed: {} -> result: {}",
                                            vm_tx.tx_id, result["status"]
                                        );

                                        // Send VM execution result
                                        let vm_result_json = json!({
                                            "event": "vm_transaction_executed",
                                            "transaction_id": vm_tx.tx_id,
                                            "module_id": vm_tx.module_id,
                                            "function": vm_tx.function,
                                            "result": result,
                                            "block_index": prev_block.index + 1
                                        })
                                        .to_string();

                                        let _ = tx.try_send(vm_result_json);
                                    }
                                    Err(e) => {
                                        warn!("VM transaction failed: {} - {}", vm_tx.tx_id, e);

                                        let vm_error_json = json!({
                                            "event": "vm_transaction_failed",
                                            "transaction_id": vm_tx.tx_id,
                                            "error": e,
                                            "block_index": prev_block.index + 1
                                        })
                                        .to_string();

                                        let _ = tx.try_send(vm_error_json);
                                    }
                                }
                            }
                        }

                        info!(
                            "Including transaction: {} -> {}, amount: {}, type: {}",
                            tx_item.sender,
                            tx_item.receiver,
                            tx_item.amount,
                            tx_item.get_transaction_type()
                        );
                        block_txs.push(tx_item);

                        if block_txs.len() >= 100000 {
                            break;
                        }
                    }

                    if !block_txs.is_empty() {
                        info!("Added {} transactions to current block", block_txs.len());
                    }

                    update_pending_transaction_count(queue.len());
                    block_txs
                }
                Err(_) => {
                    error!("Failed to lock pending transactions queue");
                    Vec::new()
                }
            }
        };

        // Create JSON representation of transactions for the block data
        let tx_json: Vec<Value> = transactions
            .iter()
            .map(|tx| {
                json!({
                    "id": tx.transaction_id, // Include transaction ID
                    "sender": tx.sender,
                    "receiver": tx.receiver,
                    "amount": tx.amount,
                    "timestamp": tx.timestamp
                })
            })
            .collect();

        // Create new block data with transactions included
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let block_data = json!({
            "block_type": "transaction",
            "index": prev_block.index + 1,
            "coin": coin.symbol,
            "timestamp": current_time,
            "miner": normalized_address,
            "transactions": tx_json,
            "metadata": {
                "network": "testnet",
                "client_version": env!("CARGO_PKG_VERSION"),
                "previous_block_hash": prev_block.hash
            }
        })
        .to_string()
        .into_bytes();

        // Create new block with transactions
        let new_block = Block::new(
            prev_block.index + 1,
            block_data,
            prev_block.hash.clone(),
            0,                    // No new tokens in regular blocks
            transactions.clone(), // Include transactions in the block - explicitly clone
            normalized_address.clone(),
            Blake3Algorithm::new(),
        );

        // Add block to chain and ensure we save the state
        BLOCKCHAIN_DATA.add_block(new_block.clone());

        // Propagate block to connected peers
        if get_peer_count() > 0 {
            if let Err(e) = propagate_block(&new_block) {
                warn!("Failed to propagate block to peers: {}", e);
            } else {
                info!(
                    "Block {} propagated to {} peers",
                    new_block.index,
                    get_peer_count()
                );
            }
        }

        // Process staking rewards
        let staking_rewards = match process_rewards(new_block.index) {
            Ok(rewards) => rewards,
            Err(e) => {
                warn!("Failed to process staking rewards: {}", e);
                0
            }
        };

        // Check if the node operator is staking as validator
        let validator_status = is_validator(&node_address);

        // If we included transactions, provide detailed logs
        if !transactions.is_empty() {
            info!(
                "Block {} includes {} transactions:",
                new_block.index,
                transactions.len()
            );
            for (i, tx) in transactions.iter().enumerate() {
                info!(
                    "  {}: {} -> {} ({})",
                    i + 1,
                    tx.sender,
                    tx.receiver,
                    tx.amount
                );
            }
        }

        // Convert timestamp to human-readable format
        let datetime =
            chrono::DateTime::<chrono::Utc>::from_timestamp(new_block.timestamp as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "Unknown time".to_string());

        // Enhanced block status update as JSON with staking info and peer info
        let status_json = json!({
            "event": "block_mined",
            "block": {
                "index": new_block.index,
                "hash": new_block.hash,
                "prev_hash": new_block.prev_hash,
                "timestamp": new_block.timestamp,
                "datetime": datetime,
                "minter": normalized_address,
                "transactions": new_block.transactions.len(),
            },
            "blockchain": {
                "height": BLOCKCHAIN_DATA.len(),
                "last_update": current_time
            },
            "staking": {
                "rewards_distributed": staking_rewards,
                "is_validator": validator_status,
                "display_rewards": staking_rewards as f64 / KA_PER_KARI as f64,
                "pool_balance": match crate::staking::get_pool_remaining_balance() {
                    Ok(balance) => balance,
                    Err(_) => 0
                },
                "pool_balance_display": match crate::staking::get_pool_remaining_balance() {
                    Ok(balance) => balance as f64 / KA_PER_KARI as f64,
                    Err(_) => 0.0
                },
                "pool_address": POOL_ADDRESS
            },
            "networking": {
                "peer_count": get_peer_count(),
                "node_id": format!("node-{}", normalized_address[..8].to_string()),
            }
        })
        .to_string();

        let _ = tx.try_send(status_json);

        // Save blockchain state - make sure it happens reliably after transaction block
        match save_blockchain() {
            Ok(_) => {
                info!(
                    "Block {} created successfully. Hash: {}...",
                    new_block.index,
                    &new_block.hash[..16]
                );
            }
            Err(e) => {
                error!("Failed to save blockchain: {}", e);
                // Notify clients of save error
                let error_json = json!({
                    "event": "blockchain_error",
                    "error": format!("Failed to save blockchain: {}", e),
                    "block_index": new_block.index,
                    "timestamp": current_time
                })
                .to_string();
                let _ = tx.try_send(error_json);
            }
        }

        // Enhanced balance checking
        if new_block.index % 5 == 0 {
            match mona_blockchain::blockchain::get_balance(&normalized_address) {
                Ok(balance) => {
                    // Enhanced balance update with more details
                    let balance_json = json!({
                        "event": "balance_update",
                        "address": normalized_address,
                        "balance": {
                            "amount": balance,
                            "display": balance as f64 / KA_PER_KARI as f64,
                            "symbol": coin.symbol,
                            "formatted": format!("{:.9} {}", balance as f64 / KA_PER_KARI as f64, coin.symbol)
                        },
                        "block_height": new_block.index,
                        "timestamp": current_time
                    }).to_string();

                    let _ = tx.try_send(balance_json);

                    // Log balance info
                    debug!(
                        "Current balance for {} is {} {}A",
                        normalized_address, balance, coin.symbol
                    );
                }
                Err(e) => {
                    warn!("Failed to get balance: {}", e);
                    // Notify of balance error
                    let error_json = json!({
                        "event": "balance_error",
                        "error": format!("Failed to get balance: {}", e),
                        "address": normalized_address,
                        "timestamp": current_time
                    })
                    .to_string();
                    let _ = tx.try_send(error_json);
                }
            }

            // Debug: Enhanced log of all balances
            if let Ok(balances) = BALANCES.lock() {
                debug!("Current balances in system ({} accounts):", balances.len());
                for (addr, bal) in balances.iter() {
                    debug!("  {} => {}", addr, bal);
                }

                // Send system-wide balance report
                let balance_report = json!({
                    "event": "system_balances",
                    "account_count": balances.len(),
                    "timestamp": current_time
                })
                .to_string();
                let _ = tx.try_send(balance_report);
            }
        }

        // Update last block time for gas fee calculation
        update_last_block_time(new_block.timestamp);

        // Sleep to control block creation rate
        thread::sleep(Duration::from_millis(420)); // 420 milliseconds for better performance
    }
}
