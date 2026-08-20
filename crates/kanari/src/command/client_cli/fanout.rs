// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Create many owned native coin objects in one Move transaction.
//!
//! This is intentionally a load-test preparation tool: parallel native-transfer
//! lanes need independent mutable transfer and gas objects.  It avoids the
//! misleading throughput and liveness results produced by funding one object
//! at a time.

use crate::command::common::{
    check_node_connection, get_rpc_endpoint, get_sender_for_tx, load_wallet_for, normalize_addr,
    resolve_sender, resolve_transaction_gas,
};
use crate::command::rpc_helpers::{
    should_wait_for_commit, sign_and_call_function, wait_for_transaction_commit,
};
use crate::command::tx_output::print_transaction_status;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use kanari_rpc_api::BuildCallFunctionRequest;
use kanari_rpc_client::RpcClient;
use kanari_types::transaction::{ObjectInput, ObjectRef};
use kanari_types::{coin::CoinModule, gas_coin::GAS_COIN};
use move_core_types::account_address::AccountAddress;
use std::time::Duration;

fn native_coin_balance(data: &[u8]) -> Option<u64> {
    CoinModule::read_balance(data)
}

#[derive(Parser, Debug)]
pub struct Fanout {
    /// Wallet that owns the source native Coin<KANARI> object.
    #[arg(long)]
    pub from: Option<String>,

    /// Wallet password. If omitted, KANARI_WALLET_PASSWORD or the interactive prompt is used.
    #[arg(short, long)]
    pub password: Option<String>,

    /// Number of new owned coin objects to create in this transaction.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=512))]
    pub count: u32,

    /// Value of each new coin in KANARI.
    #[arg(long)]
    pub amount: f64,

    /// RPC endpoint.
    #[arg(long = "rpc")]
    pub rpc_endpoint: Option<String>,

    /// Maximum seconds to wait for the fanout transaction to commit.
    #[arg(long, default_value_t = 120)]
    pub commit_timeout_sec: u64,
}

impl Fanout {
    pub async fn execute(&self) -> Result<()> {
        const MIST_PER_KANARI: f64 = 1_000_000_000.0;
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let from_addr = resolve_sender(self.from.clone())?;
        let wallet = load_wallet_for(&from_addr, self.password.clone())?;
        let (gas_limit, gas_price) = resolve_transaction_gas(None, None);
        let amount_mist = (self.amount * MIST_PER_KANARI).round() as u64;
        ensure!(
            amount_mist > 0,
            "--amount is too small; it rounds to 0 Mist"
        );

        let total_mist = amount_mist
            .checked_mul(u64::from(self.count))
            .context("fanout amount overflow")?;
        let client = RpcClient::new(&rpc);
        check_node_connection(&client, &rpc).await?;
        let sender_for_tx = get_sender_for_tx(&wallet, &from_addr)?;
        let amounts = vec![amount_mist; self.count as usize];

        let native_coin_type = format!("0x2::coin::Coin<{GAS_COIN}>");
        let owner = client.get_owner(&from_addr).await?;
        let source = owner
            .owned_objects
            .unwrap_or_default()
            .into_iter()
            .filter(|object| object.type_ == native_coin_type)
            .filter_map(|object| {
                // Coin<T> is a structured BCS value.  Its balance is not the
                // trailing eight bytes (the UID occupies the tail), so always
                // use the canonical parser shared with the RPC server.
                let balance = native_coin_balance(&object.data)?;
                Some((object, balance))
            })
            .filter(|(_, balance)| *balance >= total_mist)
            .max_by_key(|(_, balance)| *balance)
            .map(|(object, _)| object)
            .context("no native coin object can fund this fanout")?;
        let source_object_id = AccountAddress::from_hex_literal(&normalize_addr(&source.id)?)
            .context("failed to encode source coin object id")?;
        // `split_vec` receives its source as `&mut Coin<T>`.  Supplying the
        // complete object reference is required here: relying on the RPC
        // builder to infer an input from the raw 32-byte ID is ambiguous when
        // module metadata is unavailable.  Without it the VM attempts to
        // deserialize the ID as a Coin value and rejects the transaction.
        let source_input = ObjectInput {
            object_ref: ObjectRef::new(
                source.id.clone(),
                Some(source.version),
                source.digest.clone(),
            ),
            owner: Some(source.owner_kind.clone()),
            mutable: true,
        };

        eprintln!(
            "Fanout: creating {} owned Coin<KANARI> object(s), {} Mist each ({} Mist total)",
            self.count, amount_mist, total_mist
        );
        let prepared = client
            .build_call_function(BuildCallFunctionRequest {
                sender: sender_for_tx,
                package: "0x2".to_string(),
                module: "pay".to_string(),
                function: "split_vec".to_string(),
                type_args: vec![GAS_COIN.to_string()],
                args: vec![source_object_id.to_vec(), bcs::to_bytes(&amounts)?],
                object_inputs: Some(vec![source_input]),
                gas_limit,
                gas_price,
                nonce: None,
                execute_immediate: Some(true),
            })
            .await
            .context("failed to build native fanout transaction")?;

        let status = sign_and_call_function(&client, &wallet, prepared)
            .await
            .context("failed to submit native fanout transaction")?;
        print_transaction_status("  ", &status);
        if should_wait_for_commit(
            status.success,
            status.previewed,
            status.submitted,
            status.committed,
        ) {
            let committed = wait_for_transaction_commit(
                &client,
                &status.hash,
                Duration::from_secs(self.commit_timeout_sec),
                Duration::from_millis(250),
            )
            .await?;
            print_transaction_status("  Final: ", &committed);
            ensure!(
                committed.success && committed.committed,
                "fanout did not commit successfully"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::native_coin_balance;

    #[test]
    fn reads_the_canonical_coin_balance_not_trailing_object_bytes() {
        let expected = 168_000_000_000u64;
        let mut object = vec![0u8; 72];
        object[32..40].copy_from_slice(&expected.to_le_bytes());
        object[64..72].copy_from_slice(&1u64.to_le_bytes());

        assert_eq!(native_coin_balance(&object), Some(expected));
    }
}
