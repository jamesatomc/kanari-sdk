use std::{str::FromStr, time::{SystemTime, UNIX_EPOCH}};
use jsonrpc_core::{Params, Result as JsonRpcResult, Error as RpcError, ErrorCode};

use mona_types::storage::load_blockchain_with_retry;
use mona_crypto::load_wallet;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};

use crate::format_kari_amount;


// Enhanced staking API structures with Move integration
#[derive(Deserialize)]
pub struct StakeParams {
    pub address: String,
    pub amount: f64,
    pub password: String,
    pub validator: bool,
    pub use_move_integration: Option<bool>, // Optional Move integration flag
}

#[derive(Deserialize)]
pub struct UnstakeParams {
    pub address: String,
    pub password: String,
    pub force: Option<bool>, // Optional force unstake flag
}

// Enhanced staking API methods with Move alignment
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
            // Calculate amount in KA units using Move-aligned constants
            const KA_PER_KARI: u64 = 1_000_000_000;
            let amount_ka = (stake_params.amount * KA_PER_KARI as f64) as u64;
            
            // Parse the address
            let address = match mona_types::address::Address::from_str(&stake_params.address) {
                Ok(addr) => addr,
                Err(_) => return Err(RpcError::invalid_params("Invalid address format")),
            };
            
            // Use enhanced staking with Move integration
            let use_move = stake_params.use_move_integration.unwrap_or(false);
            match panorama::staking::stake_tokens_enhanced(&address, amount_ka, stake_params.validator, use_move) {
                Ok(staked_node) => {
                    // Calculate unlock timing using Move constants
                    let lock_period_hours = 24; // STAKING_LOCK_PERIOD_MS / 3600000
                    let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                    let _can_unstake = current_time >= staked_node.unlock_time;
                    
                    // Format the enhanced response
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
                        "lock_period_hours": lock_period_hours,
                        "status": "staked",
                        "commission_rate": staked_node.commission_rate,
                        "commission_rate_percent": staked_node.commission_rate as f64 / 100.0,
                        "delegated_stake": staked_node.delegated_stake,
                        "delegated_stake_formatted": format_kari_amount(staked_node.delegated_stake),
                        "is_active": staked_node.is_active,
                        "move_object_id": staked_node.move_object_id.map(|id| id.address().to_hex_literal()),
                        "move_integration": use_move,
                        "security_hash": staked_node.security_hash,
                        "penalty_rate": 10.0, // EARLY_UNSTAKE_PENALTY_BASIS_POINTS / 100
                        "reward_rate_annual": 0.01, // REWARD_RATE_BASIS_POINTS / 100
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
            
            // Use enhanced unstaking with penalty calculation
            let force_unstake = unstake_params.force.unwrap_or(false);
            match panorama::staking::unstake_tokens_enhanced(&address, force_unstake) {
                Ok((withdrawn_amount, rewards, penalty)) => {
                    // Format the enhanced response
                    Ok(json!({
                        "address": unstake_params.address,
                        "withdrawn_amount": withdrawn_amount,
                        "withdrawn_amount_formatted": format_kari_amount(withdrawn_amount),
                        "rewards": rewards,
                        "rewards_formatted": format_kari_amount(rewards),
                        "penalty": penalty,
                        "penalty_formatted": format_kari_amount(penalty),
                        "total_returned": withdrawn_amount + rewards,
                        "total_returned_formatted": format_kari_amount(withdrawn_amount + rewards),
                        "force_unstake": force_unstake,
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
    
    // Get enhanced staking info
    match panorama::staking::get_enhanced_staking_info(&address) {
        Some(node) => {
            // Check if the lock period has passed
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            
            let can_unstake = current_time >= node.unlock_time;
            let early_unstake_penalty = panorama::staking::calculate_early_unstake_penalty(node.staked_amount);
            
            // Format the enhanced response
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
                "time_remaining": if can_unstake { 0 } else { node.unlock_time - current_time },
                "accumulated_rewards": node.accumulated_rewards,
                "accumulated_rewards_formatted": format_kari_amount(node.accumulated_rewards),
                "commission_rate": node.commission_rate,
                "commission_rate_percent": node.commission_rate as f64 / 100.0,
                "delegated_stake": node.delegated_stake,
                "delegated_stake_formatted": format_kari_amount(node.delegated_stake),
                "is_active": node.is_active,
                "early_unstake_penalty": early_unstake_penalty,
                "early_unstake_penalty_formatted": format_kari_amount(early_unstake_penalty),
                "move_object_id": node.move_object_id.map(|id| id.address().to_hex_literal()),
                "security_hash": node.security_hash,
                "estimated_daily_reward": if node.is_validator {
                    let daily_reward_rate = 0.01 / 365.0; // REWARD_RATE_BASIS_POINTS annual to daily
                    (node.staked_amount as f64 * daily_reward_rate / 10000.0).round() as u64
                } else {
                    0
                },
                "lock_period_remaining_hours": if can_unstake { 0 } else { (node.unlock_time - current_time) / 3600 },
            }))
        },
        None => {
            // Not staking - show Move-aligned requirements
            Ok(json!({
                "address": address_str,
                "is_staking": false,
                "minimum_node_stake": panorama::staking::MIN_NODE_STAKE,
                "minimum_node_stake_formatted": format_kari_amount(panorama::staking::MIN_NODE_STAKE),
                "minimum_validator_stake": panorama::staking::MIN_VALIDATOR_STAKE,
                "minimum_validator_stake_formatted": format_kari_amount(panorama::staking::MIN_VALIDATOR_STAKE),
                "lock_period_hours": 24,
                "penalty_rate_percent": 10.0,
                "annual_reward_rate_percent": 0.01,
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
    
    // Get enhanced staking pool stats
    let pool_state = panorama::staking::get_pool_state();
    let validator_list = panorama::staking::get_validator_list();
    
    // Calculate additional statistics
    let total_delegated: u64 = validator_list.iter().map(|v| v.stake_amount).sum();
    let average_commission: f64 = if !validator_list.is_empty() {
        validator_list.iter().map(|v| v.commission_rate as f64).sum::<f64>() / validator_list.len() as f64
    } else {
        0.0
    };
    
    // Format the enhanced response
    Ok(json!({
        "pool_state": {
            "total_staked": pool_state.total_staked,
            "total_staked_formatted": format_kari_amount(pool_state.total_staked),
            "validator_count": pool_state.validator_count,
            "node_count": pool_state.node_count,
            "reward_rate": pool_state.reward_rate,
            "last_reward_epoch": pool_state.last_reward_epoch,
        },
        "validators": {
            "total_count": validator_list.len(),
            "active_count": validator_list.iter().filter(|v| v.is_active).count(),
            "total_delegated": total_delegated,
            "total_delegated_formatted": format_kari_amount(total_delegated),
            "average_commission_rate": average_commission,
            "validator_list": validator_list.iter().map(|v| json!({
                "address": v.address.to_hex_literal(),
                "stake_amount": v.stake_amount,
                "stake_amount_formatted": format_kari_amount(v.stake_amount),
                "commission_rate": v.commission_rate,
                "commission_rate_percent": v.commission_rate as f64 / 100.0,
                "is_active": v.is_active,
                "last_epoch_reward": v.last_epoch_reward,
            })).collect::<Vec<_>>(),
        },
        "requirements": {
            "minimum_node_stake": panorama::staking::MIN_NODE_STAKE,
            "minimum_node_stake_formatted": format_kari_amount(panorama::staking::MIN_NODE_STAKE),
            "minimum_validator_stake": panorama::staking::MIN_VALIDATOR_STAKE,
            "minimum_validator_stake_formatted": format_kari_amount(panorama::staking::MIN_VALIDATOR_STAKE),
        },
        "parameters": {
            "lock_period_hours": 24,
            "early_unstake_penalty_percent": 10.0,
            "annual_reward_rate_basis_points": panorama::staking::REWARD_RATE_BASIS_POINTS,
            "annual_reward_rate_percent": panorama::staking::REWARD_RATE_BASIS_POINTS as f64 / 100.0,
        },
        "move_integration": panorama::simulation::is_move_integration_enabled(),
    }))
}

// New enhanced API methods
pub fn claim_rewards(params: Params) -> JsonRpcResult<JsonValue> {
    // Parse address parameter
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
        _ => return Err(RpcError::invalid_params("Expected array parameters")),
    };
    
    let address = match mona_types::address::Address::from_str(&address_str) {
        Ok(addr) => addr,
        Err(_) => return Err(RpcError::invalid_params("Invalid address format")),
    };
    
    // Get current staking info to check rewards
    match panorama::staking::get_enhanced_staking_info(&address) {
        Some(node) => {
            if node.accumulated_rewards > 0 {
                Ok(json!({
                    "address": address_str,
                    "claimable_rewards": node.accumulated_rewards,
                    "claimable_rewards_formatted": format_kari_amount(node.accumulated_rewards),
                    "can_claim": true,
                    "message": "Rewards available for claiming through wallet interface"
                }))
            } else {
                Ok(json!({
                    "address": address_str,
                    "claimable_rewards": 0,
                    "can_claim": false,
                    "message": "No rewards available to claim"
                }))
            }
        },
        None => {
            Err(RpcError {
                code: ErrorCode::InvalidParams,
                message: "Address is not staking".to_string(),
                data: None,
            })
        }
    }
}
