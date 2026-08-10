// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::{
    RpcRequest, RpcResponse, RpcServerState, internal_error_response, invalid_params_response,
    parse_params, respond_with_serialize,
};
use kanari_rpc_api::{
    BlockInfo, CompareCanonicalStateSnapshotRequest, GetCanonicalStateSnapshotRequest,
    GetSmtStatusRequest, RpcEvent,
};
use serde_json;

const MAX_CANONICAL_SNAPSHOT_ENTRIES: usize = 1_000;
const MAX_CANONICAL_COMPARE_ENTRIES: usize = 1_000;
static EXPENSIVE_RPC_PERMIT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

fn try_acquire_expensive_rpc(
    id: u64,
) -> Result<tokio::sync::SemaphorePermit<'static>, Box<RpcResponse>> {
    EXPENSIVE_RPC_PERMIT.try_acquire().map_err(|_| {
        Box::new(invalid_params_response(
            id,
            "Another expensive diagnostic request is already running; retry later",
        ))
    })
}

fn parse_height(id: u64, params: &serde_json::Value) -> Result<u64, Box<RpcResponse>> {
    serde_json::from_value(params.clone())
        .map_err(|e| Box::new(invalid_params_response(id, e.to_string())))
}

fn to_rpc_block_info(block: kanari_rpc_api::BlockData) -> BlockInfo {
    let events = block
        .events
        .into_iter()
        .map(|e| RpcEvent {
            key: e.key,
            sequence_number: e.sequence_number,
            type_tag: e.type_tag,
            event_data: e.event_data,
        })
        .collect();

    BlockInfo {
        height: block.height,
        timestamp: block.timestamp,
        hash: block.hash,
        prev_hash: block.prev_hash,
        tx_count: block.tx_count,
        state_root: block.state_root,
        events,
    }
}

/// Handle get block request
pub async fn handle_get_block(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let height = match parse_height(request.id, &request.params) {
        Ok(height) => height,
        Err(response) => return *response,
    };

    match state.engine.get_block(height) {
        Some(block) => respond_with_serialize(request.id, to_rpc_block_info(block)),
        None => internal_error_response(request.id, "Block not found"),
    }
}

/// Handle get full block request
pub async fn handle_get_full_block(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let height = match parse_height(request.id, &request.params) {
        Ok(height) => height,
        Err(response) => return *response,
    };

    match state.engine.get_full_block(height) {
        Some(block) => respond_with_serialize(request.id, block),
        None => internal_error_response(request.id, "Block not found"),
    }
}

/// Handle get block height request
pub async fn handle_get_block_height(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let stats = state.engine.get_stats();
    RpcResponse {
        jsonrpc: "2.0".to_string(),
        result: Some(serde_json::json!(stats.height)),
        error: None,
        id: request.id,
    }
}

/// Handle get stats request
pub async fn handle_get_stats(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let stats = state.engine.get_stats();
    let blockchain_stats = kanari_rpc_api::BlockchainStats {
        height: stats.height,
        total_blocks: stats.total_blocks,
        total_transactions: stats.total_transactions,
        pending_transactions: stats.pending_transactions,
        total_owners: stats.total_owners,
        total_supply: stats.total_supply,
        state_root: stats.state_root,
    };
    respond_with_serialize(request.id, blockchain_stats)
}

pub async fn handle_get_smt_status(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let req = if request.params.is_null() || request.params == serde_json::json!([]) {
        GetSmtStatusRequest { audit: false }
    } else {
        match parse_params(request.id, &request.params) {
            Ok(req) => req,
            Err(response) => return *response,
        }
    };

    let _permit = if req.audit {
        match try_acquire_expensive_rpc(request.id) {
            Ok(permit) => Some(permit),
            Err(response) => return *response,
        }
    } else {
        None
    };

    match state.engine.smt_status(req.audit) {
        Ok(status) => respond_with_serialize(request.id, status),
        Err(error) => {
            internal_error_response(request.id, format!("SMT diagnostics failed: {error}"))
        }
    }
}

pub async fn handle_get_canonical_state_snapshot(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let _permit = match try_acquire_expensive_rpc(request.id) {
        Ok(permit) => permit,
        Err(response) => return *response,
    };

    let req = if request.params.is_null() || request.params == serde_json::json!([]) {
        GetCanonicalStateSnapshotRequest {
            limit: Some(MAX_CANONICAL_SNAPSHOT_ENTRIES),
            prefix: None,
        }
    } else {
        match parse_params(request.id, &request.params) {
            Ok(req) => req,
            Err(response) => return *response,
        }
    };

    let limit = req
        .limit
        .unwrap_or(MAX_CANONICAL_SNAPSHOT_ENTRIES)
        .min(MAX_CANONICAL_SNAPSHOT_ENTRIES);
    match state
        .engine
        .canonical_state_snapshot_response(Some(limit), req.prefix.as_deref())
    {
        Ok(snapshot) => respond_with_serialize(request.id, snapshot),
        Err(error) => internal_error_response(
            request.id,
            format!("Failed to read canonical state snapshot: {error:#}"),
        ),
    }
}

pub async fn handle_compare_canonical_state_snapshot(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let req: CompareCanonicalStateSnapshotRequest = match parse_params(request.id, &request.params)
    {
        Ok(req) => req,
        Err(response) => return *response,
    };

    let _permit = match try_acquire_expensive_rpc(request.id) {
        Ok(permit) => permit,
        Err(response) => return *response,
    };
    if req.entries.len() > MAX_CANONICAL_COMPARE_ENTRIES {
        return invalid_params_response(
            request.id,
            format!(
                "Canonical snapshot comparison accepts at most {MAX_CANONICAL_COMPARE_ENTRIES} entries"
            ),
        );
    }

    match state.engine.compare_canonical_state_snapshot(&req) {
        Ok(diff) => respond_with_serialize(request.id, diff),
        Err(error) => internal_error_response(
            request.id,
            format!("Failed to compare canonical state snapshot: {error:#}"),
        ),
    }
}
