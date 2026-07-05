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
use kanari_types::signed_object_transaction::SignedObjectTransaction;

use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::{
    balance::{
        handle_get_account, handle_get_all_balances, handle_get_token_balance, handle_list_tokens,
    },
    block::{handle_get_block, handle_get_block_height, handle_get_full_block, handle_get_stats},
    module::{
        handle_get_module, handle_get_object, handle_get_owned_objects, handle_list_modules,
        handle_verify_module,
    },
    nft::{handle_get_nfts_by_collection, handle_get_owned_nfts, handle_list_collections},
    transaction::{handle_get_transaction, handle_view_function},
};

pub mod balance;
pub mod block;
pub mod module;
pub mod nft;
pub mod object_transaction;
pub mod transaction;

type TransactionBroadcaster = Arc<dyn Fn(SignedObjectTransaction) -> Result<()> + Send + Sync>;

/// RPC server state
#[derive(Clone)]
pub struct RpcServerState {
    pub engine: Arc<BlockchainEngine>,
    transaction_broadcaster: Option<TransactionBroadcaster>,
}

impl RpcServerState {
    pub fn new(engine: Arc<BlockchainEngine>) -> Self {
        Self {
            engine,
            transaction_broadcaster: None,
        }
    }

    pub fn with_transaction_broadcaster(
        engine: Arc<BlockchainEngine>,
        broadcaster: impl Fn(SignedObjectTransaction) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            engine,
            transaction_broadcaster: Some(Arc::new(broadcaster)),
        }
    }

    pub fn broadcast_submitted_transaction(&self, signed_tx: SignedObjectTransaction) {
        if let Some(broadcaster) = &self.transaction_broadcaster
            && let Err(e) = broadcaster(signed_tx)
        {
            tracing::warn!("Failed to broadcast submitted object transaction: {}", e);
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
        .layer(cors)
        .with_state(state)
}

/// Handle RPC request
async fn handle_rpc(
    State(state): State<RpcServerState>,
    Json(request): Json<RpcRequest>,
) -> impl IntoResponse {
    info!("RPC request: method={}, id={}", request.method, request.id);

    let response = match request.method.as_str() {
        object_transaction::SUBMIT_OBJECT_TRANSACTION => {
            object_transaction::submit(&state, &request).await
        }
        object_transaction::GET_PENDING_OBJECT_TRANSACTIONS => {
            object_transaction::pending(&state, &request).await
        }
        object_transaction::CANCEL_OBJECT_TRANSACTION => {
            object_transaction::cancel(&state, &request).await
        }
        // Account & Balance; read-only object/account compatibility queries.
        methods::GET_ACCOUNT => handle_get_account(&state, &request).await,
        methods::GET_TOKEN_BALANCE => handle_get_token_balance(&state, &request).await,
        methods::LIST_TOKENS => handle_list_tokens(&state, &request).await,
        methods::GET_ALL_BALANCES => handle_get_all_balances(&state, &request).await,

        // Blocks & Transactions; submission is object-only.
        methods::GET_BLOCK => handle_get_block(&state, &request).await,
        methods::GET_FULL_BLOCK => handle_get_full_block(&state, &request).await,
        methods::GET_TRANSACTION => handle_get_transaction(&state, &request).await,
        methods::GET_ALL_TRANSACTIONS => {
            transaction::handle_get_all_transactions(&state, &request).await
        }
        methods::GET_BLOCK_HEIGHT => handle_get_block_height(&state, &request).await,
        methods::GET_STATS => handle_get_stats(&state, &request).await,

        // Health
        methods::HEALTH => handle_health(&state, &request).await,
        methods::GET_NETWORK_STATUS => handle_network_status(&state, &request).await,

        // Module operations
        methods::GET_MODULE => handle_get_module(&state, &request).await,
        methods::LIST_MODULES => handle_list_modules(&state, &request).await,
        methods::VERIFY_MODULE => handle_verify_module(&state, &request).await,

        // Function views only. Legacy call/publish/submit are deliberately absent.
        methods::VIEW_FUNCTION => handle_view_function(&state, &request).await,

        // Object queries
        methods::GET_OBJECT => handle_get_object(&state, &request).await,
        methods::GET_OWNED_OBJECTS => handle_get_owned_objects(&state, &request).await,

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
    broadcaster: impl Fn(SignedObjectTransaction) -> Result<()> + Send + Sync + 'static,
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
        uptime_seconds: 0,
        sync_status: "synced".to_string(),
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
