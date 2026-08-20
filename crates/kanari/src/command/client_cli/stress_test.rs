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
use kanari_rpc_api::TransactionErrorReason;
use kanari_rpc_api::{BuildNativeTransferRequest, ObjectTransferData, TransactionStatus};
use kanari_rpc_client::RpcClient;
use kanari_types::gas_coin::GAS_COIN;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::time::sleep;

const MAX_OBJECT_BUSY_RETRIES: usize = 1200;
const OBJECT_BUSY_RETRY_DELAY_MS: u64 = 250;
const MAX_LANE_SATURATION_RETRIES: usize = 1200;
const LANE_SATURATION_RETRY_DELAY_MS: u64 = 250;
// This is a client-side load-test backoff only. The node's rate limiter stays
// enabled so the campaign exercises the production admission boundary.
const MAX_RATE_LIMIT_RETRIES: usize = 120;
const RATE_LIMIT_RETRY_DELAY_MS: u64 = 1_000;

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

fn is_rate_limited(error: &anyhow::Error) -> bool {
    let message = format!("{:#}", error);
    message.contains("Rate limit exceeded")
        || message.contains("rate limit exceeded")
        || message.contains("code: -32005")
}

/// True only for failures where retrying the exact same signed transaction via
/// another honest RPC replica is safe.  Validation and execution failures must
/// never be hidden by endpoint failover.
fn is_rpc_transport_failure(error: &anyhow::Error) -> bool {
    let message = format!("{:#}", error).to_ascii_lowercase();
    [
        "connection refused",
        "connection reset",
        "connection aborted",
        "error sending request",
        "failed to connect",
        "tcp connect error",
        "timed out",
        "broken pipe",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

/// A pending transaction can be acknowledged by one RPC and then be absent
/// from another replica if the accepting node crashes before dissemination.
/// Retrying the already-signed payload is safe in this state; the transaction
/// hash, nonce, object refs, and signature do not change.
fn is_commit_unobservable(error: &anyhow::Error) -> bool {
    is_rpc_transport_failure(error)
        || format!("{:#}", error)
            .to_ascii_lowercase()
            .contains("transaction not found")
}

/// A retry may reach a replica that has already seen the exact payload through
/// gossip while the caller is still unable to observe its final status.  These
/// admission errors are idempotent acknowledgements, not execution failures;
/// continue polling that replica rather than marking a healthy recovery as a
/// failed transaction.
fn is_idempotent_retransmission_result(error: &anyhow::Error) -> bool {
    let message = format!("{:#}", error).to_ascii_lowercase();
    message.contains("already executed") || message.contains("already in pending pool")
}

/// Observe a transaction through any configured honest RPC replica.
///
/// During a planned validator restart the last replica used for submission can
/// disappear just after the transaction commits elsewhere.  The stress client
/// must distinguish that observation failure from a failed transaction, while
/// still requiring an actual committed status from at least one replica.
async fn wait_for_transaction_commit_from_any_rpc(
    rpc_endpoints: &[String],
    tx_hash: &str,
    timeout: Duration,
) -> Result<TransactionStatus> {
    let started = Instant::now();
    let mut last_unobservable = None;

    while started.elapsed() < timeout {
        for endpoint in rpc_endpoints {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break;
            }
            let observation_timeout = remaining.min(Duration::from_secs(2));
            let client = RpcClient::new(endpoint);
            match wait_for_transaction_commit(
                &client,
                tx_hash,
                observation_timeout,
                Duration::from_millis(250),
            )
            .await
            {
                Ok(status) => return Ok(status),
                Err(error) if is_commit_unobservable(&error) => last_unobservable = Some(error),
                Err(error) => return Err(error.context("commit observation failed")),
            }
        }
        sleep(Duration::from_millis(250)).await;
    }

    Err(last_unobservable.unwrap_or_else(|| {
        anyhow::anyhow!("No RPC replica returned a committed status for transaction {tx_hash}")
    }))
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

    /// Additional RPC replicas used only for safe transport failover. When a
    /// primary cannot confirm a commit, the exact same signed payload may be
    /// retransmitted to one of these replicas; no transaction is rebuilt.
    #[arg(long = "rpc-fallback")]
    pub rpc_fallback_endpoints: Vec<String>,

    /// Maximum time to wait for each submitted transaction to commit.
    ///
    /// A fault-injection run may deliberately restart validators while the
    /// transaction is pending, so callers can raise this without weakening
    /// transaction validation or consensus safety.
    #[arg(long, default_value_t = 60)]
    pub commit_timeout_sec: u64,
}

impl StressTest {
    pub async fn execute(&self) -> Result<()> {
        let mut rpc_endpoints = vec![get_rpc_endpoint(self.rpc_endpoint.clone())];
        for endpoint in &self.rpc_fallback_endpoints {
            let endpoint = endpoint.trim();
            if !endpoint.is_empty() && !rpc_endpoints.iter().any(|known| known == endpoint) {
                rpc_endpoints.push(endpoint.to_owned());
            }
        }
        let mut active_rpc_index = 0usize;
        let mut rpc = rpc_endpoints[active_rpc_index].clone();
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

        let mut client = RpcClient::new(&rpc);
        let mut last_connect_error = None;
        let mut last_observed_height = None;
        for (index, endpoint) in rpc_endpoints.iter().enumerate() {
            let candidate = RpcClient::new(endpoint);
            match candidate.get_block_height().await {
                Ok(height) => {
                    active_rpc_index = index;
                    rpc = endpoint.clone();
                    client = candidate;
                    last_observed_height = Some(height);
                    break;
                }
                Err(error) => last_connect_error = Some((endpoint, error)),
            }
        }
        let mut last_observed_height =
            last_observed_height.with_context(|| match last_connect_error {
                Some((endpoint, error)) => {
                    format!("Cannot connect to any RPC server; last endpoint {endpoint}: {error:#}")
                }
                None => "Cannot connect to any RPC server".to_owned(),
            })?;
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
        let mut submitted_transactions =
            Vec::<(String, ObjectTransferData)>::with_capacity(self.count as usize);
        let mut in_flight_object_ids = HashSet::<String>::new();
        let mut in_flight_transactions = Vec::<(String, Vec<String>)>::new();
        let start_time = Instant::now();
        let report_interval = (self.count / 20).max(1);

        for i in 0..self.count {
            let mut retry_count = 0usize;
            let mut lane_retry_count = 0usize;
            let mut rate_limit_retry_count = 0usize;
            let mut rpc_failover_count = 0usize;
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
                        .submit_object_transfer(signed.clone())
                        .await
                        .context("Failed to submit transaction")?;

                    Ok::<_, anyhow::Error>((
                        tx_nonce,
                        coin_object_id,
                        gas_object_id,
                        status,
                        signed,
                    ))
                }
                .await;

                match attempt_result {
                    Err(error)
                        if is_rpc_transport_failure(&error)
                            && rpc_failover_count + 1 < rpc_endpoints.len() =>
                    {
                        rpc_failover_count += 1;
                        active_rpc_index = (active_rpc_index + 1) % rpc_endpoints.len();
                        rpc = rpc_endpoints[active_rpc_index].clone();
                        client = RpcClient::new(&rpc);
                        eprintln!(
                            "  [{}/{}] RPC transport failure; failing over to {} ({}/{})",
                            i + 1,
                            self.count,
                            rpc,
                            rpc_failover_count,
                            rpc_endpoints.len() - 1,
                        );
                        sleep(Duration::from_millis(250)).await;
                    }
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
                    Err(error)
                        if is_rate_limited(&error)
                            && rate_limit_retry_count < MAX_RATE_LIMIT_RETRIES =>
                    {
                        rate_limit_retry_count += 1;
                        if rate_limit_retry_count == 1 || rate_limit_retry_count.is_multiple_of(10)
                        {
                            eprintln!(
                                "  [{}/{}] RPC rate limited; preserving server admission control and retrying (retry {}/{})",
                                i + 1,
                                self.count,
                                rate_limit_retry_count,
                                MAX_RATE_LIMIT_RETRIES
                            );
                        }
                        sleep(Duration::from_millis(RATE_LIMIT_RETRY_DELAY_MS)).await;
                    }
                    other => break other,
                }
            };

            match result {
                Ok((tx_nonce, coin_object_id, gas_object_id, status, signed)) => {
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
                    submitted_transactions.push((status.hash.clone(), signed));
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

        if !submitted_transactions.is_empty() {
            eprintln!();
            eprintln!(
                "--- Verifying {} submitted transactions committed successfully ---",
                submitted_transactions.len()
            );
            let mut committed_success = 0u64;
            for (index, (hash, signed)) in submitted_transactions.iter().enumerate() {
                let commit_started_at = Instant::now();
                // Do not wait for the full user timeout on the accepting node:
                // it may have acknowledged the pending tx and then died before
                // broadcasting it. A fallback receives the identical signed
                // payload, never a rebuilt transaction.
                let observation_timeout = Duration::from_secs(self.commit_timeout_sec.min(5));
                let mut commit_result = wait_for_transaction_commit(
                    &client,
                    hash,
                    observation_timeout,
                    Duration::from_millis(250),
                )
                .await;
                let mut failover_count = 0usize;
                while matches!(&commit_result, Err(error) if is_commit_unobservable(error))
                    && failover_count + 1 < rpc_endpoints.len()
                {
                    failover_count += 1;
                    active_rpc_index = (active_rpc_index + 1) % rpc_endpoints.len();
                    rpc = rpc_endpoints[active_rpc_index].clone();
                    client = RpcClient::new(&rpc);
                    eprintln!(
                        "  [{}/{}] commit not observable; retransmitting same signed payload via {} ({}/{})",
                        index + 1,
                        submitted_transactions.len(),
                        rpc,
                        failover_count,
                        rpc_endpoints.len() - 1,
                    );
                    match client.submit_object_transfer(signed.clone()).await {
                        Ok(status) if status.success => {}
                        Ok(status) => {
                            eprintln!(
                                "  [{}/{}] fallback submit returned status={} for hash={}; continuing commit check",
                                index + 1,
                                submitted_transactions.len(),
                                status.status,
                                hash,
                            );
                        }
                        Err(error) if is_commit_unobservable(&error) => {
                            commit_result = Err(error.context("fallback RPC transport failure"));
                            continue;
                        }
                        Err(error) if is_idempotent_retransmission_result(&error) => {
                            eprintln!(
                                "  [{}/{}] fallback already knows hash={}; continuing commit check",
                                index + 1,
                                submitted_transactions.len(),
                                hash,
                            );
                        }
                        Err(error) => {
                            commit_result =
                                Err(error.context("fallback RPC rejected retransmission"));
                            break;
                        }
                    }
                    commit_result = wait_for_transaction_commit(
                        &client,
                        hash,
                        observation_timeout,
                        Duration::from_millis(250),
                    )
                    .await;
                }
                // The short per-replica observation is only for fast failover.
                // Once every endpoint has been tried, retain the command-wide
                // timeout and poll every replica. A two-node crash
                // intentionally removes quorum for a few seconds, and the
                // node that accepted the transaction may restart before its
                // local history endpoint is reachable again.
                if matches!(&commit_result, Err(error) if is_commit_unobservable(error)) {
                    let total_timeout = Duration::from_secs(self.commit_timeout_sec);
                    if let Some(remaining) = total_timeout.checked_sub(commit_started_at.elapsed())
                        && !remaining.is_zero()
                    {
                        eprintln!(
                            "  [{}/{}] waiting up to {:.1}s for quorum recovery before declaring hash={} failed",
                            index + 1,
                            submitted_transactions.len(),
                            remaining.as_secs_f64(),
                            hash,
                        );
                        commit_result = wait_for_transaction_commit_from_any_rpc(
                            &rpc_endpoints,
                            hash,
                            remaining,
                        )
                        .await;
                    }
                }
                match commit_result {
                    Ok(status) if status.success && status.committed => {
                        committed_success += 1;
                    }
                    Ok(status) => {
                        fail_count += 1;
                        eprintln!(
                            "  [{}/{}] COMMIT FAILED status={} committed={} hash={}",
                            index + 1,
                            submitted_transactions.len(),
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
                            submitted_transactions.len(),
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
    use super::{
        is_commit_unobservable, is_idempotent_retransmission_result, is_lane_saturated,
        is_object_busy_or_unavailable, is_rate_limited, is_rpc_transport_failure,
    };

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

    #[test]
    fn retries_rpc_rate_limit_responses_only() {
        let error = anyhow::anyhow!("RPC error: Rate limit exceeded; retry later (code -32005)");
        assert!(is_rate_limited(&error));
        assert!(!is_rate_limited(&anyhow::anyhow!(
            "RPC error: invalid signature"
        )));
    }

    #[test]
    fn fails_over_only_for_rpc_transport_errors() {
        assert!(is_rpc_transport_failure(&anyhow::anyhow!(
            "Failed to submit transaction: error sending request for url: connection refused"
        )));
        assert!(!is_rpc_transport_failure(&anyhow::anyhow!(
            "RPC error: invalid signature"
        )));
    }

    #[test]
    fn retransmits_pending_transaction_missing_from_a_replica() {
        assert!(is_commit_unobservable(&anyhow::anyhow!(
            "Failed to fetch transaction abc: RPC error: Transaction not found (code: -32603)"
        )));
        assert!(!is_commit_unobservable(&anyhow::anyhow!(
            "RPC error: invalid signature"
        )));
    }

    #[test]
    fn accepts_idempotent_retransmission_admission_results() {
        assert!(is_idempotent_retransmission_result(&anyhow::anyhow!(
            "Submission failed: transaction abc already executed"
        )));
        assert!(is_idempotent_retransmission_result(&anyhow::anyhow!(
            "Submission failed: transaction abc already in pending pool"
        )));
        assert!(!is_idempotent_retransmission_result(&anyhow::anyhow!(
            "Submission failed: invalid signature"
        )));
    }
}
