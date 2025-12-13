kanari-crypto examples
======================

This folder contains small examples demonstrating signing and verification using the `kanari-crypto` crate.

Prerequisites
-------------

- Rust toolchain (stable) and Cargo installed.
- From the workspace root (project root), run the examples using `cargo run`.

Available examples
------------------

- `verify_with_keypair` — Demonstrates signing and verifying using a `KeyPair` (uses Ed25519+Dilithium3 hybrid by default in the example).
- `verify_k256_hybrid` — Demonstrates signing and verifying using K256+Dilithium3 hybrid.
- `verify_dilithium2` — Demonstrates signing and verifying with Dilithium2.
- `verify_dilithium3` — Demonstrates signing and verifying with Dilithium3.
- `verify_dilithium5` — Demonstrates signing and verifying with Dilithium5.

How to run
----------

From the repository root, run any example with:

```bash
cargo run -p kanari-crypto --example verify_with_keypair
cargo run -p kanari-crypto --example verify_k256_hybrid
cargo run -p kanari-crypto --example verify_dilithium2
cargo run -p kanari-crypto --example verify_dilithium3
cargo run -p kanari-crypto --example verify_dilithium5
```

Notes
-----

- PQC signatures (Dilithium, etc.) are large; expect signature sizes in the kilobyte range.
- Examples generate keys at runtime (not using persistent storage) and print signature length and verification result.
- If you want to run multiple examples programmatically, wrap the `cargo run` calls in a script.

Questions or changes
--------------------

If you want additional examples (e.g., verifying using stored keystore wallets or tagged addresses), tell me which scenario and I'll add one.
