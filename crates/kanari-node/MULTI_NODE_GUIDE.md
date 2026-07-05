# Kanari Multi-Node Setup Guide

Guide for running multiple `kanari-node` validators with libp2p networking and explicit consensus signing keys.

## Build

```powershell
cargo build -p kanari-node
```

For release binaries:

```powershell
cargo build -p kanari-node --release
```

## Consensus Keys

DAG consensus no longer falls back to deterministic demo keys. Every validator must start with:

- one unique private consensus signing key
- one shared `consensus-public-keys.json` file containing the public keys for the whole authority set

Generate local keys for a 4-node test cluster:

```powershell
cargo run --bin kanari-node -- consensus-keygen --node-count 4 --output-dir .\consensus-keys --force
```

This creates:

```text
consensus-keys/
  consensus-public-keys.json
  node1-consensus-private-key.hex
  node2-consensus-private-key.hex
  node3-consensus-private-key.hex
  node4-consensus-private-key.hex
```

Keep private key files out of git and do not reuse one private key across validators.

## Fast Local Setup

The PowerShell setup script generates consensus keys automatically when they are missing.

```powershell
.\setup-multi-node.ps1 -NodeCount 4 -Network devnet -ResetSourceData -ResetReplicaData -ResetConsensusKeys
```

By default it stores keys under:

```text
%USERPROFILE%\.kanari\consensus-keys
```

Start nodes in separate terminals:

```powershell
.\start-node.ps1 -NodeId 1 -Network devnet -Authorities "0x1,0x2,0x3,0x4"
```

```powershell
.\start-node.ps1 -NodeId 2 -Network devnet -Authorities "0x1,0x2,0x3,0x4" -Bootstrap "/ip4/127.0.0.1/tcp/19000"
```

```powershell
.\start-node.ps1 -NodeId 3 -Network devnet -Authorities "0x1,0x2,0x3,0x4" -Bootstrap "/ip4/127.0.0.1/tcp/19000"
```

```powershell
.\start-node.ps1 -NodeId 4 -Network devnet -Authorities "0x1,0x2,0x3,0x4" -Bootstrap "/ip4/127.0.0.1/tcp/19000"
```

`start-node.ps1` reads the matching `node<N>-consensus-private-key.hex` file and the shared `consensus-public-keys.json` from the consensus key directory. RPC binds to `0.0.0.0` by default so explorer and wallet clients on the LAN can reach the node. Use firewall rules or pass a narrower `-RpcHost` value when running outside a trusted dev network.

## Manual Start

Manual start commands must pass the consensus private key and public-key map explicitly.

### Node 1

```powershell
$node1Key = (Get-Content .\consensus-keys\node1-consensus-private-key.hex -Raw).Trim()

cargo run --bin kanari-node -- start `
  --network devnet `
  --p2p-port 19000 `
  --rpc-port 19001 `
  --data-dir data/node1 `
  --authority-id 0x1 `
  --authorities 0x1,0x2,0x3,0x4 `
  --consensus-private-key-hex $node1Key `
  --consensus-public-keys .\consensus-keys\consensus-public-keys.json
```

### Node 2

```powershell
$node2Key = (Get-Content .\consensus-keys\node2-consensus-private-key.hex -Raw).Trim()

cargo run --bin kanari-node -- start `
  --network devnet `
  --p2p-port 19010 `
  --rpc-port 19011 `
  --data-dir data/node2 `
  --authority-id 0x2 `
  --authorities 0x1,0x2,0x3,0x4 `
  --consensus-private-key-hex $node2Key `
  --consensus-public-keys .\consensus-keys\consensus-public-keys.json `
  --bootstrap "/ip4/<node1-ip>/tcp/19000"
```

### Node 3

```powershell
$node3Key = (Get-Content .\consensus-keys\node3-consensus-private-key.hex -Raw).Trim()

cargo run --bin kanari-node -- start `
  --network devnet `
  --p2p-port 19020 `
  --rpc-port 19021 `
  --data-dir data/node3 `
  --authority-id 0x3 `
  --authorities 0x1,0x2,0x3,0x4 `
  --consensus-private-key-hex $node3Key `
  --consensus-public-keys .\consensus-keys\consensus-public-keys.json `
  --bootstrap "/ip4/<node1-ip>/tcp/19000"
```

### Node 4

```powershell
$node4Key = (Get-Content .\consensus-keys\node4-consensus-private-key.hex -Raw).Trim()

cargo run --bin kanari-node -- start `
  --network devnet `
  --p2p-port 19030 `
  --rpc-port 19031 `
  --data-dir data/node4 `
  --authority-id 0x4 `
  --authorities 0x1,0x2,0x3,0x4 `
  --consensus-private-key-hex $node4Key `
  --consensus-public-keys .\consensus-keys\consensus-public-keys.json `
  --bootstrap "/ip4/<node1-ip>/tcp/19000"
```

## Important Start Options

- `--network <NETWORK>`: selects `devnet`, `testnet`, or `mainnet`
- `--authority-id <ID>`: validator authority ID, for example `0x1`
- `--authorities <IDS>`: comma-separated committee, for example `0x1,0x2,0x3,0x4`
- `--consensus-private-key-hex <HEX>`: 32-byte Ed25519 seed hex for this validator
- `--consensus-public-keys <PATH>`: JSON map of authority ID to public key hex
- `--p2p-port <PORT>`: P2P networking port
- `--rpc-port <PORT>`: RPC server port
- `--rpc-host <HOST>`: RPC bind address
- `--data-dir <PATH>`: blockchain and state data directory
- `--bootstrap <MULTIADDR>`: bootstrap peer, can be specified multiple times
- `--relay-server`: enable circuit relay server mode

## Relay Server Mode

Relay mode also needs consensus keys when the node participates as a validator.

```powershell
$node1Key = (Get-Content .\consensus-keys\node1-consensus-private-key.hex -Raw).Trim()

kanari-node start `
  --network devnet `
  --p2p-port 19000 `
  --rpc-port 19001 `
  --authority-id 0x1 `
  --authorities 0x1,0x2,0x3,0x4 `
  --consensus-private-key-hex $node1Key `
  --consensus-public-keys .\consensus-keys\consensus-public-keys.json `
  --relay-server
```

## Data Directories

Each node must have a separate data directory.

Windows:

```powershell
--data-dir C:\Users\<Username>\.kanari\kanari-db\node1
--data-dir C:\Users\<Username>\.kanari\kanari-db\node2
--data-dir C:\Users\<Username>\.kanari\kanari-db\node3
--data-dir C:\Users\<Username>\.kanari\kanari-db\node4
```

Linux/macOS:

```bash
--data-dir ~/.kanari/kanari-db/node1
--data-dir ~/.kanari/kanari-db/node2
--data-dir ~/.kanari/kanari-db/node3
--data-dir ~/.kanari/kanari-db/node4
```

## RPC Endpoints

Local endpoints:

- Node 1: `http://127.0.0.1:19001`
- Node 2: `http://127.0.0.1:19011`
- Node 3: `http://127.0.0.1:19021`
- Node 4: `http://127.0.0.1:19031`

To expose RPC to the LAN, bind with `--rpc-host 0.0.0.0` or a specific machine IP. Only do this on a trusted network or behind firewall rules.

## Troubleshooting

### Node Fails With Missing Consensus Key

Run `consensus-keygen`, then pass `--consensus-private-key-hex` and `--consensus-public-keys`, or use `start-node.ps1` with the correct `-ConsensusKeyDir`.

### Node Fails With Consensus Public Key Mismatch

The private key for this node does not match the public key listed for its `--authority-id`. Regenerate the key set or use the correct private key file for that node.

### Nodes Cannot Find Each Other

Check unique P2P ports, firewall rules, and `--bootstrap` multiaddrs. On the same LAN, mDNS can discover peers automatically.

### Block Sync Not Working

Check logs, verify every node uses the same `--authorities` list and `consensus-public-keys.json`, then restart one follower after the source node is healthy.
