# Fork Resolution และ NAT Traversal

เอกสารนี้อธิบายการทำงานของ Fork Resolution และ NAT Traversal ใน Kanari blockchain

## Fork Resolution

### ภาพรวม

Fork resolution เป็นกลไกสำหรับจัดการกับสถานการณ์ที่ blockchain แยกออกเป็นหลาย chains (forks) โดยใช้ **longest chain rule** ในการเลือก canonical chain

### กลไกการทำงาน

1. **Fork Detection**
   - เมื่อ node รับ blocks จากเครือข่าย จะตรวจสอบว่า blocks เหล่านั้นสอดคล้องกับ local chain หรือไม่
   - ถ้าพบความแตกต่าง แสดงว่ามี fork เกิดขึ้น

2. **Common Ancestor Identification**
   - หา block สุดท้ายที่เหมือนกันระหว่าง 2 chains
   - Block นี้เรียกว่า "common ancestor"

3. **Chain Work Comparison**
   - คำนวณ "work" ของแต่ละ chain (ปัจจุบันใช้ความยาวของ chain)
   - เปรียบเทียบว่า chain ไหนมี work มากกว่า

4. **Chain Reorganization**
   - ถ้า fork chain มี work มากกว่า → reorganize ไปใช้ fork chain
   - ถ้า main chain ยังมี work มากกว่า → เก็บ fork ไว้เฉยๆ

5. **State Rebuild**
   - หลังจาก reorganize จะ rebuild transaction hash index
   - เก็บ old chain ไว้เป็น fork สำหรับ potential rollback

### API และตัวอย่าง

```rust
use kanari_core::blockchain::Blockchain;

let mut blockchain = Blockchain::new();

// รับ blocks จาก peer
let fork_blocks = receive_blocks_from_peer();

// จัดการ fork
match blockchain.handle_fork(fork_blocks) {
    Ok(true) => {
        println!("Reorganized to longer fork chain");
        // Rebuild state if needed
    }
    Ok(false) => {
        println!("Kept current chain (fork was shorter)");
    }
    Err(e) => {
        eprintln!("Fork handling error: {}", e);
    }
}

// ดู canonical chain
let canonical = blockchain.get_canonical_chain();
println!("Current chain height: {}", canonical.len() - 1);

// ดู stored forks
let forks = blockchain.get_forks();
println!("Number of stored forks: {}", forks.len());

// Prune old forks (เก็บแค่ recent 100 blocks)
blockchain.prune_forks(100);
```

### สถานการณ์ตัวอย่าง

#### Scenario 1: Fork ยาวกว่า (Reorganize)

```
Before:
  Genesis → Block 1 → Block 2 → Block 3 (main chain, work = 4)

Fork received:
  Genesis → Block 1 → Block 2' → Block 3' → Block 4' (fork, work = 5)

After reorganization:
  Main:  Genesis → Block 1 → Block 2' → Block 3' → Block 4'
  Forks: [Block 2, Block 3]
```

#### Scenario 2: Fork สั้นกว่า (Keep main chain)

```
Before:
  Genesis → Block 1 → Block 2 → Block 3 → Block 4 (main chain, work = 5)

Fork received:
  Genesis → Block 1 → Block 2' → Block 3' (fork, work = 4)

After:
  Main:  Genesis → Block 1 → Block 2 → Block 3 → Block 4
  Forks: [Block 2', Block 3']
```

### Implementation Details

**File**: `crates/kanari-core/src/blockchain/mod.rs`

**Key Methods**:

- `handle_fork(fork_blocks: Vec<Block>) -> Result<bool>`
- `get_canonical_chain() -> &[Block]`
- `get_forks() -> &[Vec<Block>]`
- `prune_forks(max_fork_depth: u64)`

**Tests**: `crates/kanari-core/tests/fork_resolution_test.rs`

---

## NAT Traversal

### ภาพรวม

NAT traversal ช่วยให้ nodes สามารถเชื่อมต่อกันได้แม้จะอยู่หลัง NAT/firewall โดยใช้ libp2p protocols:

- **Relay** - ใช้ relay node เป็นตัวกลาง
- **DCUtR** - Hole punching เพื่อสร้าง direct connection
- **AutoNAT** - ตรวจสอบ NAT status

### กลไกการทำงาน

#### 1. Relay Protocol

```
Node A (NAT) → Relay Node ← Node B (NAT)
     └──────── Relayed Connection ────────┘
```

- Node ที่อยู่หลัง NAT เชื่อมต่อกับ relay node
- Relay node ส่งต่อข้อมูลระหว่าง nodes
- ทำงานเหมือน VPN แต่สำหรับ P2P

#### 2. DCUtR (Direct Connection Upgrade through Relay)

```
Step 1: Relay connection
  Node A → Relay → Node B

Step 2: Coordinate hole punch
  Node A ← Relay → Node B
    ↓                ↓
  Open port    Open port

Step 3: Direct connection
  Node A ←──────────→ Node B
```

- ใช้ relay connection เพื่อ coordinate
- ทั้ง 2 nodes พยายามเชื่อมต่อกันโดยตรงพร้อมกัน
- สำเร็จ → ใช้ direct connection (เร็วกว่า, ไม่พึ่ง relay)

#### 3. AutoNAT

- ตรวจสอบว่า node มี public IP หรือไม่
- ช่วยให้ node รู้ว่าควรใช้ relay หรือไม่

### Configuration

**File**: `crates/kanari-node/Cargo.toml`

```toml
libp2p = { version = "0.56.0", features = [
    "tcp", "noise", "yamux", "gossipsub", "mdns", "kad",
    "identify", "macros", "tokio",
    "relay",    # Relay protocol
    "dcutr",    # Hole punching
    "autonat"   # NAT detection
] }
```

### P2P Behavior

**File**: `crates/kanari-node/src/p2p.rs`

```rust
#[derive(NetworkBehaviour)]
pub struct KanariBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub relay_client: relay::client::Behaviour,  // NEW
    pub dcutr: dcutr::Behaviour,                 // NEW
    pub autonat: autonat::Behaviour,             // NEW
}
```

### Event Handling

```rust
// Relay events
SwarmEvent::Behaviour(KanariBehaviourEvent::RelayClient(
    relay::client::Event::ReservationReqAccepted { relay_peer_id, .. }
)) => {
    info!("Relay reservation accepted by {}", relay_peer_id);
}

// DCUtR events
SwarmEvent::Behaviour(KanariBehaviourEvent::Dcutr(event)) => {
    info!("DCUtR event: {:?}", event);
    // Events: RemoteInitiatedDirectConnectionUpgrade,
    //         InitiatedDirectConnectionUpgrade,
    //         DirectConnectionUpgradeSucceeded,
    //         DirectConnectionUpgradeFailed
}

// AutoNAT events
SwarmEvent::Behaviour(KanariBehaviourEvent::Autonat(
    autonat::Event::StatusChanged { old, new }
)) => {
    info!("NAT status changed from {:?} to {:?}", old, new);
}
```

### การใช้งาน

#### แบบอัตโนมัติ (Default)

```bash
# Nodes จะใช้ NAT traversal อัตโนมัติ
kanari-node start --p2p-port 19000 --rpc-port 19001
```

- mDNS และ Kademlia จะค้นหา relay nodes
- AutoNAT จะตรวจสอบ NAT status
- DCUtR จะพยายาม hole punch อัตโนมัติ

#### Relay Node (Future feature)

```bash
# Start dedicated relay node
kanari-node start --p2p-port 19000 --relay-mode
```

**Note**: ปัจจุบันยังไม่มี `--relay-mode` flag แต่สามารถเพิ่มได้ใน future

### Logs ที่เกี่ยวข้อง

```
# NAT detection
INFO kanari_node: NAT status changed from Unknown to Public

# Relay connection
INFO kanari_node: Relay reservation accepted by 12D3KooW...

# Hole punching
INFO kanari_node: DCUtR event: RemoteInitiatedDirectConnectionUpgrade
INFO kanari_node: DCUtR event: DirectConnectionUpgradeSucceeded
```

### Troubleshooting

#### ปัญหา: ไม่สามารถเชื่อมต่อผ่าน NAT

**การแก้ไข**:

1. ตรวจสอบว่ามี relay nodes ในเครือข่ายหรือไม่
2. ตรวจสอบ firewall settings
3. ดู NAT status จาก AutoNAT logs

#### ปัญหา: Hole punching ล้มเหลว

**การแก้ไข**:

1. Symmetric NAT อาจไม่รองรับ hole punching
2. ใช้ relay connection แทน (อัตโนมัติ)
3. พิจารณาใช้ public relay nodes

### Implementation Files

- `crates/kanari-node/Cargo.toml` - Dependencies
- `crates/kanari-node/src/p2p.rs` - P2P network implementation
- `crates/kanari-node/MULTI_NODE_GUIDE.md` - Multi-node setup guide

### Benefits

1. **ความสามารถในการเข้าถึง**: Nodes หลัง NAT สามารถเข้าร่วมเครือข่ายได้
2. **ประสิทธิภาพ**: DCUtR ลด latency โดยสร้าง direct connections
3. **ความยืดหยุ่น**: Fallback ไปใช้ relay เมื่อ hole punch ไม่สำเร็จ
4. **Automatic**: ไม่ต้องตั้งค่าเพิ่มเติม

---

## สรุป

### Fork Resolution

- ✅ Longest chain rule
- ✅ Automatic reorganization
- ✅ Fork storage และ pruning
- ✅ Complete test coverage

### NAT Traversal

- ✅ Relay protocol support
- ✅ DCUtR hole punching
- ✅ AutoNAT detection
- ✅ Event handling และ logging

ทั้ง 2 features พร้อมใช้งานแล้ว! 🎉
