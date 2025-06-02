// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use std::{str::FromStr, time::{SystemTime, UNIX_EPOCH}};
use jsonrpc_core::{Params, Result as JsonRpcResult, Error as RpcError, ErrorCode};

use mona_blockchain::blockchain::load_blockchain_with_retry;
use mona_crypto::load_wallet;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::format_kari_amount;


// Staking API structures
#[derive(Deserialize)]
pub struct StakeParams {
    pub address: String,
    pub amount: f64,
    pub password: String,
    pub validator: bool,
}

#[derive(Deserialize)]
pub struct UnstakeParams {
    pub address: String,
    pub password: String,
}

// Staking API methods
pub fn stake_tokens(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse stake params
    let stake_params: StakeParams = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid parameters: {}", e)))?;
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // Validate address and password by loading the wallet
    match load_wallet(&stake_params.address, &stake_params.password) {
        Ok(_) => {
            // Calculate amount in KA units (1 KARI = 10^9 KA)
            const KA_PER_KARI: u64 = 1_000_000_000;
            let amount_ka = (stake_params.amount * KA_PER_KARI as f64) as u64;
            
            // Parse the address
            let address = match mona_types::address::Address::from_str(&stake_params.address) {
                Ok(addr) => addr,
                Err(_) => return Err(RpcError::invalid_params("Invalid address format")),
            };
            
            // Stake tokens
            match panorama::staking::stake_tokens(&address, amount_ka, stake_params.validator) {
                Ok(staked_node) => {
                    // Format the response
                    Ok(json!({
                        "address": stake_params.address,
                        "staked_amount": staked_node.staked_amount,
                        "staked_amount_formatted": format_kari_amount(staked_node.staked_amount),
                        "is_validator": staked_node.is_validator,
                        "staked_at": staked_node.staked_at,
                        "unlock_time": staked_node.unlock_time,
                        "unlock_date": chrono::DateTime::<chrono::Utc>::from_timestamp(staked_node.unlock_time as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| "Unknown time".to_string()),
                        "status": "staked",
                        "validator_rewards_rate": mona_types::kari::STAKING_REWARD_PERCENTAGE * 100.0
                    }))
                },
                Err(e) => {
                    Err(RpcError {
                        code: ErrorCode::InternalError,
                        message: format!("Failed to stake tokens: {}", e),
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

pub fn unstake_tokens(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse unstake params
    let unstake_params: UnstakeParams = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid parameters: {}", e)))?;
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // Validate address and password by loading the wallet
    match load_wallet(&unstake_params.address, &unstake_params.password) {
        Ok(_) => {
            // Parse the address
            let address = match mona_types::address::Address::from_str(&unstake_params.address) {
                Ok(addr) => addr,
                Err(_) => return Err(RpcError::invalid_params("Invalid address format")),
            };
            
            // Unstake tokens
            match panorama::staking::unstake_tokens(&address) {
                Ok((withdrawn_amount, rewards)) => {
                    // Format the response
                    Ok(json!({
                        "address": unstake_params.address,
                        "withdrawn_amount": withdrawn_amount,
                        "withdrawn_amount_formatted": format_kari_amount(withdrawn_amount),
                        "rewards": rewards,
                        "rewards_formatted": format_kari_amount(rewards),
                        "total_returned": withdrawn_amount + rewards,
                        "total_returned_formatted": format_kari_amount(withdrawn_amount + rewards),
                        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs(),
                        "status": "unstaked"
                    }))
                },
                Err(e) => {
                    Err(RpcError {
                        code: ErrorCode::InternalError,
                        message: format!("Failed to unstake tokens: {}", e),
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

pub fn get_staking_info(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse address - modify to handle array format properly
    let address_str: String = match params {
        Params::Array(arr) => {
            if arr.is_empty() {
                return Err(RpcError::invalid_params("Address parameter missing"));
            }
            match arr[0].as_str() {
                Some(addr) => addr.to_string(),
                None => return Err(RpcError::invalid_params("Invalid address format")),
            }
        },
        Params::Map(map) => {
            match map.get("address").and_then(|v| v.as_str()) {
                Some(addr) => addr.to_string(),
                None => return Err(RpcError::invalid_params("Address parameter missing or invalid")),
            }
        },
        _ => return Err(RpcError::invalid_params("Expected array or object parameters")),
    };
    
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // Parse the address
    let address = match mona_types::address::Address::from_str(&address_str) {
        Ok(addr) => addr,
        Err(_) => return Err(RpcError::invalid_params("Invalid address format")),
    };
    
    // Get staking info
    match panorama::staking::get_staking_info(&address) {
        Some(node) => {
            // Check if the lock period has passed
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            
            let can_unstake = current_time >= node.unlock_time;
            let early_unstake_penalty = if can_unstake { 0 } else { node.staked_amount / 10 };
            
            // Format the response
            Ok(json!({
                "address": address_str,
                "is_staking": true,
                "staked_amount": node.staked_amount,
                "staked_amount_formatted": format_kari_amount(node.staked_amount),
                "is_validator": node.is_validator,
                "staked_at": node.staked_at,
                "unlock_time": node.unlock_time,
                "unlock_date": chrono::DateTime::<chrono::Utc>::from_timestamp(node.unlock_time as i64, 0)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "Unknown time".to_string()),
                "lock_status": if can_unstake { "unlocked" } else { "locked" },
                "time_remaining": if can_unstake { 
                    0 
                } else { 
                    node.unlock_time - current_time
                },
                "accumulated_rewards": node.accumulated_rewards,
                "accumulated_rewards_formatted": format_kari_amount(node.accumulated_rewards),
                "early_unstake_penalty": early_unstake_penalty,
                "early_unstake_penalty_formatted": format_kari_amount(early_unstake_penalty),
                "estimated_return": if node.is_validator {
                    let daily_reward_rate = mona_types::kari::STAKING_REWARD_PERCENTAGE / 365.0;
                    (node.staked_amount as f64 * daily_reward_rate).round() as u64
                } else {
                    0
                },
                "estimated_return_period": "daily"
            }))
        },
        None => {
            // Not staking
            Ok(json!({
                "address": address_str,
                "is_staking": false,
                "minimum_staking_amount": mona_types::kari::NODE_STAKING_MINIMUM_KA,
                "minimum_staking_formatted": format_kari_amount(mona_types::kari::NODE_STAKING_MINIMUM_KA),
                "minimum_validator_amount": mona_types::kari::VALIDATOR_STAKING_MINIMUM_KA,
                "minimum_validator_formatted": format_kari_amount(mona_types::kari::VALIDATOR_STAKING_MINIMUM_KA)
            }))
        }
    }
}

pub fn get_staking_stats(_params: Params) -> JsonRpcResult<JsonValue> {
    // Load blockchain data if needed
    if let Err(e) = load_blockchain_with_retry() {
        return Err(RpcError {
            code: ErrorCode::InternalError,
            message: format!("Failed to load blockchain: {}", e),
            data: None,
        });
    }
    
    // Get staking pool stats
    let pool = panorama::staking::get_staking_stats();
    
    // Get pool balance
    let pool_balance = match panorama::staking::get_pool_remaining_balance() {
        Ok(balance) => balance,
        Err(_) => 0,
    };
    
    // Format the response
    Ok(json!({
        "total_staked": pool.total_staked,
        "total_staked_formatted": format_kari_amount(pool.total_staked),
        "nodes_count": pool.nodes_count,
        "validators_count": pool.validators_count,
        "rewards_distributed": pool.total_rewards_distributed,
        "rewards_distributed_formatted": format_kari_amount(pool.total_rewards_distributed),
        "pool_address": mona_types::kari::POOL_ADDRESS,
        "pool_balance": pool_balance,
        "pool_balance_formatted": format_kari_amount(pool_balance),
        "annual_reward_rate": mona_types::kari::STAKING_REWARD_PERCENTAGE * 100.0,
        "minimum_node_requirement": {
            "amount": mona_types::kari::NODE_STAKING_MINIMUM_KA,
            "formatted": format_kari_amount(mona_types::kari::NODE_STAKING_MINIMUM_KA)
        },
        "minimum_validator_requirement": {
            "amount": mona_types::kari::VALIDATOR_STAKING_MINIMUM_KA,
            "formatted": format_kari_amount(mona_types::kari::VALIDATOR_STAKING_MINIMUM_KA)
        }
    }))
}
