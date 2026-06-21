# Mainnet Operations

This runbook matches the current validator CLI, checkpoint-certificate model, persistent checkpoint journal, and operator scripts.

> **Mainnet blocker:** checkpoint certificate structures, persistence, and inbound verification exist, but distributed quorum-signature aggregation and adversarial multi-validator testing must be completed before public mainnet. Treat this document as a controlled rehearsal runbook until that gate is closed.

## Operator Files

- `validator-committee.example.json` — schema-v2 operator manifest.
- `setup-multi-node.ps1` — creates a fresh committee manifest and keys without deleting existing data.
- `start-node.ps1` — starts one authority from the manifest.
- `monitor-cluster-health.ps1` — validates health, authority identity, height, supply and state root.
- `test-p2p-broadcast.ps1` — checks checkpoint convergence after an optional real transaction submission.
- `backup-node-data.ps1` / `restore-node-data.ps1` — stopped-node backup and checksum-verified restore.

## Required Configuration

All validators must agree on:

- `network = mainnet`
- `chain_id = kanari-v2-mysticeti`
- `protocol_version = 1`
- current epoch
- ordered authority IDs
- exactly the same `consensus-public-keys.json`
- quorum `(2 * authority_count) / 3 + 1`

Each validator must have:

- a unique authority ID;
- a unique 32-byte Ed25519 private seed stored in a file;
- a unique persistent data directory;
- reachable P2P configuration;
- RPC bound to localhost unless protected by firewall and authenticated gateway.

Never pass a consensus private key as a command-line hex value. The supported flag is:

```text
--consensus-private-key-file <PATH>
```

On Unix the node rejects symlinks, non-regular files, and private-key files with group/world permissions. Use mode `0600` or stricter and a key directory with mode `0700`.

## Checkpoint Certificate Contract

The canonical signing domain is:

```text
kanari:checkpoint-certificate:v1
```

The certificate binds:

- epoch and chain ID;
- protocol version;
- sequence and checkpoint hash;
- previous checkpoint hash;
- state root;
- optional certified DAG vertex;
- committee digest derived from the sorted authority/public-key map;
- unique Ed25519 signatures and total voting power.

Voting power is currently one per authority. Verification rejects duplicate signers, unknown signers, invalid signatures, mismatched committee digest, incorrect chain/epoch/protocol values, and voting power below quorum. A non-empty checkpoint received through sync without a certificate is rejected.

Certificates are created, verified and persisted internally. There is no operator CLI flag for supplying a certificate.

## Preflight

1. Build the exact reviewed revision:

   ```powershell
   cargo build -p kanari-node --release
   ```

2. Prepare a schema-v2 committee manifest from `validator-committee.example.json`.
3. Generate keys into a new empty key directory:

   ```powershell
   kanari-node consensus-keygen `
     --node-count 4 `
     --output-dir C:\kanari\mainnet\consensus-keys
   ```

4. Distribute only each validator's own private-key file to that host.
5. Distribute the same reviewed public-key map and manifest to every validator.
6. Confirm each data directory is dedicated and backed up.
7. Confirm firewall policy and P2P reachability.
8. Confirm no validator reuses an authority ID, private key, or data directory.
9. Confirm distributed checkpoint-signature aggregation is enabled and has passed fault-injection tests. If not, **no-go**.

## Start Shape

Use the operator manifest:

```powershell
.\start-node.ps1 `
  -CommitteeConfig C:\kanari\mainnet\validator-committee.json `
  -AuthorityId 0x1
```

Equivalent binary invocation:

```powershell
kanari-node start `
  --network mainnet `
  --authority-id 0x1 `
  --authorities 0x1,0x2,0x3,0x4 `
  --data-dir C:\kanari\mainnet\validator1 `
  --p2p-port 19000 `
  --rpc-host 127.0.0.1 `
  --rpc-port 19001 `
  --consensus-private-key-file C:\kanari\mainnet\consensus-keys\node1-consensus-private-key.hex `
  --consensus-public-keys C:\kanari\mainnet\consensus-keys\consensus-public-keys.json
```

The node fails fast if the private key is absent, malformed, insecure on Unix, or does not match the authority's public key.

## Staged Rollout

1. Start one source validator.
2. Confirm `kanari_health.status = ok` and persistent storage is available.
3. Start validators one at a time.
4. After each join, run:

   ```powershell
   .\monitor-cluster-health.ps1 `
     -CommitteeConfig C:\kanari\mainnet\validator-committee.json
   ```

5. Submit controlled transactions through the reviewed wallet or RPC client.
6. Verify P2P convergence:

   ```powershell
   .\test-p2p-broadcast.ps1 `
     -CommitteeConfig C:\kanari\mainnet\validator-committee.json `
     -SubmitAction { .\submit-reviewed-test-transaction.ps1 }
   ```

7. Restart one follower, then the source validator, checking convergence after each restart.
8. Do not expose user traffic until all validators converge on height, supply and state root.

## Monitoring

Required JSON-RPC checks:

- `kanari_health`
- `kanari_getStats`
- `kanari_getNetworkStatus`

Alert conditions:

- RPC unavailable;
- health status not `ok`;
- supply invariants false;
- persistent storage unavailable on mainnet;
- authority ID or authority set mismatch;
- height lag beyond the approved window;
- supply mismatch;
- state-root mismatch;
- certificate verification errors in sync logs;
- repeated checkpoint rejection or missing previous hash.

Certificate details are not currently exposed as a dedicated JSON-RPC response. Use logs and persisted checkpoint data for certificate diagnostics.

## Backup And Restore

Stop all `kanari-node` processes before filesystem backup.

```powershell
.\backup-node-data.ps1 `
  -SourceDataDir C:\kanari\mainnet\validator1 `
  -BackupRoot C:\kanari\backups `
  -Label validator1-preupgrade `
  -RpcUrl http://127.0.0.1:19001
```

The backup includes a SHA-256 file manifest and metadata containing checkpoint height/state root when RPC is supplied.

Restore only to a fresh empty directory:

```powershell
.\restore-node-data.ps1 `
  -BackupDir C:\kanari\backups\validator1-preupgrade-YYYYMMDD-HHMMSS `
  -TargetDataDir C:\kanari\restore-test\validator1
```

After restore, start with the same committee manifest and keys, then verify health, authority ID, height, supply and state root.

## Go/No-Go

### Go only when

- canonical CI and full multi-node tests pass;
- certificate aggregation reaches quorum under normal operation;
- invalid, duplicate and unknown signatures are rejected;
- checkpoint sync refuses missing/invalid certificates;
- restart and journal recovery preserve checkpoint metadata and certificates;
- backup/restore drill passes checksum verification;
- all validators converge on identical height, supply and state root;
- operational monitoring and rollback ownership are assigned.

### No-go when

- only a local signing key is available for a committee requiring multiple signatures;
- validators use different public-key maps or authority ordering;
- any state-root or supply divergence appears;
- certificate aggregation, Byzantine testing, crash recovery, or atomic persistence remains unverified.
