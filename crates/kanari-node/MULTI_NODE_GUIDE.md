# Kanari Multi-Node Setup Guide

This guide matches the current `kanari-node` CLI, Mysticeti-backed DAG runtime, and checkpoint-certificate format.

## Current Model

A validator starts with three coordinated inputs:

1. an authority ID and the ordered committee authority list;
2. one local Ed25519 private-key **file** plus the shared public-key map;
3. an operator committee manifest describing network, chain, epoch, protocol version, ports and data directories.

`validator-committee.example.json` is an operator manifest consumed by the PowerShell scripts. The Rust binary does not accept that JSON directly; `start-node.ps1` translates it into the supported CLI flags.

The active DAG chain ID is `kanari-v2-mysticeti`. Checkpoint certificates use protocol version `1` and signing domain `kanari:checkpoint-certificate:v1`.

## Build

```powershell
cargo build -p kanari-node --release
```

## Generate A Fresh Local Committee

Choose new directories. The setup script refuses to overwrite node data or an incomplete key directory.

```powershell
cd crates\kanari-node

.\setup-multi-node.ps1 `
  -NodeCount 4 `
  -Network devnet `
  -DataRoot "$env:USERPROFILE\.kanari\clusters\devnet-v2" `
  -ConsensusKeyDir "$env:USERPROFILE\.kanari\consensus-keys\devnet-v2"
```

This creates:

```text
<ConsensusKeyDir>/
  consensus-public-keys.json
  node1-consensus-private-key.hex
  node2-consensus-private-key.hex
  ...

<DataRoot>/
  validator-committee.local.json
```

Private-key files contain a 32-byte Ed25519 seed encoded as 64 hexadecimal characters. On Unix, `kanari-node` rejects symlinks, non-regular files, and files with group/world permissions.

Start the generated committee:

```powershell
.\setup-multi-node.ps1 `
  -NodeCount 4 `
  -Network devnet `
  -DataRoot "$env:USERPROFILE\.kanari\clusters\devnet-v2" `
  -ConsensusKeyDir "$env:USERPROFILE\.kanari\consensus-keys\devnet-v2" `
  -StartNodes
```

## Start One Validator

```powershell
.\start-node.ps1 `
  -CommitteeConfig "$env:USERPROFILE\.kanari\clusters\devnet-v2\validator-committee.local.json" `
  -AuthorityId 0x2
```

The script validates that:

- the manifest is schema version 2 or newer;
- authority IDs are unique and match the authority entries;
- `chain_id` is `kanari-v2-mysticeti`;
- `protocol_version` is `1`;
- the private-key file contains exactly 64 hex characters;
- the public-key map contains exactly the configured authority set;
- the binary receives `--consensus-private-key-file`, never secret key material on the command line.

Equivalent manual command:

```powershell
cargo run -p kanari-node -- start `
  --network devnet `
  --authority-id 0x2 `
  --authorities 0x1,0x2,0x3,0x4 `
  --data-dir "$env:USERPROFILE\.kanari\clusters\devnet-v2\node2" `
  --p2p-port 19010 `
  --rpc-host 127.0.0.1 `
  --rpc-port 19011 `
  --consensus-private-key-file "$env:USERPROFILE\.kanari\consensus-keys\devnet-v2\node2-consensus-private-key.hex" `
  --consensus-public-keys "$env:USERPROFILE\.kanari\consensus-keys\devnet-v2\consensus-public-keys.json" `
  --bootstrap /ip4/127.0.0.1/tcp/19000
```

## Checkpoint Certificates

Each non-empty synced checkpoint must carry a certificate. The engine verifies:

- chain ID, epoch and protocol version;
- checkpoint sequence, hash, previous hash and state root;
- certified DAG vertex and committee digest;
- unique committee signers;
- Ed25519 signatures over canonical certificate signing bytes;
- voting power of `1` per configured authority;
- quorum `(2 * committee_size) / 3 + 1`.

Certificates are created, verified and persisted internally. Operators do not pass certificates through the CLI.

### Readiness limitation

The certificate structures and sync verification are present, but distributed quorum-signature aggregation must be completed and adversarially tested before a public multi-validator mainnet. The node intentionally refuses uncertified synced checkpoints rather than silently accepting them.

## RPC And Monitoring

RPC defaults to `127.0.0.1`. Bind `0.0.0.0` only behind firewall and authenticated gateway controls.

```powershell
.\monitor-cluster-health.ps1 `
  -CommitteeConfig "$env:USERPROFILE\.kanari\clusters\devnet-v2\validator-committee.local.json"
```

The monitor uses actual JSON-RPC methods:

- `kanari_health`
- `kanari_getStats`
- `kanari_getNetworkStatus`

It checks network, authority identity, health, supply invariants, height, total supply and state root. Certificate status is not currently exposed as a JSON-RPC field; certificate verification happens during checkpoint sync.

## P2P Convergence Test

Without a transaction submission action, the script checks current convergence:

```powershell
.\test-p2p-broadcast.ps1 `
  -CommitteeConfig "$env:USERPROFILE\.kanari\clusters\devnet-v2\validator-committee.local.json"
```

To test a real submitted transaction, provide a script block that invokes your wallet or JSON-RPC submission code:

```powershell
.\test-p2p-broadcast.ps1 `
  -CommitteeConfig "$env:USERPROFILE\.kanari\clusters\devnet-v2\validator-committee.local.json" `
  -SubmitAction { .\submit-test-transaction.ps1 }
```

Success means the source checkpoint advances and every validator converges on equal height, supply and state root.

## Data And Recovery

Every validator requires a unique data directory. An empty directory creates local genesis on first start. Never copy a live RocksDB/SQLite directory as a valid backup.

```powershell
.\backup-node-data.ps1 `
  -SourceDataDir "$env:USERPROFILE\.kanari\clusters\devnet-v2\node1" `
  -BackupRoot "$env:USERPROFILE\.kanari\backups" `
  -Label node1-preupgrade
```

Restore into a fresh directory:

```powershell
.\restore-node-data.ps1 `
  -BackupDir "$env:USERPROFILE\.kanari\backups\node1-preupgrade-YYYYMMDD-HHMMSS" `
  -TargetDataDir "$env:USERPROFILE\.kanari\restore-test\node1"
```

The backup/restore scripts verify SHA-256 hashes. The data directory contains state, checkpoint metadata/journal/certificates, object storage, peer data and indexer data when present.

## Troubleshooting

### Missing key file

Run `consensus-keygen` into a new empty directory or correct `consensus_private_key_file` in the manifest. Do not paste a private key into the command line.

### Public-key mismatch

The local private key must derive the public key assigned to the same authority ID in `consensus-public-keys.json`.

### Certificate rejection

Check that every validator uses the same authority list and public-key map. A different map produces a different committee digest. Also verify chain ID, epoch, protocol version, checkpoint sequence and previous hash.

### Nodes do not converge

Check unique ports, reachable bootstrap multiaddrs, firewall rules, authority IDs and logs for certificate, state-root or transaction-signature failures.
