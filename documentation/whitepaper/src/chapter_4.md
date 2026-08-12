# 4. State, Storage, and Supply

## 4.1 Canonical state

Canonical state consists of committed objects, Move resources/modules, and checkpoint metadata. Derived owner indexes, access versions, and visible supply caches are rebuildable and must not silently alter the canonical root.

## 4.2 Sparse Merkle roots

The SMT uses domain-separated leaf and node hashes. Incremental overlays update affected paths and verify the resulting root before persistence. Full materialization remains an audit and repair path, not the normal hot path.

## 4.3 RocksDB recovery

Checkpoint markers and canonical indexes are persisted with state overlays. Startup validates markers, object references, and root metadata. Recovery audits compare height, transaction count, state root, object indexes, and native supply across validators.

## 4.4 Native supply invariant

For native KANARI:

`total_supply = circulating_supply + object_locked_supply + untracked_supply`

Mint, burn, transfer, split, merge, gas, escrow lock, and escrow release preserve this invariant. Correctly tracked operations finish with `untracked_supply = 0`. Invalid treasury/object state fails closed.

## 4.5 Payment economics

Zero-price developer configurations do not mean zero system cost. Validators still pay for CPU, memory, network, storage, compaction, and recovery.

## 4.6 Canonical versus derived data

The canonical root covers logical state that must be replayable. Owner indexes, access-version caches, query projections, and metrics are derived data and may be rebuilt. A cache must not silently change canonical state during restart or speculative execution.

## 4.7 Incremental root algorithm

An SMT update changes a leaf and its sibling path. Kanari batches affected leaves through an overlay and reuses unchanged subtrees; it verifies ordering, duplicate-key behavior, node hashes, and the final root before persistence. Serial, parallel, property, and recovery tests are required to establish equivalence to full recomputation.

## 4.8 Durability boundary

RocksDB provides the storage engine and WAL/recovery behavior; Kanari adds checkpoint markers, schema validation, and root/supply audits. Durability is only as strong as the configured filesystem and device. Unix and Windows deployments must test their actual sync policy and power-loss recovery.

## 4.9 State transition and root

Let `S_h` be canonical state at checkpoint `h`, `C_h` the accepted changeset, and `R_h` the state root:

`S_(h+1) = Apply(S_h, C_h)`

`R_(h+1) = H(CanonicalEncode(S_(h+1)))`

The incremental SMT must produce the same `R_(h+1)` as a full recomputation. A recovery audit therefore checks both the persisted root and the replayed root.
