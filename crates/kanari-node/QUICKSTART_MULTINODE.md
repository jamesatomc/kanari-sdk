# Quick Start — Certificate-Aware Multi-Node Cluster

## 1. Build

From the repository root:

```powershell
cargo build -p kanari-node --release
cd crates\kanari-node
```

## 2. Create A Fresh Committee

Use new directories; the setup script will not erase existing databases or keys.

```powershell
$dataRoot = "$env:USERPROFILE\.kanari\clusters\devnet-v2"
$keyRoot = "$env:USERPROFILE\.kanari\consensus-keys\devnet-v2"

.\setup-multi-node.ps1 `
  -NodeCount 4 `
  -Network devnet `
  -DataRoot $dataRoot `
  -ConsensusKeyDir $keyRoot
```

Generated files include:

```text
$dataRoot\validator-committee.local.json
$keyRoot\consensus-public-keys.json
$keyRoot\node1-consensus-private-key.hex
$keyRoot\node2-consensus-private-key.hex
...
```

The manifest records:

- `chain_id = kanari-v2-mysticeti`
- `protocol_version = 1`
- epoch and authority set
- quorum `(2N/3)+1`
- P2P/RPC ports and bootstrap addresses
- per-validator data and private-key file paths

## 3. Start The Committee

```powershell
.\setup-multi-node.ps1 `
  -NodeCount 4 `
  -Network devnet `
  -DataRoot $dataRoot `
  -ConsensusKeyDir $keyRoot `
  -StartNodes
```

Or start one validator manually through the manifest:

```powershell
.\start-node.ps1 `
  -CommitteeConfig "$dataRoot\validator-committee.local.json" `
  -AuthorityId 0x1
```

The node receives `--consensus-private-key-file`; secret key material is not placed in the process command line.

## 4. Check The Cluster

```powershell
.\monitor-cluster-health.ps1 `
  -CommitteeConfig "$dataRoot\validator-committee.local.json"
```

This calls the real JSON-RPC methods `kanari_health`, `kanari_getStats`, and `kanari_getNetworkStatus` and compares height, supply and state root.

Manual JSON-RPC example:

```powershell
$body = @{
  jsonrpc = '2.0'
  method = 'kanari_getStats'
  params = @{}
  id = 1
} | ConvertTo-Json

Invoke-RestMethod `
  -Uri 'http://127.0.0.1:19001' `
  -Method Post `
  -ContentType 'application/json' `
  -Body $body
```

## 5. Test P2P Convergence

```powershell
.\test-p2p-broadcast.ps1 `
  -CommitteeConfig "$dataRoot\validator-committee.local.json"
```

For a real transaction test, pass your own submission action:

```powershell
.\test-p2p-broadcast.ps1 `
  -CommitteeConfig "$dataRoot\validator-committee.local.json" `
  -SubmitAction { .\submit-test-transaction.ps1 }
```

The test succeeds only when validators converge on the same checkpoint height, total supply and state root. Synced non-empty checkpoints are accepted only after checkpoint-certificate verification.

## Important

- RPC binds to `127.0.0.1` by default.
- Every validator needs a unique private key and data directory.
- Every validator must use the same ordered authority set and public-key map.
- An empty data directory initializes genesis on first start.
- Distributed quorum-signature aggregation must be completed and adversarially tested before public mainnet use.

See [MULTI_NODE_GUIDE.md](MULTI_NODE_GUIDE.md) and [MAINNET_OPERATIONS.md](MAINNET_OPERATIONS.md).
