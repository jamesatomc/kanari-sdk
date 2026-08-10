// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::config::{DEFAULT_SENDER_COUNT, HarnessConfig, HarnessMode};
use crate::execution::{
    execute_admission_path, execute_immediate, execute_owned_fast_path, execute_production_path,
};
use crate::report::HarnessReport;
use crate::workload::{
    build_funded_owned_fastpath_workload, build_signed_workload, prepare_engine,
};
use anyhow::{Result, bail};
use std::time::Instant;

pub fn run_harness(config: &HarnessConfig) -> Result<HarnessReport> {
    eprintln!(
        "Kanari benchmarks: creating engine and preparing {} txs in {} mode",
        config.tx_count,
        config.mode.as_str()
    );

    let use_persistent_state = use_persistent_state_backend(config);
    let prepared = prepare_engine(use_persistent_state)?;
    let engine = &prepared.engine;
    eprintln!(
        "State backend: {}",
        if use_persistent_state {
            "rocksdb-temp (production SMT incremental root)"
        } else {
            "in-memory (full state-root recompute fallback)"
        }
    );

    let sender_count = configured_sender_count(config);
    eprintln!("Deriving {sender_count} deterministic benchmark senders...");
    let use_funded_owned_workload = matches!(
        config.mode,
        HarnessMode::OwnedFastPath | HarnessMode::Production
    );
    if use_funded_owned_workload {
        eprintln!("Using funded owned-object fanout workload for successful fastpath commits.");
    } else {
        eprintln!("Using zero-gas native benchmark workload; funding is not required.");
    }
    eprintln!("Signing transactions...");
    let signed_txs = if use_funded_owned_workload {
        build_funded_owned_fastpath_workload(engine, config, sender_count)?
    } else {
        build_signed_workload(config, sender_count)?
    };

    eprintln!("Starting benchmark...");
    let mut submit_secs = None;
    let mut produce_secs = None;
    let start = Instant::now();
    let (block_info, duration_secs) = match config.mode {
        HarnessMode::Admission => {
            let (block_info, submit) = execute_admission_path(engine, signed_txs)?;
            submit_secs = Some(submit);
            (block_info, submit)
        }
        HarnessMode::OwnedFastPath => {
            let (block_info, submit, produce) = execute_owned_fast_path(engine, signed_txs)?;
            submit_secs = Some(submit);
            produce_secs = Some(produce);
            (block_info, submit + produce)
        }
        HarnessMode::Production => {
            let (block_info, submit, produce) = execute_production_path(engine, signed_txs)?;
            submit_secs = Some(submit);
            produce_secs = Some(produce);
            (block_info, submit + produce)
        }
        HarnessMode::Immediate => {
            let block_info = execute_immediate(engine, signed_txs)?;
            (block_info, start.elapsed().as_secs_f64())
        }
    };
    maybe_print_rocksdb_profile(engine);
    let tps = if duration_secs > 0.0 {
        block_info.tx_count as f64 / duration_secs
    } else {
        block_info.tx_count as f64
    };

    Ok(HarnessReport {
        requested_txs: config.tx_count,
        mode: config.mode.clone(),
        block_info,
        duration_secs,
        submit_secs,
        produce_secs,
        tps,
        target_tps: config.target_tps,
    })
}

fn rocksdb_profile_enabled() -> bool {
    std::env::var("KANARI_BENCH_ROCKSDB_PROFILE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn maybe_print_rocksdb_profile(engine: &kanari_core::BlockchainEngine) {
    if !rocksdb_profile_enabled() {
        return;
    }

    let Some(store) = engine.persistent_store.as_ref() else {
        eprintln!("RocksDB profile: backend=in-memory");
        return;
    };

    print_rocksdb_profile(store, "post-run");

    if env_flag("KANARI_BENCH_ROCKSDB_FLUSH_PROFILE") {
        let started = Instant::now();
        match store.flush() {
            Ok(()) => eprintln!(
                "RocksDB flush profile: flush_secs={:.6}",
                started.elapsed().as_secs_f64()
            ),
            Err(err) => eprintln!("RocksDB flush profile: flush_error={err}"),
        }
        print_rocksdb_profile(store, "post-flush");
    }

    if env_flag("KANARI_BENCH_ROCKSDB_COMPACT_PROFILE") {
        let started = Instant::now();
        store.compact();
        eprintln!(
            "RocksDB compact profile: compact_secs={:.6}",
            started.elapsed().as_secs_f64()
        );
        print_rocksdb_profile(store, "post-compact");
    }
}

fn print_rocksdb_profile(
    store: &kanari_core::kanari_move_runtime_v1::storage::persistent_store::PersistentStore,
    label: &str,
) {
    eprintln!("RocksDB profile ({label}):");
    for property in [
        "rocksdb.estimate-pending-compaction-bytes",
        "rocksdb.cur-size-active-mem-table",
        "rocksdb.cur-size-all-mem-tables",
        "rocksdb.num-immutable-mem-table",
        "rocksdb.mem-table-flush-pending",
        "rocksdb.compaction-pending",
        "rocksdb.num-running-compactions",
        "rocksdb.num-running-flushes",
        "rocksdb.num-files-at-level0",
        "rocksdb.estimate-live-data-size",
        "rocksdb.total-sst-files-size",
    ] {
        match store.rocksdb_int_property(property) {
            Ok(Some(value)) => eprintln!("  {property}={value}"),
            Ok(None) => {}
            Err(err) => eprintln!("  {property}=<error: {err}>"),
        }
    }

    match store.rocksdb_property("rocksdb.stats") {
        Ok(Some(stats)) => {
            eprintln!("RocksDB stats begin");
            for line in stats.lines().take(160) {
                eprintln!("{line}");
            }
            eprintln!("RocksDB stats end");
        }
        Ok(None) => {}
        Err(err) => eprintln!("  rocksdb.stats=<error: {err}>"),
    }
}

pub fn run_many(config: &HarnessConfig) -> Result<Vec<HarnessReport>> {
    (0..config.runs)
        .map(|_| run_harness(config))
        .collect::<Result<Vec<_>>>()
}

pub fn render_reports(config: &HarnessConfig, reports: &[HarnessReport]) -> String {
    if reports.len() == 1 {
        return if config.json {
            reports[0].render_json()
        } else {
            reports[0].render_text()
        };
    }

    let (min_tps, median_tps, max_tps) = tps_summary(reports);
    if config.json {
        let runs = reports
            .iter()
            .map(HarnessReport::render_json)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"runs\":{},\"median_tps\":{:.6},\"min_tps\":{:.6},\"max_tps\":{:.6},\"results\":[{}]}}",
            config.runs, median_tps, min_tps, max_tps, runs
        )
    } else {
        let mut output = format!(
            "Kanari benchmark summary\nruns={}\nmedian_tps={:.2}\nmin_tps={:.2}\nmax_tps={:.2}",
            config.runs, median_tps, min_tps, max_tps
        );
        for (index, report) in reports.iter().enumerate() {
            output.push_str(&format!("\nrun={} tps={:.2}", index + 1, report.tps));
        }
        output
    }
}

pub fn ensure_targets(reports: &[HarnessReport]) -> Result<()> {
    if reports.len() > 1
        && let Some(target_tps) = reports.first().and_then(|report| report.target_tps)
        && reports
            .iter()
            .all(|report| report.target_tps == Some(target_tps))
    {
        let (_, median_tps, _) = tps_summary(reports);
        if median_tps < target_tps {
            bail!(
                "TPS target not reached: median {:.2} < {:.2} (runs={})",
                median_tps,
                target_tps,
                reports.len()
            );
        }
        return Ok(());
    }

    for report in reports {
        ensure_target_tps(report)?;
    }
    Ok(())
}

pub fn ensure_target_tps(report: &HarnessReport) -> Result<()> {
    if let Some(target_tps) = report.target_tps
        && report.tps < target_tps
    {
        bail!(
            "TPS target not reached: {:.2} < {:.2} (mode={}, txs={})",
            report.tps,
            target_tps,
            report.mode.as_str(),
            report.requested_txs
        );
    }

    Ok(())
}

fn configured_sender_count(config: &HarnessConfig) -> usize {
    config
        .sender_count
        .or_else(|| {
            std::env::var("KANARI_BENCH_SENDERS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
        })
        .unwrap_or(DEFAULT_SENDER_COUNT)
        .min(config.tx_count)
}

fn use_persistent_state_backend(_config: &HarnessConfig) -> bool {
    std::env::var("KANARI_BENCH_STATE_BACKEND")
        .ok()
        .map(|value| matches!(value.as_str(), "rocksdb" | "rocksdb-temp" | "persistent"))
        .unwrap_or(false)
}

fn tps_summary(reports: &[HarnessReport]) -> (f64, f64, f64) {
    let mut sorted_tps = reports.iter().map(|report| report.tps).collect::<Vec<_>>();
    sorted_tps.sort_by(|a, b| a.total_cmp(b));
    let min_tps = *sorted_tps.first().unwrap_or(&0.0);
    let max_tps = *sorted_tps.last().unwrap_or(&0.0);
    let median_tps = sorted_tps[sorted_tps.len() / 2];
    (min_tps, median_tps, max_tps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanari_core::CheckpointProductionInfo;

    #[test]
    fn target_tps_guard_fails_below_threshold() {
        let report = HarnessReport {
            requested_txs: 5,
            mode: HarnessMode::Immediate,
            block_info: CheckpointProductionInfo {
                vertex_id: "vertex".to_string(),
                round: 1,
                tx_count: 5,
                executed: 5,
                failed: 0,
                events: vec![],
                checkpoint: None,
                vertex: None,
            },
            duration_secs: 1.0,
            submit_secs: None,
            produce_secs: None,
            tps: 5.0,
            target_tps: Some(100_000.0),
        };

        let err = ensure_target_tps(&report).unwrap_err();
        assert!(err.to_string().contains("TPS target not reached"));
    }

    #[test]
    #[ignore = "long-running soak test; enable explicitly with --ignored"]
    fn production_soak_test() {
        let duration_secs = std::env::var("KANARI_SOAK_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(24 * 60 * 60);
        let tx_count = std::env::var("KANARI_SOAK_TXS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(10_000);
        let min_tps = std::env::var("KANARI_SOAK_MIN_TPS")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0);

        let deadline = Instant::now() + std::time::Duration::from_secs(duration_secs);
        let config = HarnessConfig {
            tx_count,
            sender_count: None,
            runs: 1,
            json: true,
            mode: HarnessMode::Production,
            target_tps: Some(min_tps),
        };
        let mut runs = 0usize;

        while Instant::now() < deadline {
            let report = run_harness(&config).expect("soak benchmark run should succeed");
            assert_eq!(report.block_info.executed, tx_count);
            assert_eq!(report.block_info.failed, 0);
            ensure_target_tps(&report).expect("soak TPS should stay above threshold");
            runs += 1;
        }

        assert!(runs > 0, "soak test should run at least once");
    }
}
