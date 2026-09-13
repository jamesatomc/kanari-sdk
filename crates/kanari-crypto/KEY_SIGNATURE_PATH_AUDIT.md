# Key and Signature Path Audit

Last checked: 2026-09-10

## Policy decision (2026-09-10): slh-dsa ships in default features

The 2026-08-09 revision stated that `slh-dsa`/SPHINCS+ is available only
behind the explicit `experimental-slh-dsa` feature. That statement is
superseded:

- `slh-dsa` is part of the default `pqc` feature set because on-chain
  verification depends on it: `kanari-system-natives` wires
  `verify_signature_sphincs` into the `sphincs_plus_sha256_robust` Move
  native, and all workspace consumers build `kanari-crypto` with default
  features. Removing it from defaults would silently disable a
  consensus-level signature scheme — a far larger risk than shipping the
  release-candidate provider.
- Residual risk (upstream `slh-dsa 0.2.0-rc.5` unaudited) is accepted with a
  kill-switch: every slh-dsa call site (`signatures/sphincs.rs`,
  `keys/pqc.rs` generation + seed derivation, provider import) is
  `#[cfg(feature = "slh-dsa")]`-gated with fail-closed fallbacks, so
  dropping `"slh-dsa"` from the `pqc` feature set degrades gracefully
  (explicit `GenerationFailed` / `InvalidFormat` errors, no silent fallback).
- Revisit when upstream `slh-dsa` reaches a stable, audited release.

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
- All 11 `CurveType`s (classical + PQC + hybrid) support deterministic BIP39
  mnemonic derivation (`keypair_from_mnemonic`, new `keypair_from_seed`,
  `import_from_seed_phrase`, HD `derive_keypair_from_path`). PQC sub-seeds are
  SHAKE256 domain-separated per algorithm; behavior is frozen by
  `tests/kat_test.rs` + `tests/fixtures/pqc_mnemonic_kat.json` (22 vectors),
  and libFuzzer (`fuzz/key_generation`) plus proptest
  (`prop_fuzz_pqc_mnemonic_derivation`) assert no-panic + determinism.
  Known accepted trade-off: a hybrid key's classical half equals the
  standalone classical key for the same mnemonic (linkable by design).
- Hybrid/PQC dispatch preserves formatted key metadata so provider prefixes are not stripped before signing.
- Direct, explicit-curve, keypair, and batch verification paths now share
  resource-exhaustion guards for oversized public-key/address text, messages,
  signatures, empty batches, and excessive batch item counts.
- Ed25519 batch verification uses `ed25519-dalek` native batch verification.
  K256/P256 and PQC/hybrid batch APIs use parallel per-signature verification
  rather than unsafe ECDSA/PQC aggregation.
- Official Wycheproof JSON corpus coverage is vendored for Ed25519, ECDSA
  P-256 SHA-256, and ECDSA secp256k1 SHA-256. The ECDSA files validate the
  underlying SHA-256 curve verifiers and do not redefine Kanari account
  K256/P256 SHA3-256 prehash semantics.

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

## Batch verification policy

- Empty batches are rejected, not treated as success.
- Oversized batches are rejected before curve-specific parsing.
- Every item is validated for address/message/signature resource limits before
  dispatch.
- `Ed25519` may use native randomized batch verification.
- `K256` and `P256` intentionally do not use custom ECDSA aggregate batch math
  in this crate. Until a formally reviewed provider is adopted, the safe
  optimized path is parallel independent verification.
- Tagged batch verification requires tagged addresses and remains fail-closed
  for untagged or malformed addresses.
