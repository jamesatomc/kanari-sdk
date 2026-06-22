// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::{
    check_node_connection, get_rpc_endpoint, get_sender_for_tx, load_wallet_for, resolve_sender,
    sign_and_submit_transaction, spendable_kanari_balance,
};
use anyhow::{Context, Result};
use clap::Parser;
use kanari_rpc_client::RpcClient;
use kanari_types::transaction::Transaction;
use kanari_types::{GasConfig, GasOperation};

#[derive(Parser, Debug)]
pub struct Burn {
    /// Wallet address to burn from (optional). If omitted, uses selected wallet in config.
    #[arg(short, long)]
    pub from: Option<String>,
    /// Amount in Kanari to burn
    #[arg(short, long)]
    pub amount: f64,
    /// Wallet password
    #[arg(short, long)]
    pub password: String,

    /// RPC endpoint
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
}

impl Burn {
    pub async fn execute(&self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let from_addr = resolve_sender(self.from.clone())?;
        let wallet = load_wallet_for(&from_addr, Some(self.password.clone()))?;

        eprintln!("Burning Kanari tokens...");
        eprintln!("  From: {}", from_addr);
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

        let sender_for_tx = get_sender_for_tx(&wallet, &from_addr)?;
        let gas = GasConfig::default();
        let gas_fee = GasOperation::Transfer
            .gas_units()
            .saturating_mul(gas.default_transaction_gas_price());
        let total_required = amount_mist
            .checked_add(gas_fee)
            .context("Burn amount plus gas fee exceeds u64")?;
        let spendable_balance = spendable_kanari_balance(&account);

        if spendable_balance < total_required {
            anyhow::bail!(
                "Insufficient KANARI balance for burn.\n  - burn amount: {} Mist\n  - estimated gas fee: {} Mist\n  - required total: {} Mist\n  - spendable balance: {} Mist",
                amount_mist,
                gas_fee,
                total_required,
                spendable_balance
            );
        }

        eprintln!("  Estimated Gas Fee: {} Mist", gas_fee);

        // Create burn transaction
        let tx = Transaction::new_burn(sender_for_tx.clone(), amount_mist, account.sequence_number);

        sign_and_submit_transaction(&client, tx, &wallet, sender_for_tx, None, Some(amount_mist))
            .await?;

        Ok(())
    }
}
