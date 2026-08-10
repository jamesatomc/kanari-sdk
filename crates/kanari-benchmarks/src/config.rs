// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result, bail};

pub const DEFAULT_SENDER_COUNT: usize = 64;
pub const HIGH_THROUGHPUT_TX_COUNT: usize = 10_000;
pub const HIGH_THROUGHPUT_SENDER_COUNT: usize = HIGH_THROUGHPUT_TX_COUNT;
pub const HIGH_THROUGHPUT_RUNS: usize = 3;
pub const HIGH_THROUGHPUT_TARGET_TPS: f64 = 50_000.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessMode {
    Admission,
    OwnedFastPath,
    Production,
    Immediate,
}

impl HarnessMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admission => "admission",
            Self::OwnedFastPath => "owned-fastpath",
            Self::Production => "production",
            Self::Immediate => "immediate",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarnessConfig {
    pub tx_count: usize,
    pub sender_count: Option<usize>,
    pub runs: usize,
    pub json: bool,
    pub mode: HarnessMode,
    pub target_tps: Option<f64>,
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            tx_count: 512,
            sender_count: None,
            runs: 1,
            json: false,
            mode: HarnessMode::Production,
            target_tps: None,
        }
    }
}

pub fn parse_args<I>(args: I) -> Result<HarnessConfig>
where
    I: IntoIterator<Item = String>,
{
    let mut config = HarnessConfig::default();
    let mut args = args.into_iter();
    let _ = args.next();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--txs" => {
                config.tx_count = args
                    .next()
                    .context("missing value after --txs")?
                    .parse::<usize>()
                    .context("failed to parse --txs as usize")?;
            }
            "--json" => config.json = true,
            "--high-throughput" => apply_high_throughput_preset(&mut config),
            "--target-tps" => {
                config.target_tps = Some(
                    args.next()
                        .context("missing value after --target-tps")?
                        .parse::<f64>()
                        .context("failed to parse --target-tps as f64")?,
                );
            }
            "--runs" => {
                config.runs = args
                    .next()
                    .context("missing value after --runs")?
                    .parse::<usize>()
                    .context("failed to parse --runs as usize")?;
            }
            "--senders" => {
                config.sender_count = Some(
                    args.next()
                        .context("missing value after --senders")?
                        .parse::<usize>()
                        .context("failed to parse --senders as usize")?,
                );
            }
            "--mode" => {
                let value = args.next().context("missing value after --mode")?;
                config.mode = match value.as_str() {
                    "admission" => HarnessMode::Admission,
                    "owned-fastpath" => HarnessMode::OwnedFastPath,
                    "production" => HarnessMode::Production,
                    "immediate" => HarnessMode::Immediate,
                    _ => bail!("unknown mode: {value}\n\n{}", usage()),
                };
            }
            "--help" | "-h" => {
                eprintln!("{}", usage());
                std::process::exit(0);
            }
            value if !value.starts_with('-') => {
                config.tx_count = value
                    .parse::<usize>()
                    .context("failed to parse positional tx count as usize")?;
            }
            _ => bail!("unknown argument: {arg}\n\n{}", usage()),
        }
    }

    validate_config(&config)?;
    Ok(config)
}

pub fn usage() -> &'static str {
    "Usage: cargo run --release -p kanari-benchmarks -- [--txs N] [--senders N] [--runs N] [--target-tps N] [--mode admission|owned-fastpath|production|immediate] [--json]\n       cargo run --release -p kanari-benchmarks -- --high-throughput --json\n       cargo run --release -p kanari-benchmarks -- [N]\n\nDefault mode is production: submit_transaction + produce_checkpoint. Admission mode measures verified mempool batch admission only. Owned-fastpath mode commits no-shared-object transactions through the checkpoint pipeline without DAG consensus."
}

fn apply_high_throughput_preset(config: &mut HarnessConfig) {
    if config.tx_count == HarnessConfig::default().tx_count {
        config.tx_count = HIGH_THROUGHPUT_TX_COUNT;
    }
    if config.sender_count.is_none() {
        config.sender_count = Some(HIGH_THROUGHPUT_SENDER_COUNT);
    }
    if config.runs == HarnessConfig::default().runs {
        config.runs = HIGH_THROUGHPUT_RUNS;
    }
    config.mode = HarnessMode::Admission;
    config.target_tps = Some(HIGH_THROUGHPUT_TARGET_TPS);
}

fn validate_config(config: &HarnessConfig) -> Result<()> {
    if config.tx_count == 0 {
        bail!("tx count must be greater than zero");
    }
    if config.runs == 0 {
        bail!("run count must be greater than zero");
    }
    if matches!(config.sender_count, Some(0)) {
        bail!("sender count must be greater than zero");
    }
    if matches!(config.target_tps, Some(value) if value <= 0.0 || !value.is_finite()) {
        bail!("target TPS must be a finite positive number");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parse_args_accepts_positional_tx_count() {
        let config = parse_args(args(&["kanari-benchmarks", "128"])).unwrap();
        assert_eq!(
            config,
            HarnessConfig {
                tx_count: 128,
                sender_count: None,
                runs: 1,
                json: false,
                mode: HarnessMode::Production,
                target_tps: None,
            }
        );
    }

    #[test]
    fn parse_args_accepts_flag_form_and_json() {
        let config = parse_args(args(&[
            "kanari-benchmarks",
            "--txs",
            "64",
            "--mode",
            "immediate",
            "--json",
        ]))
        .unwrap();
        assert_eq!(
            config,
            HarnessConfig {
                tx_count: 64,
                sender_count: None,
                runs: 1,
                json: true,
                mode: HarnessMode::Immediate,
                target_tps: None,
            }
        );
    }

    #[test]
    fn parse_args_accepts_sender_count() {
        let config = parse_args(args(&[
            "kanari-benchmarks",
            "--txs",
            "128",
            "--senders",
            "32",
        ]))
        .unwrap();
        assert_eq!(config.sender_count, Some(32));
    }

    #[test]
    fn parse_args_accepts_runs() {
        let config =
            parse_args(args(&["kanari-benchmarks", "--txs", "128", "--runs", "3"])).unwrap();
        assert_eq!(config.runs, 3);
    }

    #[test]
    fn parse_args_accepts_high_throughput_preset() {
        let config = parse_args(args(&["kanari-benchmarks", "--high-throughput"])).unwrap();
        assert_eq!(config.tx_count, HIGH_THROUGHPUT_TX_COUNT);
        assert_eq!(config.sender_count, Some(HIGH_THROUGHPUT_SENDER_COUNT));
        assert_eq!(config.runs, HIGH_THROUGHPUT_RUNS);
        assert_eq!(config.mode, HarnessMode::Admission);
        assert_eq!(config.target_tps, Some(HIGH_THROUGHPUT_TARGET_TPS));
    }

    #[test]
    fn parse_args_accepts_target_tps() {
        let config = parse_args(args(&["kanari-benchmarks", "--target-tps", "60000"])).unwrap();
        assert_eq!(config.target_tps, Some(60_000.0));
    }

    #[test]
    fn parse_args_rejects_zero_tx_count() {
        let err = parse_args(args(&["kanari-benchmarks", "--txs", "0"])).unwrap_err();
        assert!(err.to_string().contains("greater than zero"));
    }
}
