use kanari_rpc_api::{ModuleInfo, ObjectInfo, RpcRequest, RpcResponse};
use move_binary_format::CompiledModule;
use std::collections::BTreeMap;

use crate::{
    RpcServerState, internal_error_response, invalid_params_response, parse_params,
    respond_with_serialize,
};

fn build_object_info(
    id: String,
    obj: kanari_move_runtime_v1::changeset::CreatedObject,
) -> ObjectInfo {
    ObjectInfo {
        id,
        owner: format!("{:#x}", obj.owner),
        type_: obj.type_,
        data: obj.data,
        version: obj.version,
    }
}

fn read_coin_balance(data: &[u8]) -> Option<u64> {
    if data.len() < 40 {
        return None;
    }

    let mut balance_bytes = [0u8; 8];
    balance_bytes.copy_from_slice(&data[32..40]);
    Some(u64::from_le_bytes(balance_bytes))
}

fn write_coin_balance(data: &mut [u8], balance: u64) -> bool {
    if data.len() < 40 {
        return false;
    }

    data[32..40].copy_from_slice(&balance.to_le_bytes());
    true
}

fn is_coin_object(object_type: &str) -> bool {
    object_type.contains("::coin::Coin<")
}

pub(crate) fn aggregate_owned_objects(objects: Vec<ObjectInfo>) -> Vec<ObjectInfo> {
    let mut aggregated = Vec::new();
    let mut coin_indices = BTreeMap::<(String, String), usize>::new();

    for obj in objects {
        if !is_coin_object(&obj.type_) {
            aggregated.push(obj);
            continue;
        }

        let Some(balance) = read_coin_balance(&obj.data) else {
            aggregated.push(obj);
            continue;
        };

        let key = (obj.owner.clone(), obj.type_.clone());
        if let Some(index) = coin_indices.get(&key).copied() {
            if let Some(existing) = aggregated.get_mut(index)
                && let Some(existing_balance) = read_coin_balance(&existing.data)
            {
                let merged_balance = existing_balance.saturating_add(balance);
                let _ = write_coin_balance(&mut existing.data, merged_balance);
                existing.version = existing.version.max(obj.version);
            }
        } else {
            coin_indices.insert(key, aggregated.len());
            aggregated.push(obj);
        }
    }

    aggregated
}

/// Handle get module
pub async fn handle_get_module(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    #[derive(serde::Deserialize)]
    struct GetModuleParams {
        address: String,
        name: String,
    }

    let params: GetModuleParams = match parse_params(request.id, &request.params) {
        Ok(p) => p,
        Err(response) => return *response,
    };

    // Get module bytecode from Move storage
    match state
        .engine
        .get_module_bytecode(&params.address, &params.name)
    {
        Some(bytecode) => {
            let module_info = ModuleInfo {
                address: params.address,
                name: params.name,
                bytecode_hash: hex::encode(&blake3::hash(&bytecode).as_bytes()[..]),
                size: bytecode.len(),
                dependencies: vec![], // TODO: Extract dependencies from bytecode
            };
            respond_with_serialize(request.id, module_info)
        }
        None => internal_error_response(request.id, "Module not found"),
    }
}

/// Handle list modules
pub async fn handle_list_modules(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    // Get modules from Move storage instead of contracts
    let modules_data = state.engine.list_all_modules();
    let modules: Vec<ModuleInfo> = modules_data
        .iter()
        .map(|(address, name)| {
            let bytecode_opt = state.engine.get_module_bytecode(address, name);
            ModuleInfo {
                address: address.clone(),
                name: name.clone(),
                bytecode_hash: bytecode_opt
                    .as_ref()
                    .map(|b| hex::encode(&blake3::hash(b).as_bytes()[..]))
                    .unwrap_or_else(|| "unknown".to_string()),
                size: bytecode_opt.as_ref().map(|b| b.len()).unwrap_or(0),
                dependencies: vec![],
            }
        })
        .collect();

    respond_with_serialize(request.id, modules)
}

/// Handle verify module
pub async fn handle_verify_module(_state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    #[derive(serde::Deserialize)]
    struct VerifyParams {
        module_bytes: Vec<u8>,
    }

    let params: VerifyParams = match parse_params(request.id, &request.params) {
        Ok(p) => p,
        Err(response) => return *response,
    };

    // Try to deserialize module
    match CompiledModule::deserialize_with_defaults(&params.module_bytes) {
        Ok(module) => {
            let module_id = module.self_id();
            RpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(serde_json::json!({
                    "valid": true,
                    "address": module_id.address().to_hex_literal(),
                    "name": module_id.name().as_str()
                })),
                error: None,
                id: request.id,
            }
        }
        Err(e) => RpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(serde_json::json!({
                "valid": false,
                "error": e.to_string()
            })),
            error: None,
            id: request.id,
        },
    }
}

/// Handle get object request
pub async fn handle_get_object(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let req: kanari_rpc_api::GetObjectRequest = match parse_params(request.id, &request.params) {
        Ok(r) => r,
        Err(response) => return *response,
    };

    let id = req.object_id;
    let state_guard = state.engine.state_read();

    if let Ok(Some(obj)) = state_guard.get_object(&id) {
        return respond_with_serialize(request.id, build_object_info(id, obj));
    }

    internal_error_response(request.id, "Object not found")
}

/// Handle get owned objects request
pub async fn handle_get_owned_objects(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let req: kanari_rpc_api::GetOwnedObjectsRequest =
        match parse_params(request.id, &request.params) {
            Ok(r) => r,
            Err(response) => return *response,
        };

    // Parse owner address using kanari_types::address::Address::parse_to_account_address
    let owner_addr = match kanari_types::address::Address::parse_to_account_address(&req.owner) {
        Ok(addr) => addr,
        Err(e) => {
            return invalid_params_response(request.id, format!("Invalid owner address: {}", e));
        }
    };

    let state_guard = state.engine.state_read();

    // Get all owned object IDs for the owner
    let owned_ids = match state_guard.get_owned_objects(&owner_addr) {
        Ok(ids) => ids,
        Err(e) => {
            return internal_error_response(
                request.id,
                format!("Failed to get owned objects: {}", e),
            );
        }
    };

    // Filter objects by type if specified
    let mut objects = Vec::new();
    for uid in owned_ids {
        if let Ok(Some(obj)) = state_guard.get_object(&uid) {
            // If object_type filter is specified, check if it matches
            if let Some(ref filter_type) = req.object_type
                && !obj.type_.contains(filter_type)
            {
                continue;
            }

            let object_id = match kanari_types::object::ObjectID::from_hex_literal(&uid) {
                Ok(object_id) => object_id,
                Err(error) => {
                    return internal_error_response(request.id, error.to_string());
                }
            };
            let reference = match state_guard.get_object_ref_exact(object_id) {
                Ok(Some(reference)) => reference,
                Ok(None) => continue,
                Err(error) => {
                    return internal_error_response(request.id, error.to_string());
                }
            };
            objects.push(serde_json::json!({
                "id": uid,
                "owner": format!("{:#x}", obj.owner),
                "type_": obj.type_,
                "data": obj.data,
                "version": obj.version,
                "digest": reference.digest.to_hex(),
            }));
        }
    }

    // Return the filtered list of objects
    RpcResponse {
        jsonrpc: "2.0".to_string(),
        result: Some(serde_json::json!({
            "objects": objects
        })),
        error: None,
        id: request.id,
    }
}
