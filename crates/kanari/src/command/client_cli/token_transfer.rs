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
use kanari_types::error::KanariUnwrapExt;
use move_core_types::language_storage::TypeTag;
use std::str::FromStr;

fn normalize_token_type(token: &str) -> String {
    if let Ok(TypeTag::Struct(st)) = TypeTag::from_str(token) {
        return format!("{}", st);
    }
    token.to_string()
}

fn coin_token_type_from_object_type(object_type: &str) -> Option<String> {
    // Fast path for canonical/non-canonical stored strings.
    if let Some(start) = object_type.find('<')
        && let Some(end) = object_type.rfind('>')
    {
        let outer = &object_type[..start];
        if outer.ends_with("::coin::Coin") || outer.ends_with("::coin::coin::Coin") {
            return Some(normalize_token_type(&object_type[start + 1..end]));
        }
    }

    // Parse path for fully canonical struct tags.
    if let Ok(TypeTag::Struct(st)) = TypeTag::from_str(object_type)
        && st.module.as_str() == "coin"
        && st.name.as_str() == "Coin"
        && let Some(TypeTag::Struct(inner)) = st.type_params.first()
    {
        return Some(format!("{}", inner));
    }

    None
}

fn read_coin_balance(data: &[u8]) -> Option<u64> {
    if data.len() < 40 {
        return None;
    }

    let mut amount_bytes = [0u8; 8];
    amount_bytes.copy_from_slice(&data[32..40]);
    Some(u64::from_le_bytes(amount_bytes))
}

#[derive(Parser, Debug)]
pub struct TokenTransfer {
    /// Sender wallet address (optional). If omitted, uses selected wallet in config.
    #[arg(short, long)]
    pub from: Option<String>,
    /// Recipient address
    #[arg(short, long)]
    pub to: String,
    /// Token type to transfer (e.g., "james::james::JAMES" or "0x...::james::JAMES")
    #[arg(long)]
    pub token: String,
    /// Amount to transfer (in base units, e.g., 100000000 for 0.1 JAMES with 9 decimals)
    #[arg(long)]
    pub amount: u64,
    /// Wallet password
    #[arg(short, long)]
    pub password: String,

    /// RPC endpoint
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
}

impl TokenTransfer {
    pub async fn execute(&self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let from_addr = resolve_sender(self.from.clone())?;
        let to_addr = normalize_addr(&self.to)?;
        let wallet = load_wallet_for(&from_addr, Some(self.password.clone()))?;
        let (gas_limit, gas_price) = resolve_transaction_gas(None, None);

        eprintln!(
            "Transferring {} units of {} token...",
            self.amount, self.token
        );
        eprintln!("  From: {}", from_addr);
        eprintln!("  To: {}", to_addr);

        // Connect to RPC server
        let client = RpcClient::new(&rpc);
        check_node_connection(&client, &rpc).await?;

        // Get account to get sequence number and owned objects
        let account = client
            .get_account(&from_addr)
            .await
            .context("Failed to get sender account")?;

        // Find coin objects of the specified token type
        let wanted_token = normalize_token_type(&self.token);
        let mut selected_coin_id = None;
        let mut selected_coin_balance = 0u64;
        let mut total_coin_balance = 0u64;
        let mut seen_coin_types = std::collections::BTreeSet::new();
        if let Some(owned_objects) = &account.owned_objects {
            for obj in owned_objects {
                if let Some(obj_token) = coin_token_type_from_object_type(&obj.type_) {
                    seen_coin_types.insert(obj_token.clone());
                    if obj_token == wanted_token {
                        let Some(coin_balance) = read_coin_balance(&obj.data) else {
                            continue;
                        };
                        total_coin_balance = total_coin_balance.saturating_add(coin_balance);
                        if selected_coin_id.is_none() && coin_balance > 0 {
                            selected_coin_id = Some(obj.id.clone());
                            selected_coin_balance = coin_balance;
                        }
                    }
                }
            }
        }

        if selected_coin_id.is_none() {
            let state_balance = account
                .token_balances
                .iter()
                .find(|(k, _)| normalize_token_type(k) == wanted_token)
                .map(|(_, v)| *v)
                .unwrap_or(0);

            let available_coin_types = if seen_coin_types.is_empty() {
                "none".to_string()
            } else {
                seen_coin_types.into_iter().collect::<Vec<_>>().join(", ")
            };

            anyhow::bail!(
                "No Coin<{}> objects found for address {}.\n  - token balance in token_balances: {}\n  - available coin object types: {}\nThis usually means the account state tracks the token, but there is no spendable Coin object for that token.",
                wanted_token,
                from_addr,
                state_balance,
                available_coin_types
            );
        }

        if total_coin_balance < self.amount {
            anyhow::bail!(
                "Insufficient Coin<{}> balance for address {}.\n  - requested: {}\n  - spendable in coin objects: {}",
                wanted_token,
                from_addr,
                self.amount,
                total_coin_balance
            );
        }

        let coin_object_id = selected_coin_id.as_ref().invariant("selected coin checked");
        eprintln!("  Using coin object: {}", coin_object_id);
        eprintln!(
            "  Selected Coin Balance: {} (base units)",
            selected_coin_balance
        );
        eprintln!(
            "  Total Spendable Coin Balance: {} (base units)",
            total_coin_balance
        );
        eprintln!("  Amount: {} (base units)", self.amount);

        let sender_tagged = get_sender_for_tx(&wallet, &from_addr)?;

        // Parse token format: address::module::struct
        let module_parts: Vec<&str> = self.token.split("::").collect();
        if module_parts.len() < 3 {
            anyhow::bail!("Invalid token format. Expected format: address::module::struct");
        }

        let package_address = module_parts[0].to_string();
        let module_name = module_parts[1].to_string();

        // Create the CallFunctionRequest for transfer_amount
        let call_req = CallFunctionRequest {
            sender: sender_tagged.clone(),
            package: package_address,
            module: module_name,
            function: "transfer_amount".to_string(),
            type_args: vec![],
            args: vec![
                move_core_types::account_address::AccountAddress::from_hex_literal(coin_object_id)
                    .context("Invalid coin object ID")?
                    .to_vec(),
                bcs::to_bytes(&self.amount).context("Failed to serialize amount")?,
                bcs::to_bytes(
                    &move_core_types::account_address::AccountAddress::from_hex_literal(&to_addr)?,
                )
                .context("Failed to serialize recipient address")?,
            ],
            gas_limit,
            gas_price,
            sequence_number: account.sequence_number,
            signature: None, // Will be set after signing
            execute_immediate: Some(true),
        };
        let final_call_req = sign_call_function_request(call_req, &wallet)?;

        eprintln!("  Gas Limit: {}", final_call_req.gas_limit);
        eprintln!("  Gas Price: {} Mist/gas", final_call_req.gas_price);
        eprintln!("  Transaction signed");

        eprintln!("  Submitting transaction to node...");

        // Use call_function instead of submit_transaction
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
