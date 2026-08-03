# Key and Signature Path Audit

Last checked: 2026-08-02

This audit maps where `kanari-crypto` key and signature APIs are consumed
outside the crate.

## High-value paths

| Area | Representative files | Current security expectation |
| --- | --- | --- |
| Transaction build/sign | `sdk/kanari_pay/lib/src/modules/transactions/operations.dart`, `crates/kanari-types/src/transaction.rs` | The client signs the exact prepared transaction bytes/hash and submits the signature as bytes. The node/core path must verify against the sender/tagged curve before execution. |
| Core execution tests | `crates/kanari-core/tests/unit/engine_tests.rs`, `crates/kanari-core/tests/unit/produce_dag_vertex_tests.rs` | Tests consistently use `KeyPair::tagged_address()` and explicit `curve_type` when signing. Untagged verification is fail-closed in `kanari-crypto`. |
| Bench workload | `crates/kanari-benchmarks/src/workload.rs` | Deterministic senders use `keypair_from_private_key`; oversized/malformed import fails before expensive PQC/hybrid parsing. |
| Wallet app curve mapping | `sdk/kanari_pay/lib/src/kanaricurve.dart` | UI/client curve names must remain aligned with Rust `CurveType` names. Provider changes must preserve curve wire names or use explicit versioning. |
| Auth private-key storage | `crates/kanari-auth/src/private_key_crypto.rs`, `crates/kanari-auth/src/auth_manager.rs`, `crates/kanari-auth/src/session.rs` | Private-key material should remain encrypted at rest and zeroized on session invalidation. |
| Consensus keys | `crates/kanari-node/src/app.rs`, `crates/mysticeti/crates/dag/src/crypto` | Consensus block signing is a separate Ed25519 path and must not be confused with user transaction PQC curves. |

## Findings

- No call site should rely on guessing curves from an untagged address; `verify_signature` rejects untagged addresses.
- `keypair_from_private_key` rejects oversized formatted private keys before decoding/parsing.
- Wallet/keystore compatibility tests cover legacy on-disk formats through public APIs.
- `kanari-crypto` production PQC signing/verification now uses the maintained
  `ml-dsa` provider path by default.
- `slh-dsa`/SPHINCS+ support is available only behind the explicit
  `experimental-slh-dsa` feature while the upstream crate remains release-candidate/unaudited.
- Hybrid/PQC dispatch preserves formatted key metadata so provider prefixes are not stripped before signing.

## Migration rule

Do not silently change existing `CurveType` wire names or wallet private-key
prefixes without an explicit migration.

The safe migration path is:

1. Keep existing `CurveType` wire names stable.
2. Generate new production PQC private keys with explicit provider prefixes (`kanamldsa`).
   Generate `kanaslh` keys only when `experimental-slh-dsa` is deliberately enabled.
3. Preserve provider metadata through signing dispatch and hybrid private-key packing.
4. Treat old `kanapqc` imports as legacy material that must be re-keyed/migrated before production signing.
5. Add wallet/app/node submit tests for old-wallet import rejection, new-wallet signing, and mixed verification.
