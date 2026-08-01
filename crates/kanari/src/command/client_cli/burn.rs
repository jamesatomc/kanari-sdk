// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::{
    check_node_connection, get_rpc_endpoint, get_sender_for_tx, load_wallet_for, resolve_sender,
    resolve_transaction_gas,
};
use crate::command::rpc_helpers::{
    should_wait_for_commit, sign_and_call_function, wait_for_transaction_commit,
};
use crate::command::tx_output::print_transaction_status;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use kanari_rpc_api::{BuildCallFunctionRequest, BuildNativeCoinConsolidationRequest};
use kanari_rpc_client::RpcClient;
use std::time::Duration;

const MAX_AUTO_CONSOLIDATION_STEPS: usize = 16;

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
    fn needs_native_coin_consolidation(error: &anyhow::Error) -> bool {
        let message = format!("{:#}", error);
        message.contains("No spendable native gas coin object found with required balance")
            || message.contains("Native burn requires one Coin<")
    }

    async fn wait_for_commit_if_needed(
        client: &RpcClient,
        status: &kanari_rpc_api::TransactionStatus,
        label: &str,
    ) -> Result<()> {
        if should_wait_for_commit(
            status.success,
            status.previewed,
            status.submitted,
            status.committed,
        ) {
            let committed = wait_for_transaction_commit(
                client,
                &status.hash,
                Duration::from_secs(20),
                Duration::from_millis(400),
            )
            .await
            .with_context(|| format!("{label} transaction did not commit in time"))?;
            print_transaction_status(&format!("  {label} Final: "), &committed);
        }

        Ok(())
    }

    pub async fn execute(&self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let from_addr = resolve_sender(self.from.clone())?;
        let wallet = load_wallet_for(&from_addr, Some(self.password.clone()))?;
        let (gas_limit, gas_price) = resolve_transaction_gas(None, None);

        eprintln!("Burning Kanari tokens...");
        eprintln!("  From: {}", from_addr);
        eprintln!("  Amount: {} KANARI", self.amount);

        const MIST_PER_KANARI: f64 = 1_000_000_000.0;
        let amount_mist = (self.amount * MIST_PER_KANARI).round() as u64;
        ensure!(
            amount_mist > 0,
            "--amount is too small; it rounds to 0 Mist"
        );
        eprintln!("  Amount (Mist): {}", amount_mist);

        let client = RpcClient::new(&rpc);
        check_node_connection(&client, &rpc).await?;

        let sender_for_tx = get_sender_for_tx(&wallet, &from_addr)?;
        let burn_args =
            vec![bcs::to_bytes(&amount_mist).context("Failed to serialize burn amount")?];
        let required_amount = amount_mist.saturating_add(gas_limit.saturating_mul(gas_price));
        let mut prepared = None;

        for step in 0..=MAX_AUTO_CONSOLIDATION_STEPS {
            match client
                .build_call_function(BuildCallFunctionRequest {
                    sender: sender_for_tx.clone(),
                    package: "0x2".to_string(),
                    module: "kanari".to_string(),
                    function: "burn".to_string(),
                    type_args: vec![],
                    args: burn_args.clone(),
                    object_inputs: None,
                    gas_limit,
                    gas_price,
                    nonce: None,
                    execute_immediate: None,
                })
                .await
            {
                Ok(request) => {
                    prepared = Some(request);
                    break;
                }
                Err(error)
                    if step < MAX_AUTO_CONSOLIDATION_STEPS
                        && Self::needs_native_coin_consolidation(&error) =>
                {
                    eprintln!(
                        "  Burn needs coin preparation; consolidating native coins ({}/{})...",
                        step + 1,
                        MAX_AUTO_CONSOLIDATION_STEPS
                    );
                    let consolidate = client
                        .build_native_coin_consolidation(BuildNativeCoinConsolidationRequest {
                            sender: sender_for_tx.clone(),
                            required_amount,
                            gas_limit,
                            gas_price,
                            nonce: None,
                            execute_immediate: None,
                        })
                        .await
                        .with_context(|| {
                            format!(
                                "Failed to prepare native coin consolidation for sender {} while preparing burn",
                                from_addr
                            )
                        })?;
                    let consolidate_status = sign_and_call_function(&client, &wallet, consolidate)
                        .await
                        .context("Failed to submit native coin consolidation for burn")?;
                    print_transaction_status("  Consolidate: ", &consolidate_status);
                    Self::wait_for_commit_if_needed(&client, &consolidate_status, "Consolidate")
                        .await?;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "Failed to build burn transaction for sender {}. Backend burn policy requires one spendable native gas coin that can cover burn amount + gas.",
                            from_addr
                        )
                    });
                }
            }
        }

        let prepared = prepared.context(
            "Failed to prepare burn transaction after automatic native coin consolidation attempts",
        )?;

        if let Some(gas_payment) = &prepared.gas_payment
            && let Some(gas_object) = gas_payment.payment_objects.first()
        {
            eprintln!("  Gas payment object: {}", gas_object.object_id);
        }

        let status = sign_and_call_function(&client, &wallet, prepared)
            .await
            .context("Failed to submit burn transaction")?;
        print_transaction_status("  ", &status);
        Self::wait_for_commit_if_needed(&client, &status, "Burn").await?;

        Ok(())
    }
}
