// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use kanari_crypto::wallet::load_wallet;
use kanari_rpc_api::{NativeTransferPolicyContract, TransactionErrorReason};
use kanari_rpc_client::{
    RpcClient, transaction_error_details as rpc_transaction_error_details,
    transaction_error_reason as rpc_transaction_error_reason,
};
use kanari_types::GasConfig;
use kanari_types::address::Address;
use log::error;
use reqwest::blocking::Client;
use std::time::Duration;

pub use crate::command::gas_and_coin_selection::{
    SelectedCoinObject, SpendableCoinObject, build_native_gas_payment, consolidate_coin_objects,
    object_call_context, select_coin_object, select_native_coin_object,
    select_native_gas_coin_object, spendable_coin_objects,
};
pub use crate::command::rpc_helpers::{
    get_owner_info, sign_and_call_function, sign_call_function_request,
};
pub use crate::command::tx_output::{
    print_json_value, print_rpc_error, print_transaction_result, print_transaction_status,
    rpc_error_reason,
};

pub fn transaction_error_reason(error: &anyhow::Error) -> Option<TransactionErrorReason> {
    rpc_transaction_error_reason(error)
}

pub fn native_transfer_policy_contract(
    error: &anyhow::Error,
) -> Option<NativeTransferPolicyContract> {
    rpc_transaction_error_details(error).and_then(|details| details.native_transfer_policy)
}

pub fn native_transfer_policy_hint(error: &anyhow::Error) -> Option<String> {
    native_transfer_policy_contract(error).map(|policy| policy.summary().to_string())
}

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

/// Load a wallet for the given normalized address.
///
/// An explicit password takes precedence. For non-interactive, local automation
/// (for example an isolated devnet test harness), `KANARI_WALLET_PASSWORD` may
/// be set. Interactive clients retain the secure password prompt by default.
pub fn load_wallet_for(
    address_normalized: &str,
    password_opt: Option<String>,
) -> Result<kanari_crypto::wallet::Wallet> {
    let password = password_opt
        .or_else(|| {
            std::env::var("KANARI_WALLET_PASSWORD")
                .ok()
                .filter(|password| !password.is_empty())
        })
        .map(Ok)
        .unwrap_or_else(|| rpassword::prompt_password("Wallet password: "))
        .context("Password required for signing")?;

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

/// Determine the sender address string for a transaction.
/// For all wallets, this returns the tagged address (Curve:PublicKey)
/// which is required for signature verification by the node.
pub fn get_sender_for_tx(
    wallet: &kanari_crypto::wallet::Wallet,
    _address_normalized: &str,
) -> Result<String> {
    // Re-derive public key from private key to get the tagged address format
    // Tagged addresses are required for signature verification in ALL scenarios
    let signing_curve = wallet
        .validated_signing_curve()
        .context("Wallet private key does not match its stored curve/address")?;
    let keypair = kanari_crypto::keys::keypair_from_private_key(&wallet.private_key, signing_curve)
        .context("Failed to derive public key from wallet")?;
    Ok(keypair.tagged_address())
}
