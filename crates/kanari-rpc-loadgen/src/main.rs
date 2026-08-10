#![allow(clippy::print_stdout)]
// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, bail};
use clap::Parser;
use reqwest::StatusCode;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{sync::Semaphore, task::JoinSet};

#[derive(Parser, Debug, Clone)]
#[command(author, version, about)]
struct Args {
    /// RPC endpoint to target. Pass multiple --rpc-url values to distribute
    /// requests across a gateway pool or directly across multiple RPC nodes.
    #[arg(
        long = "rpc-url",
        value_name = "URL",
        default_value = "http://127.0.0.1:6767"
    )]
    rpc_urls: Vec<String>,

    #[arg(long, default_value_t = 10_000)]
    requests: usize,

    #[arg(long, default_value_t = 256)]
    concurrency: usize,

    #[arg(long, default_value_t = 5)]
    timeout_secs: u64,

    #[arg(long, default_value_t = 0)]
    malformed_every: usize,

    #[arg(long, default_value_t = 0)]
    oversized_every: usize,

    #[arg(long, default_value_t = 1_048_576)]
    oversized_bytes: usize,

    #[arg(long, default_value_t = false)]
    fail_on_rate_limit: bool,

    /// Fail the run if observed throughput is below this request/second value.
    #[arg(long)]
    min_rps: Option<f64>,

    /// Fail the run if p99 latency exceeds this many milliseconds.
    #[arg(long)]
    max_p99_ms: Option<u128>,

    /// Fail the run if malformed/oversized client rejections exceed this percent.
    #[arg(long)]
    max_client_rejected_percent: Option<f64>,

    /// Fail multi-endpoint runs when any endpoint deviates from perfect
    /// round-robin distribution by more than this percent.
    #[arg(long)]
    max_endpoint_imbalance_percent: Option<f64>,
}

#[derive(Debug)]
struct Sample {
    endpoint_index: usize,
    rpc_url: String,
    status: Option<StatusCode>,
    latency_ms: u128,
    error: Option<String>,
}

impl Sample {
    fn ok_2xx(&self) -> bool {
        self.status.is_some_and(|s| s.is_success())
    }

    fn rate_limited(&self) -> bool {
        self.status == Some(StatusCode::TOO_MANY_REQUESTS)
    }

    fn client_rejected(&self) -> bool {
        self.status
            .is_some_and(|s| s.is_client_error() && s != StatusCode::TOO_MANY_REQUESTS)
    }

    fn server_error(&self) -> bool {
        self.status.is_some_and(|s| s.is_server_error())
    }

    fn network_error(&self) -> bool {
        self.status.is_none()
    }
}

fn request_body(id: usize, args: &Args) -> String {
    if args.malformed_every > 0 && id.is_multiple_of(args.malformed_every) {
        return r#"{"jsonrpc":"2.0","method":"kanari_getStats","params":"#.to_string();
    }

    if args.oversized_every > 0 && id.is_multiple_of(args.oversized_every) {
        let blob = "x".repeat(args.oversized_bytes);
        return json!({
            "jsonrpc": "2.0",
            "method": "kanari_getStats",
            "params": { "blob": blob },
            "id": id,
        })
        .to_string();
    }

    let method = if id.is_multiple_of(2) {
        "kanari_health"
    } else {
        "kanari_getStats"
    };
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": {},
        "id": id,
    })
    .to_string()
}

async fn send_one(
    client: reqwest::Client,
    endpoint_index: usize,
    rpc_url: String,
    body: String,
) -> Sample {
    let started = Instant::now();
    let result = client
        .post(&rpc_url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await;

    let latency_ms = started.elapsed().as_millis();
    match result {
        Ok(response) => Sample {
            endpoint_index,
            rpc_url,
            status: Some(response.status()),
            latency_ms,
            error: None,
        },
        Err(error) => Sample {
            endpoint_index,
            rpc_url,
            status: None,
            latency_ms,
            error: Some(error.to_string()),
        },
    }
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[index.saturating_sub(1).min(sorted.len() - 1)]
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if args.requests == 0 {
        bail!("--requests must be greater than zero");
    }
    if args.concurrency == 0 {
        bail!("--concurrency must be greater than zero");
    }
    validate_percent(
        "max-client-rejected-percent",
        args.max_client_rejected_percent,
    )?;
    validate_percent(
        "max-endpoint-imbalance-percent",
        args.max_endpoint_imbalance_percent,
    )?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(args.timeout_secs))
        .pool_max_idle_per_host(args.concurrency)
        .build()
        .context("failed to build reqwest client")?;

    for rpc_url in &args.rpc_urls {
        let probe: Value = client
            .post(rpc_url)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "kanari_health",
                "params": {},
                "id": 0,
            }))
            .send()
            .await
            .with_context(|| format!("RPC endpoint is not reachable: {rpc_url}"))?
            .json()
            .await
            .with_context(|| format!("RPC endpoint did not return JSON: {rpc_url}"))?;

        if probe.get("result").is_none() {
            bail!("RPC endpoint did not return a health result from {rpc_url}: {probe}");
        }
    }

    println!("Kanari Rust RPC load generator");
    println!("  urls={}", args.rpc_urls.join(", "));
    println!(
        "  requests={} concurrency={} timeout={}s malformed_every={} oversized_every={}",
        args.requests,
        args.concurrency,
        args.timeout_secs,
        args.malformed_every,
        args.oversized_every
    );

    let semaphore = Arc::new(Semaphore::new(args.concurrency));
    let completed = Arc::new(AtomicUsize::new(0));
    let mut set = JoinSet::new();
    let started = Instant::now();

    for id in 0..args.requests {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .context("semaphore closed")?;
        let client = client.clone();
        let endpoint_index = id % args.rpc_urls.len();
        let rpc_url = args.rpc_urls[endpoint_index].clone();
        let body = request_body(id, &args);
        let completed = completed.clone();

        set.spawn(async move {
            let _permit = permit;
            let sample = send_one(client, endpoint_index, rpc_url, body).await;
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if done.is_multiple_of(1_000) {
                println!("  completed={done}");
            }
            sample
        });
    }

    let mut samples = Vec::with_capacity(args.requests);
    while let Some(result) = set.join_next().await {
        samples.push(result.context("load task panicked")?);
    }

    let elapsed = started.elapsed();
    let mut latencies: Vec<_> = samples.iter().map(|s| s.latency_ms).collect();
    latencies.sort_unstable();

    let success = samples.iter().filter(|s| s.ok_2xx()).count();
    let rate_limited = samples.iter().filter(|s| s.rate_limited()).count();
    let client_rejected = samples.iter().filter(|s| s.client_rejected()).count();
    let server_errors = samples.iter().filter(|s| s.server_error()).count();
    let network_errors = samples.iter().filter(|s| s.network_error()).count();
    let rps = samples.len() as f64 / elapsed.as_secs_f64().max(0.001);

    println!("complete");
    println!(
        "  completed={}/{} duration={:.2}s rps={:.2}",
        samples.len(),
        args.requests,
        elapsed.as_secs_f64(),
        rps
    );
    println!(
        "  success={} rate_limited={} client_rejected={} server_errors={} network_errors={}",
        success, rate_limited, client_rejected, server_errors, network_errors
    );
    let p50 = percentile(&latencies, 50.0);
    let p95 = percentile(&latencies, 95.0);
    let p99 = percentile(&latencies, 99.0);
    let max_latency = latencies.last().copied().unwrap_or_default();
    println!("  latency_ms p50={p50} p95={p95} p99={p99} max={max_latency}");

    let mut by_endpoint: BTreeMap<(usize, &str), usize> = BTreeMap::new();
    for sample in &samples {
        *by_endpoint
            .entry((sample.endpoint_index, sample.rpc_url.as_str()))
            .or_default() += 1;
    }
    let endpoint_counts: Vec<usize> = (0..args.rpc_urls.len())
        .map(|slot| {
            samples
                .iter()
                .filter(|sample| sample.endpoint_index == slot)
                .count()
        })
        .collect();
    if args.rpc_urls.len() > 1 {
        println!("  endpoint_distribution:");
        for ((slot, url), count) in by_endpoint {
            println!("    slot[{slot}] {url}: {count}");
        }
    }

    if let Some(first_error) = samples.iter().find_map(|s| s.error.as_ref()) {
        println!("  first_network_error={first_error}");
    }

    if samples.len() != args.requests {
        bail!("lost request results");
    }
    if server_errors > 0 {
        bail!("server errors observed");
    }
    if network_errors > 0 {
        bail!("network errors observed");
    }
    if args.fail_on_rate_limit && rate_limited > 0 {
        bail!("rate limits observed");
    }
    if let Some(min_rps) = args.min_rps
        && rps < min_rps
    {
        bail!("throughput below threshold: observed {rps:.2} rps < required {min_rps:.2} rps");
    }
    if let Some(max_p99_ms) = args.max_p99_ms
        && p99 > max_p99_ms
    {
        bail!("p99 latency above threshold: observed {p99} ms > allowed {max_p99_ms} ms");
    }
    if let Some(max_client_rejected_percent) = args.max_client_rejected_percent {
        let rejected_percent = client_rejected as f64 * 100.0 / samples.len().max(1) as f64;
        if rejected_percent > max_client_rejected_percent {
            bail!(
                "client rejection rate above threshold: observed {rejected_percent:.2}% > allowed {max_client_rejected_percent:.2}%"
            );
        }
    }
    if let Some(max_endpoint_imbalance_percent) = args.max_endpoint_imbalance_percent
        && args.rpc_urls.len() > 1
    {
        let expected = samples.len() as f64 / args.rpc_urls.len() as f64;
        for (slot, count) in endpoint_counts.iter().enumerate() {
            let deviation = (*count as f64 - expected).abs() * 100.0 / expected.max(1.0);
            if deviation > max_endpoint_imbalance_percent {
                bail!(
                    "endpoint slot[{slot}] imbalance above threshold: observed {deviation:.2}% > allowed {max_endpoint_imbalance_percent:.2}%"
                );
            }
        }
    }

    Ok(())
}

fn validate_percent(name: &str, value: Option<f64>) -> Result<()> {
    if let Some(value) = value
        && !(0.0..=100.0).contains(&value)
    {
        bail!("--{name} must be between 0 and 100");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_with_patterns(malformed_every: usize, oversized_every: usize) -> Args {
        Args {
            rpc_urls: vec!["http://127.0.0.1:6767".to_string()],
            requests: 10,
            concurrency: 2,
            timeout_secs: 5,
            malformed_every,
            oversized_every,
            oversized_bytes: 8,
            fail_on_rate_limit: false,
            min_rps: None,
            max_p99_ms: None,
            max_client_rejected_percent: None,
            max_endpoint_imbalance_percent: None,
        }
    }

    #[test]
    fn request_body_prefers_malformed_over_oversized() {
        let args = args_with_patterns(2, 2);
        let body = request_body(2, &args);
        assert!(body.ends_with(r#""params":"#));
    }

    #[test]
    fn request_body_can_generate_bounded_oversized_probe() {
        let args = args_with_patterns(0, 3);
        let body = request_body(3, &args);
        assert!(body.contains("\"kanari_getStats\""));
        assert!(body.contains("\"xxxxxxxx\""));
    }

    #[test]
    fn percentile_handles_empty_and_edges() {
        assert_eq!(percentile(&[], 95.0), 0);
        assert_eq!(percentile(&[10, 20, 30, 40], 50.0), 20);
        assert_eq!(percentile(&[10, 20, 30, 40], 99.0), 40);
    }

    #[test]
    fn sample_classification_is_stable() {
        let sample = Sample {
            endpoint_index: 0,
            rpc_url: "http://node".to_string(),
            status: Some(StatusCode::TOO_MANY_REQUESTS),
            latency_ms: 1,
            error: None,
        };
        assert!(sample.rate_limited());
        assert!(!sample.client_rejected());

        let malformed = Sample {
            endpoint_index: 0,
            rpc_url: "http://node".to_string(),
            status: Some(StatusCode::BAD_REQUEST),
            latency_ms: 1,
            error: None,
        };
        assert!(malformed.client_rejected());
        assert!(!malformed.server_error());
    }

    #[test]
    fn percent_thresholds_are_validated() {
        validate_percent("example", None).unwrap();
        validate_percent("example", Some(0.0)).unwrap();
        validate_percent("example", Some(100.0)).unwrap();
        assert!(validate_percent("example", Some(-0.1)).is_err());
        assert!(validate_percent("example", Some(100.1)).is_err());
    }
}
