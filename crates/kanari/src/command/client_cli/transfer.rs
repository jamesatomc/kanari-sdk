// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::{
    check_node_connection, get_rpc_endpoint, get_sender_for_tx, load_wallet_for, normalize_addr,
    resolve_sender, resolve_transaction_gas, sign_call_function_request,
};
use anyhow::{Context, Result};
use clap::Parser;
use kanari_rpc_api::CallFunctionRequest;
use kanari_rpc_client::RpcClient;
use kanari_types::kanari::KANARI_TOKEN_TYPE;

fn read_coin_balance(data: &[u8]) -> Option<u64> {
    if data.len() < 40 {
        return None;
    }

    let mut amount_bytes = [0u8; 8];
    amount_bytes.copy_from_slice(&data[32..40]);
    Some(u64::from_le_bytes(amount_bytes))
}

#[derive(Parser, Debug)]
pub struct Transfer {
    /// Sender wallet address (optional). If omitted, uses selected wallet in config.
    #[arg(short, long)]
    pub from: Option<String>,
    /// Recipient address
    #[arg(short, long)]
    pub to: String,
    /// Amount in Kanari (will be converted to Mist)
    #[arg(short, long)]
    pub amount: f64,
    /// Wallet password
    #[arg(short, long)]
    pub password: String,

    /// RPC endpoint
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
}

impl Transfer {
    pub async fn execute(&self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let from_addr = resolve_sender(self.from.clone())?;
        let to_addr = normalize_addr(&self.to)?;
        let wallet = load_wallet_for(&from_addr, Some(self.password.clone()))?;
        let (gas_limit, gas_price) = resolve_transaction_gas(None, None);

        eprintln!("Transferring Kanari tokens...");
        eprintln!("  From: {}", from_addr);
        eprintln!("  To: {}", to_addr);
        eprintln!("  Amount: {} KANARI", self.amount);

        // Convert Kanari to Mist (1 KANARI = 10^9 Mist)
        const MIST_PER_KANARI: f64 = 1_000_000_000.0;
        let amount_mist = (self.amount * MIST_PER_KANARI).round() as u64;
        eprintln!("  Amount (Mist): {}", amount_mist);

        // Connect to RPC server
        let client = RpcClient::new(&rpc);
        check_node_connection(&client, &rpc).await?;

        // Get account to get sequence number
        let account = client
            .get_account(&from_addr)
            .await
            .context("Failed to get sender account")?;

        let sender_tagged = get_sender_for_tx(&wallet, &from_addr)?;

        let owned_objects = account
            .owned_objects
            .as_ref()
            .context("Sender account has no owned object list from RPC")?;

        let mut selected_coin_id = None;
        let mut selected_coin_balance = 0u64;
        let mut total_coin_balance = 0u64;

        for obj in owned_objects {
            if obj.type_ != format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE) {
                continue;
            }

            let Some(coin_balance) = read_coin_balance(&obj.data) else {
                continue;
            };

            total_coin_balance = total_coin_balance.saturating_add(coin_balance);

            if selected_coin_id.is_none() && coin_balance > 0 {
                selected_coin_id = Some(obj.id.clone());
                selected_coin_balance = coin_balance;
            }
        }

        let coin_object_id = selected_coin_id.with_context(|| {
            format!(
                "No spendable Coin<{}> object found for {}",
                KANARI_TOKEN_TYPE, from_addr
            )
        })?;

        if total_coin_balance < amount_mist {
            anyhow::bail!(
                "Insufficient Coin<{}> balance for {}.\n  - requested: {} Mist\n  - spendable in coin objects: {} Mist",
                KANARI_TOKEN_TYPE,
                from_addr,
                amount_mist,
                total_coin_balance
            );
        }

        eprintln!("  Using coin object: {}", coin_object_id);
        eprintln!("  Selected Coin Balance (Mist): {}", selected_coin_balance);
        eprintln!(
            "  Total Spendable Coin Balance (Mist): {}",
            total_coin_balance
        );

        let call_req = CallFunctionRequest {
            sender: sender_tagged.clone(),
            package: "0x2".to_string(),
            module: "kanari".to_string(),
            function: "transfer".to_string(),
            type_args: vec![],
            args: vec![
                move_core_types::account_address::AccountAddress::from_hex_literal(&coin_object_id)
                    .context("Invalid coin object ID")?
                    .to_vec(),
                bcs::to_bytes(&amount_mist).context("Failed to serialize amount")?,
                bcs::to_bytes(
                    &move_core_types::account_address::AccountAddress::from_hex_literal(&to_addr)?,
                )
                .context("Failed to serialize recipient address")?,
            ],
            gas_limit,
            gas_price,
            sequence_number: account.sequence_number,
            signature: None,
            execute_immediate: Some(true),
        };
        let final_call_req = sign_call_function_request(call_req, &wallet)?;

        eprintln!("  Gas Limit: {}", final_call_req.gas_limit);
        eprintln!("  Gas Price: {} Mist/gas", final_call_req.gas_price);
        eprintln!("  Transaction signed");
        eprintln!("  Submitting transaction to node...");

        let status = client
            .call_function(final_call_req)
            .await
            .context("Failed to submit transaction")?;

        eprintln!("  Transaction submitted successfully");
        eprintln!("  Transaction hash: {}", status.hash);
        eprintln!("  Status: {}", status.status);

        Ok(())
    }
}
