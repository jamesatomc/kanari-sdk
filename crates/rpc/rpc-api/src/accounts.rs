// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;
use jsonrpc_core::{Params, Result as JsonRpcResult, Error as RpcError, ErrorCode};

use mona_crypto::{list_wallet_files, load_wallet};
use mona_types::address::Address;
use mona_blockchain::{blockchain::{load_blockchain_with_retry, BALANCES, BLOCKCHAIN_DATA}, chain_id::CHAIN_ID};
use panorama::simulation::process_transfer;
use panorama::utils::format_kari_amount;
use serde_json::{json, Value as JsonValue};



use serde::Deserialize;

use tokio::sync::mpsc;




// Blockchain API structures
#[derive(Deserialize)]
pub struct TransferParams {
    pub from: String,
    pub to: String,
    pub amount: f64,
    pub password: String,
    pub priority_boost: Option<u64>, // Optional priority boost for gas fee
}


// Blockchain API methods
pub fn get_blockchain_status(_params: Params) -> JsonRpcResult<JsonValue> {
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    let block_count = BLOCKCHAIN_DATA.len();
    
    // Get the latest block info
    let latest_block = if block_count > 0 {
        let block = BLOCKCHAIN_DATA.get_block(block_count - 1);
        block.map(|b| json!({
            "index": b.index,
            "hash": b.hash,
            "timestamp": b.timestamp,
            "transactions": b.transactions.len(),
            "miner": b.address,
        }))
    } else {
        None
    };
    
    // Get genesis timestamp if available
    let genesis_timestamp = if block_count > 0 {
        BLOCKCHAIN_DATA.get_block(0).map(|b| b.timestamp)
    } else {
        None
    };
    
    // Count transactions across all blocks
    let total_transactions = BLOCKCHAIN_DATA.iter()
        .into_iter()
        .fold(0, |acc, block| acc + block.transactions.len());
    
    let response = json!({
        "chain_id": CHAIN_ID.to_string(),
        "block_height": block_count,
        "block_count": block_count,
        "latest_block": latest_block,
        "total_transactions": total_transactions,
        "genesis_timestamp": genesis_timestamp,
    });
    
    Ok(response)
}

pub fn list_accounts(_params: Params) -> JsonRpcResult<JsonValue> {
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    let balances = match BALANCES.lock() {
        Ok(balances) => balances,
        Err(_) => return Err(RpcError {
            code: ErrorCode::InternalError,
            message: "Failed to access balances".to_string(),
            data: None,
        }),
    };
    
    let mut accounts = Vec::new();
    for (address_str, balance) in balances.iter() {
        // Parse the address string into Address type
        let address = match Address::from_str(address_str) {
            Ok(addr) => addr,
            Err(_) => continue, // Skip invalid addresses
        };

        // Count transactions for this account
        let tx_count = BLOCKCHAIN_DATA.iter()
            .into_iter()
            .fold(0, |acc, block| {
                // Count matching transactions in each block and accumulate
                acc + block.transactions.iter()
                    .filter(|tx| tx.sender == address || tx.receiver == address)
                    .count()
            });
            
        accounts.push(json!({
            "address": address_str,
            "balance": balance,
            "balance_formatted": format_kari_amount(*balance),
            "transaction_count": tx_count,
            "is_contract": false,
        }));
    }
    
    Ok(json!({
        "accounts": accounts,
        "total": accounts.len(),
    }))
}

pub fn transfer_tokens(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse transfer params
    let transfer_params: TransferParams = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid parameters: {}", e)))?;
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // Validate sender's address and password by loading the wallet
    match load_wallet(&transfer_params.from, &transfer_params.password) {
        Ok(_) => {
            // Calculate amount in KA units (1 KARI = 10^9 KA)
            const KA_PER_KARI: u64 = 1_000_000_000;
            let amount_ka = (transfer_params.amount * KA_PER_KARI as f64) as u64;
            
            // Create a channel for transaction notifications
            let (tx, _rx) = mpsc::channel::<String>(10);
            
            // Process transfer with password for signing and priority boost
            match process_transfer(
                &transfer_params.from, 
                &transfer_params.to, 
                amount_ka, 
                &transfer_params.password,
                transfer_params.priority_boost,
                &tx
            ) {
                Ok(transaction) => {
                    // Check if the transaction was signed correctly
                    let signature_status = if transaction.signature.is_empty() {
                        "unsigned"
                    } else {
                        match panorama::transfer_tokens::verify_transaction::verify_transaction(&transaction) {
                            Ok(true) => "valid",
                            Ok(false) => "invalid",
                            Err(_) => "unknown"
                        }
                    };
                    
                    // Include gas fee information in the response
                    Ok(json!({
                        "transaction_id": transaction.transaction_id,
                        "sender": transaction.sender,
                        "receiver": transaction.receiver,
                        "amount": transaction.amount,
                        "amount_formatted": format_kari_amount(transaction.amount),
                        "gas_fee": transaction.gas_fee,
                        "gas_fee_formatted": format_kari_amount(transaction.gas_fee),
                        "gas_collector": panorama::utils::GAS_FEE_COLLECTOR,
                        "total_cost": panorama::utils::calculate_total_transaction_cost(transaction.amount, transaction.gas_fee),
                        "total_cost_formatted": format_kari_amount(panorama::utils::calculate_total_transaction_cost(transaction.amount, transaction.gas_fee)),
                        "timestamp": transaction.timestamp,
                        "status": "pending", // Status is pending until included in a block
                        "signed": !transaction.signature.is_empty(),
                        "signature_status": signature_status  // Add signature verification status
                    }))
                },
                Err(e) => {
                    Err(RpcError {
                        code: ErrorCode::InternalError,
                        message: format!("Transfer failed: {}", e),
                        data: None,
                    })
                }
            }
        },
        Err(_) => {
            Err(RpcError {
                code: ErrorCode::InvalidParams,
                message: "Invalid wallet password".to_string(),
                data: None,
            })
        }
    }
}

pub fn get_wallets(_params: Params) -> JsonRpcResult<JsonValue> {
    match list_wallet_files() {
        Ok(wallets) => {
            let wallet_list = wallets.into_iter()
                .map(|(name, is_selected)| {
                    let address = name.trim_end_matches(".enc");
                    json!({
                        "address": address,
                        "selected": is_selected
                    })
                })
                .collect::<Vec<_>>();
                
            Ok(json!({
                "wallets": wallet_list,
                "count": wallet_list.len()
            }))
        },
        Err(e) => {
            Err(RpcError {
                code: ErrorCode::InternalError,
                message: format!("Failed to list wallets: {}", e),
                data: None,
            })
        }
    }
}

