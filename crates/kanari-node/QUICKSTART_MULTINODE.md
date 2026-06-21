# Quick Start Guide - Multi-Node Setup

## Build

```powershell
cd C:\Users\Pukpuy\Desktop\kanari-sdk
cargo build -p kanari-node
```

## Start A Local 3-Node Cluster

`kanari-node` requires explicit DAG consensus keys. The setup script generates them and passes them to each node.

```powershell
cd C:\Users\Pukpuy\Desktop\kanari-sdk\crates\kanari-node
.\setup-multi-node.ps1 -NodeCount 3 -Network devnet -ResetSourceData -ResetReplicaData -ResetConsensusKeys
```

Generated keys are stored in:

```text
%USERPROFILE%\.kanari\consensus-keys
```

Files:

- `consensus-public-keys.json`
- `node1-consensus-private-key.hex`
- `node2-consensus-private-key.hex`
- `node3-consensus-private-key.hex`

## Start One Node Manually With Script

```powershell
cd C:\Users\Pukpuy\Desktop\kanari-sdk\crates\kanari-node
.\start-node.ps1 -NodeId 1 -Network devnet -Authorities "0x1,0x2,0x3"
```

Node 2 and 3 can bootstrap from node 1:

```powershell
.\start-node.ps1 -NodeId 2 -Network devnet -Authorities "0x1,0x2,0x3" -Bootstrap "/ip4/<node1-ip>/tcp/19000"
.\start-node.ps1 -NodeId 3 -Network devnet -Authorities "0x1,0x2,0x3" -Bootstrap "/ip4/<node1-ip>/tcp/19000"
```

## Manual Run Without Scripts

Generate consensus keys:

```powershell
cargo run --bin kanari-node -- consensus-keygen --node-count 3 --output-dir .\consensus-keys --force
```

Start node 1:

```powershell
cargo run --bin kanari-node -- start `
  --network devnet `
  --p2p-port 19000 `
  --rpc-port 19001 `
  --data-dir data\node1 `
  --authority-id 0x1 `
  --authorities 0x1,0x2,0x3 `
  --consensus-private-key-file .\consensus-keys\node1-consensus-private-key.hex `
  --consensus-public-keys .\consensus-keys\consensus-public-keys.json
```

Start node 2:

```powershell
cargo run --bin kanari-node -- start `
  --network devnet `
  --p2p-port 19010 `
  --rpc-port 19011 `
  --data-dir data\node2 `
  --authority-id 0x2 `
  --authorities 0x1,0x2,0x3 `
  --consensus-private-key-file .\consensus-keys\node2-consensus-private-key.hex `
  --consensus-public-keys .\consensus-keys\consensus-public-keys.json `
  --bootstrap /ip4/<node1-ip>/tcp/19000
```

Start node 3:

```powershell
cargo run --bin kanari-node -- start `
  --network devnet `
  --p2p-port 19020 `
  --rpc-port 19021 `
  --data-dir data\node3 `
  --authority-id 0x3 `
  --authorities 0x1,0x2,0x3 `
  --consensus-private-key-file .\consensus-keys\node3-consensus-private-key.hex `
  --consensus-public-keys .\consensus-keys\consensus-public-keys.json `
  --bootstrap /ip4/<node1-ip>/tcp/19000
```

## Check Status

```powershell
kanari-node stats
kanari-node account 0x1
kanari-node block 0
```

RPC defaults:

- Node 1: `http://127.0.0.1:19001`
- Node 2: `http://127.0.0.1:19011`
- Node 3: `http://127.0.0.1:19021`

## Notes

- Each node must have a separate `--data-dir`.
- Each node must have unique P2P/RPC ports.
- Each node must have its own consensus private key.
- All nodes must share the same `consensus-public-keys.json`.
- Do not commit private key files.

See the full guide: [MULTI_NODE_GUIDE.md](MULTI_NODE_GUIDE.md)
