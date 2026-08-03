use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
    response::Response,
};
use kanari_core::kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject};
use kanari_core::kanari_move_runtime_v1::state::{OwnerState, StateManager};
use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_rpc_api::methods;
use kanari_types::balance::BalanceRecord;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::gas_coin::GAS_COIN;
use kanari_types::transaction::{SignedTransaction, Transaction};
use move_core_types::account_address::AccountAddress;
use std::sync::OnceLock;
use tokio::sync::{Mutex, MutexGuard};
use tower::util::ServiceExt;

async fn test_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().await
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
    let owner = AccountAddress::from_hex_literal("0x1111").invariant("valid owner address");
    let coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);

    let mut cs = ChangeSet::new();
    cs.add_treasury(owner, GAS_COIN.to_string(), state.total_supply + 500);
    cs.created_objects.push((
        "0xaaa1".to_string(),
        CreatedObject {
            owner,
            owner_kind: kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                owner.to_hex_literal(),
            ),
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
            owner_kind: kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                owner.to_hex_literal(),
            ),
            uid: None,
            id: None,
            type_: coin_type,
            data: coin_data(200),
            version: 2,
        },
    ));
    state
        .apply_changeset(&cs)
        .invariant("seed runtime state changeset");
    let mut owner_state = state
        .get_owner_state(&owner)
        .unwrap_or_else(|| OwnerState::new(owner));
    owner_state.set_token_balance(GAS_COIN.to_string(), BalanceRecord::new(500));
    state
        .save_owner_state(&owner_state)
        .invariant("seed runtime state owner state");
    assert_eq!(
        state
            .get_owner_state(&owner)
            .invariant("seeded owner state")
            .get_token_balance(GAS_COIN),
        500
    );
}

fn build_test_router() -> Router {
    let mut engine = BlockchainEngine::new_in_memory().invariant("in-memory engine");
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
        .invariant("build rpc request");

    let response: Response = app.oneshot(request).await.invariant("rpc response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .invariant("rpc body bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).invariant("rpc json body");
    assert!(
        json.get("error").is_none() || json.get("error").invariant("rpc error field").is_null(),
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
        .invariant("build rpc request");

    let response: Response = app.oneshot(request).await.invariant("rpc response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .invariant("rpc body bytes");
    serde_json::from_slice(&body).invariant("rpc json body")
}

async fn raw_rpc_request(app: Router, body: impl Into<Body>) -> Response {
    let request = Request::builder()
        .method("POST")
        .uri("/rpc")
        .header("content-type", "application/json")
        .body(body.into())
        .invariant("build raw rpc request");

    app.oneshot(request).await.invariant("raw rpc response")
}

#[tokio::test]
async fn rpc_runtime_backed_endpoints_smoke() {
    let guard = test_guard().await;
    let app = build_test_router();

    let health = rpc_call(app.clone(), methods::HEALTH, serde_json::json!([]), 1).await;
    assert!(health["status"].as_str().is_some());
    assert!(health["persistent_storage_available"].is_boolean());
    assert!(health["supply_invariants_ok"].is_boolean());

    let gas_info = rpc_call(app.clone(), methods::GET_GAS_INFO, serde_json::json!([]), 3).await;
    assert_eq!(gas_info["model"], kanari_types::GAS_MODEL);
    assert!(gas_info["requested_gas_price"].is_u64());
    assert!(gas_info["effective_gas_price"].is_u64());

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
    assert!(stats["total_supply"].as_u64().invariant("total supply") >= 500);
    assert!(stats["total_owners"].as_u64().is_some());

    let smt_status = rpc_call(
        app.clone(),
        methods::GET_SMT_STATUS,
        serde_json::json!([]),
        20,
    )
    .await;
    assert_eq!(smt_status["enabled"], false);
    assert_eq!(smt_status["audit_requested"], false);
    assert_eq!(smt_status["audit_performed"], false);
    assert!(smt_status["effective_root"].as_str().is_some());
    assert!(smt_status["runtime_schema_version"].as_u64().is_some());

    let smt_audit = rpc_call_response(
        app.clone(),
        methods::GET_SMT_STATUS,
        serde_json::json!({ "audit": true }),
        21,
    )
    .await;
    assert!(smt_audit["error"].is_null());
    assert_eq!(smt_audit["result"]["audit_requested"], true);
    assert!(smt_audit["result"]["audit_performed"].is_boolean());

    let height = rpc_call(
        app.clone(),
        methods::GET_BLOCK_HEIGHT,
        serde_json::json!([]),
        3,
    )
    .await;
    assert!(height.as_u64().is_some());

    let owner = rpc_call(
        app.clone(),
        methods::GET_OWNER,
        serde_json::json!("0x1111"),
        4,
    )
    .await;
    assert!(hex_ends_with(&owner["owner"], "1111"));
    assert_eq!(owner["balances"][GAS_COIN], 500);
    assert!(
        !owner["owned_objects"]
            .as_array()
            .invariant("json array")
            .is_empty()
    );

    let all_balances = rpc_call(
        app.clone(),
        methods::GET_OWNER_BALANCES,
        serde_json::json!({ "owner": "0x1111" }),
        5,
    )
    .await;
    let balances = all_balances["balances"].as_array().invariant("json array");
    assert_eq!(balances.len(), 1);
    assert_eq!(balances[0]["token_type"], GAS_COIN);
    assert_eq!(balances[0]["balance"], 500);

    let tokens = rpc_call(app.clone(), methods::LIST_TOKENS, serde_json::json!([]), 6).await;
    let token_list = tokens.as_array().invariant("json array");
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

    let snapshot = rpc_call_response(
        app.clone(),
        methods::GET_CANONICAL_STATE_SNAPSHOT,
        serde_json::json!({}),
        70,
    )
    .await;
    assert!(snapshot["error"].is_null());
    assert!(snapshot["result"]["entries"].is_array());

    let diff = rpc_call_response(
        app.clone(),
        methods::COMPARE_CANONICAL_STATE_SNAPSHOT,
        serde_json::json!({
            "entries": []
        }),
        71,
    )
    .await;
    assert!(diff["error"].is_null());
    assert!(diff["result"].is_object());

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
    let objects = owned["objects"].as_array().invariant("json array");
    assert!(!objects.is_empty());
    assert!(
        objects
            .iter()
            .all(|object| object["version"].as_u64().is_some())
    );
    drop(guard);
}

#[tokio::test]
async fn rpc_adversarial_inputs_are_rejected_without_server_error() {
    let guard = test_guard().await;
    let app = build_test_router();

    let malformed = raw_rpc_request(
        app.clone(),
        Body::from(r#"{"jsonrpc":"2.0","method":"kanari_getStats","params":"#),
    )
    .await;
    assert!(
        malformed.status().is_client_error(),
        "malformed JSON should be rejected before handler, got {}",
        malformed.status()
    );

    let unknown = rpc_call_response(
        app.clone(),
        "kanari_debugUnlockEverything",
        serde_json::json!({ "admin": true }),
        9_001,
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32601);
    assert!(unknown["result"].is_null());

    let invalid_params = rpc_call_response(
        app.clone(),
        methods::GET_OBJECT,
        serde_json::json!({ "object_id": ["not", "a", "string"] }),
        9_002,
    )
    .await;
    assert_eq!(invalid_params["error"]["code"], -32602);
    assert!(invalid_params["result"].is_null());

    let oversized_params = rpc_call_response(
        app.clone(),
        methods::GET_STATS,
        serde_json::json!({ "blob": "x".repeat(1024 * 1024) }),
        9_003,
    )
    .await;
    assert!(
        oversized_params["error"].is_null() || oversized_params["error"]["code"].is_i64(),
        "oversized but valid JSON must not panic or produce malformed response: {oversized_params}"
    );

    let get_on_rpc = Request::builder()
        .method(Method::GET)
        .uri("/rpc")
        .body(Body::empty())
        .invariant("build GET /rpc request");
    let get_response: Response = app.oneshot(get_on_rpc).await.invariant("GET /rpc response");
    assert!(
        get_response.status().is_client_error(),
        "GET /rpc should not be accepted as JSON-RPC POST"
    );

    drop(guard);
}

#[tokio::test]
async fn metrics_endpoint_exports_prometheus_text() {
    let guard = test_guard().await;
    let app = build_test_router();

    let request = Request::builder()
        .method(Method::GET)
        .uri("/metrics")
        .body(Body::empty())
        .invariant("build metrics request");

    let response: Response = app.oneshot(request).await.invariant("rpc response");
    assert_eq!(response.status(), StatusCode::OK);
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(content_type.starts_with("text/plain"));

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .invariant("rpc body bytes");
    let text = String::from_utf8(body.to_vec()).invariant("metrics text utf8");
    assert!(text.contains("# HELP dropped_invalid_pending_tx_total"));
    assert!(text.contains("dropped_invalid_pending_tx_total"));
    drop(guard);
}

#[tokio::test]
async fn submitted_transaction_hash_is_queryable() {
    let guard = test_guard().await;
    let app = build_test_router();

    let sender = generate_keypair(CurveType::Ed25519).invariant("sender keypair");
    let recipient = generate_keypair(CurveType::Ed25519).invariant("recipient keypair");
    let sender_tagged = sender.tagged_address();
    let recipient_address =
        move_core_types::account_address::AccountAddress::from_hex_literal(&recipient.address)
            .invariant("recipient account address")
            .to_hex_literal();

    let transaction = Transaction::new_transfer_with_object_ref_and_gas(
        sender_tagged.clone(),
        kanari_types::transaction::ObjectRef::new("0xaaaa", Some(1), Some("0xtest".to_string())),
        recipient_address.clone(),
        1,
        1,
        1_000_000,
        1,
    );
    let mut signed_tx = SignedTransaction::new(transaction);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .invariant("sign transaction");

    let submitted = rpc_call(
        app.clone(),
        methods::SUBMIT_OBJECT_TRANSFER,
        serde_json::json!({
            "sender": sender_tagged,
            "coin_object_id": "0xaaaa",
            "coin_object_ref": {
                "object_id": "0xaaaa",
                "version": 1,
                "digest": "0xtest"
            },
            "recipient": recipient_address,
            "amount": 1,
            "gas_limit": 1_000_000,
            "gas_price": 1,
            "nonce": 1,
            "signature": signed_tx.signature,
        }),
        10,
    )
    .await;
    let hash = submitted["hash"]
        .as_str()
        .invariant("submitted tx hash")
        .to_string();

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
    assert!(all.as_array().invariant("json array").iter().any(|tx| {
        tx["hash"]
            .as_str()
            .map(|candidate| candidate == format!("0x{}", hash))
            .unwrap_or(false)
    }));

    drop(guard);
}

#[tokio::test]
async fn submit_transaction_can_execute_immediately() {
    let guard = test_guard().await;
    let mut engine = BlockchainEngine::new_in_memory().invariant("in-memory engine");
    engine.set_authorities(
        "0x1".to_string(),
        vec!["0x1".to_string(), "0x2".to_string(), "0x3".to_string()],
    );

    let sender = generate_keypair(CurveType::Ed25519).invariant("sender keypair");
    let recipient = generate_keypair(CurveType::Ed25519).invariant("recipient keypair");

    let sender_tagged = sender.tagged_address();
    let recipient_address = recipient.address.clone();
    let transaction = Transaction::new_transfer_with_object_ref(
        sender_tagged.clone(),
        kanari_types::transaction::ObjectRef::new("0xaaaa", Some(1), Some("0xtest".to_string())),
        recipient_address.clone(),
        1,
        0,
    );
    let mut signed_tx = SignedTransaction::new(transaction);
    signed_tx
        .sign(&sender.private_key, sender.curve_type)
        .invariant("sign transaction");

    let tx_hash = hex::encode(signed_tx.transaction_hash());
    engine
        .submit_transactions_batch_with_metadata(
            vec![signed_tx],
            kanari_core::engine::PendingTransactionMetadata {
                previewed: true,
                preview_gas_used: Some(42),
                preview_effects: Some(kanari_types::transaction::TransactionEffects {
                    status: "success".to_string(),
                    gas_used: 42,
                    gas_payment: None,
                    input_objects: Vec::new(),
                    shared_inputs: Vec::new(),
                    immutable_inputs: Vec::new(),
                    gas_object_refs: Vec::new(),
                    object_changes: Vec::new(),
                    created: Vec::new(),
                    mutated: Vec::new(),
                    deleted: Vec::new(),
                    transferred: Vec::new(),
                    causal_edges: Vec::new(),
                    error_message: None,
                }),
            },
        )
        .invariant("submit previewed pending transaction");

    let app = create_router(RpcServerState::new(Arc::new(engine)));
    let fetched = rpc_call(
        app.clone(),
        methods::GET_TRANSACTION,
        serde_json::json!({ "hash": tx_hash.clone() }),
        20,
    )
    .await;

    assert_eq!(fetched["hash"], format!("0x{}", tx_hash));
    assert_eq!(fetched["status"], "simulated_pending");
    assert_eq!(fetched["previewed"], true);
    assert_eq!(fetched["submitted"], true);
    assert_eq!(fetched["committed"], false);
    assert_eq!(fetched["gas_used"], 42);
    assert!(fetched["effects"].is_object());
    assert_eq!(fetched["effects"]["gas_used"], 42);

    let all = rpc_call(
        app,
        methods::GET_ALL_TRANSACTIONS,
        serde_json::json!({ "limit": 10 }),
        21,
    )
    .await;
    assert!(all.as_array().invariant("json array").iter().any(|tx| {
        tx["hash"]
            .as_str()
            .map(|candidate| candidate == format!("0x{}", tx_hash))
            .unwrap_or(false)
            && tx["previewed"] == true
            && tx["status"] == "simulated_pending"
            && tx["gas_used"] == 42
            && tx["effects"]["gas_used"] == 42
    }));

    drop(guard);
}

#[tokio::test]
async fn submit_transaction_rejects_missing_signature() {
    let guard = test_guard().await;
    let app = build_test_router();

    let response = rpc_call_response(
        app,
        methods::SUBMIT_OBJECT_TRANSFER,
        serde_json::json!({
            "sender": "0x1111",
            "coin_object_id": "0xaaaa",
            "recipient": "0x2222",
            "amount": 1,
            "gas_limit": 1_000_000,
            "gas_price": 1,
            "nonce": 1,
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
