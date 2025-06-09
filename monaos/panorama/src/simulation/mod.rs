use consensus_pos::Blake3Algorithm;
use log::{error, info, warn, debug};
use tokio::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::collections::VecDeque;
use std::str::FromStr;

use mona_blockchain::block::{Block, Transaction};
use mona_blockchain::blockchain::{save_blockchain, BALANCES, BLOCKCHAIN_DATA, normalize_address};
use crate::transfer_tokens::transfer_tokens;
use mona_types::address::Address;
use mona_types::kari::{KARI, KA_PER_KARI, POOL_ADDRESS, POOL_RESERVED_KA, POOL_RESERVED_KARI, TOTAL_SUPPLY_KA, TOTAL_SUPPLY_KARI, VALIDATOR_STAKING_MINIMUM_KARI, NODE_STAKING_MINIMUM_KARI};
use crate::utils::{update_pending_transaction_count, update_last_block_time, calculate_gas_fee, format_gas_fee_display};
use crate::staking::{load_staking_state, process_rewards, is_validator};
use p2p_protocol::node::{NodeConfig, start_node, stop_node, propagate_block, get_peer_count};

pub mod create_genesis_block;
use create_genesis_block::create_genesis_block;

pub mod move_contract;
pub use move_contract::MoveContractSimulator;


// Function to parse and normalize address
fn parse_address(address: &str) -> Result<Address, String> {
    Address::from_str(address)
        .map_err(|_| format!("Invalid address format: {}", address))
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
        },
        Err(_) => false
    }
}

// Improved function to ensure transactions are committed properly
pub fn process_transfer(
    from_address: &str,
    to_address: &str,
    amount: u64,
    password: &str,
    priority_boost: Option<u64>,  // Add optional priority boost
    tx: &mpsc::Sender<String>
) -> Result<Transaction, String> {
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
    match transfer_tokens(&from.to_hex_literal(), &to.to_hex_literal(), amount, password, gas_fee) {
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
                // Direct string formatting instead of JSON
                let tx_status = format!(
                    "{{\"event\":\"transaction_created\",\"transaction\":{{\"id\":\"{}\",\"sender\":\"{}\",\"receiver\":\"{}\",\"amount\":{},\"gas_fee\":{},\"gas_fee_display\":\"{}\",\"gas_collector\":\"{}\",\"total_cost\":{},\"timestamp\":{},\"signed\":{},\"signature_status\":\"{}\"}},\"status\":\"pending\"}}",
                    transaction.transaction_id,
                    transaction.sender.to_hex_literal(),
                    transaction.receiver.to_hex_literal(),
                    amount,
                    transaction.gas_fee,
                    format_gas_fee_display(transaction.gas_fee),
                    crate::utils::GAS_FEE_COLLECTOR,
                    crate::utils::calculate_total_transaction_cost(amount, transaction.gas_fee),
                    transaction.timestamp,
                    !transaction.signature.is_empty(),
                    signature_status
                );
                
                let _ = tx.try_send(tx_status);
                
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
                    },
                    false => Err("Failed to add transaction to blockchain".to_string())
                }
            }
        },
        Err(e) => {
            // Direct string formatting for error
            let error_msg = format!(
                "{{\"event\":\"transaction_error\",\"error\":\"{}\",\"details\":{{\"sender\":\"{}\",\"receiver\":\"{}\",\"amount\":{},\"gas_fee\":{},\"gas_fee_display\":\"{}\",\"total_cost\":{}}}}}",
                e,
                from_address,
                to_address,
                amount,
                gas_fee,
                gas_fee_display,
                crate::utils::calculate_total_transaction_cost(amount, gas_fee)
            );
            
            let _ = tx.try_send(error_msg);
            
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
        
    // Create block data - use direct string formatting instead of JSON
    let block_data = format!(
        "{{\"block_type\":\"forced_transaction\",\"timestamp\":{},\"transactions\":[{{\"id\":\"{}\",\"sender\":\"{}\",\"receiver\":\"{}\",\"amount\":{},\"gas_fee\":{},\"timestamp\":{}}}]}}",
        timestamp,
        transaction.transaction_id,
        transaction.sender.to_hex_literal(),
        transaction.receiver.to_hex_literal(),
        transaction.amount,
        transaction.gas_fee,
        transaction.timestamp
    ).into_bytes();

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
        },
        Err(e) => {
            error!("Failed to save emergency transaction block: {}", e);
            false
        }
    }
}

pub fn run_blockchain(
    running: Arc<Mutex<bool>>, 
    address: String,
    tx: mpsc::Sender<String>
) {
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
            // Direct string formatting for error
            let error_msg = format!(
                "{{\"event\":\"blockchain_error\",\"error\":\"Invalid node address: {}\",\"timestamp\":{}}}",
                e,
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
            );
            let _ = tx.try_send(error_msg);
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
        listen_port: 51303, // Use fixed default port instead of dynamic calculation
        discovery_nodes: vec![
            // List of discovery nodes for peer discovery
            "devnet.kanari.site:51303".to_string(),
            "testnet.kanari.site:51303".to_string(),
            "mainnet.kanari.site:51303".to_string(),
        ],
        max_peers: 50, // Increased max peers for better network connectivity
        is_validator: is_validator(&node_address), // Dynamically check if this node is a validator
        use_tls: false, // TLS disabled by default
        cert_path: Some(format!("{}/certs/node.crt", common::get_kari_dir().display())),
        key_path: Some(format!("{}/certs/node.key", common::get_kari_dir().display())),
    };
    
    // Log node network configuration
    info!("Node network configuration: {}:{} (validator: {})", 
          node_config.listen_ip, node_config.listen_port, node_config.is_validator);
    
    // Start node networking
    if let Err(e) = start_node(node_config, tx.clone()) {
        warn!("Failed to start node networking: {}", e);
    } else {
        info!("Node networking started");
    }

    // Send initial status - direct formatting
    let init_status = format!(
        "{{\"event\":\"blockchain_initializing\",\"coin\":{{\"name\":\"{}\",\"symbol\":\"{}\",\"decimals\":{},\"total_supply\":{},\"display_supply\":{}}},\"node_address\":\"{}\",\"staking\":{{\"validator_minimum\":{},\"node_minimum\":{}}},\"timestamp\":{}}}",
        coin.name,
        coin.symbol,
        coin.decimals,
        coin.total_supply,
        TOTAL_SUPPLY_KARI,
        normalized_address,
        VALIDATOR_STAKING_MINIMUM_KARI,
        NODE_STAKING_MINIMUM_KARI,
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    );
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
        
        let status_msg = format!(
            "{{\"event\":\"blockchain_loaded\",\"blocks\":{},\"last_block\":{{\"index\":{},\"hash\":\"{}\",\"timestamp\":{}}},\"timestamp\":{}}}",
            BLOCKCHAIN_DATA.len(),
            last_block.index,
            last_block.hash,
            last_block.timestamp,
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
        );
        
        let _ = tx.try_send(status_msg);

        // Check balance using both original and normalized address for troubleshooting
        match mona_blockchain::blockchain::get_balance(&normalized_address) {
            Ok(balance) => {
                let balance_in_kari = balance as f64 / KA_PER_KARI as f64;
                info!(
                    "Address {} has {:.9} {} ({} {}A)",
                    normalized_address, balance_in_kari, coin.symbol, balance, coin.symbol
                );
            }
            Err(e) => warn!("Failed to get balance for address {}: {}", normalized_address, e),
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
                Err(e) => debug!("Failed to get balance for original address {}: {}", address, e),
            }
        }
    } else {
        // Create genesis block with enhanced coin info
        let genesis_block = create_genesis_block(&node_address, &coin);
        BLOCKCHAIN_DATA.add_block(genesis_block.clone());

        // Direct string formatting for genesis info
        let datetime = chrono::DateTime::<chrono::Utc>::from_timestamp(genesis_block.timestamp as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown time".to_string());
            
        let genesis_msg = format!(
            "{{\"event\":\"genesis_created\",\"block\":{{\"index\":{},\"hash\":\"{}\",\"timestamp\":{},\"datetime\":\"{}\"}},\"coin\":{{\"name\":\"{}\",\"symbol\":\"{}\",\"decimals\":{}}},\"minter\":\"{}\",\"total_supply\":{{\"amount\":{},\"display\":{},\"symbol\":\"{}\"}}}}",
            genesis_block.index,
            genesis_block.hash,
            genesis_block.timestamp,
            datetime,
            coin.name,
            coin.symbol,
            coin.decimals,
            normalized_address,
            TOTAL_SUPPLY_KA,
            TOTAL_SUPPLY_KARI,
            coin.symbol
        );
        
        let _ = tx.try_send(genesis_msg);

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
                info!("Reserved {} KARI for pool address: {}", POOL_RESERVED_KARI, pool_addr_str);
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
    info!("Starting block production with node address: {}", normalized_address);
    
    // Block production loop
    loop {
        if !*running.lock().unwrap() {
            info!("Blockchain simulation stopped");
            
            // Stop node networking
            if let Err(e) = stop_node() {
                warn!("Error stopping node: {}", e);
            }
            
            // Direct formatting for shutdown
            let shutdown_msg = format!(
                "{{\"event\":\"blockchain_stopped\",\"timestamp\":{}}}",
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
            );
            let _ = tx.try_send(shutdown_msg);
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
                    // Take up to 100000 transactions for this block
                    let mut block_txs = Vec::new();
                    
                    // Log transaction queue status
                    info!("Processing transaction queue with {} pending transactions", queue.len());
                    
                    while let Some(tx) = queue.pop_front() {
                        info!("Including transaction: {} -> {}, amount: {}", 
                            tx.sender, tx.receiver, tx.amount);
                        block_txs.push(tx);
                        if block_txs.len() >= 100000 {
                            break;
                        }
                    }
                    
                    if !block_txs.is_empty() {
                        info!("Added {} transactions to current block", block_txs.len());
                    }
                    
                    // Update the pending transaction count for gas fee calculation
                    update_pending_transaction_count(queue.len());
                    
                    block_txs
                },
                Err(_) => {
                    error!("Failed to lock pending transactions queue");
                    Vec::new() // Empty vector on error
                }
            }
        };

        // Create transaction list as formatted string instead of JSON array
        let tx_list: Vec<String> = transactions.iter().map(|tx| {
            format!(
                "{{\"id\":\"{}\",\"sender\":\"{}\",\"receiver\":\"{}\",\"amount\":{},\"timestamp\":{}}}",
                tx.transaction_id,
                tx.sender,
                tx.receiver,
                tx.amount,
                tx.timestamp
            )
        }).collect();
        
        // Create new block data with transactions
        let current_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
            
        let block_data = format!(
            "{{\"block_type\":\"transaction\",\"index\":{},\"coin\":\"{}\",\"timestamp\":{},\"miner\":\"{}\",\"transactions\":[{}],\"metadata\":{{\"network\":\"testnet\",\"client_version\":\"{}\",\"previous_block_hash\":\"{}\"}}}}",
            prev_block.index + 1,
            coin.symbol,
            current_time,
            normalized_address,
            tx_list.join(","),
            env!("CARGO_PKG_VERSION"),
            prev_block.hash
        ).into_bytes();

        // Create new block with transactions
        let new_block = Block::new(
            prev_block.index + 1,
            block_data,
            prev_block.hash.clone(),
            0,          // No new tokens in regular blocks
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
                info!("Block {} propagated to {} peers", new_block.index, get_peer_count());
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
            info!("Block {} includes {} transactions:", new_block.index, transactions.len());
            for (i, tx) in transactions.iter().enumerate() {
                info!("  {}: {} -> {} ({})", i+1, tx.sender, tx.receiver, tx.amount);
            }
        }

        // Convert timestamp to human-readable format
        let datetime = chrono::DateTime::<chrono::Utc>::from_timestamp(new_block.timestamp as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Unknown time".to_string());

        // Enhanced block status update - direct formatting
        let status_msg = format!(
            "{{\"event\":\"block_mined\",\"block\":{{\"index\":{},\"hash\":\"{}\",\"prev_hash\":\"{}\",\"timestamp\":{},\"datetime\":\"{}\",\"minter\":\"{}\",\"transactions\":{}}},\"blockchain\":{{\"height\":{},\"last_update\":{}}},\"staking\":{{\"rewards_distributed\":{},\"is_validator\":{},\"display_rewards\":{},\"pool_balance\":{},\"pool_balance_display\":{},\"pool_address\":\"{}\"}},\"networking\":{{\"peer_count\":{},\"node_id\":\"{}\"}}}}",
            new_block.index,
            new_block.hash,
            new_block.prev_hash,
            new_block.timestamp,
            datetime,
            normalized_address,
            new_block.transactions.len(),
            BLOCKCHAIN_DATA.len(),
            current_time,
            staking_rewards,
            validator_status,
            staking_rewards as f64 / KA_PER_KARI as f64,
            match crate::staking::get_pool_remaining_balance() {
                Ok(balance) => balance,
                Err(_) => 0
            },
            match crate::staking::get_pool_remaining_balance() {
                Ok(balance) => balance as f64 / KA_PER_KARI as f64,
                Err(_) => 0.0
            },
            POOL_ADDRESS,
            get_peer_count(),
            format!("node-{}", &normalized_address[..8])
        );
        
        let _ = tx.try_send(status_msg);

        // Save blockchain state - make sure it happens reliably after transaction block
        match save_blockchain() {
            Ok(_) => {
                info!(
                    "Block {} created successfully. Hash: {}...",
                    new_block.index,
                    &new_block.hash[..16]
                );
            },
            Err(e) => {
                error!("Failed to save blockchain: {}", e);
                // Notify clients of save error
                let error_msg = format!(
                    "{{\"event\":\"blockchain_error\",\"error\":\"Failed to save blockchain: {}\",\"block_index\":{},\"timestamp\":{}}}",
                    e,
                    new_block.index,
                    current_time
                );
                let _ = tx.try_send(error_msg);
            },
        }

        // Enhanced balance checking
        if new_block.index % 5 == 0 {
            match mona_blockchain::blockchain::get_balance(&normalized_address) {
                Ok(balance) => {
                    let balance_msg = format!(
                        "{{\"event\":\"balance_update\",\"address\":\"{}\",\"balance\":{{\"amount\":{},\"display\":{},\"symbol\":\"{}\",\"formatted\":\"{:.9} {}\"}},\"block_height\":{},\"timestamp\":{}}}",
                        normalized_address,
                        balance,
                        balance as f64 / KA_PER_KARI as f64,
                        coin.symbol,
                        balance as f64 / KA_PER_KARI as f64,
                        coin.symbol,
                        new_block.index,
                        current_time
                    );
                    
                    let _ = tx.try_send(balance_msg);
                    
                    // Log balance info
                    debug!(
                        "Current balance for {} is {} {}A",
                        normalized_address, balance, coin.symbol
                    );
                },
                Err(e) => {
                    warn!("Failed to get balance: {}", e);
                    let error_msg = format!(
                        "{{\"event\":\"balance_error\",\"error\":\"Failed to get balance: {}\",\"address\":\"{}\",\"timestamp\":{}}}",
                        e,
                        normalized_address,
                        current_time
                    );
                    let _ = tx.try_send(error_msg);
                },
            }
            
            // Debug: Enhanced log of all balances
            if let Ok(balances) = BALANCES.lock() {
                debug!("Current balances in system ({} accounts):", balances.len());
                for (addr, bal) in balances.iter() {
                    debug!("  {} => {}", addr, bal);
                }
                
                let balance_report = format!(
                    "{{\"event\":\"system_balances\",\"account_count\":{},\"timestamp\":{}}}",
                    balances.len(),
                    current_time
                );
                let _ = tx.try_send(balance_report);
            }
        }

        // Update last block time for gas fee calculation
        update_last_block_time(new_block.timestamp);

        // Sleep to control block creation rate
        thread::sleep(Duration::from_millis(420)); // 420 milliseconds for better performance
    }
}