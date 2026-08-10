# Wycheproof fixtures

These JSON files are vendored from the official C2SP Wycheproof repository:

- <https://github.com/C2SP/wycheproof>
- Source path: `testvectors_v1/`

Included files:

- `ed25519_test.json`
- `ecdsa_secp256r1_sha256_test.json`
- `ecdsa_secp256k1_sha256_test.json`
- `LICENSE.wycheproof`

The ECDSA SHA-256 vectors are tested against underlying k256/p256 SHA-256
verification semantics. Kanari account-level K256/P256 signatures intentionally
use SHA3-256 prehashing, so these files are not used to redefine Kanari account
signature semantics.
