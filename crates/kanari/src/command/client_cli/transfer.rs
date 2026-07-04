// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::{
    check_node_connection, get_rpc_endpoint, get_sender_for_tx, load_wallet_for, normalize_addr,
    resolve_sender, resolve_transaction_gas,
};
use anyhow::{Context, Result};
use clap::Parser;
use kanari_rpc_client::RpcClient;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use kanari_types::object::{ObjectDigest, ObjectID, ObjectRef};
use kanari_types::object_transaction::{
    GasData, ObjectTransactionData, ObjectTransactionKind, TransactionExpiration,
};
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use move_core_types::account_address::AccountAddress;

fn read_coin_balance(data: &[u8]) -> Option<u64> {
    let bytes: [u8; 8] = data.get(32..40)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn object_ref(id: &str, version: u64, digest: &str) -> Result<ObjectRef> {
    let digest = hex::decode(digest.trim_start_matches("0x"))
        .context("Invalid object digest returned by RPC")?;
    Ok(ObjectRef::new(
        ObjectID::from_hex_literal(id).context("Invalid coin object ID")?,
        version,
        ObjectDigest::from_bytes(&digest)?,
    ))
}

#[derive(Parser, Debug)]
pub struct Transfer {
    #[arg(short, long)]
    pub from: Option<String>,
    #[arg(short, long)]
    pub to: String,
    #[arg(short, long)]
    pub amount: f64,
    #[arg(short, long)]
    pub password: String,
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
}

impl Transfer {
    pub async fn execute(&self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let from_addr = resolve_sender(self.from.clone())?;
        let to_addr = normalize_addr(&self.to)?;
        let wallet = load_wallet_for(&from_addr, Some(self.password.clone()))?;
        let (gas_budget, gas_price) = resolve_transaction_gas(None, None);

        eprintln!("Transferring Kanari tokens...");
        eprintln!("  From: {}", from_addr);
        eprintln!("  To: {}", to_addr);
        eprintln!("  Amount: {} KANARI", self.amount);

        const MIST_PER_KANARI: f64 = 1_000_000_000.0;
        let amount_mist = (self.amount * MIST_PER_KANARI).round() as u64;
        if amount_mist == 0 {
            anyhow::bail!("Transfer amount must be greater than zero");
        }
        eprintln!("  Amount (Mist): {}", amount_mist);

        let client = RpcClient::new(&rpc);
        check_node_connection(&client, &rpc).await?;

        let coin_type = format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE);
        let mut coins = client
            .get_owned_object_refs(&from_addr, Some(coin_type.clone()))
            .await
            .context("Failed to get exact sender coin objects")?;
        coins.sort_by(|left, right| left.id.cmp(&right.id));

        let maximum_gas_fee = gas_budget
            .checked_mul(gas_price)
            .context("Maximum gas fee overflow")?;
        let required = amount_mist
            .checked_add(maximum_gas_fee)
            .and_then(|value| value.checked_add(1))
            .context("Payment plus maximum gas fee overflow")?;

        let mut selected_refs = Vec::new();
        let mut selected_balance = 0u64;
        let mut total_balance = 0u64;
        for coin in coins {
            if coin.type_ != coin_type {
                continue;
            }
            let Some(balance) = read_coin_balance(&coin.data) else {
                continue;
            };
            total_balance = total_balance
                .checked_add(balance)
                .context("Coin balance overflow")?;
            if selected_balance < required {
                selected_refs.push(object_ref(&coin.id, coin.version, &coin.digest)?);
                selected_balance = selected_balance
                    .checked_add(balance)
                    .context("Selected coin balance overflow")?;
            }
        }

        if selected_balance < required {
            anyhow::bail!(
                "Insufficient Coin<{}> balance. requested={} Mist, max_gas={} Mist, spendable={} Mist",
                KANARI_TOKEN_TYPE,
                amount_mist,
                maximum_gas_fee,
                total_balance
            );
        }

        eprintln!("  Selected coin objects: {}", selected_refs.len());
        for reference in &selected_refs {
            eprintln!("    - {}", reference.object_id);
        }
        eprintln!("  Selected Coin Balance (Mist): {}", selected_balance);
        eprintln!("  Total Spendable Coin Balance (Mist): {}", total_balance);
        eprintln!("  Gas Budget: {}", gas_budget);
        eprintln!("  Gas Price: {} Mist/gas", gas_price);

        let sender = AccountAddress::from_hex_literal(&from_addr)
            .context("Invalid sender account address")?;
        let recipient = AccountAddress::from_hex_literal(&to_addr)
            .context("Invalid recipient account address")?;
        let data = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::Pay {
                coins: selected_refs.clone(),
                recipient,
                amount: amount_mist,
            },
            GasData {
                payment: selected_refs,
                owner: sender,
                price: gas_price,
                budget: gas_budget,
            },
            TransactionExpiration::None,
        )?;
        let mut transaction =
            SignedObjectTransaction::new(data, get_sender_for_tx(&wallet, &from_addr)?)?;
        transaction
            .sign_sender(&wallet.private_key, wallet.curve_type)
            .context("Failed to sign object transaction")?;

        eprintln!("  Object transaction signed");
        eprintln!("  Submitting transaction for consensus...");
        let status = client
            .submit_object_transaction(transaction)
            .await
            .context("Failed to submit object transaction")?;
        eprintln!("  Transaction submitted successfully");
        eprintln!("  Transaction digest: {}", status.hash);
        eprintln!("  Status: {}", status.status);
        Ok(())
    }
}
