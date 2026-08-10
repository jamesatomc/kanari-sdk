// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_rpc_api::{
    GetObjectByRefRequest, GetObjectsRequest, ModuleInfo, ObjectInfo, RpcRequest, RpcResponse,
};
use move_binary_format::CompiledModule;

use crate::{
    RpcServerState, internal_error_response, invalid_params_response, parse_params,
    respond_with_serialize,
};

fn extract_module_functions(bytecode: &[u8]) -> Vec<String> {
    CompiledModule::deserialize_with_defaults(bytecode)
        .ok()
        .map(|module| {
            module
                .function_defs
                .iter()
                .filter_map(|def| {
                    let handle = module.function_handle_at(def.function);
                    let name = module.identifier_at(handle.name).as_str().to_string();
                    (!name.is_empty()).then_some(name)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn extract_module_dependencies(bytecode: &[u8]) -> Vec<String> {
    let mut dependencies = CompiledModule::deserialize_with_defaults(bytecode)
        .ok()
        .map(|module| {
            module
                .immediate_dependencies()
                .into_iter()
                .map(|dependency| {
                    format!(
                        "{}::{}",
                        dependency.address().to_hex_literal(),
                        dependency.name().as_str()
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    dependencies.sort_unstable();
    dependencies.dedup();
    dependencies
}

fn build_object_info(
    id: String,
    obj: kanari_move_runtime_v1::changeset::CreatedObject,
) -> ObjectInfo {
    let digest = format!("0x{}", hex::encode(blake3::hash(&obj.data).as_bytes()));
    ObjectInfo {
        id,
        owner: format!("{:#x}", obj.owner),
        owner_kind: obj.owner_kind,
        type_: obj.type_,
        data: obj.data,
        version: obj.version,
        digest: Some(digest),
    }
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
            let functions = extract_module_functions(&bytecode);
            let module_info = ModuleInfo {
                address: params.address,
                name: params.name,
                bytecode_hash: hex::encode(&blake3::hash(&bytecode).as_bytes()[..]),
                size: bytecode.len(),
                dependencies: extract_module_dependencies(&bytecode),
                function_count: functions.len(),
                functions,
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
            let functions = bytecode_opt
                .as_deref()
                .map(extract_module_functions)
                .unwrap_or_default();
            let dependencies = bytecode_opt
                .as_deref()
                .map(extract_module_dependencies)
                .unwrap_or_default();
            ModuleInfo {
                address: address.clone(),
                name: name.clone(),
                bytecode_hash: bytecode_opt
                    .as_ref()
                    .map(|b| hex::encode(&blake3::hash(b).as_bytes()[..]))
                    .unwrap_or_else(|| "unknown".to_string()),
                size: bytecode_opt.as_ref().map(|b| b.len()).unwrap_or(0),
                dependencies,
                function_count: functions.len(),
                functions,
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

            objects.push(build_object_info(uid, obj));
        }
    }

    // Return the filtered list of objects
    respond_with_serialize(
        request.id,
        kanari_rpc_api::OwnedObjectsResponse {
            summary: Some(kanari_rpc_api::OwnerObjectSummary {
                owner: req.owner,
                total_objects: objects.len(),
                object_changes_hint: "Use kanari_getObject for object-first reads and transaction effects for mutations".to_string(),
            }),
            objects,
        },
    )
}

/// Handle get objects by type request
pub async fn handle_get_objects_by_type(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let req: kanari_rpc_api::GetObjectsByTypeRequest =
        match parse_params(request.id, &request.params) {
            Ok(r) => r,
            Err(response) => return *response,
        };

    let state_guard = state.engine.state_read();
    drop(state_guard);
    let objects = match state.engine.get_objects_by_type(&req.object_type) {
        Ok(objects) => objects,
        Err(e) => {
            return internal_error_response(
                request.id,
                format!("Failed to get objects by type: {}", e),
            );
        }
    };

    respond_with_serialize(
        request.id,
        kanari_rpc_api::OwnedObjectsResponse {
            objects,
            summary: None,
        },
    )
}

pub async fn handle_get_objects(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let req: GetObjectsRequest = match parse_params(request.id, &request.params) {
        Ok(r) => r,
        Err(response) => return *response,
    };

    let objects = match state.engine.query_objects(
        req.owner.as_deref(),
        req.owner_kind,
        req.object_type.as_deref(),
        req.min_version,
        req.max_version,
    ) {
        Ok(objects) => objects,
        Err(e) => {
            return internal_error_response(request.id, format!("Failed to query objects: {}", e));
        }
    };

    respond_with_serialize(
        request.id,
        kanari_rpc_api::OwnedObjectsResponse {
            summary: Some(kanari_rpc_api::OwnerObjectSummary {
                owner: req.owner.unwrap_or_else(|| "*".to_string()),
                total_objects: objects.len(),
                object_changes_hint:
                    "Prefer object refs and object graph edges for versioned/object-centric reads"
                        .to_string(),
            }),
            objects,
        },
    )
}

pub async fn handle_get_object_by_ref(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let req: GetObjectByRefRequest = match parse_params(request.id, &request.params) {
        Ok(r) => r,
        Err(response) => return *response,
    };

    if req.object_ref.version.is_none() || req.object_ref.digest.is_none() {
        return invalid_params_response(
            request.id,
            "object_ref must include (object_id, version, digest)",
        );
    }

    match state.engine.get_object_by_ref(&req.object_ref) {
        Ok(Some(object)) => respond_with_serialize(request.id, object),
        Ok(None) => internal_error_response(request.id, "Object ref not found"),
        Err(e) => {
            internal_error_response(request.id, format!("Failed to resolve object ref: {}", e))
        }
    }
}
