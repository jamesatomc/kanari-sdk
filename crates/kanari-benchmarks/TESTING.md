# Kanari Benchmarks Testing

This crate measures Kanari throughput using explicit benchmark modes.

The default mode is `production`, which exercises the full local blockchain production path with
an in-memory engine and a signed zero-gas native workload:

1. Generate and sign deterministic native transactions.
2. Submit every transaction through the batch mempool API.
3. Produce a Mysticeti/DAG checkpoint through `BlockchainEngine::produce_checkpoint`.
4. Report production TPS from the measured `produce_checkpoint` window and show submit time separately.

## Quick Checks

Run the crate tests:

```powershell
cargo test -p kanari-benchmarks --quiet
```

Run the production benchmark:

```powershell
cargo run --release -p kanari-benchmarks -- --txs 1024 --json
```

Run the 60k TPS production target benchmark:

```powershell
cargo run --release -p kanari-benchmarks -- --high-throughput --json
```

Equivalent explicit command:

```powershell
cargo run --release -p kanari-benchmarks -- --mode production --txs 10000 --senders 10000 --runs 3 --target-tps 60000 --json
```

## Modes

- `production`
  Full local production path: mempool submission plus repeated `produce_checkpoint` calls until
  the submitted batch is drained. Transactions sharing a primary access lane are intentionally
  placed in later checkpoints, so this measures end-to-end local batch throughput rather than
  silently measuring only the first conflict-free wave.
- `immediate`
  Executes and applies transactions one by one. The built-in workload is deliberately a
  zero-effect native workload optimized for checkpoint production, so it has no spendable coin
  object for direct execution and will be rejected in this mode. Do not use its result as a TPS
  comparison; use a separately funded workload if testing direct execution.

## High Throughput Target

`--high-throughput` is the quick production-path target for 60k TPS. It expands to:

- `--mode production`
- `--txs 10000`
- `--senders 10000`
- `--runs 3`
- `--target-tps 60000`

Every run must meet the target. If a run drops below 60k TPS, the process exits with an error so CI and local scripts catch it immediately.

## Interpreting Results

Example output:

```json
{
  "requested_txs": 128,
  "mode": "production",
  "executed": 128,
  "failed": 0,
  "tx_count": 128,
  "submit_secs": 0.0128,
  "produce_secs": 0.0122,
  "duration_secs": 0.123456,
  "tps": 1036.806635
}
```

Important fields:

- `mode`: benchmark path being measured
- `executed`: transactions that completed successfully
- `failed`: transactions that failed in the selected mode
- `tx_count`: transactions included in the measured path
- `duration_secs`: measured benchmark window. In `production` and `owned-fastpath`
  mode this is `submit_secs + produce_secs` so long chunked runs report honest
  wall-clock throughput.
- `submit_secs`: total time spent submitting every chunk into the mempool
- `produce_secs`: time spent in block/DAG/checkpoint production, excluding submit time
- `tps`: `tx_count / duration_secs`
- `target_tps`: configured TPS threshold, when set
- `target_status`: `pass` or `fail`, when a threshold is set

## Notes

- Use `--release` for throughput numbers.
- Treat `production` as the honest local blockchain TPS benchmark without disk I/O.
- Production may use the engine's zero-effect native fast path; its `executed`/`failed` counts
  still come from checkpoint execution and state validation.
- Larger `--txs` values can take much longer in production mode because they include signature verification, state-root work, and block production.
- On the current Windows test machine, `production --txs 10000 --senders 10000 --runs 3` measured about 65k median TPS after the SMT/zero-effect fast-path work.

## Soak Test

The long-running production soak test is ignored by default. Run it explicitly:

```powershell
$env:KANARI_SOAK_SECONDS=86400
$env:KANARI_SOAK_TXS=10000
$env:KANARI_SOAK_MIN_TPS=1
cargo test -p kanari-benchmarks production_soak_test -- --ignored --nocapture
```

For a 72-hour run, set `KANARI_SOAK_SECONDS=259200`.

## Persistent RocksDB Profile

Use the dedicated runner before making storage performance claims. It executes
the full production workload against a temporary RocksDB store and records
memtable, L0, pending-compaction, flush, compaction, and write-stall metrics:

```powershell
.\scripts\run-persistent-rocksdb-benchmark-profile.ps1 -Transactions 10000 -Senders 10000
```

The command intentionally does not set a TPS target: disk throughput depends on
the machine and storage configuration. Treat a nonzero `total-stops`,
`total-delays`, `compaction-pending`, or `estimate-pending-compaction-bytes` as
a signal to investigate the RocksDB configuration and workload before release.
