# SMT (Sparse Merkle Tree) Crate

## Overview

This crate provides a **Sparse Merkle Tree** implementation optimized for blockchain state storage. It's used by Kanari blockchain for account state verification and cryptographic proofs.

## Features

- **256-bit keyspace**: Full binary tree with 2^256 possible keys
- **Sparse storage**: Only stores non-default nodes in RocksDB
- **Blake3 hashing**: Fast, secure cryptographic hash function
- **Membership proofs**: Prove a key-value pair exists in the tree
- **Non-membership proofs**: Prove a key does NOT exist (returns default)
- **Persistent**: All state persisted in RocksDB
- **Snapshot support**: Export/import full tree state for historical proofs

## Usage

### Basic Operations

```rust
use smt::SparseMerkleTree;

// Open SMT (or create new one)
let smt = SparseMerkleTree::open(Some("./data".into()))?;

// Insert key-value pairs
let mut batch = vec![
    (b"alice".to_vec(), b"100".to_vec()),
    (b"bob".to_vec(), b"200".to_vec()),
];
smt.batch_write(batch)?;

// Get root hash
let root = smt.root_hash()?;
println!("Root: {}", hex::encode(root));

// Get value
let value = smt.get(b"alice")?;
assert_eq!(value, Some(b"100".to_vec()));

// Generate proof
let (is_member, leaf, siblings) = smt.proof(b"alice")?;
assert!(is_member);
```

### Batch Updates

For efficient state updates:

```rust
// Batch write multiple key-value pairs
let updates = vec![
    (b"key1".to_vec(), b"value1".to_vec()),
    (b"key2".to_vec(), b"value2".to_vec()),
    (b"key3".to_vec(), b"value3".to_vec()),
];

smt.batch_write(updates)?;
```

### Snapshots

For historical state proofs:

```rust
// Export snapshot at current height
let snapshot = smt.export_snapshot()?;

// Save to database
store.save_snapshot(block_height, snapshot)?;

// Later: load snapshot to verify historical state
let historical_snapshot = store.load_snapshot(old_height)?;
```

## Hash Functions

All hashing uses Blake3:

```rust
use smt::hash::{digest, hash_leaf, hash_node};

// Hash arbitrary data
let hash = digest(b"hello world");

// Hash a leaf node: H(0x00 || key_hash || value)
let leaf_hash = hash_leaf(&key_hash, &value);

// Hash internal node: H(0x01 || left || right)
let node_hash = hash_node(&left_child, &right_child);
```

## Architecture

```md
Root (depth 0)
├─ Node (depth 1) - left subtree
│  ├─ Node (depth 2)
│  │  ├─ ...
│  │  └─ Leaf (depth 256) - actual data
│  └─ Node (depth 2)
└─ Node (depth 1) - right subtree
   └─ ...
```

### Storage Keys

- **Node key**: `smt:node:<depth>:<prefix_bytes>`
- **Data key**: `smt:data:<key_hash>`

### Default Hashes

Empty subtrees use precomputed default hashes to avoid storing them:

```rust
// Leaf default: H(0x00 || 32_zeros || 32_zeros)
default[256] = hash_leaf(&[0; 32], &[0; 32])

// Internal node defaults (bottom-up)
for depth in (0..256).rev() {
    default[depth] = hash_node(&default[depth+1], &default[depth+1])
}
```

## Comparison with Transaction Merkle Tree

| Feature | SMT | Transaction Merkle Tree |
| --------- | ----- | ------------------------- |
| **Purpose** | State storage | Transaction verification |
| **Keyspace** | 256-bit (sparse) | Transaction count (dense) |
| **Storage** | RocksDB (persistent) | In-memory (ephemeral) |
| **Proof Type** | Membership + Non-membership | Inclusion only |
| **Update** | Per transaction | Per block |
| **Use Case** | Account state, balances | Light client sync |
| **Location** | `crates/smt/` | `crates/kanari-core/src/blockchain/merkle.rs` |

## Performance

- **Insert**: O(256) = O(1) constant time (fixed depth)
- **Lookup**: O(256) = O(1) constant time
- **Proof Generation**: O(256) = O(1) constant size
- **Batch Write**: Optimized with RocksDB WriteBatch

Blake3 is extremely fast (~10 GB/s), making hash operations negligible.

## Security

- **Collision Resistance**: Blake3 provides 128-bit security
- **Second Preimage Resistance**: Cannot create different tree with same root
- **Deterministic**: Same key-value pairs always produce same root
- **Verifiable**: Proofs can be verified without full tree

## Testing

Run tests:

```bash
cargo test --package smt
```

## Integration

Used by:

- `kanari-move-runtime` - State management and verification
- `kanari-core` - Blockchain state root computation
- `kanari-rpc-server` - State proof generation for light clients

## References

- [Jellyfish Merkle Tree (Diem)](https://developers.diem.com/papers/jellyfish-merkle-tree/2021-01-14.pdf)
- [Ethereum State Trie](https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/)
- [Blake3 Specification](https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf)

## License

Apache-2.0
