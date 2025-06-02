// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use chrono;
use jsonrpc_core::{Error as RpcError, ErrorCode, Params, Result as JsonRpcResult};
use mona_types::address::Address;
use mona_blockchain::{blockchain::{BLOCKCHAIN_DATA, get_balance, load_blockchain_with_retry}, chain_id::CHAIN_ID};
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use std::str::FromStr;

use crate::format_kari_amount;

// Search transaction parameters
#[derive(Deserialize)]
pub struct SearchTransactionsParams {
    pub address: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

// Account API structures
#[derive(Deserialize)]
pub struct AccountParams {
    pub address: String,
}

// New API method to show information about all blocks
pub fn get_all_blocks(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse optional limit parameter
    let limit: Option<usize> = match params.parse() {
        Ok(limit) => Some(limit),
        Err(_) => None, // If parsing fails, no limit will be applied
    };
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    let blocks = BLOCKCHAIN_DATA.iter();
    let block_count = blocks.len();
    
    // Apply the limit if provided
    let blocks_to_show = if let Some(limit) = limit {
        if limit < block_count {
            // Take the most recent blocks if a limit is specified
            blocks.into_iter().skip(block_count - limit).collect::<Vec<_>>()
        } else {
            blocks
        }
    } else {
        blocks
    };
    
    // Convert blocks to JSON format
    let blocks_json: Vec<JsonValue> = blocks_to_show
        .into_iter()
        .map(|block| {
            // Format transactions
            let transactions_json: Vec<JsonValue> = block.transactions
                .iter()
                .map(|tx| {
                    json!({
                        "id": tx.transaction_id,
                        "sender": tx.sender,
                        "receiver": tx.receiver,
                        "amount": tx.amount,
                        "amount_formatted": format_kari_amount(tx.amount),
                        "timestamp": tx.timestamp
                    })
                })
                .collect();
                
            // Format the block
            json!({
                "index": block.index,
                "hash": block.hash,
                "prev_hash": block.prev_hash,
                "timestamp": block.timestamp,
                "datetime": chrono::DateTime::<chrono::Utc>::from_timestamp(block.timestamp as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "Unknown time".to_string()),
                "miner": block.address,
                "transactions": transactions_json,
                "transaction_count": block.transactions.len(),
                "tokens_minted": block.tokens
            })
        })
        .collect();
    
    // Create the response JSON
    let response = json!({
        "chain_id": CHAIN_ID.to_string(),
        "block_count": block_count,
        "blocks_returned": blocks_json.len(),
        "blocks": blocks_json
    });
    
    Ok(response)
}

pub fn get_account_details(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse account address
    let account_params: AccountParams = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid parameters: {}", e)))?;
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // Parse the address string into Address type
    let address = match Address::from_str(&account_params.address) {
        Ok(addr) => addr,
        Err(_) => return Err(RpcError::invalid_params("Invalid address format")),
    };

    // Get account balance
    let balance = match get_balance(&account_params.address) {
        Ok(balance) => balance,
        Err(_) => return Err(RpcError::invalid_params("Account not found")),
    };
    
    // Find all transactions involving this account
    let transactions = BLOCKCHAIN_DATA.iter()
        .into_iter()
        .flat_map(|block| {
            block.transactions.iter()
                .filter(|tx| tx.sender == address || tx.receiver == address)
                .map(|tx| {
                    // Determine if this is incoming or outgoing for this address
                    let tx_type = if tx.receiver == address { "incoming" } else { "outgoing" };
                    
                    json!({
                        "id": tx.transaction_id,
                        "type": tx_type,
                        "sender": tx.sender.to_string(),
                        "receiver": tx.receiver.to_string(),
                        "amount": tx.amount,
                        "amount_formatted": format_kari_amount(tx.amount),
                        "timestamp": tx.timestamp,
                        "datetime": chrono::DateTime::<chrono::Utc>::from_timestamp(tx.timestamp as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| "Unknown time".to_string()),
                        "block_index": block.index,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    
    // Determine if this is a contract address (simplified check, can be expanded)
    let is_contract = false; // Add contract detection logic if available
    let account_type = if is_contract { "contract" } else { "wallet" };
    
    // Create response
    Ok(json!({
        "address": account_params.address,
        "balance": balance,
        "balance_formatted": format_kari_amount(balance),
        "account_type": account_type,
        "is_contract": is_contract,
        "transaction_count": transactions.len(),
        "transactions": transactions
    }))
}

pub fn search_transactions(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse search parameters
    let search_params: SearchTransactionsParams = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid parameters: {}", e)))?;
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // Parse the address string into Address type
    let search_address = match Address::from_str(&search_params.address) {
        Ok(addr) => addr,
        Err(_) => return Err(RpcError::invalid_params("Invalid address format")),
    };

    // Find all transactions involving this address (either as sender or receiver)
    let mut transactions = BLOCKCHAIN_DATA.iter()
        .into_iter()
        .flat_map(|block| {
            block.transactions.iter()
                .filter(|tx| tx.sender == search_address || tx.receiver == search_address)
                .map(|tx| {
                    // Determine if this is incoming or outgoing for the search address
                    let tx_type = if tx.receiver == search_address { "incoming" } else { "outgoing" };
                    
                    json!({
                        "id": tx.transaction_id,
                        "type": tx_type,
                        "sender": tx.sender.to_string(),
                        "receiver": tx.receiver.to_string(),
                        "amount": tx.amount,
                        "amount_formatted": format_kari_amount(tx.amount),
                        "timestamp": tx.timestamp,
                        "datetime": chrono::DateTime::<chrono::Utc>::from_timestamp(tx.timestamp as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| "Unknown time".to_string()),
                        "block_index": block.index,
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    
    // Sort transactions by timestamp, most recent first
    transactions.sort_by(|a, b| {
        let a_time = a["timestamp"].as_u64().unwrap_or(0);
        let b_time = b["timestamp"].as_u64().unwrap_or(0);
        b_time.cmp(&a_time)
    });
    
    // Apply pagination if provided
    let total_count = transactions.len();
    let offset = search_params.offset.unwrap_or(0);
    
    // Apply offset and limit
    let transactions = if offset < transactions.len() {
        let paginated = &transactions[offset..];
        if let Some(limit) = search_params.limit {
            if limit < paginated.len() {
                paginated[..limit].to_vec()
            } else {
                paginated.to_vec()
            }
        } else {
            paginated.to_vec()
        }
    } else {
        vec![]
    };
    
    // Create response
    Ok(json!({
        "address": search_params.address,
        "total_transactions": total_count,
        "returned_transactions": transactions.len(),
        "offset": offset,
        "transactions": transactions
    }))
}

// New API method to search for a transaction by its ID
pub fn get_transaction_by_id(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse transaction ID parameter
    let tx_id: String = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid transaction ID: {}", e)))?;
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // Search for the transaction in all blocks
    for block in BLOCKCHAIN_DATA.iter() {
        for tx in &block.transactions {
            if tx.transaction_id == tx_id {
                // Found the transaction
                let datetime = chrono::DateTime::<chrono::Utc>::from_timestamp(tx.timestamp as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "Unknown time".to_string());
                
                // Get sender and receiver balances
                let sender_balance = match get_balance(&tx.sender.to_hex_literal()) {
                    Ok(balance) => balance,
                    Err(_) => 0,
                };
                
                let receiver_balance = match get_balance(&tx.receiver.to_hex_literal()) {
                    Ok(balance) => balance,
                    Err(_) => 0,
                };
                
                // Add gas fee information
                return Ok(json!({
                    "transaction": {
                        "id": tx.transaction_id,
                        "sender": tx.sender.to_hex_literal(),
                        "receiver": tx.receiver.to_hex_literal(),
                        "amount": tx.amount,
                        "amount_formatted": format_kari_amount(tx.amount),
                        "gas_fee": tx.gas_fee,
                        "gas_fee_formatted": format_kari_amount(tx.gas_fee),
                        "gas_collector": panorama::utils::GAS_FEE_COLLECTOR,
                        "total_cost": panorama::utils::calculate_total_transaction_cost(tx.amount, tx.gas_fee),
                        "total_cost_formatted": format_kari_amount(panorama::utils::calculate_total_transaction_cost(tx.amount, tx.gas_fee)),
                        "timestamp": tx.timestamp,
                        "datetime": datetime,
                        "signature": tx.signature
                    },
                    "block": {
                        "index": block.index,
                        "hash": block.hash,
                        "timestamp": block.timestamp
                    },
                    "balances": {
                        "sender": {
                            "address": tx.sender.to_hex_literal(),
                            "balance": sender_balance,
                            "formatted": format_kari_amount(sender_balance)
                        },
                        "receiver": {
                            "address": tx.receiver.to_hex_literal(),
                            "balance": receiver_balance,
                            "formatted": format_kari_amount(receiver_balance)
                        }
                    }
                }));
            }
        }
    }
    
    // Transaction not found
    Err(RpcError {
        code: ErrorCode::InvalidParams,
        message: format!("Transaction with ID {} not found", tx_id),
        data: None,
    })
}

// New API method to get transaction status
pub fn get_transaction_status(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse transaction ID parameter
    let tx_id: String = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid transaction ID: {}", e)))?;
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // First check if the transaction is in a block (confirmed)
    for block in BLOCKCHAIN_DATA.iter() {
        for tx in &block.transactions {
            if tx.transaction_id == tx_id {
                return Ok(json!({
                    "transaction_id": tx_id,
                    "status": "confirmed",
                    "block_index": block.index,
                    "block_hash": block.hash,
                    "timestamp": tx.timestamp,
                    "confirmation_time": block.timestamp
                }));
            }
        }
    }
    
    // If not found in blocks, it might be pending
    // In a more advanced implementation, we would check the pending transaction queue
    
    // Not found at all
    Ok(json!({
        "transaction_id": tx_id,
        "status": "unknown",
        "message": "Transaction not found in blockchain"
    }))
}

// Update get_gas_fee_info to provide dynamic gas fee information
pub fn get_gas_fee_info(_params: Params) -> JsonRpcResult<JsonValue> {
    
    // Get current network stats
    let network_stats = panorama::utils::get_network_stats();
    
    // Calculate sample gas fees for different priority levels
    let base_fee = panorama::utils::calculate_gas_fee(None);
    let medium_fee = panorama::utils::calculate_gas_fee(Some(5));
    let high_fee = panorama::utils::calculate_gas_fee(Some(10));
    
    Ok(json!({
        "gas_fee": {
            "current": base_fee,
            "current_formatted": format_kari_amount(base_fee),
            "medium_priority": medium_fee,
            "medium_priority_formatted": format_kari_amount(medium_fee),
            "high_priority": high_fee,
            "high_priority_formatted": format_kari_amount(high_fee),
            "min_fee": panorama::utils::MIN_GAS_FEE,
            "max_fee": panorama::utils::MAX_GAS_FEE
        },
        "gas_collector": {
            "address": panorama::utils::GAS_FEE_COLLECTOR,
            "balance": get_balance(panorama::utils::GAS_FEE_COLLECTOR).unwrap_or(0)
        },
        "network_stats": {
            "pending_transactions": network_stats.pending_transactions,
            "last_block_time": network_stats.last_block_time,
            "congestion_level": if network_stats.pending_transactions < 10 {
                "low"
            } else if network_stats.pending_transactions < 50 {
                "medium"
            } else {
                "high"
            },
        },
        "gas_enabled": true,
        "system_info": {
            "description": "Dynamic gas fee system",
            "unit": "KA"
        }
    }))
}