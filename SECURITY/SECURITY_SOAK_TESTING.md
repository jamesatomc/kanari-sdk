# Adversarial soak testing

The repository includes opt-in, long-running adversarial tests. They complement normal unit/property tests; they are not a substitute for an independent audit or a production network exercise.

Run one soak iteration:

```powershell
cargo test -p kanari-node long_run_malformed_compressed_payloads_are_bounded -- --ignored
cargo test -p kanari-core long_run_byzantine_native_blocks_cannot_advance_checkpoint -- --ignored
```

Run continuously (default 8 hours):

```powershell
.\scripts\run-adversarial-soak.ps1
```

Run the broader fuzz/conflict campaign:

```powershell
.\scripts\run-fuzz-campaign.ps1 `
  -Hours 8 `
  -Workers 1 `
  -IncludeIgnoredLongRuns
```

The runner checks that:

- arbitrary and compressed-bomb P2P input stays within the decompression budget and never panics;
- arbitrary malformed Byzantine Mysticeti native blocks cannot advance checkpoint height;
- conflict-aware speculative scheduling preserves canonical order and keeps
  conflicting object/gas keys out of the same wave.

For a network-level exercise, run the four-node devnet separately and inject process restarts, delayed peers, duplicate messages, and malformed network traffic from an isolated test host. Do not conduct these tests against shared or production validators.

## Production RPC load exercise

Use the Rust load generator for real public-RPC measurements. The PowerShell
runner builds the optimized binary and can distribute requests across multiple
RPC endpoints. Run it from a separate client machine when possible; localhost
results mainly measure loopback and one host's scheduler.

Single gateway or load balancer:

```powershell
.\scripts\run-production-rpc-load.ps1 `
  -RpcUrl "http://rpc-gateway.example:19001" `
  -Requests 100000 `
  -Concurrency 1024 `
  -IncludeMalformed `
  -IncludeOversized `
  -MinRps 5000 `
  -MaxP99Ms 250 `
  -MaxClientRejectedPercent 10
```

Direct four-node pool:

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

Pass `-FailOnRateLimit` only when the test should fail on HTTP 429. For public
RPC, a controlled number of 429 responses is acceptable when the gateway or
per-node limiter is intentionally protecting validator resources.

The `MinRps`, `MaxP99Ms`, `MaxClientRejectedPercent`, and
`MaxEndpointImbalancePercent` gates convert the benchmark into a pass/fail
release check. Keep the thresholds workload-specific; do not copy local loopback
numbers into public capacity claims.

Production hardware profile:

```powershell
.\scripts\run-production-benchmark-profile.ps1 `
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

Record CPU model, RAM, disk type, OS, node commit hash, binary profile, RPC
limits, validator count, requests, concurrency, achieved RPS, p99 latency,
rate-limit percentage, server/network errors, and final four-node root
convergence. A benchmark without hardware and commit metadata is not a release
claim.

For a disposable local four-node devnet, use:

```powershell
.\scripts\run-local-four-node-rpc-load.ps1 `
  -Requests 100000 `
  -Concurrency 1024 `
  -IncludeMalformed `
  -IncludeOversized
```

This creates a temporary devnet under `.codex-runlogs`, starts four release
nodes on loopback, runs the production RPC load script across all four RPC
ports, and stops the node processes in a `finally` block.

## Four-node transaction/root consistency exercise

Use this when you want to verify the full path: wallet-signed transaction,
RPC submission, mempool admission, Mysticeti DAG checkpointing, P2P sync, and
state convergence across four authorities.

Kanari execution now schedules speculative waves with a conservative
conflict-aware planner before Move execution. Transactions with disjoint
sender/object/gas conflict keys can enter the same speculative wave; transactions
with overlapping keys are placed in later waves before the runtime's read/write
set validation and serial retry safety net.

The engine logs `[parallel execution]` counters for each batch, including wave
count, speculative committed transactions, serial retry transactions, and retry
reasons such as execution error, conflict, epoch change, apply rejection, or
supply validation rejection. Use these counters when comparing scheduler changes
or production load-test runs.

```powershell
.\scripts\run-local-four-node-tx-load.ps1 `
  -Password '@Password12345678' `
  -Count 20 `
  -Amount 0.000000001
```

The runner creates a disposable four-node devnet under `.codex-runlogs`, submits
transactions through node 1, waits until all four RPC nodes report the same
height, transaction count, and state root, then stops all node processes.

Recent local release run:

- artifacts: `.codex-runlogs/four-node-tx-load-20260723-221722`
- submitted transactions: 20
- all four nodes finalized checkpoint 20
- observed peer state: `height=20`, `txs=20`, `our_height=20`, `our_txs=20`
- no node process was left running after the test

## Four-node transaction chaos with delay and duplicate gossip

Use this when you want the full transaction path plus real process restarts,
artificial P2P delay, and duplicate gossipsub publishes. The chaos knobs are
disabled by default and are only activated by the runner's environment variables.

```powershell
.\scripts\run-local-four-node-parallel-tx-chaos.ps1 `
  -KeystorePath "$env:USERPROFILE\.kanari\kanari_config\kanari.keystore" `
  -Password '@Password12345678' `
  -Senders @(
    '0xsender1...',
    '0xsender2...',
    '0xsender3...',
    '0xsender4...'
  ) `
  -Recipient '0xrecipient...' `
  -CountPerSender 250 `
  -FundSenders `
  -FaucetCoinsPerSender 16 `
  -ChaosRounds 4 `
  -P2pPublishDelayMs 250 `
  -P2pDuplicatePublishes 2 `
  -P2pChannelCapacity 8192 `
  -MaxConcurrentSyncMessages 512
```

Expected pass condition:

- every tx lane exits successfully;
- all four RPC nodes become readable after restarts;
- final height, transaction count, and state root converge across all nodes;
- node logs have no panic, WAL error, state-root divergence, or checkpoint
  validation failure.

For a multi-hour campaign, use the wrapper below. It repeats disposable
four-node devnets until the deadline, rotating ports between iterations so a
stale process cannot silently poison the next run.

```powershell
.\scripts\run-chaos-network-campaign.ps1 `
  -Hours 6 `
  -KeystorePath "$env:USERPROFILE\.kanari\kanari_config\kanari.keystore" `
  -Password '@Password12345678' `
  -Senders @(
    '0xsender1...',
    '0xsender2...',
    '0xsender3...',
    '0xsender4...'
  ) `
  -Recipient '0xrecipient...' `
  -CountPerSender 250 `
  -ChaosRoundsPerIteration 4 `
  -P2pPublishDelayMs 250 `
  -P2pDuplicatePublishes 2
```

## Four-node RPC chaos exercise

Use this to validate process-loss/restart behavior under RPC pressure. The
runner starts four temporary release nodes, verifies baseline root convergence,
runs gated RPC load, stops follower node 4, runs load through the three
surviving nodes, restarts node 4, then waits for all four nodes to converge
again.

```powershell
.\scripts\run-local-four-node-chaos.ps1 `
  -Requests 20000 `
  -Concurrency 256 `
  -MinRps 1000 `
  -MaxP99Ms 500 `
  -MaxEndpointImbalancePercent 10
```

This is a local process-chaos test, not a replacement for a distributed network
partition exercise. Run larger variants from isolated hosts before mainnet
release.

Recent smoke run:

- artifacts: `.codex-runlogs/four-node-chaos-20260723-223123`
- baseline four-node load and follower-loss three-node load both completed
  without server or network errors;
- restarted follower rejoined and all four nodes reported the same state root.

## CI gates

`.github/workflows/security-gates.yml` runs deterministic security gates on push
and pull request:

- `kanari-rpc-loadgen` unit tests;
- one-shot P2P decompression/DoS ignored test;
- one-shot Byzantine Mysticeti native-block ignored test.

The same workflow has a manual `workflow_dispatch` option `run_chaos=true` to
run the disposable four-node RPC chaos gate on a Windows runner.
It also has `run_fuzz_campaign=true` for a bounded Linux campaign.
