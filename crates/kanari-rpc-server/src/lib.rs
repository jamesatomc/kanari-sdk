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
    transaction::{
        handle_call_function, handle_get_transaction, handle_publish_module,
        handle_submit_transaction, handle_view_function,
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
        broadcaster: impl Fn(SignedTransaction) -> Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            engine,
            transaction_broadcaster: Some(Arc::new(broadcaster)),
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
        methods::GET_BLOCK_HEIGHT => handle_get_block_height(&state, &request).await,
        methods::GET_STATS => handle_get_stats(&state, &request).await,
        methods::SUBMIT_TRANSACTION => handle_submit_transaction(&state, &request).await,

        // Health
        methods::HEALTH => handle_health(&state, &request).await,
        methods::GET_NETWORK_STATUS => handle_network_status(&state, &request).await,

        // Module operations
        methods::PUBLISH_MODULE => handle_publish_module(&state, &request).await,
        methods::GET_MODULE => handle_get_module(&state, &request).await,
        methods::LIST_MODULES => handle_list_modules(&state, &request).await,
        methods::VERIFY_MODULE => handle_verify_module(&state, &request).await,

        // Function calls
        methods::CALL_FUNCTION => handle_call_function(&state, &request).await,
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
        uptime_seconds: 0, // TODO: Track actual uptime
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
        response::Response,
    };
    use kanari_core::kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
    use kanari_core::kanari_move_runtime_v1::state::{Account, StateManager};
    use kanari_crypto::keys::{CurveType, generate_keypair};
    use kanari_rpc_api::methods;
    use kanari_types::balance::BalanceRecord;
    use kanari_types::kanari::KANARI_TOKEN_TYPE;
    use kanari_types::transaction::{SignedTransaction, Transaction};
    use move_core_types::account_address::AccountAddress;
    use std::sync::{Mutex, OnceLock};
    use tower::util::ServiceExt;

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    fn coin_data(amount: u64) -> Vec<u8> {
        let mut data = vec![0u8; 40];
        data[32..40].copy_from_slice(&amount.to_le_bytes());
        data
    }

    fn hex_ends_with(value: &serde_json::Value, suffix: &str) -> bool {
        value
            .as_str()
            .map(|s| {
                s.trim_start_matches("0x")
                    .ends_with(suffix.trim_start_matches("0x"))
            })
            .unwrap_or(false)
    }

    fn seed_runtime_state(state: &mut StateManager) {
        let owner = AccountAddress::from_hex_literal("0x1111").unwrap();
        let coin_type = format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE);

        let mut cs = ChangeSet::new();
        cs.add_treasury(owner, KANARI_TOKEN_TYPE.to_string(), 500);
        cs.created_objects.push((
            "0xaaa1".to_string(),
            CreatedObject {
                owner,
                uid: None,
                id: None,
                type_: coin_type.clone(),
                data: coin_data(300),
                version: 1,
            },
        ));
        cs.created_objects.push((
            "0xaaa2".to_string(),
            CreatedObject {
                owner,
                uid: None,
                id: None,
                type_: coin_type,
                data: coin_data(200),
                version: 2,
            },
        ));
        state.apply_changeset(&cs).unwrap();
        let mut account = state
            .get_account(&owner)
            .unwrap_or_else(|| Account::new(owner));
        account.set_token_balance(KANARI_TOKEN_TYPE.to_string(), BalanceRecord::new(500));
        state.save_account(&account).unwrap();
        assert_eq!(
            state
                .get_account(&owner)
                .unwrap()
                .get_token_balance(KANARI_TOKEN_TYPE),
            500
        );
    }

    fn build_test_router() -> Router {
        let mut engine = BlockchainEngine::new_in_memory().unwrap();
        engine.set_authorities(
            "0x1".to_string(),
            vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()],
        );
        let engine = Arc::new(engine);
        {
            let mut state = engine.state_write();
            seed_runtime_state(&mut state);
        }
        create_router(RpcServerState::new(engine))
    }

    fn fund_test_account(engine: &BlockchainEngine, address: &str, balance: u64) {
        let owner = AccountAddress::from_hex_literal(address).unwrap();
        let mut account = Account::with_native_balance(owner, balance);
        account.set_token_balance(KANARI_TOKEN_TYPE.to_string(), BalanceRecord::new(balance));
        engine
            .state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .save_account(&account)
            .unwrap();
    }

    async fn rpc_call(
        app: Router,
        method: &str,
        params: serde_json::Value,
        id: u64,
    ) -> serde_json::Value {
        let request = Request::builder()
            .method("POST")
            .uri("/rpc")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": params,
                    "id": id
                })
                .to_string(),
            ))
            .unwrap();

        let response: Response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            json.get("error").is_none() || json.get("error").unwrap().is_null(),
            "unexpected rpc error: {json}"
        );
        json["result"].clone()
    }

    async fn rpc_call_response(
        app: Router,
        method: &str,
        params: serde_json::Value,
        id: u64,
    ) -> serde_json::Value {
        let request = Request::builder()
            .method("POST")
            .uri("/rpc")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": method,
                    "params": params,
                    "id": id
                })
                .to_string(),
            ))
            .unwrap();

        let response: Response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn rpc_runtime_backed_endpoints_smoke() {
        let guard = test_guard();
        let app = build_test_router();

        let health = rpc_call(app.clone(), methods::HEALTH, serde_json::json!([]), 1).await;
        assert!(health["status"].as_str().is_some());
        assert!(health["persistent_storage_available"].is_boolean());
        assert!(health["supply_invariants_ok"].is_boolean());

        let network_status = rpc_call(
            app.clone(),
            methods::GET_NETWORK_STATUS,
            serde_json::json!([]),
            10,
        )
        .await;
        assert_eq!(network_status["local_authority_id"], "0x1");
        assert_eq!(network_status["authority_count"], 3);
        assert_eq!(network_status["authorities"][0]["authority_id"], "0x1");
        assert_eq!(network_status["authorities"][0]["local"], true);

        let stats = rpc_call(app.clone(), methods::GET_STATS, serde_json::json!([]), 2).await;
        assert_eq!(stats["total_supply"], 500);
        assert!(stats["total_accounts"].as_u64().is_some());

        let height = rpc_call(
            app.clone(),
            methods::GET_BLOCK_HEIGHT,
            serde_json::json!([]),
            3,
        )
        .await;
        assert!(height.as_u64().is_some());

        let account = rpc_call(
            app.clone(),
            methods::GET_ACCOUNT,
            serde_json::json!("0x1111"),
            4,
        )
        .await;
        assert!(hex_ends_with(&account["address"], "1111"));
        assert_eq!(account["token_balances"][KANARI_TOKEN_TYPE], 500);
        assert_eq!(account["owned_objects"].as_array().unwrap().len(), 1);

        let all_balances = rpc_call(
            app.clone(),
            methods::GET_ALL_BALANCES,
            serde_json::json!({ "address": "0x1111" }),
            5,
        )
        .await;
        let balances = all_balances["balances"].as_array().unwrap();
        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0]["token_type"], KANARI_TOKEN_TYPE);
        assert_eq!(balances[0]["balance"], 500);

        let tokens = rpc_call(app.clone(), methods::LIST_TOKENS, serde_json::json!([]), 6).await;
        let token_list = tokens.as_array().unwrap();
        assert!(!token_list.is_empty());
        assert!(token_list.iter().any(|token| {
            token["token_type"]
                .as_str()
                .map(|ty| ty.contains("KANARI"))
                .unwrap_or(false)
        }));

        let object = rpc_call(
            app.clone(),
            methods::GET_OBJECT,
            serde_json::json!({ "object_id": "0xaaa1" }),
            7,
        )
        .await;
        assert!(hex_ends_with(&object["id"], "aaa1"));
        assert_eq!(object["version"], 1);

        let owned = rpc_call(
            app,
            methods::GET_OWNED_OBJECTS,
            serde_json::json!({
                "owner": "0x1111",
                "object_type": "::coin::Coin<"
            }),
            8,
        )
        .await;
        let objects = owned["objects"].as_array().unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0]["version"], 2);
        drop(guard);
    }

    #[tokio::test]
    async fn metrics_endpoint_exports_prometheus_text() {
        let guard = test_guard();
        let app = build_test_router();

        let request = Request::builder()
            .method(Method::GET)
            .uri("/metrics")
            .body(Body::empty())
            .unwrap();

        let response: Response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(content_type.starts_with("text/plain"));

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(text.contains("# HELP dag_vertices_created_total"));
        assert!(text.contains("# TYPE dag_active_vertices gauge"));
        drop(guard);
    }

    #[tokio::test]
    async fn submitted_transaction_hash_is_queryable() {
        let guard = test_guard();
        let app = build_test_router();

        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        let sender_tagged = sender.tagged_address();
        let recipient_address = recipient.address.clone();

        let mut transaction =
            Transaction::new_transfer(sender_tagged.clone(), recipient_address.clone(), 1, 0);
        if let Transaction::ExecuteFunction {
            gas_limit,
            gas_price,
            ..
        } = &mut transaction
        {
            *gas_limit = 1_000_000;
            *gas_price = 1;
        }
        let mut signed_tx = SignedTransaction::new(transaction);
        signed_tx
            .sign(&sender.private_key, sender.curve_type)
            .unwrap();

        let submitted = rpc_call(
            app.clone(),
            methods::SUBMIT_TRANSACTION,
            serde_json::json!({
                "sender": sender_tagged,
                "recipient": recipient_address,
                "amount": 1,
                "gas_limit": 1_000_000,
                "gas_price": 1,
                "sequence_number": 0,
                "signature": signed_tx.signature,
            }),
            10,
        )
        .await;
        let hash = submitted["hash"].as_str().unwrap().to_string();

        let fetched = rpc_call(
            app.clone(),
            methods::GET_TRANSACTION,
            serde_json::json!({ "hash": hash }),
            11,
        )
        .await;
        assert_eq!(fetched["hash"], format!("0x{}", hash));
        assert_eq!(fetched["status"], "pending");

        let all = rpc_call(
            app,
            methods::GET_ALL_TRANSACTIONS,
            serde_json::json!({ "limit": 10 }),
            12,
        )
        .await;
        assert!(all.as_array().unwrap().iter().any(|tx| {
            tx["hash"]
                .as_str()
                .map(|candidate| candidate == format!("0x{}", hash))
                .unwrap_or(false)
        }));

        drop(guard);
    }

    #[tokio::test]
    async fn submit_transaction_can_execute_immediately() {
        let guard = test_guard();
        let mut engine = BlockchainEngine::new_in_memory().unwrap();
        engine.set_authorities(
            "0x1".to_string(),
            vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()],
        );

        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        fund_test_account(&engine, &sender.address, 2_000_000);

        let sender_tagged = sender.tagged_address();
        let recipient_address = recipient.address.clone();
        let transaction = Transaction::new_transfer_with_gas(
            sender_tagged.clone(),
            recipient_address.clone(),
            1,
            0,
            1_000_000,
            1,
        );
        let mut signed_tx = SignedTransaction::new(transaction);
        signed_tx
            .sign(&sender.private_key, sender.curve_type)
            .unwrap();

        let app = create_router(RpcServerState::new(Arc::new(engine)));
        let submitted = rpc_call(
            app,
            methods::SUBMIT_TRANSACTION,
            serde_json::json!({
                "sender": sender_tagged,
                "recipient": recipient_address,
                "amount": 1,
                "gas_limit": 1_000_000,
                "gas_price": 1,
                "sequence_number": 0,
                "signature": signed_tx.signature,
                "execute_immediate": true,
            }),
            20,
        )
        .await;

        assert_eq!(submitted["status"], "executed");
        assert_eq!(submitted["action"], "submit");
        assert!(submitted["changeset"].is_object());

        drop(guard);
    }

    #[tokio::test]
    async fn submit_transaction_rejects_missing_signature() {
        let guard = test_guard();
        let app = build_test_router();

        let response = rpc_call_response(
            app,
            methods::SUBMIT_TRANSACTION,
            serde_json::json!({
                "sender": "0x1111",
                "recipient": "0x2222",
                "amount": 1,
                "gas_limit": 1_000_000,
                "gas_price": 1,
                "sequence_number": 0,
                "execute_immediate": true,
            }),
            30,
        )
        .await;

        assert_eq!(response["error"]["code"], -32602);
        assert_eq!(response["error"]["message"], "Missing or empty signature");
        assert!(response["result"].is_null());

        drop(guard);
    }
}
