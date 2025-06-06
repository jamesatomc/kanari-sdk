use consensus_pos::Blake3Algorithm;
use log::{error, info};
use mona_types::address::Address;
use mona_types::storage::block::Block;
use mona_types::Transaction;


use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde_json::json;


use  mona_types::storage::normalize_address;

use mona_types::kari::{KARI, POOL_ADDRESS, POOL_RESERVED_KA, POOL_RESERVED_KARI, TOTAL_SUPPLY_KARI};


/// Create a genesis block containing the total supply of Kari tokens
pub(crate) fn create_genesis_block(address: &Address, coin: &KARI) -> Block<Blake3Algorithm> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs();

    // Initialize pool transaction
    let pool_address = match normalize_address(POOL_ADDRESS) {
        Ok(addr) => addr,
        Err(e) => {
            error!("Invalid pool address: {}, using genesis address instead", e);
            address.clone()
        },
    };
    
    let pool_transaction = Transaction {
        transaction_id: format!("genesis_pool_reserve_{}", timestamp),
        sender: address.clone(),
        receiver: pool_address,
        amount: POOL_RESERVED_KA,
        timestamp,
        gas_fee: 0, // No gas fee for genesis transaction
        signature: Vec::new(), // No signature needed for genesis transaction
        data: None, // Add missing data field
    };
    
    // Create transactions list with pool transaction
    let mut transactions = Vec::new();
    transactions.push(pool_transaction);

    // Enhanced genesis data with pool allocation information
    let genesis_data = json!({
        "block_type": "genesis",
        "coin": {
            "name": coin.name,
            "symbol": coin.symbol,
            "decimals": coin.decimals,
            "total_supply": {
                "amount": coin.total_supply,
                "display": TOTAL_SUPPLY_KARI,
                "symbol": coin.symbol
            },
            "block_reward": coin.block_reward,
            "max_supply": coin.max_supply,
            "pool_allocation": {
                "address": POOL_ADDRESS,
                "amount": POOL_RESERVED_KA,
                "display_amount": POOL_RESERVED_KARI
            }
        },
        "network": {
            "name": "Kanari Testnet",
            "version": env!("CARGO_PKG_VERSION"),
            "timestamp": timestamp,
            "datetime": chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp as i64, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "Unknown time".to_string())
        },
        "genesis_address": address.to_hex_literal()
    }).to_string().into_bytes();

    info!("Creating genesis block for {} with {} total supply", coin.name, coin.total_supply);
    info!("Reserving {} {} ({} {}A) for pool address: {}", 
        POOL_RESERVED_KARI, coin.symbol, POOL_RESERVED_KA, coin.symbol, POOL_ADDRESS);
    
    Block::new(
        0,
        genesis_data,
        "0".repeat(64),
        coin.total_supply,
        transactions,  // Include the pool allocation transaction
        address.to_hex_literal(),
        Blake3Algorithm::new(),
    )
}