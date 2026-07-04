// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::{RpcRequest, RpcResponse, RpcServerState};
use crate::{internal_error_response, invalid_params_response, respond_with_serialize};
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use serde::{Deserialize, Serialize};

pub const SUBMIT_OBJECT_TRANSACTION: &str = "kanari_submitObjectTransaction";
pub const EXECUTE_OBJECT_TRANSACTION: &str = "kanari_executeObjectTransaction";
pub const GET_PENDING_OBJECT_TRANSACTIONS: &str = "kanari_getPendingObjectTransactions";

#[derive(Debug, Deserialize)]
struct ExecuteRequest {
    digest: String,
    gas_used: u64,
}

#[derive(Debug, Serialize)]
struct SubmitResponse {
    digest: String,
}

pub async fn submit(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let transaction: SignedObjectTransaction = match serde_json::from_value(request.params.clone()) {
        Ok(transaction) => transaction,
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };
    match state.engine.submit_protocol_transaction(transaction) {
        Ok(digest) => respond_with_serialize(
            request.id,
            SubmitResponse { digest: format!("0x{}", hex::encode(digest)) },
        ),
        Err(error) => invalid_params_response(request.id, error.to_string()),
    }
}

pub async fn execute(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let input: ExecuteRequest = match serde_json::from_value(request.params.clone()) {
        Ok(input) => input,
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };
    let digest = match hex::decode(input.digest.trim_start_matches("0x")) {
        Ok(digest) => digest,
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };
    match state.engine.execute_submitted_object_command(&digest, input.gas_used) {
        Ok(effects) => respond_with_serialize(request.id, effects),
        Err(error) => internal_error_response(request.id, error.to_string()),
    }
}

pub async fn pending(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    match state.engine.pending_object_transactions() {
        Ok(transactions) => respond_with_serialize(request.id, transactions),
        Err(error) => internal_error_response(request.id, error.to_string()),
    }
}
