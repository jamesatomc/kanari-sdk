# Kanari SDK Sandbox Audit

Date: 2026-07-23

This audit simulates a sandboxed development/runtime posture: tests are run with
temporary state paths, no production node data is required, and checks focus on
path handling, secret exposure, bounded network/RPC behavior, consensus/DAG
invariants, and deterministic state execution.

## Commands executed

```powershell
$env:KANARI_STATE_DB="$env:TEMP\kanari-audit-state-core"
cargo test -p kanari-core --tests

$env:KANARI_STATE_DB="$env:TEMP\kanari-audit-state-runtime"
cargo test -p kanari-move-runtime-v1 --tests

cargo test -p kanari-node --tests
cargo test -p kanari-rpc-server --tests

cargo check --manifest-path sdk/wallet/Cargo.toml

cargo test -p kanari-node long_run_malformed_compressed_payloads_are_bounded -- --ignored --test-threads=1
cargo test -p kanari-core long_run_byzantine_native_blocks_cannot_advance_checkpoint -- --ignored --test-threads=1
```

## Results

- `kanari-core`: passed, 77 tests passed, 2 long-running soak tests ignored.
- `kanari-move-runtime-v1`: passed, runtime/unit/integration tests passed.
- `kanari-node`: passed, 27 tests passed, 1 long-running decompression/DoS soak test ignored.
- `kanari-rpc-server`: passed, 19 tests passed.
- `sdk/wallet`: passed `cargo check`.
- Ignored one-shot soaks:
  - `long_run_malformed_compressed_payloads_are_bounded`: passed.
  - `long_run_byzantine_native_blocks_cannot_advance_checkpoint`: passed.

## Public RPC load/DoS probe

Added `scripts/run-rpc-load-dos.ps1` for loopback or trusted-gateway testing.
It sends concurrent JSON-RPC health/stats requests and can mix malformed and
oversized payloads:

```powershell
.\scripts\run-rpc-load-dos.ps1 `
  -RpcUrl http://127.0.0.1:6767 `
  -Requests 500 `
  -Concurrency 32 `
  -IncludeMalformed `
  -IncludeOversized
```

Run this only against your own node or trusted load balancer.

For production-grade measurement, use the Rust load generator:

```powershell
cargo run -p kanari-rpc-loadgen --release -- `
  --rpc-url http://127.0.0.1:6767 `
  --requests 10000 `
  --concurrency 256 `
  --malformed-every 17 `
  --oversized-every 23 `
  --min-rps 1000 `
  --max-p99-ms 500 `
  --max-client-rejected-percent 10
```

The load generator also supports multiple `--rpc-url` values. Requests are
distributed round-robin across endpoints, which makes it usable for direct
four-node measurements or gateway-pool verification:

```powershell
.\scripts\run-production-rpc-load.ps1 `
  -RpcUrl @(
    "http://192.168.1.101:19001",
    "http://192.168.1.101:19003",
    "http://192.168.1.101:19005",
    "http://192.168.1.101:19007"
  ) `
  -Requests 200000 `
  -Concurrency 2048 `
  -IncludeMalformed `
  -IncludeOversized `
  -MinRps 10000 `
  -MaxP99Ms 300 `
  -MaxClientRejectedPercent 10 `
  -MaxEndpointImbalancePercent 5
```

The Rust generator now supports explicit pass/fail gates for minimum
throughput, maximum p99 latency, maximum client-rejected percentage, and
multi-endpoint distribution imbalance. This makes RPC load tests suitable as CI
or release gates instead of manual benchmark notes only.

Local baseline using release `kanari-rpc-loadgen` against a temporary debug
`kanari-node local` process:

- completed: 10,000/10,000
- duration: 2.06s
- observed throughput: ~4,862 req/s
- success: 9,411
- client rejected malformed/oversized requests: 589
- rate limited: 0
- server errors: 0
- network errors: 0
- latency: p50 41 ms, p95 117 ms, p99 180 ms, max 447 ms

This is a local loopback baseline. For production claims, run the same Rust
generator from separate client machines against the actual load balancer and
release validator/RPC binaries.

Local production-style loopback using release `kanari-rpc-loadgen` against a
temporary release `kanari-node local` process:

| Scenario | Completed | Throughput | Success | Client rejected | 5xx/server errors | Network errors | Latency |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Healthy JSON-RPC, concurrency 256 | 10,000/10,000 | ~5,202 req/s | 10,000 | 0 | 0 | 0 | p50 40 ms, p95 114 ms, p99 150 ms, max 199 ms |
| Mixed malformed/oversized, concurrency 256 | 10,000/10,000 | ~5,814 req/s | 9,411 | 589 | 0 | 0 | p50 35 ms, p95 97 ms, p99 167 ms, max 377 ms |
| Healthy JSON-RPC, concurrency 512 | 20,000/20,000 | ~5,596 req/s | 20,000 | 0 | 0 | 0 | p50 83 ms, p95 169 ms, p99 237 ms, max 476 ms |

This release-node result confirms the RPC server rejects malformed/oversized
requests without producing server-side 5xx responses or dropping network
connections in the local loopback test. It is still a single-machine result; a
public RPC capacity number must be measured from separate client machines
through the production load balancer.

The production runner was smoke-tested against a temporary release local node
with two configured endpoint slots pointing to the same node:

- completed: 100/100
- success: 94
- client rejected malformed/oversized requests: 6
- server errors: 0
- network errors: 0
- endpoint distribution: slot[0] 50 requests, slot[1] 50 requests

Disposable four-node release devnet load result using
`scripts/run-local-four-node-rpc-load.ps1`:

- run artifacts: `.codex-runlogs/prod-load-20260723-220637`
- endpoints: `http://127.0.0.1:19101`, `:19111`, `:19121`, `:19131`
- completed: 100,000/100,000
- observed throughput: ~8,870 req/s
- success: 94,117
- client rejected malformed/oversized requests: 5,883
- rate limited: 0
- server errors: 0
- network errors: 0
- latency: p50 1 ms, p95 3 ms, p99 3 ms, max 12 ms
- endpoint distribution: 25,000 requests per endpoint

The disposable four-node run starts and stops release node processes
automatically, uses workspace-local temporary data directories, and does not
reset the user's normal `%USERPROFILE%\.kanari` node databases.

Disposable four-node transaction/root consistency result using
`scripts/run-local-four-node-tx-load.ps1`:

- run artifacts: `.codex-runlogs/four-node-tx-load-20260723-221722`
- submitted transactions: 20
- all four nodes finalized checkpoint 20
- observed peer state: `height=20`, `txs=20`, `our_height=20`, `our_txs=20`
- no node process was left running after the test

This covers the write path in addition to read-only RPC load: CLI-signed
transactions enter RPC, pass mempool admission, are included in Mysticeti DAG
checkpoints, propagate over P2P sync, and converge across the four local
authority stores.

Added a local four-node RPC chaos runner:

```powershell
.\scripts\run-local-four-node-chaos.ps1 `
  -Requests 20000 `
  -Concurrency 256 `
  -MinRps 1000 `
  -MaxP99Ms 500 `
  -MaxEndpointImbalancePercent 10
```

It starts a disposable four-node devnet, runs gated RPC load, stops one follower,
runs gated load through the three surviving nodes, restarts the follower, and
requires all four nodes to report matching height, transaction count, and state
root before passing.

Local smoke result:

- artifacts: `.codex-runlogs/four-node-chaos-20260723-223123`
- baseline four-node load: 40/40 requests, ~3,246 req/s, p99 3 ms, no server/network errors
- follower-loss three-node load: 40/40 requests, ~3,633 req/s, p99 3 ms, no server/network errors
- restarted node 4 became ready and all four nodes reconverged to the same
  state root `56277fbbe9438e88a554eda13c97a4c11231e1eeae63c3272efc73a8af82802e`

Added `.github/workflows/security-gates.yml` so push/pull-request CI runs the
deterministic security gates automatically. The workflow also exposes a manual
`run_chaos=true` option for the disposable four-node chaos exercise.

Local loopback result with a temporary `kanari-node local` process:

- completed: 500/500
- success: 470
- client rejected malformed/oversized requests: 30
- server errors: 0
- network errors: 0
- observed throughput from this PowerShell job-based harness: ~7.73 req/s
- latency: p50 128 ms, p95 144 ms, p99 167 ms

## Security properties covered by current tests

- Mysticeti DAG signature validation rejects invalid remote vertices.
- Checkpoint sync no longer depends on a fixed root lookup window.
- Snapshot import rejects tampered state and uncertified checkpoint roots.
- Required persistent engine rejects invalid persistent store paths instead of
  falling back to memory.
- Validator backup restore rejects path traversal.
- P2P decompression bomb protection and chunk reassembly bounds are tested.
- Mainnet rejects plaintext consensus key files and requires encrypted identity
  material.
- RPC transaction submission rejects missing signatures and gas/coin overlap.
- RPC load tooling now has regression tests for malformed/oversized request
  generation, latency percentile calculation, HTTP status classification, and
  pass/fail threshold validation.
- CI now has deterministic security gates for RPC loadgen regressions, bounded
  P2P decompression/DoS, and Byzantine Mysticeti native-block rejection.
- A manual four-node chaos gate validates RPC load during follower loss/restart
  and requires post-restart state convergence.
- Move runtime tests cover deterministic state roots, object ownership, gas OOG
  behavior for natives, access-version conflict detection, and supply invariants.

## Finding fixed during this audit

`sdk/wallet/src/main.rs` printed the example mnemonic and derived private keys by
default. It now redacts both by default. To intentionally print secrets in a
local demo, set:

```powershell
$env:KANARI_WALLET_EXAMPLE_SHOW_SECRETS="1"
```

Do not enable this in CI, public terminals, logs, or production environments.

## Remaining sandbox/production gaps

- Long-running soak tests are still ignored by default and should be run in a
  dedicated CI profile before public releases.
- Public RPC deployments still need an external load balancer/firewall policy in
  front of per-node rate/concurrency limits.
- Third-party dependencies and vendored `third_party/` code were not deeply
  audited in this pass.
- This pass validates local sandbox behavior, not a multi-week live validator
  run with adversarial peers.
