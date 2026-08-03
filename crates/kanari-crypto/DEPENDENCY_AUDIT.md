# Kanari Crypto Dependency Audit

Last checked: 2026-08-02

Command highlights:

```text
cargo tree -p kanari-crypto --all-features
cargo clippy -p kanari-crypto --all-features -- -D warnings
cargo test -p kanari-crypto --all-features --quiet
```

## Result

`kanari-crypto` production PQC signing and verification no longer depends on
`pqcrypto-*`.

Current `kanari-crypto` PQC providers:

- Default/production feature set: `ml-dsa 0.1.1` for Dilithium-compatible
  ML-DSA-44/65/87 signing.
- Experimental-only feature set: `slh-dsa 0.2.0-rc.5` for
  SPHINCS+/SLH-DSA-SHA2-256f signing behind `experimental-slh-dsa`.

`slh-dsa` is intentionally not part of the default production dependency tree
while it remains a release candidate and lacks an independent audit.

Historical PQC findings that previously affected this crate:

- `pqcrypto-dilithium 0.5.0` — `RUSTSEC-2024-0380`, unmaintained.
- `pqcrypto-sphincsplus 0.7.2` — `RUSTSEC-2026-0160`, unmaintained because PQClean is archived.
- `pqcrypto-traits 0.3.5` — `RUSTSEC-2026-0162`, unmaintained because PQClean is archived.
- `pqcrypto-internals 0.2.11` — `RUSTSEC-2026-0163`, unmaintained because PQClean is archived.

Other workspace findings can still appear in workspace-wide `cargo audit`
because unrelated workspace members/dependencies are outside this crate's
provider path.

## Current mitigation in this crate

- New Dilithium-compatible keys are generated with `kanamldsa` provider metadata.
- New SPHINCS+/SLH-DSA keys are generated with `kanaslh` provider metadata
  only when `experimental-slh-dsa` is enabled.
- Hybrid and PQC signing dispatch preserves formatted key metadata.
- Wallet private-key formatting preserves `kanamldsa`, `kanaslh`, `kanapqc`,
  `kanahybrid`, and `kanari` prefixes instead of re-prefixing provider keys.
- PQC private key import rejects oversized formatted private keys before parsing.
- PQC/hybrid import tests include deterministic malformed seed corpus and higher proptest case counts.
- Signature verification tests include malformed tagged-address/signature corpus and oversized signature rejection coverage.
- Wallet/keystore compatibility tests cover legacy missing metadata, legacy array-form encrypted data, and older wallet TOML.

## Required production follow-up

- Run the long crypto fuzz campaign in scheduled CI/nightly with multi-hour duration.
- Re-evaluate `slh-dsa` before enabling it by default, after it leaves
  release-candidate status or receives an independent audit.
- Continue running `cargo audit` in CI on dependency updates and block newly introduced vulnerabilities.
