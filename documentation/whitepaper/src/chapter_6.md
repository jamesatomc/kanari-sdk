# 6. Operations, APIs, and Performance

## 6.1 Integration surfaces

Applications use the CLI and JSON-RPC for wallet operations, object queries, module publishing, transaction submission, and committed-effect inspection. Move packages define asset, escrow, marketplace, and DeFi behavior.

## 6.2 Multi-node operation

Validators must use the same binary and compatible persistence schema, isolated data directories, stable peer identity, and monitored disk capacity. Operators should alert on root mismatch, stalled synchronization, unexpected supply, RPC errors, and RocksDB write stalls.

## 6.3 Measured results

The current campaign reports:

• 4-node live chaos with duplicate P2P publishes, delay, node kills, restarts, and adversarial RPC: roots and supply converged; no server/network errors.
• Persistent 4-node profile: 100/100 successful transactions, approximately 47 TPS aggregate lane throughput on the tested Windows host.
• In-memory owned-object production benchmark: approximately 13K TPS under its stated workload.

These values are not interchangeable. Persistent RocksDB, signature choice, object fanout, validator count, and transaction mix materially change throughput.

## 6.4 Load methodology

Every benchmark should record commit, OS, CPU/RAM, validator count, storage backend, sender count, object fanout, transaction count, duration, failures, root convergence, and supply convergence. Setup fanout must be separated from execution TPS.

## 6.5 Capacity interpretation

Throughput is a vector: persistent versus in-memory state, signature scheme, transaction bytes, object count, shared-object contention, checkpoint frequency, network delay, and compaction all matter. Capacity reports should include p50/p95/p99 latency, success/failure counts, CPU, RSS, disk writes, compaction bytes, queue depth, and recovery time.

## 6.6 Production topology

Nodes may be placed in different regions when advertised addresses, firewall rules, time synchronization, bandwidth, and latency are configured correctly. Public RPC should use TLS or a hardened reverse proxy. P2P should use stable identities and explicit bootstrap/static peers; admin/debug endpoints should remain private or strongly authenticated.

## 6.7 Observability

Alert on root divergence, supply mismatch, untracked supply, checkpoint stagnation, pending growth, sync gaps, P2P queue saturation, RocksDB write stalls, compaction debt, failed signatures, and repeated nonce/object-version failures. Logs must never contain passwords or private keys.

## 6.8 Measurement equations

For (N) committed transactions over elapsed time Δt, measured throughput is:

`TPS = N / Delta_t`

This excludes setup unless explicitly stated. For a parallel run with lanes (L_k), aggregate throughput is:

`TPS_aggregate = sum(N_k / Delta_t_k)`

The report must also state failures, storage backend, signature scheme, fanout, and root/supply convergence; TPS alone is not a safety or capacity claim.
