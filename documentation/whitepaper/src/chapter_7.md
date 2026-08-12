# 7. Roadmap and Conclusion

## 7.1 Near-term work

1. Complete multi-hour and multi-day live validator soak with real wallets.
2. Finish persistent 10K+ capacity campaigns after setup fanout is provisioned efficiently.
3. Profile RocksDB compaction, flush, and write-stall behavior under sustained load.
4. Expand nightly fuzz corpora for BCS/RPC, object authorization, consensus, and SMT.
5. Audit every Move native and RPC build/submit path.

## 7.2 Longer-term work

• migrate away from unmaintained dependencies where compatibility permits;
• independently review PQC providers and signature batch verification;
• add production metrics, alerting, backup verification, and operator runbooks;
• add lane-specific schedulers for owned, shared, and hot objects;
• add a streaming validator backup format with compatibility-preserving migration.

## 7.3 Conclusion

Kanari keeps consensus, execution, storage, and application authorization explicit. The network advances only when real work is committed, and operators can verify that validators agree on state root and supply after failures. This supports continued testnet and product development while preserving an evidence-based path toward production readiness.

## 7.4 Release gates

A release candidate should have clean reproducible builds, migration tests for supported state and wallet formats, full unit/integration tests, adversarial RPC and Move authorization tests, four-node crash/restart convergence, persistent-load results with hardware metadata, dependency audit output, and an operator runbook. An unmet gate is recorded as a limitation, not hidden by a compatibility fallback.

## 7.5 Research directions

Priorities include efficient owned-object batching, adaptive object lanes, streaming backup encryption, state-root batching with equivalence tests, ordering-fairness analysis, batch signature verification, and independently reviewed PQC providers. Each must preserve deterministic replay, supply conservation, and migration semantics.

## 7.6 Closing statement

Kanari should be evaluated like a distributed financial system: by invariants, adversarial tests, operational evidence, and independent review—not by a headline benchmark alone.
