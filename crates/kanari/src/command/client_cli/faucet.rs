// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::get_rpc_endpoint;
use crate::command::rpc_helpers::{should_wait_for_commit, wait_for_transaction_commit};
use anyhow::{Context, Result};
use clap::Parser;
use kanari_rpc_client::RpcClient;
use std::time::Duration;

#[derive(Parser, Debug)]
pub struct Faucet {
    /// Recipient address (optional). If omitted, uses configured active_address
    #[arg(short, long)]
    pub to: Option<String>,

    /// Amount in Kanari
    #[arg(short, long)]
    pub amount: f64,

    /// Dev wallet address override (optional)
    #[arg(long)]
    pub dev_address: Option<String>,

    /// Dev wallet password (optional; falls back to KANARI_PASSWORD env)
    #[arg(long)]
    pub dev_password: Option<String>,

    /// RPC endpoint
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,

    /// Maximum seconds to wait for the faucet transaction to commit.
    #[arg(long, default_value_t = 120)]
    pub commit_timeout_sec: u64,
}

impl Faucet {
    pub async fn execute(&self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());

        let status = kanari_faucet::request_from_dev(
            self.dev_address.as_deref(),
            self.dev_password.as_deref(),
            self.to.as_deref(),
            self.amount,
            &rpc,
        )
        .await
        .context("Faucet request failed")?;

        eprintln!(
            "Faucet tx submitted: hash={} status={} success={} previewed={} submitted={} committed={}",
            status.hash,
            status.status,
            status.success,
            status.previewed,
            status.submitted,
            status.committed
        );

        if should_wait_for_commit(
            status.success,
            status.previewed,
            status.submitted,
            status.committed,
        ) {
            eprintln!("Waiting for faucet transaction commit...");
            let client = RpcClient::new(&rpc);
            let committed = wait_for_transaction_commit(
                &client,
                &status.hash,
                Duration::from_secs(self.commit_timeout_sec),
                Duration::from_millis(400),
            )
            .await?;
            eprintln!(
                "Faucet final: hash={} status={} success={} previewed={} submitted={} committed={}",
                committed.hash,
                committed.status,
                committed.success,
                committed.previewed,
                committed.submitted,
                committed.committed
            );
        }

        Ok(())
    }
}
