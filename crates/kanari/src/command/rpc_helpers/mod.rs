// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, bail, ensure};
use kanari_crypto::wallet::Wallet;
use kanari_rpc_api::{
    CallFunctionRequest, GetObjectRequest, ObjectInfo, ObjectTransferData, OwnerInfo,
    PublishModuleRequest, PublishPackageRequest, RpcRequest, RpcResponse, TransactionDetails,
    TransactionResult, TransactionStatus, methods,
};
use kanari_rpc_client::{RpcClient, as_rpc_client_error};
use kanari_types::transaction::{
    ObjectInput, ObjectOwnerKind, ObjectRef, PublishedModule, SignedTransaction, Transaction,
};
use reqwest::blocking::Client;
use std::time::Duration;
use tokio::time::sleep;

use crate::command::common::normalize_addr;
use crate::command::tx_output::{print_json_value, print_rpc_error, print_transaction_result};
pub(crate) fn map_nonce_error(error: String) -> anyhow::Error {
    anyhow::anyhow!(error)
}

pub fn should_wait_for_commit(
    success: bool,
    previewed: bool,
    submitted: bool,
    committed: bool,
) -> bool {
    let _ = previewed;
    success && submitted && !committed
}

pub fn sign_call_function_request(
    mut request: CallFunctionRequest,
    wallet: &Wallet,
) -> Result<CallFunctionRequest> {
    ensure!(
        request
            .signature
            .as_ref()
            .map(|sig| sig.is_empty())
            .unwrap_or(true),
        "Refusing to overwrite existing call-function signature"
    );
    let transaction = Transaction::ExecuteFunction {
        sender: request.sender.clone(),
        module: format!("{}::{}", request.package, request.module),
        function: request.function.clone(),
        type_args: request.type_args.clone(),
        args: request.args.clone(),
        object_inputs: request.object_inputs.clone().unwrap_or_default(),
        gas_payment: request.gas_payment.clone(),
        gas_limit: request.gas_limit,
        gas_price: request.gas_price,
        nonce: request.canonical_nonce().map_err(map_nonce_error)?,
    };

    let mut signed_tx = SignedTransaction::new(transaction);
    let signing_curve = wallet
        .validated_signing_curve()
        .context("Wallet private key does not match its stored curve/address")?;
    signed_tx
        .sign(&wallet.private_key, signing_curve)
        .context("Failed to sign transaction")?;
    request.signature = Some(signed_tx.signature);
    Ok(request)
}

pub fn sign_publish_module_request(
    mut request: PublishModuleRequest,
    wallet: &Wallet,
) -> Result<PublishModuleRequest> {
    ensure!(
        request
            .signature
            .as_ref()
            .map(|sig| sig.is_empty())
            .unwrap_or(true),
        "Refusing to overwrite existing publish-module signature"
    );
    let transaction = Transaction::PublishModule {
        sender: request.sender.clone(),
        module_bytes: request.module_bytes.clone(),
        module_name: request.module_name.clone(),
        gas_payment: request.gas_payment.clone(),
        gas_limit: request.gas_limit,
        gas_price: request.gas_price,
        nonce: request.canonical_nonce().map_err(map_nonce_error)?,
    };

    let mut signed_tx = SignedTransaction::new(transaction);
    let signing_curve = wallet
        .validated_signing_curve()
        .context("Wallet private key does not match its stored curve/address")?;
    signed_tx
        .sign(&wallet.private_key, signing_curve)
        .context("Failed to sign module transaction")?;
    request.signature = Some(signed_tx.signature);
    Ok(request)
}

pub fn sign_publish_package_request(
    request: PublishPackageRequest,
    wallet: &Wallet,
) -> Result<PublishPackageRequest> {
    sign_publish_package_request_as(request, wallet, false)
}

pub fn sign_publish_package_request_as(
    mut request: PublishPackageRequest,
    wallet: &Wallet,
    upgrade: bool,
) -> Result<PublishPackageRequest> {
    ensure!(
        request
            .signature
            .as_ref()
            .map(|sig| sig.is_empty())
            .unwrap_or(true),
        "Refusing to overwrite existing publish/upgrade-package signature"
    );
    let modules = request
        .modules
        .iter()
        .map(|module| PublishedModule {
            module_name: module.module_name.clone(),
            module_bytes: module.module_bytes.clone(),
        })
        .collect();
    let nonce = request.canonical_nonce().map_err(map_nonce_error)?;
    let transaction = if upgrade {
        Transaction::UpgradePackage {
            sender: request.sender.clone(),
            modules,
            gas_payment: request.gas_payment.clone(),
            gas_limit: request.gas_limit,
            gas_price: request.gas_price,
            nonce,
        }
    } else {
        Transaction::PublishPackage {
            sender: request.sender.clone(),
            modules,
            gas_payment: request.gas_payment.clone(),
            gas_limit: request.gas_limit,
            gas_price: request.gas_price,
            nonce,
        }
    };

    let mut signed_tx = SignedTransaction::new(transaction);
    let signing_curve = wallet
        .validated_signing_curve()
        .context("Wallet private key does not match its stored curve/address")?;
    signed_tx
        .sign(&wallet.private_key, signing_curve)
        .context("Failed to sign package transaction")?;
    request.signature = Some(signed_tx.signature);
    Ok(request)
}

pub fn sign_object_transfer_request(
    mut request: ObjectTransferData,
    wallet: &Wallet,
) -> Result<ObjectTransferData> {
    ensure!(
        request
            .signature
            .as_ref()
            .map(|sig| sig.is_empty())
            .unwrap_or(true),
        "Refusing to overwrite existing object-transfer signature"
    );
    let coin_object_ref = request
        .coin_object_ref
        .clone()
        .context("coin_object_ref is required to sign object transfer transaction")?;

    let mut transaction = Transaction::new_transfer_with_object_ref_and_gas(
        request.sender.clone(),
        coin_object_ref,
        request.recipient.clone(),
        request.amount,
        request.canonical_nonce().map_err(map_nonce_error)?,
        request.gas_limit,
        request.gas_price,
    );
    if let Transaction::ExecuteFunction { gas_payment, .. } = &mut transaction
        && request.gas_payment.is_some()
    {
        *gas_payment = request.gas_payment.clone();
    }

    let mut signed_tx = SignedTransaction::new(transaction);
    let signing_curve = wallet
        .validated_signing_curve()
        .context("Wallet private key does not match its stored curve/address")?;
    signed_tx
        .sign(&wallet.private_key, signing_curve)
        .context("Failed to sign object transfer transaction")?;
    request.signature = Some(signed_tx.signature);
    Ok(request)
}

pub fn submit_blocking_rpc(
    client: &Client,
    rpc_endpoint: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<RpcResponse> {
    let rpc_request = RpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: 1,
    };

    client
        .post(rpc_endpoint)
        .json(&rpc_request)
        .send()
        .context("Failed to send RPC request")?
        .error_for_status()
        .context("RPC server returned HTTP error status")?
        .json::<RpcResponse>()
        .context("Failed to parse RPC response")
}

pub fn require_rpc_result(
    rpc_response: RpcResponse,
    missing_context: &str,
) -> Result<serde_json::Value> {
    if let Some(err) = rpc_response.error {
        print_rpc_error("", &err);
        bail!("RPC returned error: {}", err.message);
    }

    rpc_response.result.context(missing_context.to_string())
}

pub fn render_transaction_submission(
    client: &Client,
    rpc_endpoint: &str,
    rpc_response: RpcResponse,
    prefix: &str,
    fail_on_transaction_failure: bool,
) -> Result<bool> {
    if let Some(err) = rpc_response.error {
        print_rpc_error(prefix, &err);
        bail!("RPC returned transaction error: {}", err.message);
    }

    let Some(result) = rpc_response.result else {
        bail!("RPC response has no result and no error");
    };

    let tx_result: TransactionResult = serde_json::from_value(result.clone())
        .context("RPC returned unexpected transaction result payload")?;
    print_transaction_result(prefix, &tx_result);
    if !tx_result.success {
        if fail_on_transaction_failure {
            bail!(
                "Transaction failed: {}",
                tx_result
                    .error_message
                    .clone()
                    .unwrap_or_else(|| tx_result.status.clone())
            );
        }
        print_json_value(prefix, "RPC result", &result);
        return Ok(false);
    }

    if should_wait_for_commit(
        tx_result.success,
        tx_result.previewed,
        tx_result.submitted,
        tx_result.committed,
    ) {
        eprintln!("{prefix}Waiting for transaction commit...");
        let committed = wait_for_transaction_commit_blocking(
            client,
            rpc_endpoint,
            &tx_result.hash,
            Duration::from_secs(20),
            Duration::from_millis(400),
        )?;
        eprintln!(
            "{prefix}Final status: {} success={} previewed={} submitted={} committed={}",
            committed.status,
            committed.success,
            committed.previewed,
            committed.submitted,
            committed.committed
        );
        if fail_on_transaction_failure && !committed.success {
            bail!(
                "Committed transaction failed with status {}",
                committed.status
            );
        }
    }

    print_json_value(prefix, "RPC result", &result);
    Ok(true)
}

pub async fn sign_and_call_function(
    client: &RpcClient,
    wallet: &Wallet,
    request: CallFunctionRequest,
) -> Result<TransactionStatus> {
    let signed_request = sign_call_function_request(request, wallet)?;
    let status = client
        .call_function(signed_request)
        .await
        .context("Failed to submit transaction")?;

    if !status.success || (!status.submitted && !status.committed) {
        bail!(
            "Transaction was not successful (status: {}, submitted: {}, committed: {}, previewed: {}). Tx hash: {}",
            status.status,
            status.submitted,
            status.committed,
            status.previewed,
            status.hash
        );
    }

    Ok(status)
}

pub fn object_input_from_info(
    object: &ObjectInfo,
    expected_sender: Option<&str>,
) -> Result<ObjectInput> {
    let owner = match &object.owner_kind {
        ObjectOwnerKind::AddressOwner(address) => {
            if let Some(sender) = expected_sender {
                let object_owner = normalize_addr(address)
                    .unwrap_or_else(|_| address.clone())
                    .to_lowercase();
                let sender = normalize_addr(sender)
                    .unwrap_or_else(|_| sender.to_string())
                    .to_lowercase();
                if object_owner != sender {
                    bail!(
                        "Object input {} is owned by {}, not sender {}",
                        object.id,
                        address,
                        sender
                    );
                }
            }
            Some(ObjectOwnerKind::AddressOwner(address.clone()))
        }
        ObjectOwnerKind::Shared => Some(ObjectOwnerKind::Shared),
        ObjectOwnerKind::Immutable => Some(ObjectOwnerKind::Immutable),
    };

    Ok(ObjectInput {
        object_ref: ObjectRef::new(
            object.id.clone(),
            Some(object.version),
            object.digest.clone(),
        ),
        owner,
        mutable: !matches!(object.owner_kind, ObjectOwnerKind::Immutable),
    })
}

pub async fn object_input_from_object_id(
    client: &RpcClient,
    object_id: &str,
    expected_sender: Option<&str>,
) -> Result<ObjectInput> {
    let object = client
        .get_object(object_id)
        .await
        .with_context(|| format!("Failed to fetch object {}", object_id))?;
    object_input_from_info(&object, expected_sender)
}

fn status_from_details(details: TransactionDetails) -> TransactionStatus {
    TransactionStatus {
        hash: details.hash,
        status: details.status,
        block_height: details.block_height,
        gas_used: details.gas_used,
        success: details.success,
        previewed: details.previewed,
        submitted: details.submitted,
        committed: details.committed,
    }
}

/// JSON-RPC error code returned by the RPC server's per-source anti-spam
/// middleware when a client exceeds its per-window request budget.
const RPC_ERROR_CODE_RATE_LIMIT: i32 = -32005;

/// True when `error` is a JSON-RPC rejection from the node's anti-spam
/// rate limiter rather than a transaction-level failure.
fn is_rate_limit_error(error: &anyhow::Error) -> bool {
    as_rpc_client_error(error)
        .map(|rpc_error| rpc_error.rpc_error().code == RPC_ERROR_CODE_RATE_LIMIT)
        .unwrap_or(false)
}

pub async fn wait_for_transaction_commit(
    client: &RpcClient,
    tx_hash: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<TransactionStatus> {
    let started = std::time::Instant::now();

    loop {
        let details = match client.get_transaction(tx_hash).await {
            Ok(details) => details,
            Err(error) if is_rate_limit_error(&error) => {
                // The node rate-limits per source IP (200 req/s), so a burst
                // of commit checks can exceed its own budget. Back off for
                // one rate-limit window and retry: the transaction itself is
                // unaffected by the rejected read.
                if started.elapsed() >= timeout {
                    return Err(error.context(format!(
                        "RPC node kept rate-limiting poll for transaction {} past the {}s timeout",
                        tx_hash,
                        timeout.as_secs()
                    )));
                }
                sleep(Duration::from_secs(1)).await;
                continue;
            }
            Err(error) => {
                return Err(error.context(format!("Failed to fetch transaction {}", tx_hash)));
            }
        };
        let status = status_from_details(details);

        if status.committed || !status.success {
            return Ok(status);
        }

        if started.elapsed() >= timeout {
            bail!(
                "Timed out waiting for transaction commit. Last status: {} (previewed={}, submitted={}, committed={})",
                status.status,
                status.previewed,
                status.submitted,
                status.committed
            );
        }

        sleep(poll_interval).await;
    }
}

pub fn wait_for_transaction_commit_blocking(
    client: &Client,
    rpc_endpoint: &str,
    tx_hash: &str,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<TransactionStatus> {
    let started = std::time::Instant::now();

    loop {
        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            method: methods::GET_TRANSACTION.to_string(),
            params: serde_json::json!({ "hash": tx_hash }),
            id: 1,
        };

        let resp = client
            .post(rpc_endpoint)
            .json(&req)
            .send()
            .with_context(|| format!("Failed to query transaction {}", tx_hash))?;
        let rpc_resp: RpcResponse = resp
            .json()
            .context("Failed to parse getTransaction RPC response")?;

        if let Some(error) = rpc_resp.error {
            if error.code == RPC_ERROR_CODE_RATE_LIMIT {
                if started.elapsed() >= timeout {
                    bail!(
                        "RPC node kept rate-limiting poll for transaction {} past the {}s timeout",
                        tx_hash,
                        timeout.as_secs()
                    );
                }
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
            print_rpc_error("", &error);
            bail!("RPC did not return transaction info for {}", tx_hash);
        }

        let details: TransactionDetails = serde_json::from_value(
            rpc_resp
                .result
                .context("RPC did not return transaction info for hash")?,
        )
        .context("Failed to decode transaction details from RPC")?;
        let status = status_from_details(details);

        if status.committed || !status.success {
            return Ok(status);
        }

        if started.elapsed() >= timeout {
            bail!(
                "Timed out waiting for transaction commit. Last status: {} (previewed={}, submitted={}, committed={})",
                status.status,
                status.previewed,
                status.submitted,
                status.committed
            );
        }

        std::thread::sleep(poll_interval);
    }
}

pub fn get_owner_info(
    client: &Client,
    rpc_endpoint: &str,
    sender_normalized: &str,
) -> Result<OwnerInfo> {
    let acct_req = RpcRequest {
        jsonrpc: "2.0".to_string(),
        method: methods::GET_OWNER.to_string(),
        params: serde_json::to_value(sender_normalized)
            .context("Failed to serialize sender for RPC")?,
        id: 1,
    };

    let resp = client
        .post(rpc_endpoint)
        .json(&acct_req)
        .send()
        .context("Failed to query owner info from RPC")?;

    let rpc_resp: RpcResponse = resp.json().context("Failed to parse owner RPC response")?;

    if let Some(error) = rpc_resp.error {
        print_rpc_error("", &error);
        bail!("RPC did not return owner info for sender");
    }

    let result = rpc_resp
        .result
        .context("RPC did not return owner info for sender")?;
    serde_json::from_value(result).context("Failed to decode owner info from RPC")
}

pub fn get_object_info(client: &Client, rpc_endpoint: &str, object_id: &str) -> Result<ObjectInfo> {
    let object_req = RpcRequest {
        jsonrpc: "2.0".to_string(),
        method: methods::GET_OBJECT.to_string(),
        params: serde_json::to_value(GetObjectRequest {
            object_id: object_id.to_string(),
        })
        .context("Failed to serialize object id for RPC")?,
        id: 1,
    };

    let resp = client
        .post(rpc_endpoint)
        .json(&object_req)
        .send()
        .with_context(|| format!("Failed to query object info for {}", object_id))?;

    let rpc_resp: RpcResponse = resp.json().context("Failed to parse object RPC response")?;

    if let Some(error) = rpc_resp.error {
        print_rpc_error("", &error);
        bail!("RPC did not return object info for {}", object_id);
    }

    let result = rpc_resp
        .result
        .with_context(|| format!("RPC did not return object info for {}", object_id))?;
    serde_json::from_value(result).context("Failed to decode object info from RPC")
}
