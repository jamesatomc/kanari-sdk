// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Library helpers for the kanari-faucet crate.
//!
//! Exposes an async function `request_from_dev` that other crates (for example
//! the `kanari` binary) can call to request tokens from the Dev wallet.

use anyhow::Context;
use kanari_common::get_main_wallet;
use kanari_rpc_api::SignedTransactionData;
use kanari_rpc_client::RpcClient;
use kanari_types::{
    GasConfig, GasOperation,
    address::Address,
    kanari::KANARI_TOKEN_TYPE,
    transaction::{SignedTransaction, Transaction},
};
use std::env;

/// Request KANARI tokens from the Dev wallet.
///
/// Parameters:
/// - `dev_address`: optional dev address hex string (if None, uses `Address::DEV_ADDRESS`).
/// - `dev_password`: optional password string; if None this function will try `KANARI_PASSWORD` env var.
/// - `to`: optional recipient address; if None this function will use `active_address` from kanari config.
/// - `amount`: amount in KANARI (not Mist).
/// - `rpc_url`: RPC server URL.
/// - `wait_for_confirmation`: optional flag to wait for transaction confirmation (default: false).
///
/// Returns the `TransactionStatus` returned by the RPC server on success.
pub async fn request_from_dev(
    dev_address: Option<&str>,
    dev_password: Option<&str>,
    to: Option<&str>,
    amount: f64,
    rpc_url: &str,
) -> anyhow::Result<kanari_rpc_api::TransactionStatus> {
    // Determine dev address
    let dev_address = dev_address
        .map(|s| s.to_string())
        .unwrap_or_else(|| Address::DEV_ADDRESS.to_string());

    // Try loading workspace-local faucet .env first (useful when called from kanari root),
    // then fall back to normal dotenv search.
    if std::path::Path::new("crates/kanari-faucet/.env").exists() {
        let _ = dotenvy::from_filename("crates/kanari-faucet/.env");
    }
    dotenvy::dotenv().ok();

    // Determine password: prefer provided, else env var
    let password = if let Some(p) = dev_password {
        p.to_string()
    } else if let Ok(envp) = env::var("KANARI_PASSWORD") {
        envp
    } else {
        anyhow::bail!("Dev password not provided to library and KANARI_PASSWORD not set")
    };

    // Determine recipient with validation
    let recipient = if let Some(r) = to {
        // Validate recipient address format
        if !r.starts_with("0x") || r.len() != 66 {
            anyhow::bail!(
                "Invalid recipient address format: {}. Expected 0x-prefixed 64 hex characters",
                r
            );
        }
        r.to_string()
    } else if let Some(m) = get_main_wallet() {
        m
    } else {
        anyhow::bail!("No recipient specified and no active_address configured")
    };

    // Load wallet
    let wallet = kanari_crypto::wallet::load_wallet(&dev_address, &password)
        .context("Failed to load Dev wallet; check address and password")?;

    // RPC client
    let client = RpcClient::new(rpc_url);
    client
        .get_block_height()
        .await
        .context("Cannot connect to RPC server")?;

    // Check if dev account exists on-chain
    let account = match client.get_account(&dev_address).await {
        Ok(acct) => acct,
        Err(e) => {
            anyhow::bail!(
                "Dev wallet account not found on-chain: {}\n\
                 The dev wallet must be initialized first by receiving a transaction.\n\
                 Error: {}",
                dev_address,
                e
            );
        }
    };

    // Check dev wallet balance before transfer
    const MIST_PER_KANARI: f64 = 1_000_000_000.0;
    let amount_mist = (amount * MIST_PER_KANARI).round() as u64;

    let gas = GasConfig::default();
    let estimated_gas_cost = GasOperation::Transfer
        .gas_units()
        .saturating_mul(gas.default_transaction_gas_price());
    let total_required = amount_mist
        .checked_add(estimated_gas_cost)
        .context("Faucet amount plus gas fee exceeds u64")?;
    let native_balance = account
        .token_balances
        .get(KANARI_TOKEN_TYPE)
        .copied()
        .unwrap_or(0);

    if native_balance < total_required {
        anyhow::bail!(
            "Insufficient balance in dev wallet: {} KANARI (required: {} KANARI)\n\
             Available: {:.9} KANARI\n\
             Required: {:.9} KANARI (transfer: {:.9} + gas: {:.9})",
            native_balance as f64 / MIST_PER_KANARI,
            total_required as f64 / MIST_PER_KANARI,
            native_balance as f64 / MIST_PER_KANARI,
            total_required as f64 / MIST_PER_KANARI,
            amount_mist as f64 / MIST_PER_KANARI,
            estimated_gas_cost as f64 / MIST_PER_KANARI
        );
    }

    // Re-derive keypair from private key to get the tagged address format
    // Tagged addresses are required for signature verification
    let keypair =
        kanari_crypto::keys::keypair_from_private_key(&wallet.private_key, wallet.curve_type)
            .context("Failed to derive public key from wallet")?;
    let sender_for_tx = keypair.tagged_address();

    let tx = Transaction::new_transfer(
        sender_for_tx.clone(),
        recipient.clone(),
        amount_mist,
        account.sequence_number,
    );

    let mut signed_tx = SignedTransaction::new(tx);
    signed_tx
        .sign(&wallet.private_key, wallet.curve_type)
        .context("Failed to sign transaction with Dev wallet")?;

    let tx_data = SignedTransactionData {
        sender: sender_for_tx,
        recipient: Some(recipient.clone()),
        amount: Some(amount_mist),
        gas_limit: signed_tx.transaction.gas_limit(),
        gas_price: signed_tx.transaction.gas_price(),
        sequence_number: account.sequence_number,
        signature: Some(signed_tx.signature.clone()),
        execute_immediate: Some(false),
    };

    eprintln!("Submitting faucet transaction...");
    eprintln!("  From: {}", dev_address);
    eprintln!("  To: {}", recipient);
    eprintln!(
        "  Amount: {:.9} KANARI",
        amount_mist as f64 / MIST_PER_KANARI
    );

    let status = client
        .submit_transaction(tx_data)
        .await
        .context("Failed to submit transaction to RPC")?;

    eprintln!("  Transaction hash: {}", status.hash);
    eprintln!("  Status: {}", status.status);

    Ok(status)
}
