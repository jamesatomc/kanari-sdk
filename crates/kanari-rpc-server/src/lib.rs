// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Kanari RPC Server
//!
//! JSON-RPC server for Kanari blockchain using Axum framework

use anyhow::Result;
use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use kanari_core::BlockchainEngine;
use kanari_indexer::IndexerReader;
use kanari_rpc_api::*;

use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

use crate::{
    balance::{
        handle_get_account, handle_get_all_balances, handle_get_balance, handle_get_token_balance,
        handle_list_tokens,
    },
    block::{
        handle_get_block, handle_get_block_height, handle_get_full_block, handle_get_stats,
        handle_produce_block,
    },
    module::{handle_get_module, handle_get_object, handle_list_modules, handle_verify_module},
    nft::{handle_get_nfts_by_collection, handle_get_owned_nfts, handle_list_collections},
    transaction::{
        handle_call_function, handle_get_transaction, handle_publish_module,
        handle_submit_transaction,
    },
};

pub mod balance;
pub mod block;
pub mod module;
pub mod nft;
pub mod transaction;

/// RPC server state
#[derive(Clone)]
pub struct RpcServerState {
    pub engine: Arc<BlockchainEngine>,
    pub indexer: Arc<IndexerReader>,
}

impl RpcServerState {
    pub fn new(engine: Arc<BlockchainEngine>, indexer: Arc<IndexerReader>) -> Self {
        Self { engine, indexer }
    }
}

// Helper to safely serialize response values and avoid panics from `to_value().unwrap()`
fn respond_with_value(id: u64, val: serde_json::Value) -> RpcResponse {
    RpcResponse {
        jsonrpc: "2.0".to_string(),
        result: Some(val),
        error: None,
        id,
    }
}

fn respond_with_serialize<T: serde::Serialize>(id: u64, v: T) -> RpcResponse {
    match serde_json::to_value(v) {
        Ok(val) => respond_with_value(id, val),
        Err(e) => RpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(RpcError::internal_error(format!(
                "Serialization failed: {}",
                e
            ))),
            id,
        },
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
        // Account & Balance
        methods::GET_ACCOUNT => handle_get_account(&state, &request).await,
        methods::GET_BALANCE => handle_get_balance(&state, &request).await,
        methods::GET_TOKEN_BALANCE => handle_get_token_balance(&state, &request).await,
        methods::LIST_TOKENS => handle_list_tokens(&state, &request).await,
        methods::GET_ALL_BALANCES => handle_get_all_balances(&state, &request).await,

        // Blocks & Transactions
        methods::GET_BLOCK => handle_get_block(&state, &request).await,
        methods::GET_FULL_BLOCK => handle_get_full_block(&state, &request).await,
        methods::GET_TRANSACTION => handle_get_transaction(&state, &request).await,
        methods::GET_ALL_TRANSACTIONS => {
            transaction::handle_get_all_transactions(&state, &request).await
        }
        methods::PRODUCE_BLOCK => handle_produce_block(&state, &request).await,
        methods::GET_BLOCK_HEIGHT => handle_get_block_height(&state, &request).await,
        methods::GET_STATS => handle_get_stats(&state, &request).await,
        methods::SUBMIT_TRANSACTION => handle_submit_transaction(&state, &request).await,

        // Health
        methods::HEALTH => handle_health(&state, &request).await,

        // Module operations
        methods::PUBLISH_MODULE => handle_publish_module(&state, &request).await,
        methods::GET_MODULE => handle_get_module(&state, &request).await,
        methods::LIST_MODULES => handle_list_modules(&state, &request).await,
        methods::VERIFY_MODULE => handle_verify_module(&state, &request).await,

        // Function calls
        methods::CALL_FUNCTION => handle_call_function(&state, &request).await,

        // Object queries
        methods::GET_OBJECT => handle_get_object(&state, &request).await,

        // NFT queries
        methods::GET_OWNED_NFTS => handle_get_owned_nfts(&state, &request).await,

        // collection queries
        methods::LIST_COLLECTIONS => handle_list_collections(&state, &request).await,
        methods::GET_NFTS_BY_COLLECTION => handle_get_nfts_by_collection(&state, &request).await,

        _ => RpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(RpcError::method_not_found(&request.method)),
            id: request.id,
        },
    };

    (StatusCode::OK, Json(response))
}

/// Start RPC server
pub async fn start_server(
    engine: Arc<BlockchainEngine>,
    indexer: Arc<IndexerReader>,
    addr: &str,
) -> Result<()> {
    let state = RpcServerState::new(engine, indexer);
    let app = create_router(state);

    info!("Starting RPC server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Handle health check
async fn handle_health(_state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let health = HealthStatus {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: 0, // TODO: Track actual uptime
        sync_status: "synced".to_string(),
    };

    respond_with_serialize(request.id, health)
}
