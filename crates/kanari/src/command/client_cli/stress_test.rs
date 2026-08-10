// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Stress test command: faucet tokens then send N transfer transactions.
//!
//! Usage:
//! ```bash
//! kanari client stress-test --to 0x... --amount 0.1 --faucet --count 1000
//! ```

use crate::command::common::{
    get_rpc_endpoint, get_sender_for_tx, load_wallet_for, native_transfer_policy_hint,
    normalize_addr, resolve_sender, resolve_transaction_gas, transaction_error_reason,
};
use crate::command::rpc_helpers::{sign_object_transfer_request, wait_for_transaction_commit};
use anyhow::{Context, Result, bail};
use clap::Parser;
use kanari_rpc_api::BuildNativeTransferRequest;
use kanari_rpc_api::TransactionErrorReason;
use kanari_rpc_client::RpcClient;
use kanari_types::gas_coin::GAS_COIN;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::time::sleep;

const MAX_OBJECT_BUSY_RETRIES: usize = 1200;
const OBJECT_BUSY_RETRY_DELAY_MS: u64 = 250;
const MAX_LANE_SATURATION_RETRIES: usize = 1200;
const LANE_SATURATION_RETRY_DELAY_MS: u64 = 250;

fn native_coin_object_count(owner: &kanari_rpc_api::OwnerInfo) -> usize {
    let native_coin_type = format!("0x2::coin::Coin<{}>", GAS_COIN);
    owner.owned_objects.as_ref().map_or(0, |objects| {
        objects
            .iter()
            .filter(|object| object.type_ == native_coin_type)
            .count()
    })
}

fn is_object_busy_or_unavailable(error: &anyhow::Error) -> bool {
    let message = format!("{:#}", error);
    matches!(
        transaction_error_reason(error),
        Some(TransactionErrorReason::NativeTransferPolicyNotSatisfied)
            | Some(TransactionErrorReason::NativeCoinConsolidationBlocked)
    ) || message.contains("No spendable native gas coin object found")
        || message.contains("No single Coin<")
        || message.contains("insufficient_transfer_coin_balance")
        || message.contains("native_transfer_policy_not_satisfied")
        || message.contains("Native transfer requires two distinct Coin<")
}

fn is_lane_saturated(error: &anyhow::Error) -> bool {
    let message = format!("{:#}", error);
    message.contains("is saturated")
        && (message.contains("primary access key") || message.contains("canonical access key"))
}

#[derive(Parser, Debug)]
pub struct StressTest {
    /// Sender wallet address (optional). If omitted, uses selected wallet in config.
    #[arg(short, long)]
    pub from: Option<String>,

    /// Recipient address
    #[arg(short, long)]
    pub to: String,

    /// Amount in Kanari per transaction (will be converted to Mist)
    #[arg(short, long)]
    pub amount: f64,

    /// Number of transactions to send (default: 1000)
    #[arg(short, long, default_value_t = 1000)]
    pub count: u64,

    /// Wallet password
    #[arg(short, long)]
    pub password: String,

    /// Request tokens from dev faucet first (funds the sender)
    #[arg(long)]
    pub faucet: bool,

    /// Faucet amount in Kanari (default: amount * count * 2 to cover all txs + gas)
    #[arg(long)]
    pub faucet_amount: Option<f64>,

    /// Dev faucet address override
    #[arg(long)]
    pub dev_address: Option<String>,

    /// Dev faucet password override
    #[arg(long)]
    pub dev_password: Option<String>,

    /// RPC endpoint
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
}

impl StressTest {
    pub async fn execute(&self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let from_addr = resolve_sender(self.from.clone())?;
        let to_addr = normalize_addr(&self.to)?;
        let wallet = load_wallet_for(&from_addr, Some(self.password.clone()))?;
        let (gas_limit, gas_price) = resolve_transaction_gas(None, None);

        const MIST_PER_KANARI: f64 = 1_000_000_000.0;
        let amount_mist = (self.amount * MIST_PER_KANARI).round() as u64;
        if amount_mist == 0 {
            bail!("--amount is too small; it rounds to 0 Mist");
        }

        eprintln!("=== Kanari Stress Test ===");
        eprintln!("  From:     {}", from_addr);
        eprintln!("  To:       {}", to_addr);
        eprintln!("  Per-tx:   {} KANARI ({} Mist)", self.amount, amount_mist);
        eprintln!("  Count:    {} transactions", self.count);
        eprintln!(
            "  Gas:      limit={} price={} Mist/gas",
            gas_limit, gas_price
        );

        let client = RpcClient::new(&rpc);

        let mut last_observed_height = client
            .get_block_height()
            .await
            .context("Cannot connect to RPC server")?;
        eprintln!("  Node height: {}", last_observed_height);

        if self.faucet {
            let faucet_amt = self
                .faucet_amount
                .unwrap_or((self.amount * self.count as f64) * 10.0);
            eprintln!();
            eprintln!("--- Faucet: Requesting {:.9} KANARI ---", faucet_amt);
            match kanari_faucet::request_from_dev(
                self.dev_address.as_deref(),
                self.dev_password.as_deref(),
                Some(&from_addr),
                faucet_amt,
                &rpc,
            )
            .await
            {
                Ok(_) => {
                    eprintln!("  Faucet done. Waiting 2s for node to process...");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => {
                    eprintln!("  Faucet warning (continuing anyway): {}", e);
                }
            }
        }

        let owner = client
            .get_owner(&from_addr)
            .await
            .context("Failed to get sender owner state")?;

        let native_balance = owner.balances.get(GAS_COIN).copied().unwrap_or(0);
        let total_needed = amount_mist.saturating_mul(self.count);
        let gas_per_tx = gas_limit.saturating_mul(gas_price);
        let total_gas = gas_per_tx.saturating_mul(self.count);
        let total_required = total_needed.saturating_add(total_gas);

        eprintln!();
        eprintln!("--- Balance check ---");
        eprintln!(
            "  Native balance: {:.9} KANARI",
            native_balance as f64 / MIST_PER_KANARI
        );
        eprintln!(
            "  Transfer total: {:.9} KANARI",
            total_needed as f64 / MIST_PER_KANARI
        );
        eprintln!(
            "  Est. gas:       {:.9} KANARI",
            total_gas as f64 / MIST_PER_KANARI
        );
        eprintln!(
            "  Required:       {:.9} KANARI",
            total_required as f64 / MIST_PER_KANARI
        );
        eprintln!(
            "  Native coins:   {} object(s)",
            native_coin_object_count(&owner)
        );
        let native_coin_count = native_coin_object_count(&owner);
        let approximate_inflight_pairs = native_coin_count / 2;
        if self.count > 1 && approximate_inflight_pairs <= 1 {
            eprintln!(
                "  Bottleneck:     only ~{} native transfer/gas pair(s); this lane will serialize on object version commits",
                approximate_inflight_pairs
            );
            eprintln!(
                "                  For parallel load, pre-fund each sender with more Coin<{}> objects.",
                GAS_COIN
            );
        }

        if native_balance < total_required {
            bail!(
                "Insufficient balance: have {:.9} KANARI, need {:.9} KANARI\n\
                 Use --faucet or request more tokens.",
                native_balance as f64 / MIST_PER_KANARI,
                total_required as f64 / MIST_PER_KANARI,
            );
        }

        let sender_tagged = get_sender_for_tx(&wallet, &from_addr)?;

        eprintln!();
        eprintln!("--- Sending {} transactions ---", self.count);

        let mut success_count = 0u64;
        let mut fail_count = 0u64;
        let mut submitted_hashes = Vec::with_capacity(self.count as usize);
        let mut in_flight_object_ids = HashSet::<String>::new();
        let mut in_flight_transactions = Vec::<(String, Vec<String>)>::new();
        let start_time = Instant::now();
        let report_interval = (self.count / 20).max(1);

        for i in 0..self.count {
            let mut retry_count = 0usize;
            let mut lane_retry_count = 0usize;
            let result = loop {
                let excluded_object_ids = in_flight_object_ids.iter().cloned().collect();
                let attempt_result = async {
                    let prepared = client
                        .build_native_transfer(BuildNativeTransferRequest {
                            sender: sender_tagged.clone(),
                            recipient: to_addr.clone(),
                            amount: amount_mist,
                            gas_limit,
                            gas_price,
                            excluded_object_ids,
                            nonce: None,
                            execute_immediate: Some(false),
                        })
                        .await
                        .map_err(|error| {
                            let policy_suffix = native_transfer_policy_hint(&error)
                                .map(|summary| format!(" Policy: {}.", summary))
                                .unwrap_or_default();
                            anyhow::anyhow!(
                                "Failed to build native transfer transaction for sender {}: {:#}{}",
                                from_addr,
                                error,
                                policy_suffix
                            )
                        })?;

                    let tx_nonce = prepared
                        .canonical_nonce()
                        .map_err(crate::command::rpc_helpers::map_nonce_error)?;
                    let coin_object_id = prepared.coin_object_id.clone();
                    let gas_object_id = prepared
                        .gas_payment
                        .as_ref()
                        .and_then(|gas| gas.payment_objects.first())
                        .map(|obj| obj.object_id.clone());

                    let signed = sign_object_transfer_request(prepared, &wallet)?;
                    let status = client
                        .submit_object_transfer(signed)
                        .await
                        .context("Failed to submit transaction")?;

                    Ok::<_, anyhow::Error>((tx_nonce, coin_object_id, gas_object_id, status))
                }
                .await;

                match attempt_result {
                    Err(error)
                        if is_object_busy_or_unavailable(&error)
                            && retry_count < MAX_OBJECT_BUSY_RETRIES =>
                    {
                        retry_count += 1;
                        if retry_count == 1 || retry_count.is_multiple_of(20) {
                            eprintln!(
                                "  [{}/{}] waiting for coin/gas object refs to commit (retry {}/{})",
                                i + 1,
                                self.count,
                                retry_count,
                                MAX_OBJECT_BUSY_RETRIES
                            );
                        }
                        let mut still_in_flight = Vec::with_capacity(in_flight_transactions.len());
                        for (hash, object_ids) in in_flight_transactions.drain(..) {
                            let release_objects = client
                                .get_transaction(&hash)
                                .await
                                .map(|details| details.committed || !details.success)
                                .unwrap_or(false);
                            if release_objects {
                                for object_id in object_ids {
                                    in_flight_object_ids.remove(&object_id);
                                }
                            } else {
                                still_in_flight.push((hash, object_ids));
                            }
                        }
                        in_flight_transactions = still_in_flight;
                        sleep(Duration::from_millis(OBJECT_BUSY_RETRY_DELAY_MS)).await;
                    }
                    Err(error)
                        if is_lane_saturated(&error)
                            && lane_retry_count < MAX_LANE_SATURATION_RETRIES =>
                    {
                        lane_retry_count += 1;
                        let current_height = client
                            .get_block_height()
                            .await
                            .unwrap_or(last_observed_height);
                        let height_advanced = current_height > last_observed_height;
                        last_observed_height = current_height;

                        if lane_retry_count == 1
                            || lane_retry_count.is_multiple_of(20)
                            || height_advanced
                        {
                            eprintln!(
                                "  [{}/{}] lane saturated, waiting for checkpoint drain (retry {}/{}, height={})",
                                i + 1,
                                self.count,
                                lane_retry_count,
                                MAX_LANE_SATURATION_RETRIES,
                                current_height
                            );
                        }

                        sleep(Duration::from_millis(LANE_SATURATION_RETRY_DELAY_MS)).await;
                    }
                    other => break other,
                }
            };

            match result {
                Ok((tx_nonce, coin_object_id, gas_object_id, status)) => {
                    if !status.success || (!status.submitted && !status.committed) {
                        fail_count += 1;
                        eprintln!(
                            "  [{}/{}] FAILED nonce={} status={} submitted={} committed={} previewed={} hash={}",
                            i + 1,
                            self.count,
                            tx_nonce,
                            status.status,
                            status.submitted,
                            status.committed,
                            status.previewed,
                            status.hash
                        );
                        continue;
                    }

                    success_count += 1;
                    submitted_hashes.push(status.hash.clone());
                    let mut used_object_ids = vec![coin_object_id.clone()];
                    if let Some(gas_object_id) = &gas_object_id {
                        used_object_ids.push(gas_object_id.clone());
                    }
                    for object_id in &used_object_ids {
                        in_flight_object_ids.insert(object_id.clone());
                    }
                    in_flight_transactions.push((status.hash.clone(), used_object_ids));
                    if i == 0 {
                        eprintln!("  First coin object: {}", coin_object_id);
                        if let Some(gas_object_id) = gas_object_id {
                            eprintln!("  First gas object:  {}", gas_object_id);
                        }
                    }
                    if (i + 1) % report_interval == 0 || i + 1 == self.count {
                        let elapsed = start_time.elapsed();
                        let tps = if elapsed.as_secs_f64() > 0.0 {
                            success_count as f64 / elapsed.as_secs_f64()
                        } else {
                            0.0
                        };
                        eprintln!(
                            "  [{}/{}] hash={} nonce={} tps={:.1}",
                            i + 1,
                            self.count,
                            &status.hash[..16.min(status.hash.len())],
                            tx_nonce,
                            tps,
                        );
                    }
                }
                Err(e) => {
                    fail_count += 1;
                    eprintln!("  [{}/{}] FAILED: {:#}", i + 1, self.count, e);
                    if i == 0 {
                        if let Some(reason) = transaction_error_reason(&e) {
                            eprintln!("           RPC reason: {}", reason);
                        }
                        if let Some(summary) = native_transfer_policy_hint(&e) {
                            eprintln!("           Policy: {}", summary);
                        } else {
                            eprintln!(
                                "           Hint: native transfer needs two distinct Coin<{}> objects",
                                GAS_COIN
                            );
                            eprintln!(
                                "                 one mutable transfer coin and one separate gas coin"
                            );
                        }
                    }
                }
            }
        }

        if !submitted_hashes.is_empty() {
            eprintln!();
            eprintln!(
                "--- Verifying {} submitted transactions committed successfully ---",
                submitted_hashes.len()
            );
            let mut committed_success = 0u64;
            for (index, hash) in submitted_hashes.iter().enumerate() {
                match wait_for_transaction_commit(
                    &client,
                    hash,
                    Duration::from_secs(60),
                    Duration::from_millis(250),
                )
                .await
                {
                    Ok(status) if status.success && status.committed => {
                        committed_success += 1;
                    }
                    Ok(status) => {
                        fail_count += 1;
                        eprintln!(
                            "  [{}/{}] COMMIT FAILED status={} committed={} hash={}",
                            index + 1,
                            submitted_hashes.len(),
                            status.status,
                            status.committed,
                            hash
                        );
                    }
                    Err(error) => {
                        fail_count += 1;
                        eprintln!(
                            "  [{}/{}] COMMIT CHECK FAILED hash={}: {:#}",
                            index + 1,
                            submitted_hashes.len(),
                            hash,
                            error
                        );
                    }
                }
            }
            success_count = committed_success;
        }

        let total_elapsed = start_time.elapsed();

        eprintln!();
        eprintln!("=== Stress Test Complete ===");
        eprintln!("  Total:    {} tx", self.count);
        eprintln!("  Success:  {}", success_count);
        eprintln!("  Failed:   {}", fail_count);
        eprintln!("  Duration: {:.2?}", total_elapsed);
        if total_elapsed.as_secs_f64() > 0.0 {
            eprintln!(
                "  TPS:      {:.1}",
                success_count as f64 / total_elapsed.as_secs_f64()
            );
        }

        if fail_count > 0 {
            bail!("{} of {} transactions failed", fail_count, self.count);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{is_lane_saturated, is_object_busy_or_unavailable};

    #[test]
    fn retries_distinct_native_coin_policy_while_prior_refs_are_pending() {
        let error = anyhow::anyhow!(
            "RPC error: Native transfer requires two distinct Coin<0x2::kanari::KANARI> objects (reason: native_transfer_policy_not_satisfied)"
        );
        assert!(is_object_busy_or_unavailable(&error));
    }

    #[test]
    fn does_not_retry_unrelated_transaction_errors() {
        let error = anyhow::anyhow!("RPC error: invalid signature");
        assert!(!is_object_busy_or_unavailable(&error));
    }

    #[test]
    fn retries_primary_lane_saturation() {
        let error = anyhow::anyhow!(
            "RPC error: Transaction lane 0xabc is saturated: 300 pending transaction(s) already target this primary access key, max 256"
        );
        assert!(is_lane_saturated(&error));
    }

    #[test]
    fn retries_congestion_lane_saturation() {
        let error = anyhow::anyhow!(
            "RPC error: Transaction congestion lane 0xabc is saturated: 300 pending transaction(s) already touch this canonical access key, max 256"
        );
        assert!(is_lane_saturated(&error));
    }

    #[test]
    fn does_not_retry_unrelated_saturation_messages() {
        let error = anyhow::anyhow!("RPC error: something else is saturated");
        assert!(!is_lane_saturated(&error));
    }
}
