# Kanari Multi-Node Setup Guide

คู่มือการรัน Kanari blockchain แบบหลาย node ด้วย libp2p

## คุณสมบัติ

- ✅ P2P networking ด้วย libp2p
- ✅ Automatic peer discovery ด้วย mDNS (สำหรับ local network)
- ✅ Kademlia DHT สำหรับ peer discovery
- ✅ Gossipsub protocol สำหรับ message propagation
- ✅ Block และ transaction synchronization
- ✅ Configurable P2P และ RPC ports

## การติดตั้ง

1. Build project:

```bash
cargo build --release
```

1. ไฟล์ที่ถูกสร้าง:

```
target/release/kanari-node
```

## การรัน Multi-Node

### Node 1 (Bootstrap Node)

```bash
kanari-node start --p2p-port 19000 --rpc-port 19001 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node1
```

### Node 2

```bash
kanari-node start --p2p-port 19010 --rpc-port 19011 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node2
```

### Node 3

```bash
kanari-node start --p2p-port 19020 --rpc-port 19021 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node3
```

## การเชื่อมต่อ Nodes

### อัตโนมัติ (Local Network)

หาก nodes ทำงานในเครือข่ายเดียวกัน จะค้นหากันเองผ่าน mDNS โดยอัตโนมัติ

## ตัวอย่างการใช้งาน

### 1. ดู Stats ของ blockchain

```bash
kanari-node stats
```

### 2. ดูข้อมูล Account

```bash
kanari-node account 0x1
```

### 3. ดูข้อมูล Block

```bash
kanari-node block 0
```

### 4. List wallets

```bash
kanari-node list-wallets
```

### 5. ดูตัวเลือกทั้งหมด

```bash
kanari-node start --help
```

**ตัวเลือกที่สำคัญ:**

- `--p2p-port <PORT>` - กำหนด port สำหรับ P2P networking (default: 19000)
- `--rpc-port <PORT>` - กำหนด port สำหรับ RPC server (default: 19001)
- `--data-dir <PATH>` - กำหนดตำแหน่งเก็บข้อมูล blockchain และ state
- `--bootstrap <MULTIADDR>` - เชื่อมต่อกับ bootstrap peer (สามารถระบุหลายครั้ง)

## โครงสร้าง P2P Network

```
┌─────────────────────────────────────────────┐
│           Kanari P2P Network                │
├─────────────────────────────────────────────┤
│                                             │
│  Node 1 (19000) ←→ Node 2 (19010)          │
│       ↑                 ↓                   │
│       └─────→ Node 3 (19020)               │
│                                             │
└─────────────────────────────────────────────┘
```

### Protocols ที่ใช้

1. **Gossipsub** - สำหรับ broadcast blocks และ transactions
   - Topic: `kanari/blocks`
   - Topic: `kanari/transactions`
   - Topic: `kanari/peers`

2. **mDNS** - Auto-discovery ใน local network

3. **Kademlia DHT** - Distributed peer discovery

4. **Noise Protocol** - Encrypted transport

5. **Yamux** - Stream multiplexing

## P2P Message Types

```rust
pub enum P2PMessage {
    NewTransaction(String),  // Transaction ใหม่
    NewBlock(String),        // Block ใหม่
    BlockRequest(u64),       // ขอ block ตาม height
    BlockResponse(String),   // ตอบกลับด้วย block data
    PeerInfo(PeerInfoMsg),   // ข้อมูล peer (height, peer_id)
}
```

## การ Sync Blocks

เมื่อ node ใหม่เข้าร่วม network:

1. Node รับ `PeerInfo` จาก peers อื่นๆ
2. เปรียบเทียบ height กับ local blockchain
3. ถ้าต่ำกว่า จะส่ง `BlockRequest` เพื่อขอ blocks ที่ขาดหายไป
4. รับ `BlockResponse` และ apply blocks ไปยัง local chain

## Port Configuration

| Service | Default Port |   การปรับแต่ง  |
|---------|--------------|--------------|
| P2P     | 19000        | `--p2p-port` |
| RPC     | 19001        | `--rpc-port` |

## Data Directory Configuration

แต่ละ node ควรมี data directory แยกกันเพื่อป้องกันข้อมูลทับกัน:

### Windows

```bash
--data-dir C:\Users\<Username>\.kanari\kanari-db\node1
--data-dir C:\Users\<Username>\.kanari\kanari-db\node2
--data-dir C:\Users\<Username>\.kanari\kanari-db\node3
```

### Linux/macOS

```bash
--data-dir ~/.kanari/kanari-db/node1
--data-dir ~/.kanari/kanari-db/node2
--data-dir ~/.kanari/kanari-db/node3
```

**หมายเหตุ:** ถ้าไม่ระบุ `--data-dir` ระบบจะใช้ default directory ซึ่งอาจทำให้ nodes ใช้ข้อมูลร่วมกัน

## ตัวอย่าง Advanced Setup

### การใช้ PowerShell Scripts (Windows)

ในโฟลเดอร์นี้มี PowerShell scripts สำหรับช่วยในการรัน multi-node:

#### 1. Setup และดูข้อมูล Configuration

```powershell
.\setup-multi-node.ps1
```

Script นี้จะ:

- สร้าง data directories สำหรับแต่ละ node
- แสดงข้อมูล configuration ของแต่ละ node
- แสดงคำสั่งสำหรับการรัน nodes

#### 2. รัน Node แต่ละตัว

```powershell
# Terminal 1
.\start-node.ps1 -NodeId 1

# Terminal 2
.\start-node.ps1 -NodeId 2

# Terminal 3
.\start-node.ps1 -NodeId 3
```

### การรัน 5 Nodes พร้อมกัน (Manual)

```bash
# Terminal 1
kanari-node start --p2p-port 19000 --rpc-port 19001 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node1

# Terminal 2
kanari-node start --p2p-port 19010 --rpc-port 19011 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node2

# Terminal 3
kanari-node start --p2p-port 19020 --rpc-port 19021 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node3

# Terminal 4
kanari-node start --p2p-port 19030 --rpc-port 19031 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node4

# Terminal 5
kanari-node start --p2p-port 19040 --rpc-port 19041 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node5
```

## RPC Endpoints

- Local (loopback):
  - Node 1: `http://127.0.0.1:19001`
  - Node 2: `http://127.0.0.1:19011`
  - Node 3: `http://127.0.0.1:19021`

- LAN (reachable from other machines on your network):
  - Node 1: `http://<machine_ip>:19001`
  - Node 2: `http://<machine_ip>:19011`
  - Node 3: `http://<machine_ip>:19021`

To expose RPC to the LAN, start each node with either `--rpc-host 0.0.0.0` (bind all interfaces) or `--rpc-host <machine_ip>` (bind a single interface). Example:

```powershell
kanari-node start --p2p-port 19000 --rpc-port 19001 --rpc-host 0.0.0.0 --data-dir C:\Users\Pukpuy\.kanari\kanari-db\node1
```

Security note: binding RPC to all interfaces exposes the API to your local network — ensure your firewall and network policies allow or block access as intended.

## การ Monitor Network

ดู logs เพื่อตรวจสอบ P2P events:

```
INFO kanari_node: Node Peer ID: 12D3KooW...
INFO kanari_node: P2P network initialized on port 19000
INFO kanari_node: Listening on /ip4/0.0.0.0/tcp/19000
INFO kanari_node: Discovered peer: 12D3KooW... at /ip4/...
INFO kanari_node: Connection established with 12D3KooW...
INFO kanari_node: Received transaction from network: 0x...
INFO kanari_node: Received new block #123 from network
```

## Troubleshooting

### ปัญหา: Nodes ไม่เจอกัน

**แก้ไข:**

1. ตรวจสอบว่าใช้ port ไม่ซ้ำกัน
2. ตรวจสอบ firewall settings
3. ลอง manual bootstrap ด้วย `--bootstrap`

### ปัญหา: Block sync ไม่ทำงาน

**แก้ไข:**

1. ตรวจสอบ logs หา errors
2. ตรวจสอบว่า blocks ถูก broadcast หรือไม่
3. Restart nodes เพื่อ re-sync

## สถาปัตยกรรม

```
┌──────────────────────────────────────────┐
│         Kanari Node                      │
├──────────────────────────────────────────┤
│                                          │
│  ┌────────────┐      ┌──────────────┐    │
│  │    RPC     │      │  P2P Network │    │
│  │  Server    │      │  (libp2p)    │    │
│  └──────┬─────┘      └──────┬───────┘    │
│         │                   │            │
│         └────┬──────────────┘            │
│              │                           │
│      ┌───────▼────────┐                  │
│      │ Blockchain     │                  │
│      │ Engine         │                  │
│      └───────┬────────┘                  │
│              │                           │
│      ┌───────▼────────┐                  │
│      │ Move Runtime   │                  │
│      │ + State        │                  │
│      └────────────────┘                  │
│                                          │
└──────────────────────────────────────────┘
```

## การพัฒนาต่อ

### ✅ คุณสมบัติที่ทำงานแล้ว

- [x] **Block synchronization** - Nodes สามารถ sync blocks จากกันได้ผ่าน P2P
- [x] **State sync optimization** - Execute transactions จาก synced blocks เพื่อ rebuild state
  - Skip sequence validation สำหรับ synced transactions
  - ใช้ main Move runtime เพื่อให้ module bytecode persist ถูกต้อง
  - Apply changesets เพื่อ update accounts, balances, modules, objects
- [x] **Transaction pool management** - Pending transaction pool พร้อม broadcast ผ่าน P2P
- [x] **P2P message propagation** - Gossipsub protocol สำหรับ broadcast blocks/transactions
- [x] **Peer discovery** - mDNS (local) และ Kademlia DHT
- [x] **Block validation logic** - ตรวจสอบ block hash, prev_hash chain, timestamp, transaction integrity
- [x] **Transaction deduplication** - ป้องกัน double spending และ replay attacks ด้วย transaction hash tracking
- [x] **Persistent peer storage** - บันทึกและโหลด peer list จาก disk เพื่อ reconnect อัตโนมัติ
- [x] **Transaction signature verification** - Verify signatures สำหรับ synced และ committed transactions
  - Block เก็บ `SignedTransaction` แทน `Transaction`
  - Verify signatures ใน `Block::verify()` และ `sync_full_block_from_data()`
  - ป้องกัน malicious blocks จาก compromised nodes
- [x] **Merkle tree for transactions** - Transaction merkle root ใน block header สำหรับ light client verification
  - ใช้ SMT's Blake3 hash function เพื่อความสอดคล้องกับ state tree
  - ทุก block มี merkle root ที่คำนวณจาก transaction hashes
  - Block validation ตรวจสอบ merkle root integrity
  - RPC endpoint `kanari_getTransactionMerkleProof` สำหรับ proof generation
  - Support proof verification สำหรับ light clients
- [x] **Fork resolution** - Logic สำหรับจัดการ chain forks และเลือก canonical chain
  - Longest chain rule - เลือก chain ที่มี work (length) มากที่สุด
  - Automatic chain reorganization เมื่อพบ fork ที่ยาวกว่า
  - เก็บ alternative chains ไว้สำหรับ rollback
  - Prune old forks เพื่อประหยัด memory
- [x] **NAT traversal** - รองรับการเชื่อมต่อข้าม WAN (relay, hole punching)
  - libp2p relay client สำหรับ relay connections
  - DCUtR (Direct Connection Upgrade through Relay) สำหรับ hole punching
  - AutoNAT สำหรับตรวจสอบ NAT status
  - Support การเชื่อมต่อ peer-to-peer ข้าม NAT/firewall

## Merkle Tree Architecture

Kanari ใช้ **2 ประเภท** ของ Merkle trees:

### 1. Sparse Merkle Tree (SMT) - State Storage

- **ตำแหน่ง**: `crates/smt/`
- **จุดประสงค์**: Account state verification และ proofs
- **ใช้สำหรับ**: Account balances, modules, objects, state root
- **Storage**: Persistent ใน RocksDB

### 2. Transaction Merkle Tree - Block Verification  

- **ตำแหน่ง**: `crates/kanari-core/src/blockchain/merkle.rs`
- **จุดประสงค์**: Light client transaction verification
- **ใช้สำหรับ**: Block header merkle root, transaction inclusion proofs
- **Storage**: In-memory, คำนวณใหม่ต่อ block

ดู [DOCS/MERKLE_TREES.md](../../../DOCS/MERKLE_TREES.md) สำหรับรายละเอียดเพิ่มเติม

### 🚧 TODO items ที่สามารถพัฒนาต่อได้

- [ ] **Consensus mechanism** - PoS, PoW, หรือ BFT consensus
- [ ] **Metrics และ monitoring dashboard** - Real-time network statistics

## Fork Resolution

Kanari รองรับ fork resolution โดยใช้ **longest chain rule**:

### การทำงาน

1. **Fork Detection** - เมื่อ node รับ blocks จาก peer อื่น จะตรวจสอบว่ามี fork เกิดขึ้นหรือไม่
2. **Common Ancestor** - หา block ที่เป็น common ancestor ระหว่าง 2 chains
3. **Chain Comparison** - เปรียบเทียบ chain work (length) ของทั้ง 2 chains
4. **Reorganization** - ถ้า fork chain ยาวกว่า จะ reorganize ไปใช้ fork chain
5. **State Rebuild** - Rebuild state จาก blocks ใน canonical chain

### API

```rust
// Handle fork และ reorganize ถ้าจำเป็น
blockchain.handle_fork(fork_blocks)?;

// ดู canonical chain (main chain)
let canonical = blockchain.get_canonical_chain();

// ดู alternative forks
let forks = blockchain.get_forks();

// Prune old forks (เก็บแค่ recent forks)
blockchain.prune_forks(max_depth);
```

### ตัวอย่าง

```
Initial state:
  Genesis -> Block 1 -> Block 2 -> Block 3 (ours)

Fork detected:
  Genesis -> Block 1 -> Block 2 -> Block 3' -> Block 4' (fork)

Result (fork is longer):
  Old chain: [Block 3] -> stored in forks
  New chain: Genesis -> Block 1 -> Block 2 -> Block 3' -> Block 4'
```

## NAT Traversal

Kanari รองรับ NAT traversal เพื่อให้ nodes สามารถเชื่อมต่อกันได้แม้อยู่หลัง NAT/firewall:

### Protocols

1. **Relay** - Nodes สามารถใช้ relay node เป็นตัวกลางในการส่งข้อมูล
2. **DCUtR** (Direct Connection Upgrade through Relay) - Hole punching เพื่อสร้าง direct connection
3. **AutoNAT** - ตรวจสอบ NAT status อัตโนมัติ

### การทำงาน

```
Node A (behind NAT) <-> Relay Node <-> Node B (behind NAT)
         |                                    |
         +-------- DCUtR Hole Punch  ---------+
         |                                    |
         +-------- Direct Connection ---------+
```

### Logs ที่เกี่ยวข้อง

```
INFO kanari_node: Relay reservation accepted by 12D3KooW...
INFO kanari_node: DCUtR event: RemoteInitiatedDirectConnectionUpgrade
INFO kanari_node: Hole punching successful with 12D3KooW...
INFO kanari_node: NAT status changed from Unknown to Public
```

### การใช้งาน

Nodes จะพยายาม NAT traversal อัตโนมัติ ไม่ต้องตั้งค่าเพิ่มเติม แต่ถ้าต้องการใช้ relay node:

```bash
# Start relay node (ต้องมี public IP)
kanari-node start --p2p-port 19000 --rpc-port 19001 --relay-mode

# Client nodes จะค้นหา relay nodes อัตโนมัติผ่าน DHT
kanari-node start --p2p-port 19010 --rpc-port 19011
```

## License

Apache-2.0
