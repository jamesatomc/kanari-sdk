// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
use clap::Subcommand;

pub mod balance;
pub mod burn;
pub mod envs;
pub mod fanout;
pub mod faucet;
pub mod objects;
pub mod owner;
pub mod stats;
pub mod stress_test;
pub mod token_transfer;
pub mod transfer;
pub mod tx;
pub mod view;

#[derive(Subcommand, Debug)]
pub enum ClientCommand {
    /// Transfer Kanari tokens to another address
    Transfer(transfer::Transfer),
    /// Transfer custom tokens (e.g., JAMES) to another address
    TokenTransfer(token_transfer::TokenTransfer),
    /// Request tokens from the Dev faucet
    Faucet(faucet::Faucet),
    /// Split a native coin into many owned coin objects for parallel load testing
    Fanout(fanout::Fanout),
    /// Burn Kanari tokens from a wallet (remove from total supply)
    Burn(burn::Burn),
    /// Show blockchain statistics
    Stats(stats::Stats),
    /// Show token balances for an owner address
    Balance(balance::Balance),
    /// Show transaction details by hash
    Tx(tx::Tx),
    /// Owner operations (object-centric state queries)
    Owner {
        #[command(subcommand)]
        command: owner::OwnerCommand,
    },
    /// Manage environments (RPC endpoints)
    Envs(envs::Envs),
    /// List owned objects
    Objects(objects::Objects),
    /// Call view functions (read-only, no transaction)
    View(view::View),
    /// Stress test: faucet tokens and send N transactions
    StressTest(stress_test::StressTest),
}

impl ClientCommand {
    pub async fn execute(&self) -> Result<()> {
        match self {
            ClientCommand::Transfer(cmd) => cmd.execute().await,
            ClientCommand::TokenTransfer(cmd) => cmd.execute().await,
            ClientCommand::Faucet(cmd) => cmd.execute().await,
            ClientCommand::Fanout(cmd) => cmd.execute().await,
            ClientCommand::Burn(cmd) => cmd.execute().await,
            ClientCommand::Stats(cmd) => cmd.execute().await,
            ClientCommand::Balance(cmd) => cmd.execute().await,
            ClientCommand::Tx(cmd) => cmd.execute().await,
            ClientCommand::Owner { command } => command.execute().await,
            ClientCommand::Envs(cmd) => cmd.execute(),
            ClientCommand::Objects(cmd) => cmd.execute().await,
            ClientCommand::View(cmd) => cmd.execute().await,
            ClientCommand::StressTest(cmd) => cmd.execute().await,
        }
    }
}
