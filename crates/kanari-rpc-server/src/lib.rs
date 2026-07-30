// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Kanari RPC Server
//!
//! JSON-RPC server for Kanari blockchain using Axum framework

use anyhow::Result;
use axum::{
    Json, Router,
    extract::State,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use kanari_core::BlockchainEngine;
use kanari_rpc_api::*;
use kanari_types::transaction::SignedTransaction;
use kanari_types::{GAS_MODEL, GasConfig, effective_gas_price};

use std::{sync::Arc, time::Instant};
use tower_http::cors::{Any, CorsLayer};
use tower_http::{limit::RequestBodyLimitLayer, timeout::TimeoutLayer};

// VM-backed RPCs are bounded separately from ordinary reads. The outer router
// limit protects connections, while this permit prevents a caller from
// occupying all executor workers with verification/execution work at once.
static VM_RPC_PERMIT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);

fn is_vm_heavy_rpc(method: &str) -> bool {
    matches!(
        method,
        methods::VERIFY_MODULE
            | methods::BUILD_PUBLISH_MODULE
            | methods::BUILD_UPGRADE_MODULE
            | methods::PUBLISH_MODULE
            | methods::UPGRADE_MODULE
            | methods::BUILD_PUBLISH_PACKAGE
            | methods::BUILD_UPGRADE_PACKAGE
            | methods::PUBLISH_PACKAGE
            | methods::UPGRADE_PACKAGE
            | methods::BUILD_CALL_FUNCTION
            | methods::BUILD_TOKEN_TRANSFER
            | methods::CALL_FUNCTION
            | methods::VIEW_FUNCTION
    )
}
use tracing::{debug, info};

use crate::{
    balance::{
        handle_get_fungible_asset, handle_get_fungible_asset_holders, handle_get_owner,
        handle_get_owner_balances, handle_get_token_balance, handle_list_tokens,
    },
    block::{
        handle_compare_canonical_state_snapshot, handle_get_block, handle_get_block_height,
        handle_get_canonical_state_snapshot, handle_get_full_block, handle_get_smt_status,
        handle_get_stats,
    },
    module::{
        handle_get_module, handle_get_object, handle_get_object_by_ref, handle_get_objects,
        handle_get_objects_by_type, handle_get_owned_objects, handle_list_modules,
        handle_verify_module,
    },
    nft::{handle_get_nfts_by_collection, handle_get_owned_nfts, handle_list_collections},
    transaction::{
        handle_build_call_function, handle_build_native_coin_consolidation,
        handle_build_native_transfer, handle_build_publish_module, handle_build_publish_package,
        handle_build_token_transfer, handle_call_function, handle_get_fungible_asset_transactions,
        handle_get_transaction, handle_publish_module, handle_publish_package,
        handle_submit_object_transfer, handle_view_function,
    },
};

pub mod balance;
pub mod block;
pub mod module;
pub mod nft;
pub mod transaction;

type TransactionBroadcaster = Arc<dyn Fn(SignedTransaction) -> Result<()> + Send + Sync>;

/// RPC server state
#[derive(Clone)]
pub struct RpcServerState {
    pub engine: Arc<BlockchainEngine>,
    transaction_broadcaster: Option<TransactionBroadcaster>,
    started_at: Instant,
}

impl RpcServerState {
    pub fn new(engine: Arc<BlockchainEngine>) -> Self {
        Self {
            engine,
            transaction_broadcaster: None,
            started_at: Instant::now(),
        }
    }

    pub fn with_transaction_broadcaster(
        engine: Arc<BlockchainEngine>,
        broadcaster: impl Fn(SignedTransaction) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            engine,
            transaction_broadcaster: Some(Arc::new(broadcaster)),
            started_at: Instant::now(),
        }
    }

    pub fn broadcast_submitted_transaction(&self, signed_tx: SignedTransaction) {
        if let Some(broadcaster) = &self.transaction_broadcaster
            && let Err(e) = broadcaster(signed_tx)
        {
            tracing::warn!("Failed to broadcast submitted transaction: {}", e);
        }
    }
}

fn error_response(id: u64, error: RpcError) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0".to_string(),
        result: None,
        error: Some(error),
        id,
    }
}

fn respond_with_value(id: u64, val: serde_json::Value) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0".to_string(),
        result: Some(val),
        error: None,
        id,
    }
}

pub(crate) fn invalid_params_response(id: u64, message: impl Into<String>) -> RpcResponse {
    error_response(id, RpcError::invalid_params(message.into()))
}

pub(crate) fn internal_error_response(id: u64, message: impl Into<String>) -> RpcResponse {
    error_response(id, RpcError::internal_error(message.into()))
}

pub(crate) fn parse_params<T: serde::de::DeserializeOwned>(
    id: u64,
    params: &serde_json::Value,
) -> Result<T, Box<RpcResponse>> {
    serde_json::from_value(params.clone())
        .map_err(|e| Box::new(invalid_params_response(id, e.to_string())))
}

pub(crate) fn parse_labeled_params<T: serde::de::DeserializeOwned>(
    id: u64,
    params: &serde_json::Value,
    label: &str,
) -> Result<T, Box<RpcResponse>> {
    serde_json::from_value(params.clone()).map_err(|e| {
        Box::new(invalid_params_response(
            id,
            format!("Invalid {}: {}", label, e),
        ))
    })
}

pub(crate) fn first_array_param(
    id: u64,
    params: &serde_json::Value,
) -> Result<&serde_json::Value, Box<RpcResponse>> {
    let arr = params
        .as_array()
        .ok_or_else(|| Box::new(invalid_params_response(id, "Expected array params")))?;
    arr.first()
        .ok_or_else(|| Box::new(invalid_params_response(id, "Empty params array")))
}

fn respond_with_serialize<T: serde::Serialize>(id: u64, v: T) -> RpcResponse {
    match serde_json::to_value(v) {
        Ok(val) => respond_with_value(id, val),
        Err(e) => internal_error_response(id, format!("Serialization failed: {}", e)),
    }
}

/// Create RPC server router
pub fn create_router(state: RpcServerState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/", post(handle_rpc))
        .route("/rpc", post(handle_rpc))
        .route("/metrics", get(handle_metrics))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
        .layer(tower::limit::ConcurrencyLimitLayer::new(128))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            std::time::Duration::from_secs(30),
        ))
        .layer(cors)
        .with_state(state)
}

/// Handle RPC request
async fn handle_rpc(
    State(state): State<RpcServerState>,
    Json(request): Json<RpcRequest>,
) -> impl IntoResponse {
    debug!("RPC request: method={}, id={}", request.method, request.id);

    let _vm_permit = if is_vm_heavy_rpc(&request.method) {
        match VM_RPC_PERMIT.try_acquire() {
            Ok(permit) => Some(permit),
            Err(_) => {
                return (
                    StatusCode::OK,
                    Json(invalid_params_response(
                        request.id,
                        "VM RPC capacity is temporarily exhausted; retry later",
                    )),
                );
            }
        }
    } else {
        None
    };

    let response = match request.method.as_str() {
        // Account & Balance
        methods::GET_OWNER => handle_get_owner(&state, &request).await,
        methods::GET_TOKEN_BALANCE => handle_get_token_balance(&state, &request).await,
        methods::LIST_TOKENS => handle_list_tokens(&state, &request).await,
        methods::GET_OWNER_BALANCES => handle_get_owner_balances(&state, &request).await,
        methods::GET_FUNGIBLE_ASSET => handle_get_fungible_asset(&state, &request).await,
        methods::GET_FUNGIBLE_ASSET_HOLDERS => {
            handle_get_fungible_asset_holders(&state, &request).await
        }
        methods::GET_FUNGIBLE_ASSET_TRANSACTIONS => {
            handle_get_fungible_asset_transactions(&state, &request).await
        }

        // Blocks & Transactions
        methods::GET_BLOCK => handle_get_block(&state, &request).await,
        methods::GET_FULL_BLOCK => handle_get_full_block(&state, &request).await,
        methods::GET_TRANSACTION => handle_get_transaction(&state, &request).await,
        methods::GET_ALL_TRANSACTIONS => {
            transaction::handle_get_all_transactions(&state, &request).await
        }
        methods::GET_BLOCK_HEIGHT => handle_get_block_height(&state, &request).await,
        methods::GET_STATS => handle_get_stats(&state, &request).await,
        methods::GET_SMT_STATUS => handle_get_smt_status(&state, &request).await,
        methods::GET_CANONICAL_STATE_SNAPSHOT => {
            handle_get_canonical_state_snapshot(&state, &request).await
        }
        methods::COMPARE_CANONICAL_STATE_SNAPSHOT => {
            handle_compare_canonical_state_snapshot(&state, &request).await
        }
        methods::BUILD_NATIVE_TRANSFER => handle_build_native_transfer(&state, &request).await,
        methods::BUILD_NATIVE_COIN_CONSOLIDATION => {
            handle_build_native_coin_consolidation(&state, &request).await
        }
        methods::SUBMIT_OBJECT_TRANSFER => handle_submit_object_transfer(&state, &request).await,

        // Health
        methods::HEALTH => handle_health(&state, &request).await,
        methods::GET_GAS_INFO => handle_gas_info(&request).await,
        methods::GET_NETWORK_STATUS => handle_network_status(&state, &request).await,

        // Module operations
        methods::BUILD_PUBLISH_MODULE | methods::BUILD_UPGRADE_MODULE => {
            handle_build_publish_module(&state, &request).await
        }
        methods::PUBLISH_MODULE | methods::UPGRADE_MODULE => {
            handle_publish_module(&state, &request).await
        }
        methods::BUILD_PUBLISH_PACKAGE | methods::BUILD_UPGRADE_PACKAGE => {
            handle_build_publish_package(&state, &request).await
        }
        methods::PUBLISH_PACKAGE | methods::UPGRADE_PACKAGE => {
            handle_publish_package(&state, &request).await
        }
        methods::GET_MODULE => handle_get_module(&state, &request).await,
        methods::LIST_MODULES => handle_list_modules(&state, &request).await,
        methods::VERIFY_MODULE => handle_verify_module(&state, &request).await,

        // Function calls
        methods::BUILD_CALL_FUNCTION => handle_build_call_function(&state, &request).await,
        methods::BUILD_TOKEN_TRANSFER => handle_build_token_transfer(&state, &request).await,
        methods::CALL_FUNCTION => handle_call_function(&state, &request).await,
        methods::VIEW_FUNCTION => handle_view_function(&state, &request).await,

        // Object queries
        methods::GET_OBJECT => handle_get_object(&state, &request).await,
        methods::GET_OBJECT_BY_REF => handle_get_object_by_ref(&state, &request).await,
        methods::GET_OBJECTS => handle_get_objects(&state, &request).await,
        methods::GET_OWNED_OBJECTS => handle_get_owned_objects(&state, &request).await,
        methods::GET_OBJECTS_BY_TYPE => handle_get_objects_by_type(&state, &request).await,

        // NFT queries
        methods::GET_OWNED_NFTS => handle_get_owned_nfts(&state, &request).await,

        // collection queries
        methods::LIST_COLLECTIONS => handle_list_collections(&state, &request).await,
        methods::GET_NFTS_BY_COLLECTION => handle_get_nfts_by_collection(&state, &request).await,

        _ => error_response(request.id, RpcError::method_not_found(&request.method)),
    };

    (StatusCode::OK, Json(response))
}

async fn handle_metrics(State(state): State<RpcServerState>) -> impl IntoResponse {
    match state.engine.export_consensus_metrics_prometheus() {
        Ok(metrics) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            metrics,
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            format!("failed to export metrics: {error}"),
        )
            .into_response(),
    }
}

/// Start RPC server
pub async fn start_server(engine: Arc<BlockchainEngine>, addr: &str) -> Result<()> {
    let state = RpcServerState::new(engine);
    let app = create_router(state);

    info!("Starting RPC server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

pub async fn start_server_with_transaction_broadcaster(
    engine: Arc<BlockchainEngine>,
    addr: &str,
    broadcaster: impl Fn(SignedTransaction) -> Result<()> + Send + Sync + 'static,
) -> Result<()> {
    let state = RpcServerState::with_transaction_broadcaster(engine, broadcaster);
    let app = create_router(state);

    info!("Starting RPC server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Handle health check
async fn handle_health(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let report = state.engine.runtime_health_report();

    let health = HealthStatus {
        status: report.status().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        sync_status: if report.guards.network == "local" {
            "local".to_string()
        } else {
            "unknown".to_string()
        },
        network: report.guards.network,
        supply_invariants_ok: report.supply_invariants_ok,
        supply_invariant_error: report.supply_invariant_error,
        fail_fast_enabled: report.guards.fail_fast_supply_enabled,
        strict_persistence_required: report.guards.strict_persistence_required,
        strict_checkpoint_roots: report.guards.strict_checkpoint_roots,
        persistent_storage_available: report.guards.persistent_storage_available,
    };

    respond_with_serialize(request.id, health)
}

async fn handle_gas_info(request: &RpcRequest) -> RpcResponse {
    let config = GasConfig::default();
    let requested = config.default_transaction_gas_price();
    respond_with_value(
        request.id,
        serde_json::json!({
            "model": GAS_MODEL,
            "requested_gas_price": requested,
            "effective_gas_price": effective_gas_price(requested),
            "minimum_gas_price": config.min_gas_price,
            "gas_limit": config.max_gas_per_tx,
            "storage_price_per_byte": config.storage_price_per_byte,
            "storage_rebate_rate": config.storage_rebate_rate,
        }),
    )
}

async fn handle_network_status(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let local_authority_id = state.engine.authority_id().to_string();
    let authorities = state
        .engine
        .authorities()
        .iter()
        .map(|authority_id| NetworkAuthorityStatus {
            authority_id: authority_id.clone(),
            local: authority_id == &local_authority_id,
        })
        .collect::<Vec<_>>();

    let status = NetworkStatus {
        local_authority_id,
        authority_count: authorities.len(),
        authorities,
    };

    respond_with_serialize(request.id, status)
}

#[cfg(test)]
#[path = "../tests/unit/lib_tests.rs"]
mod tests;
