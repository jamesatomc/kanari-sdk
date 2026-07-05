// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::{RpcRequest, RpcResponse, RpcServerState};
use crate::{internal_error_response, invalid_params_response, respond_with_serialize};
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use serde::{Deserialize, Serialize};

pub const SUBMIT_OBJECT_TRANSACTION: &str = "kanari_submitObjectTransaction";
pub const GET_PENDING_OBJECT_TRANSACTIONS: &str = "kanari_getPendingObjectTransactions";
pub const CANCEL_OBJECT_TRANSACTION: &str = "kanari_cancelObjectTransaction";

#[derive(Debug, Serialize)]
struct SubmitResponse {
    digest: String,
    status: &'static str,
    effects: Option<kanari_types::object_effects::ObjectTransactionEffectsV1>,
}

#[derive(Debug, Serialize)]
struct PendingResponse {
    digest: String,
    sender: String,
    mutable_objects: Vec<String>,
    transaction: SignedObjectTransaction,
}

#[derive(Debug, Deserialize)]
struct CancelRequest {
    digest: String,
}

#[derive(Debug, Serialize)]
struct CancelResponse {
    digest: String,
    cancelled: bool,
    status: &'static str,
}

pub async fn submit(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let transaction: SignedObjectTransaction = match serde_json::from_value(request.params.clone())
    {
        Ok(transaction) => transaction,
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };
    let broadcast = transaction.clone();
    match state.engine.execute_protocol_transaction(transaction) {
        Ok((digest, effects)) => {
            state.broadcast_submitted_transaction(broadcast);
            respond_with_serialize(
                request.id,
                SubmitResponse {
                    digest: format!("0x{}", hex::encode(digest)),
                    status: "executed_object_effects",
                    effects: Some(effects),
                },
            )
        }
        Err(error) => invalid_params_response(request.id, error.to_string()),
    }
}

pub async fn pending(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    match state.engine.pending_object_transactions() {
        Ok(transactions) => {
            let mut values = Vec::with_capacity(transactions.len());
            for transaction in transactions {
                let digest = match transaction.digest() {
                    Ok(digest) => digest,
                    Err(error) => return internal_error_response(request.id, error.to_string()),
                };
                values.push(PendingResponse {
                    digest: format!("0x{}", hex::encode(digest)),
                    sender: transaction.data.sender.to_hex_literal(),
                    mutable_objects: transaction
                        .data
                        .mutable_input_ids()
                        .into_iter()
                        .map(|id| id.to_hex_literal())
                        .collect(),
                    transaction,
                });
            }
            respond_with_serialize(request.id, values)
        }
        Err(error) => internal_error_response(request.id, error.to_string()),
    }
}

pub async fn cancel(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let input: CancelRequest = match serde_json::from_value(request.params.clone()) {
        Ok(input) => input,
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };
    let digest = match hex::decode(input.digest.trim_start_matches("0x")) {
        Ok(digest) if digest.len() == 32 => digest,
        Ok(_) => {
            return invalid_params_response(
                request.id,
                "Object transaction digest must contain 32 bytes",
            );
        }
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };

    match state.engine.release_object_transaction(&digest) {
        Ok(transaction) => respond_with_serialize(
            request.id,
            CancelResponse {
                digest: format!("0x{}", hex::encode(digest)),
                cancelled: transaction.is_some(),
                status: if transaction.is_some() {
                    "cancelled"
                } else {
                    "not_pending"
                },
            },
        ),
        Err(error) => internal_error_response(request.id, error.to_string()),
    }
}
