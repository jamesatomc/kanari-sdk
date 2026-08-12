# Security dependency audit policy

Run the local dependency/static audit with:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\run-security-audit.ps1
```

Current temporary RustSec risk acceptances:

- `RUSTSEC-2026-0118` / `RUSTSEC-2026-0119` (`hickory-proto 0.25.2`)
  - Source: optional `libp2p-mdns` lockfile entry from `libp2p`.
  - Mitigation: production default disables the `kanari-node/p2p-mdns` feature and `kanari-node`'s active dependency tree no longer includes `hickory-proto`.
  - Follow-up: remove the ignore when `libp2p` publishes a compatible release that moves mDNS to `hickory-proto >=0.26.1` or removes the vulnerable dependency path.
- `RUSTSEC-2023-0071` (`rsa 0.9.10`)
  - Source: `kanari-system-natives` RS256 native verification.
  - Mitigation: runtime path performs public-key signature verification only; no RSA private-key operation is exposed in production runtime code.
  - Follow-up: remove the ignore when the upstream `rsa` crate publishes a fixed version or when RS256 is replaced by a maintained verification backend.

Do not add a new ignored advisory without recording:

1. the dependency path,
2. the production exposure,
3. the mitigation,
4. the removal condition.
