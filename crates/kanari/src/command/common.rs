// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, bail};
use kanari_crypto::wallet::load_wallet;
use kanari_rpc_api::{
    AccountInfo, RpcRequest, RpcResponse, SignedTransactionData, TransactionStatus,
};
use kanari_rpc_client::RpcClient;
use kanari_types::address::Address;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
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

fn read_coin_balance(data: &[u8]) -> Option<u64> {
    if data.len() < 40 {
        return None;
    }

    let mut amount_bytes = [0u8; 8];
    amount_bytes.copy_from_slice(&data[32..40]);
    Some(u64::from_le_bytes(amount_bytes))
}

pub fn get_account_info(
    client: &Client,
    rpc_endpoint: &str,
    sender_normalized: &str,
) -> Result<AccountInfo> {
    use kanari_rpc_api::methods;

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
        .context("Failed to query account info from RPC")?;

    let rpc_resp: RpcResponse = resp
        .json()
        .context("Failed to parse account RPC response")?;

    if let Some(error) = rpc_resp.error {
        bail!("RPC error while querying account: {}", error.message);
    }

    let result = rpc_resp.result.context("RPC did not return account info")?;
    serde_json::from_value(result).context("Failed to decode account info")
}

pub fn spendable_kanari_balance(account: &AccountInfo) -> u64 {
    let token_balance = account
        .token_balances
        .get(KANARI_TOKEN_TYPE)
        .copied()
        .unwrap_or(0);

    let object_balance = account
        .owned_objects
        .as_ref()
        .map(|objects| {
            objects
                .iter()
                .filter(|obj| obj.type_ == format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE))
                .filter_map(|obj| read_coin_balance(&obj.data))
                .fold(0u64, u64::saturating_add)
        })
        .unwrap_or(0);

    token_balance.max(object_balance)
}

pub fn ensure_can_pay_gas(
    account: &AccountInfo,
    sender_normalized: &str,
    gas_cost_mist: u64,
    action: &str,
) -> Result<()> {
    let spendable = spendable_kanari_balance(account);
    if spendable < gas_cost_mist {
        bail!(
            "Insufficient KANARI balance for {} gas.\n  - estimated gas fee: {} Mist\n  - spendable balance: {} Mist",
            action,
            gas_cost_mist,
            spendable
        );
    }

    eprintln!(
        "   Gas fee covered: {} Mist available for {}",
        spendable, sender_normalized
    );
    Ok(())
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
    Ok(get_account_info(client, rpc_endpoint, sender_normalized)?.sequence_number)
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
