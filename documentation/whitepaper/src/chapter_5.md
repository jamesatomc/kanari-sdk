# 5. Cryptography and Security

## 5.1 Cryptographic agility

`kanari-crypto` supports Ed25519, K256, P256, Dilithium/ML-DSA2/3/5, Falcon512/1024, SPHINCS+ SHA256 robust, and K256+Dilithium3 or Ed25519+Dilithium3 hybrids. It also provides AES-256-GCM, Argon2id, SHA3, SHAKE256, and BLAKE3.

Classical signatures remain useful for compatibility and speed. Hybrid or post-quantum schemes are appropriate for keys requiring long-term protection. PQC provider maturity and independent-audit status must be tracked.

## 5.2 Wallet safety

Curve metadata, private-key material, and derived address must agree. Malformed seeds, wrong lengths, hybrid truncation, and key/address mismatches fail closed. HD derivation paths should be versioned in wallet migrations.

## 5.3 RPC boundary

RPC rejects malformed JSON/BCS, invalid signatures, nonce replay, stale object versions/digests, duplicate mutable inputs, gas overlap, oversized requests, and rate-limit abuse. Admin/debug methods require separate deployment controls.

## 5.4 DeFi authorization

Runtime object policy prevents unsafe Coin mutation, but it cannot infer application roles. Every public Move entry point that mutates an escrow, pool, vault, or admin object must check the signer and lifecycle state.

## 5.5 Audit posture

Nightly CI repeats boundary/property tests for RPC, object policy, consensus, and SMT and uploads a RustSec report. This is engineering assurance, not a substitute for an independent audit.

## 5.6 Algorithm selection

Classical signatures are smaller and usually faster. ML-DSA and SLH-DSA use the NIST standard names while retaining compatibility names where needed. Hybrid signatures require both constituent verifications. Address derivation, serialization, signature framing, and migration rules must match across wallet, RPC, node, and Move-native paths.

## 5.7 Key lifecycle

Private keys must use a cryptographically secure RNG, authenticated encryption at rest, and zeroization where supported. Stored curve, derivation path, public key, and address are cross-checked on load; mismatch fails closed. Malformed or truncated key material must never panic or silently select another curve.

## 5.8 Adversarial boundary

Treat every RPC, P2P, Move argument, keystore, and backup byte as hostile. Enforce limits before allocation, canonical decoding, signature-domain separation, nonce/replay checks, object digest/version checks, and bounded work per request. A NIST algorithm name or passing unit test is not an independent audit.

## 5.9 Authenticated encryption model

For plaintext `P`, key `K`, nonce `N`, and associated metadata `A`, authenticated encryption returns ciphertext and tag:

`AEAD_Encrypt(K, N, P, A) -> (C, tag)`

Decryption must reject unless the tag verifies:

`AEAD_Decrypt(K, N, C, A, tag) -> P` or `Reject`

Wallet and backup formats must bind version, curve, and metadata through authenticated data so a valid ciphertext cannot be relabeled as another key type.
