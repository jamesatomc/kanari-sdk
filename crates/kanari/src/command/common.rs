// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, bail};
use kanari_crypto::wallet::{Wallet, load_wallet};
use kanari_rpc_api::{CallFunctionRequest, SignedTransactionData, TransactionStatus};
use kanari_rpc_client::RpcClient;
use kanari_types::GasConfig;
use kanari_types::address::Address;
use kanari_types::transaction::{SignedTransaction, Transaction};
use log::error;
use reqwest::blocking::Client;
use rpassword;
use std::time::Duration;

/// Normalize and validate an address string to a 0x-prefixed 64-hex format
pub fn normalize_addr(a: &str) -> Result<String> {
    use std::str::FromStr;
    // Use the central Address type to handle tagged addresses, public keys, and hex literals
    let addr = Address::from_str(a).with_context(|| format!("Invalid address: {}", a))?;
    Ok(addr.to_hex_literal())
}

/// Determine the RPC endpoint to use.
pub fn resolve_transaction_gas(gas_limit: Option<u64>, gas_price: Option<u64>) -> (u64, u64) {
    let config = GasConfig::default();
    (
        gas_limit.unwrap_or_else(|| config.default_transaction_gas_limit()),
        gas_price.unwrap_or_else(|| config.default_transaction_gas_price()),
    )
}

pub fn get_rpc_endpoint(rpc_opt: Option<String>) -> String {
    rpc_opt
        .or_else(kanari_common::get_active_rpc)
        .unwrap_or_else(|| "http://127.0.0.1:6767".to_string())
}

/// Resolve the sender address from either an option or the selected wallet.
pub fn resolve_sender(from_opt: Option<String>) -> Result<String> {
    let addr = if let Some(f) = from_opt {
        f
    } else {
        kanari_crypto::wallet::get_selected_wallet().ok_or_else(|| {
            anyhow::anyhow!("No sender provided and no selected wallet set. Use --from or run `kanari keytool load-wallet` to select one.")
        })?
    };
    normalize_addr(&addr)
}

/// Load a wallet for the given normalized address, prompting for a password if not provided.
pub fn load_wallet_for(
    address_normalized: &str,
    password_opt: Option<String>,
) -> Result<kanari_crypto::wallet::Wallet> {
    let password = match password_opt {
        Some(p) => p,
        None => rpassword::prompt_password("Wallet password: ")
            .context("Password required for signing")?,
    };

    let w = load_wallet(address_normalized, &password)
        .context("Failed to load wallet. Make sure the wallet exists and password is correct")?;

    Ok(w)
}

/// Build a blocking HTTP client with optional timeout (seconds)
pub fn build_blocking_client(timeout_secs: u64) -> Result<Client> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .context("Failed to build HTTP client")?;
    Ok(client)
}

/// Check node connection and return block height (async)
pub async fn check_node_connection(client: &RpcClient, rpc: &str) -> Result<u64> {
    match client.get_block_height().await {
        Ok(height) => {
            eprintln!("  Connected to node (height: {})", height);
            Ok(height)
        }
        Err(_) => {
            error!("  Cannot connect to RPC server at {}", rpc);
            error!("  Please start the node first: cargo run --bin kanari-node");
            Err(anyhow::anyhow!("RPC server not available"))
        }
    }
}

pub fn sign_call_function_request(
    mut request: CallFunctionRequest,
    wallet: &Wallet,
) -> Result<CallFunctionRequest> {
    let transaction = Transaction::ExecuteFunction {
        sender: request.sender.clone(),
        module: format!("{}::{}", request.package, request.module),
        function: request.function.clone(),
        type_args: request.type_args.clone(),
        args: request.args.clone(),
        gas_limit: request.gas_limit,
        gas_price: request.gas_price,
        sequence_number: request.sequence_number,
    };

    let mut signed_tx = SignedTransaction::new(transaction);
    signed_tx
        .sign(&wallet.private_key, wallet.curve_type)
        .context("Failed to sign transaction")?;
    request.signature = Some(signed_tx.signature);
    Ok(request)
}
/// Sign and submit a transaction to the RPC node (async)
pub async fn sign_and_submit_transaction(
    client: &RpcClient,
    tx: Transaction,
    wallet: &kanari_crypto::wallet::Wallet,
    sender_tagged: String,
    recipient_normalized: Option<String>,
    amount_mist: Option<u64>,
) -> Result<TransactionStatus> {
    eprintln!("  Gas Limit: {}", tx.gas_limit());
    eprintln!("  Gas Price: {} Mist/gas", tx.gas_price());

    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&wallet.private_key, wallet.curve_type)
        .context("Failed to sign transaction")?;
    eprintln!("  Transaction signed");

    eprintln!("  Executing transaction on node...");

    let tx_data = SignedTransactionData {
        sender: sender_tagged,
        recipient: recipient_normalized,
        amount: amount_mist,
        gas_limit: signed_tx.transaction.gas_limit(),
        gas_price: signed_tx.transaction.gas_price(),
        sequence_number: signed_tx.transaction.sequence_number(),
        signature: Some(signed_tx.signature.clone()),
        execute_immediate: Some(true),
    };

    let status = client
        .submit_transaction(tx_data)
        .await
        .context("Failed to submit transaction")?;

    // Guard against false-success UX when RPC returns failed/unknown statuses.
    if status.status != "pending" && status.status != "executed" && status.status != "committed" {
        bail!(
            "Transaction was not successful (status: {}). Tx hash: {}",
            status.status,
            status.hash
        );
    }

    eprintln!("  Transaction completed successfully");
    eprintln!("  Transaction hash: {}", status.hash);
    eprintln!("  Status: {}", status.status);

    Ok(status)
}

/// Query account sequence number from RPC for a sender address (normalized)
pub fn get_account_sequence(
    client: &Client,
    rpc_endpoint: &str,
    sender_normalized: &str,
) -> Result<u64> {
    use kanari_rpc_api::{RpcRequest, RpcResponse, methods};

    let acct_req = RpcRequest {
        jsonrpc: "2.0".to_string(),
        method: methods::GET_ACCOUNT.to_string(),
        params: serde_json::to_value(sender_normalized)
            .context("Failed to serialize sender for RPC")?,
        id: 1,
    };

    let resp = client
        .post(rpc_endpoint)
        .json(&acct_req)
        .send()
        .context("Failed to query account sequence number from RPC")?;

    let rpc_resp: RpcResponse = resp
        .json()
        .context("Failed to parse account RPC response")?;

    if let Some(result) = rpc_resp.result {
        if let Some(sn) = result.get("sequence_number").and_then(|v| v.as_u64()) {
            Ok(sn)
        } else {
            bail!("Account RPC result missing 'sequence_number'");
        }
    } else {
        bail!("RPC did not return account info for sender");
    }
}

/// Determine the sender address string for a transaction.
/// For all wallets, this returns the tagged address (Curve:PublicKey)
/// which is required for signature verification by the node.
pub fn get_sender_for_tx(
    wallet: &kanari_crypto::wallet::Wallet,
    _address_normalized: &str,
) -> Result<String> {
    // Re-derive public key from private key to get the tagged address format
    // Tagged addresses are required for signature verification in ALL scenarios
    let keypair =
        kanari_crypto::keys::keypair_from_private_key(&wallet.private_key, wallet.curve_type)
            .context("Failed to derive public key from wallet")?;
    Ok(keypair.tagged_address())
}
