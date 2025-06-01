use jsonrpc_core::{Error, ErrorCode, Result as JsonRpcResult, Value};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use mona_vm::{
    VM_STATE,
    VMTransaction,
    execute_vm_transaction
};
use mona_blockchain::blockchain::{BLOCKCHAIN_DATA, submit_transaction};
use move_core_types::account_address::AccountAddress;

const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 100;

#[derive(Debug, Serialize, Deserialize)]
struct ListModulesParams {
    limit: Option<usize>,
    offset: Option<usize>,
    address_filter: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetModuleParams {
    module_id: String,
    include_bytecode: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExecuteFunctionParams {
    module_id: String,
    function: String,
    args: Option<Vec<FunctionArg>>,
    sender: Option<String>,
    gas_budget: Option<u64>,
    submit_to_blockchain: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FunctionArg {
    arg_type: String,
    value: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetModuleTransactionsParams {
    module_id: String,
    limit: Option<usize>,
    offset: Option<usize>,
    function_filter: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeployModuleParams {
    bytecode: String, // hex-encoded bytecode
    sender: Option<String>,
    gas_budget: Option<u64>,
}

/// List deployed Move modules with enhanced filtering
pub fn list_modules(params: jsonrpc_core::Params) -> JsonRpcResult<Value> {
    let params: ListModulesParams = parse_params(params)?;
    
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = params.offset.unwrap_or(0);
    
    match VM_STATE.try_read() {
        Ok(state) => {
            let mut modules: Vec<_> = state.modules.values().collect();
            
            // Apply address filter if provided
            if let Some(ref address_filter) = params.address_filter {
                modules.retain(|module| {
                    module.address.to_hex_literal().contains(address_filter) ||
                    module.module_id.contains(address_filter)
                });
            }
            
            // Apply pagination
            let total = modules.len();
            let modules = modules.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
            
            let module_data: Vec<Value> = modules.iter()
                .map(|module| {
                    serde_json::json!({
                        "module_id": module.module_id,
                        "address": module.address.to_hex_literal(),
                        "name": module.name,
                        "bytecode_size": module.bytecode.len(),
                        "deploy_block_height": module.deploy_block_height,
                        "public_functions": module.public_functions,
                        "function_count": module.public_functions.len(),
                        "deploy_timestamp": get_block_timestamp(module.deploy_block_height),
                    })
                })
                .collect();
            
            Ok(serde_json::json!({
                "modules": module_data,
                "total": total,
                "limit": limit,
                "offset": offset,
                "filter": params.address_filter
            }))
        },
        Err(e) => Err(internal_error(format!("Failed to access VM state: {}", e)))
    }
}

/// Get detailed information about a specific module
pub fn get_module(params: jsonrpc_core::Params) -> JsonRpcResult<Value> {
    let params: GetModuleParams = parse_params(params)?;
    let include_bytecode = params.include_bytecode.unwrap_or(false);
    
    match VM_STATE.try_read() {
        Ok(state) => {
            match state.modules.get(&params.module_id) {
                Some(module) => {
                    let mut result = serde_json::json!({
                        "module_id": module.module_id,
                        "address": module.address.to_hex_literal(),
                        "name": module.name,
                        "bytecode_size": module.bytecode.len(),
                        "deploy_block_height": module.deploy_block_height,
                        "deploy_timestamp": get_block_timestamp(module.deploy_block_height),
                        "public_functions": module.public_functions,
                        "function_count": module.public_functions.len(),
                        "version": "1.0",
                        "dependencies": get_module_dependencies(&module.module_id),
                        "events": get_module_events(&module.module_id),
                    });
                    
                    if include_bytecode {
                        result["bytecode"] = Value::String(hex::encode(&module.bytecode));
                    }
                    
                    Ok(result)
                },
                None => Err(not_found_error(format!("Module not found: {}", params.module_id)))
            }
        },
        Err(e) => Err(internal_error(format!("Failed to access VM state: {}", e)))
    }
}

/// Get transactions related to a specific module
pub fn get_module_transactions(params: jsonrpc_core::Params) -> JsonRpcResult<Value> {
    let params: GetModuleTransactionsParams = parse_params(params)?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let offset = params.offset.unwrap_or(0);
    
    let blocks = BLOCKCHAIN_DATA.iter();
    let mut transactions = Vec::new();
    
    // Search through all blocks for VM transactions related to this module
    for block in blocks {
        for tx in &block.transactions {
            if tx.is_vm_transaction() {
                if let Some(data) = &tx.data {
                    if let Ok(data_str) = std::str::from_utf8(data) {
                        if data_str.contains(&params.module_id) {
                            // Check function filter if provided
                            if let Some(ref function_filter) = params.function_filter {
                                if !data_str.contains(function_filter) {
                                    continue;
                                }
                            }
                            
                            if let Some((module_id, function)) = tx.get_vm_function_info() {
                                if module_id == params.module_id {
                                    transactions.push(serde_json::json!({
                                        "tx_id": tx.transaction_id,
                                        "module_id": module_id,
                                        "function": function,
                                        "sender": tx.sender.to_hex_literal(),
                                        "receiver": tx.receiver.to_hex_literal(),
                                        "gas_fee": tx.gas_fee,
                                        "timestamp": tx.timestamp,
                                        "block_height": block.index,
                                        "block_hash": block.hash,
                                        "transaction_type": tx.get_transaction_type(),
                                    }));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Apply pagination
    let total = transactions.len();
    transactions.sort_by(|a, b| {
        b["timestamp"].as_u64().cmp(&a["timestamp"].as_u64())
    });
    
    let paginated_transactions: Vec<_> = transactions.into_iter()
        .skip(offset)
        .take(limit)
        .collect();
    
    Ok(serde_json::json!({
        "transactions": paginated_transactions,
        "total": total,
        "limit": limit,
        "offset": offset,
        "module_id": params.module_id,
        "function_filter": params.function_filter
    }))
}

/// Execute a function in a Move module
pub fn execute_function(params: jsonrpc_core::Params) -> JsonRpcResult<Value> {
    let params: ExecuteFunctionParams = parse_params(params)?;
    
    // Parse and validate arguments
    let args = match parse_function_args(params.args) {
        Ok(a) => a,
        Err(e) => return Err(invalid_params_error(format!("Invalid arguments: {}", e)))
    };
    
    let sender = params.sender.unwrap_or_else(|| "0x1".to_string());
    let gas_budget = params.gas_budget.unwrap_or(1_000_000);
    let submit_to_blockchain = params.submit_to_blockchain.unwrap_or(false);
    
    // Create VM transaction
    let vm_tx = VMTransaction::new(
        sender.clone(),
        params.module_id.clone(),
        params.function.clone(),
        args,
        gas_budget
    );
    
    // If submitting to blockchain, create and submit blockchain transaction first
    if submit_to_blockchain {
        match create_and_submit_blockchain_transaction(&vm_tx) {
            Ok(tx_id) => {
                // Execute VM transaction
                match execute_vm_transaction(&vm_tx) {
                    Ok(mut result) => {
                        result["blockchain_tx_id"] = Value::String(tx_id);
                        result["submitted_to_blockchain"] = Value::Bool(true);
                        result["execution_mode"] = Value::String("blockchain".to_string());
                        Ok(result)
                    },
                    Err(e) => Err(execution_error(format!("VM execution failed: {}", e)))
                }
            },
            Err(e) => Err(execution_error(format!("Blockchain submission failed: {}", e)))
        }
    } else {
        // Execute only in VM (simulation mode)
        match execute_vm_transaction(&vm_tx) {
            Ok(mut result) => {
                result["submitted_to_blockchain"] = Value::Bool(false);
                result["execution_mode"] = Value::String("simulation".to_string());
                Ok(result)
            },
            Err(e) => Err(execution_error(format!("VM execution failed: {}", e)))
        }
    }
}

/// Deploy a new Move module
pub fn deploy_module(params: jsonrpc_core::Params) -> JsonRpcResult<Value> {
    let params: DeployModuleParams = parse_params(params)?;
    
    // Decode bytecode from hex
    let bytecode = match hex::decode(&params.bytecode) {
        Ok(b) => b,
        Err(e) => return Err(invalid_params_error(format!("Invalid bytecode hex: {}", e)))
    };
    
    let sender = params.sender.unwrap_or_else(|| "0x1".to_string());
    let gas_budget = params.gas_budget.unwrap_or(3_000_000);
    
    // Create deployment transaction
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    let tx_id = format!("deploy_{}_{}", sender, timestamp);
    let deployment_data = format!("VM_MODULE_DEPLOYMENT:{}:bytecode:{}", sender, gas_budget);
    
    let blockchain_tx = mona_blockchain::block::Transaction {
        transaction_id: tx_id.clone(),
        sender: mona_types::address::Address::from_hex_literal(&sender)
            .map_err(|_| invalid_params_error("Invalid sender address".to_string()))?,
        receiver: mona_types::address::Address::from_hex_literal(&sender)
            .map_err(|_| invalid_params_error("Invalid sender address".to_string()))?,
        amount: 0,
        gas_fee: gas_budget,
        timestamp,
        signature: Vec::new(),
        data: Some(deployment_data.into_bytes()),
    };
    
    // Submit to blockchain
    match submit_transaction(blockchain_tx) {
        Ok(()) => {
            Ok(serde_json::json!({
                "status": "success",
                "tx_id": tx_id,
                "sender": sender,
                "gas_budget": gas_budget,
                "bytecode_size": bytecode.len(),
                "timestamp": timestamp,
                "message": "Module deployment transaction submitted to blockchain"
            }))
        },
        Err(e) => Err(execution_error(format!("Deployment failed: {}", e)))
    }
}

/// Get current VM state with detailed information
pub fn get_vm_state(_params: jsonrpc_core::Params) -> JsonRpcResult<Value> {
    match VM_STATE.try_read() {
        Ok(state) => {
            let modules_count = state.modules.len();
            let modules_by_address = group_modules_by_address(&state);
            let recent_modules: Vec<String> = state.modules.values()
                .filter(|m| m.deploy_block_height > 0)
                .map(|m| m.module_id.clone())
                .take(5)
                .collect();
            
            Ok(serde_json::json!({
                "modules_count": modules_count,
                "last_execution": state.last_execution,
                "execution_count": state.execution_count,
                "last_signer": state.last_signer.clone().unwrap_or_default(),
                "modules_by_address": modules_by_address,
                "recent_modules": recent_modules,
                "blockchain_height": BLOCKCHAIN_DATA.len(),
                "vm_version": "1.0.0",
                "supported_features": ["function_execution", "module_deployment", "event_emission"]
            }))
        },
        Err(e) => Err(internal_error(format!("Failed to access VM state: {}", e)))
    }
}

/// Get gas estimation for function execution
pub fn estimate_gas(params: jsonrpc_core::Params) -> JsonRpcResult<Value> {
    let params: ExecuteFunctionParams = parse_params(params)?;
    
    // Parse arguments for estimation
    let args = match parse_function_args(params.args) {
        Ok(a) => a,
        Err(e) => return Err(invalid_params_error(format!("Invalid arguments: {}", e)))
    };
    
    // Create VM transaction for estimation
    let vm_tx = VMTransaction::new(
        params.sender.unwrap_or_else(|| "0x1".to_string()),
        params.module_id,
        params.function,
        args,
        10_000_000 // High gas limit for estimation
    );
    
    // Execute in simulation mode to get gas usage
    match execute_vm_transaction(&vm_tx) {
        Ok(result) => {
            let estimated_gas = result["gas_used"].as_u64().unwrap_or(0);
            let recommended_gas = (estimated_gas as f64 * 1.2) as u64; // 20% buffer
            
            Ok(serde_json::json!({
                "estimated_gas": estimated_gas,
                "recommended_gas": recommended_gas,
                "max_gas": 10_000_000,
                "gas_price": 1, // 1 KA per gas unit
                "estimated_cost": estimated_gas,
                "recommended_cost": recommended_gas,
            }))
        },
        Err(e) => Err(execution_error(format!("Gas estimation failed: {}", e)))
    }
}

// Helper functions

fn create_and_submit_blockchain_transaction(vm_tx: &VMTransaction) -> Result<String, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    
    let vm_data = format!("VM:{}:{}:{}", vm_tx.module_id, vm_tx.function, vm_tx.gas_budget);
    
    let blockchain_tx = mona_blockchain::block::Transaction {
        transaction_id: vm_tx.tx_id.clone(),
        sender: mona_types::address::Address::from_hex_literal(&vm_tx.sender)
            .map_err(|_| "Invalid sender address".to_string())?,
        receiver: mona_types::address::Address::from_hex_literal(&vm_tx.sender)
            .map_err(|_| "Invalid receiver address".to_string())?,
        amount: 0,
        gas_fee: vm_tx.gas_budget,
        timestamp,
        signature: vm_tx.signature.clone().unwrap_or_default(),
        data: Some(vm_data.into_bytes()),
    };
    
    submit_transaction(blockchain_tx)
        .map_err(|e| format!("Blockchain submission failed: {}", e))
        .map(|_| vm_tx.tx_id.clone())
}

fn get_block_timestamp(block_height: u32) -> u64 {
    if let Some(block) = BLOCKCHAIN_DATA.get_block(block_height as usize) {
        block.timestamp
    } else {
        0
    }
}

fn get_module_dependencies(_module_id: &str) -> Vec<String> {
    // TODO: Implement dependency analysis
    vec![]
}

fn get_module_events(_module_id: &str) -> Vec<Value> {
    // TODO: Implement event tracking
    vec![]
}

fn group_modules_by_address(state: &mona_vm::VMState) -> std::collections::HashMap<String, usize> {
    let mut grouped = std::collections::HashMap::new();
    
    for module in state.modules.values() {
        let address = module.address.to_hex_literal();
        *grouped.entry(address).or_insert(0) += 1;
    }
    
    grouped
}

fn parse_function_args(args_opt: Option<Vec<FunctionArg>>) -> Result<Vec<Vec<u8>>, String> {
    let mut parsed_args = Vec::new();
    
    let args = match args_opt {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    
    for arg in args {
        match arg.arg_type.as_str() {
            "address" => {
                if let Some(addr_str) = arg.value.as_str() {
                    match AccountAddress::from_hex_literal(addr_str) {
                        Ok(addr) => parsed_args.push(addr.to_vec()),
                        Err(_) => return Err(format!("Invalid address format: {}", addr_str))
                    }
                } else {
                    return Err("Address argument must be a string".to_string());
                }
            },
            "u8" => {
                if let Some(n) = arg.value.as_u64() {
                    if n <= u8::MAX as u64 {
                        parsed_args.push(vec![n as u8]);
                    } else {
                        return Err(format!("Value {} too large for u8", n));
                    }
                } else {
                    return Err("u8 argument must be a number".to_string());
                }
            },
            "u64" => {
                if let Some(n) = arg.value.as_u64() {
                    parsed_args.push(n.to_le_bytes().to_vec());
                } else {
                    return Err("u64 argument must be a number".to_string());
                }
            },
            "u128" => {
                if let Some(n) = arg.value.as_u64() {
                    parsed_args.push((n as u128).to_le_bytes().to_vec());
                } else if let Some(s) = arg.value.as_str() {
                    match s.parse::<u128>() {
                        Ok(n) => parsed_args.push(n.to_le_bytes().to_vec()),
                        Err(_) => return Err(format!("Invalid u128 value: {}", s))
                    }
                } else {
                    return Err("u128 argument must be a number or string".to_string());
                }
            },
            "bool" => {
                if let Some(b) = arg.value.as_bool() {
                    parsed_args.push(vec![if b { 1 } else { 0 }]);
                } else {
                    return Err("bool argument must be a boolean".to_string());
                }
            },
            "string" => {
                if let Some(s) = arg.value.as_str() {
                    parsed_args.push(s.as_bytes().to_vec());
                } else {
                    return Err("string argument must be a string".to_string());
                }
            },
            "vector<u8>" | "bytes" => {
                if let Some(s) = arg.value.as_str() {
                    if s.starts_with("0x") {
                        match hex::decode(&s[2..]) {
                            Ok(bytes) => parsed_args.push(bytes),
                            Err(_) => return Err(format!("Invalid hex string: {}", s))
                        }
                    } else {
                        parsed_args.push(s.as_bytes().to_vec());
                    }
                } else {
                    return Err("bytes argument must be a hex string or regular string".to_string());
                }
            },
            _ => return Err(format!("Unsupported argument type: {}", arg.arg_type))
        }
    }
    
    Ok(parsed_args)
}

fn parse_params<T>(params: jsonrpc_core::Params) -> Result<T, Error> 
where
    T: serde::de::DeserializeOwned,
{
    params.parse().map_err(|e| {
        Error {
            code: ErrorCode::InvalidParams,
            message: format!("Invalid parameters: {}", e),
            data: None,
        }
    })
}

fn internal_error(msg: String) -> Error {
    Error {
        code: ErrorCode::InternalError,
        message: msg,
        data: None,
    }
}

fn invalid_params_error(msg: String) -> Error {
    Error {
        code: ErrorCode::InvalidParams,
        message: msg,
        data: None,
    }
}

fn not_found_error(msg: String) -> Error {
    Error {
        code: ErrorCode::ServerError(-32004),
        message: msg,
        data: None,
    }
}

fn execution_error(msg: String) -> Error {
    Error {
        code: ErrorCode::ServerError(-32005),
        message: msg,
        data: None,
    }
}
