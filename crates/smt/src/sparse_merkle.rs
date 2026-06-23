// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::hash::{digest, hash_leaf, hash_node};
use crate::open_or_get_db;
use anyhow::Result;
use once_cell::sync::Lazy;
use rocksdb::IteratorMode;
use rocksdb::WriteBatch;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard};

/// Precomputed default hashes for SMT levels (256 levels + 1 leaf default)
static DEFAULT_HASHES: Lazy<Vec<[u8; 32]>> = Lazy::new(|| {
    let mut default_hashes = vec![[0u8; 32]; 257];
    default_hashes[256] = hash_leaf(&[0u8; 32], &[0u8; 32]);

    for d in (0..256).rev() {
        default_hashes[d] = hash_node(&default_hashes[d + 1], &default_hashes[d + 1]);
    }
    default_hashes
});

/// Sparse Merkle Tree implementation (256-bit keyspace using BLAKE3).
/// - Leaf hash: H(0x00 || key_hash || value)
/// - Node hash: H(0x01 || left || right)
///   Stores only non-default nodes in RocksDB under keys `smt:node:<depth>:<prefix_bytes>`.
#[derive(Debug)]
pub struct SparseMerkleTree {
    backend: SparseMerkleBackend,
    default_hashes: &'static [[u8; 32]],
}

#[derive(Debug, Clone)]
enum SparseMerkleBackend {
    RocksDb(Arc<rocksdb::DB>),
    Memory(Arc<RwLock<HashMap<Vec<u8>, Vec<u8>>>>),
}

const ROOT_NODE_KEY: [u8; 4] = [b'n', b':', 0, 0];
type NodeCacheKey = [u8; 36];
const MEMORY_SPECULATIVE_REBUILD_THRESHOLD: usize = 2_048;

fn fill_node_key_for_hash(out: &mut [u8; 36], depth: usize, key_hash: &[u8; 32]) -> usize {
    let prefix_bytes = depth.div_ceil(8);
    out[0] = b'n';
    out[1] = b':';
    out[2..4].copy_from_slice(&(depth as u16).to_be_bytes());
    out[4..4 + prefix_bytes].copy_from_slice(&key_hash[..prefix_bytes]);

    let excess = (prefix_bytes * 8) - depth;
    if excess > 0 {
        out[4 + prefix_bytes - 1] &= 0xFF_u8 << excess;
    }

    4 + prefix_bytes
}

fn node_cache_key(depth: usize, key_hash: &[u8; 32]) -> (NodeCacheKey, usize) {
    let mut key = [0u8; 36];
    let len = fill_node_key_for_hash(&mut key, depth, key_hash);
    (key, len)
}

fn flip_last_key_bit(node_key: &mut [u8], depth: usize) {
    let (byte_idx, bit_in_byte) = key_bit_position(depth);
    node_key[4 + byte_idx] ^= 1u8 << bit_in_byte;
}

fn key_bit_position(depth: usize) -> (usize, usize) {
    let bit_index = depth - 1;
    (bit_index / 8, 7 - (bit_index % 8))
}

fn bit_is_left_at_depth(key_hash: &[u8; 32], depth: usize) -> bool {
    let (byte_idx, bit_in_byte) = key_bit_position(depth);
    ((key_hash[byte_idx] >> bit_in_byte) & 1u8) == 0
}

fn mask_hash_to_depth(mut key_hash: [u8; 32], depth: usize) -> [u8; 32] {
    if depth == 0 {
        return [0u8; 32];
    }

    let full_bytes = depth / 8;
    let remaining_bits = depth % 8;
    let zero_from = if remaining_bits == 0 {
        full_bytes
    } else {
        full_bytes + 1
    };

    if remaining_bits != 0 {
        key_hash[full_bytes] &= 0xFF_u8 << (8 - remaining_bits);
    }
    for byte in key_hash.iter_mut().skip(zero_from) {
        *byte = 0;
    }

    key_hash
}

fn node_cache_capacity(item_count: usize) -> usize {
    item_count.saturating_mul(32).clamp(4_096, 262_144)
}

fn data_key(key_hash: &[u8; 32]) -> [u8; 34] {
    let mut out = [0u8; 34];
    out[0] = b'd';
    out[1] = b':';
    out[2..].copy_from_slice(key_hash);
    out
}

impl SparseMerkleTree {
    fn read_raw_from_memory_guard(
        guard: Option<&RwLockReadGuard<'_, HashMap<Vec<u8>, Vec<u8>>>>,
        key: &[u8],
    ) -> Option<Vec<u8>> {
        guard.and_then(|entries| entries.get(key).cloned())
    }

    pub fn new(db: Arc<rocksdb::DB>) -> Self {
        Self {
            backend: SparseMerkleBackend::RocksDb(db),
            default_hashes: &DEFAULT_HASHES,
        }
    }

    pub fn new_in_memory(store: Arc<RwLock<HashMap<Vec<u8>, Vec<u8>>>>) -> Self {
        Self {
            backend: SparseMerkleBackend::Memory(store),
            default_hashes: &DEFAULT_HASHES,
        }
    }

    pub fn open(path_opt: Option<PathBuf>) -> Result<Self> {
        let db = open_or_get_db(path_opt)?;
        Ok(Self::new(db))
    }

    /// Export all stored SMT key/value pairs (node entries and data entries)
    /// as a vector of raw bytes. This is used for creating lightweight
    /// snapshots that can later be used to serve historical proofs.
    pub fn export_snapshot(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut out: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        match &self.backend {
            SparseMerkleBackend::RocksDb(db) => {
                let iter = db.iterator(IteratorMode::Start);
                for item in iter {
                    let (k, v) = item?;
                    let key = k.to_vec();
                    if key.starts_with(b"n:") || key.starts_with(b"d:") {
                        out.push((key, v.to_vec()));
                    }
                }
            }
            SparseMerkleBackend::Memory(store) => {
                let guard = store
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                for (key, value) in guard.iter() {
                    if key.starts_with(b"n:") || key.starts_with(b"d:") {
                        out.push((key.clone(), value.clone()));
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn root_hash(&self) -> Result<[u8; 32]> {
        // root is stored at depth 0 with empty prefix
        if let Some(v) = self.read_raw(&ROOT_NODE_KEY)? {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&v);
            Ok(arr)
        } else {
            Ok(self.default_hashes[0])
        }
    }

    pub fn root_hash_with_changes(
        &self,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
    ) -> Result<[u8; 32]> {
        if updates.is_empty() && deletes.is_empty() {
            return self.root_hash();
        }

        if let SparseMerkleBackend::Memory(store) = &self.backend {
            let change_count = updates.len().saturating_add(deletes.len());
            if change_count > MEMORY_SPECULATIVE_REBUILD_THRESHOLD {
                let guard = store
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let mut existing = Vec::with_capacity(guard.len());
                for (key, value) in guard.iter() {
                    if key.len() != 34 || !key.starts_with(b"d:") {
                        continue;
                    }
                    let mut key_hash = [0u8; 32];
                    key_hash.copy_from_slice(&key[2..]);
                    existing.push((key_hash, value.as_slice()));
                }
                existing.sort_unstable_by_key(|(left, _)| *left);

                let mut hashed_updates = Vec::with_capacity(updates.len());
                for (index, (key, value)) in updates.iter().enumerate() {
                    hashed_updates.push((digest(key), index, value.as_slice()));
                }
                hashed_updates.sort_unstable_by(
                    |(left_hash, left_index, _), (right_hash, right_index, _)| {
                        left_hash.cmp(right_hash).then(left_index.cmp(right_index))
                    },
                );
                let mut deduped_updates = Vec::with_capacity(hashed_updates.len());
                for (key_hash, _, value) in hashed_updates {
                    if let Some((last_hash, last_value)) = deduped_updates.last_mut()
                        && *last_hash == key_hash
                    {
                        *last_value = value;
                    } else {
                        deduped_updates.push((key_hash, value));
                    }
                }

                let mut hashed_deletes = deletes.iter().map(|key| digest(key)).collect::<Vec<_>>();
                hashed_deletes.sort_unstable();
                hashed_deletes.dedup();

                let mut filtered_deletes =
                    Vec::with_capacity(hashed_deletes.len().saturating_sub(deduped_updates.len()));
                let mut delete_index = 0usize;
                let mut update_index = 0usize;
                while delete_index < hashed_deletes.len() {
                    while update_index < deduped_updates.len()
                        && deduped_updates[update_index].0 < hashed_deletes[delete_index]
                    {
                        update_index += 1;
                    }
                    if update_index < deduped_updates.len()
                        && deduped_updates[update_index].0 == hashed_deletes[delete_index]
                    {
                        delete_index += 1;
                        continue;
                    }
                    filtered_deletes.push(hashed_deletes[delete_index]);
                    delete_index += 1;
                }

                let mut merged =
                    Vec::with_capacity(existing.len().saturating_add(deduped_updates.len()));
                let mut existing_index = 0usize;
                let mut update_index = 0usize;
                let mut delete_index = 0usize;

                while existing_index < existing.len() || update_index < deduped_updates.len() {
                    let take_update = match (
                        existing.get(existing_index).map(|(key_hash, _)| key_hash),
                        deduped_updates
                            .get(update_index)
                            .map(|(key_hash, _)| key_hash),
                    ) {
                        (Some(existing_hash), Some(update_hash)) => update_hash <= existing_hash,
                        (None, Some(_)) => true,
                        _ => false,
                    };

                    let next_key = if take_update {
                        deduped_updates[update_index].0
                    } else {
                        existing[existing_index].0
                    };

                    while delete_index < filtered_deletes.len()
                        && filtered_deletes[delete_index] < next_key
                    {
                        delete_index += 1;
                    }
                    let is_deleted = delete_index < filtered_deletes.len()
                        && filtered_deletes[delete_index] == next_key;

                    if take_update {
                        if existing_index < existing.len() && existing[existing_index].0 == next_key
                        {
                            existing_index += 1;
                        }
                        if !is_deleted {
                            merged.push((next_key, deduped_updates[update_index].1));
                        }
                        update_index += 1;
                    } else {
                        if !is_deleted {
                            merged.push((next_key, existing[existing_index].1));
                        }
                        existing_index += 1;
                    }

                    if is_deleted {
                        delete_index += 1;
                    }
                }

                return Ok(compute_sparse_root_from_hashed_entry_refs(&merged));
            }
        }

        use std::collections::HashMap;

        let mut node_cache: HashMap<NodeCacheKey, [u8; 32]> = HashMap::with_capacity(
            node_cache_capacity(updates.len().saturating_add(deletes.len())),
        );
        let memory_guard = match &self.backend {
            SparseMerkleBackend::Memory(store) => Some(
                store
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ),
            SparseMerkleBackend::RocksDb(_) => None,
        };
        let mut root = self.root_hash()?;

        for key in deletes {
            let kh = digest(key.as_slice());
            let mut cur = self.default_hashes[256];

            for depth in (1..=256).rev() {
                let (byte_idx, bit_in_byte) = key_bit_position(depth);

                let (mut sibling_key, sibling_key_len) = node_cache_key(depth, &kh);
                flip_last_key_bit(&mut sibling_key, depth);

                let sibling_hash = if let Some(hash) = node_cache.get(&sibling_key) {
                    *hash
                } else if let Some(v) = Self::read_raw_from_memory_guard(
                    memory_guard.as_ref(),
                    &sibling_key[..sibling_key_len],
                ) {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&v);
                    arr
                } else if let Some(v) = self.read_raw(&sibling_key[..sibling_key_len])? {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&v);
                    arr
                } else {
                    self.default_hashes[depth]
                };

                let bit_is_left = ((kh[byte_idx] >> bit_in_byte) & 1u8) == 0;
                let (left, right) = if bit_is_left {
                    (cur, sibling_hash)
                } else {
                    (sibling_hash, cur)
                };
                let parent = hash_node(&left, &right);

                flip_last_key_bit(&mut sibling_key, depth);
                node_cache.insert(sibling_key, cur);
                cur = parent;
            }

            root = cur;
        }

        for (key, value) in updates {
            let kh = digest(key.as_slice());
            let mut cur = hash_leaf(&kh, value);

            for depth in (1..=256).rev() {
                let (byte_idx, bit_in_byte) = key_bit_position(depth);

                let (mut sibling_key, sibling_key_len) = node_cache_key(depth, &kh);
                flip_last_key_bit(&mut sibling_key, depth);

                let sibling_hash = if let Some(hash) = node_cache.get(&sibling_key) {
                    *hash
                } else if let Some(v) = Self::read_raw_from_memory_guard(
                    memory_guard.as_ref(),
                    &sibling_key[..sibling_key_len],
                ) {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&v);
                    arr
                } else if let Some(v) = self.read_raw(&sibling_key[..sibling_key_len])? {
                    let mut arr = [0u8; 32];
                    arr.copy_from_slice(&v);
                    arr
                } else {
                    self.default_hashes[depth]
                };

                let bit_is_left = ((kh[byte_idx] >> bit_in_byte) & 1u8) == 0;
                let (left, right) = if bit_is_left {
                    (cur, sibling_hash)
                } else {
                    (sibling_hash, cur)
                };
                let parent = hash_node(&left, &right);

                flip_last_key_bit(&mut sibling_key, depth);
                node_cache.insert(sibling_key, cur);
                cur = parent;
            }

            root = cur;
        }

        Ok(root)
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let kh = digest(key);
        let data_key = data_key(&kh);
        self.read_raw(&data_key)
    }

    /// Produce a proof for `key`. Returns (is_member, leaf_hash, siblings bottom-up)
    pub fn proof(&self, key: &[u8]) -> Result<(bool, [u8; 32], Vec<[u8; 32]>)> {
        let kh = digest(key);
        let data_key = data_key(&kh);

        // check membership
        let value = self.read_raw(&data_key)?;
        let is_member = value.is_some();

        // leaf hash
        let leaf_hash = if let Some(val) = value {
            hash_leaf(&kh, &val)
        } else {
            self.default_hashes[256]
        };

        let mut siblings: Vec<[u8; 32]> = Vec::with_capacity(256);
        let mut sibling_key = [0u8; 36];

        // traverse from leaf depth down to 1 and collect sibling at each level
        for depth in (1..=256).rev() {
            let sibling_key_len = fill_node_key_for_hash(&mut sibling_key, depth, &kh);
            flip_last_key_bit(&mut sibling_key[..sibling_key_len], depth);
            if let Some(v) = self.read_raw(&sibling_key[..sibling_key_len])? {
                let mut a = [0u8; 32];
                a.copy_from_slice(&v);
                siblings.push(a);
            } else {
                siblings.push(self.default_hashes[depth]);
            }
        }

        Ok((is_member, leaf_hash, siblings))
    }

    /// Insert a batch of key/value pairs. Simple implementation that calls
    /// `insert` per entry. This can be optimized later to build a single
    /// write batch updating multiple leaves/parents.
    pub fn insert(&self, kvs: &[(Vec<u8>, Vec<u8>)]) -> Result<()> {
        // Apply the whole batch together so sibling reads can observe writes
        // from earlier leaves without paying repeated backend round-trips.
        let mut node_cache: HashMap<NodeCacheKey, [u8; 32]> =
            HashMap::with_capacity(node_cache_capacity(kvs.len()));
        let memory_guard = match &self.backend {
            SparseMerkleBackend::Memory(store) => Some(
                store
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ),
            SparseMerkleBackend::RocksDb(_) => None,
        };
        let mut puts = Vec::new();
        let mut deletes = Vec::new();

        for (k, v) in kvs.iter() {
            let key = k.as_slice();
            let value = v.as_slice();
            let kh = digest(key);
            let leaf_hash = hash_leaf(&kh, value);

            let data_key = data_key(&kh);
            puts.push((data_key.to_vec(), value.to_vec()));

            let mut cur = leaf_hash;

            for depth in (1..=256).rev() {
                let (byte_idx, bit_in_byte) = key_bit_position(depth);

                let (mut node_k, node_k_len) = node_cache_key(depth, &kh);
                flip_last_key_bit(&mut node_k, depth);

                let sibling_hash = if let Some(h) = node_cache.get(&node_k) {
                    *h
                } else if let Some(vb) =
                    Self::read_raw_from_memory_guard(memory_guard.as_ref(), &node_k[..node_k_len])
                {
                    let mut a = [0u8; 32];
                    a.copy_from_slice(&vb);
                    a
                } else if let Some(vb) = self.read_raw(&node_k[..node_k_len])? {
                    let mut a = [0u8; 32];
                    a.copy_from_slice(&vb);
                    a
                } else {
                    self.default_hashes[depth]
                };

                let bit = ((kh[byte_idx] >> bit_in_byte) & 1u8) == 0;
                let (left, right) = if bit {
                    (cur, sibling_hash)
                } else {
                    (sibling_hash, cur)
                };

                let parent_arr = hash_node(&left, &right);

                flip_last_key_bit(&mut node_k, depth);
                if cur == self.default_hashes[depth] {
                    deletes.push(node_k[..node_k_len].to_vec());
                } else {
                    puts.push((node_k[..node_k_len].to_vec(), cur.to_vec()));
                }
                node_cache.insert(node_k, cur);

                cur = parent_arr;
            }

            puts.push((ROOT_NODE_KEY.to_vec(), cur.to_vec()));
        }

        drop(memory_guard);
        self.apply_raw_batch(&puts, &deletes)
    }

    /// Delete a single key from the tree, updating parent nodes. This will
    /// remove the stored data entry and propagate default hashes upward,
    /// deleting node records when they equal the default value.
    /// Delete a batch of keys
    pub fn delete(&self, keys: &[Vec<u8>]) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }

        let mut keyed: Vec<[u8; 32]> = Vec::with_capacity(keys.len());
        for k in keys.iter() {
            keyed.push(digest(k.as_slice()));
        }

        keyed.sort();
        keyed.dedup();

        let mut node_cache: HashMap<NodeCacheKey, [u8; 32]> =
            HashMap::with_capacity(node_cache_capacity(keyed.len()));
        let memory_guard = match &self.backend {
            SparseMerkleBackend::Memory(store) => Some(
                store
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            ),
            SparseMerkleBackend::RocksDb(_) => None,
        };
        let mut puts = Vec::new();
        let mut deletes = Vec::new();

        for kh in keyed.into_iter() {
            let data_key = data_key(&kh);
            deletes.push(data_key.to_vec());

            let mut cur = self.default_hashes[256];

            for depth in (1..=256).rev() {
                let (byte_idx, bit_in_byte) = key_bit_position(depth);

                let (mut node_k, node_k_len) = node_cache_key(depth, &kh);
                flip_last_key_bit(&mut node_k, depth);

                let sibling_hash = if let Some(h) = node_cache.get(&node_k) {
                    *h
                } else if let Some(v) =
                    Self::read_raw_from_memory_guard(memory_guard.as_ref(), &node_k[..node_k_len])
                {
                    let mut a = [0u8; 32];
                    a.copy_from_slice(&v);
                    a
                } else if let Some(v) = self.read_raw(&node_k[..node_k_len])? {
                    let mut a = [0u8; 32];
                    a.copy_from_slice(&v);
                    a
                } else {
                    self.default_hashes[depth]
                };

                let bit = ((kh[byte_idx] >> bit_in_byte) & 1u8) == 0;
                let (left, right) = if bit {
                    (cur, sibling_hash)
                } else {
                    (sibling_hash, cur)
                };

                let parent_arr = hash_node(&left, &right);

                flip_last_key_bit(&mut node_k, depth);
                if cur == self.default_hashes[depth] {
                    deletes.push(node_k[..node_k_len].to_vec());
                    node_cache.insert(node_k, self.default_hashes[depth]);
                } else {
                    puts.push((node_k[..node_k_len].to_vec(), cur.to_vec()));
                    node_cache.insert(node_k, cur);
                }

                cur = parent_arr;
            }

            if cur == self.default_hashes[0] {
                deletes.push(ROOT_NODE_KEY.to_vec());
            } else {
                puts.push((ROOT_NODE_KEY.to_vec(), cur.to_vec()));
            }
        }

        drop(memory_guard);
        self.apply_raw_batch(&puts, &deletes)
    }

    fn read_raw(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        match &self.backend {
            SparseMerkleBackend::RocksDb(db) => Ok(db.get(key)?),
            SparseMerkleBackend::Memory(store) => Ok(store
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(key)
                .cloned()),
        }
    }

    fn apply_raw_batch(&self, puts: &[(Vec<u8>, Vec<u8>)], deletes: &[Vec<u8>]) -> Result<()> {
        match &self.backend {
            SparseMerkleBackend::RocksDb(db) => {
                let mut batch = WriteBatch::default();
                for (key, value) in puts {
                    batch.put(key, value);
                }
                for key in deletes {
                    batch.delete(key);
                }
                db.write(batch)?;
            }
            SparseMerkleBackend::Memory(store) => {
                let mut guard = store
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                for (key, value) in puts {
                    guard.insert(key.clone(), value.clone());
                }
                for key in deletes {
                    guard.remove(key);
                }
            }
        }
        Ok(())
    }
}

/// Produce the canonical default hash vector used by the SMT.
pub fn default_hashes() -> &'static [[u8; 32]] {
    &DEFAULT_HASHES
}

pub fn compute_sparse_root(entries: &[(Vec<u8>, Vec<u8>)]) -> [u8; 32] {
    if entries.is_empty() {
        return DEFAULT_HASHES[0];
    }

    let mut current = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let key_hash = digest(key);
        current.push((key_hash, hash_leaf(&key_hash, value)));
    }

    compute_sparse_root_from_leaf_hashes(current)
}

fn compute_sparse_root_from_hashed_entry_refs(entries: &[([u8; 32], &[u8])]) -> [u8; 32] {
    if entries.is_empty() {
        return DEFAULT_HASHES[0];
    }

    let mut current = Vec::with_capacity(entries.len());
    for (key_hash, value) in entries {
        current.push((*key_hash, hash_leaf(key_hash, value)));
    }

    compute_sparse_root_from_leaf_hashes(current)
}

fn compute_sparse_root_from_leaf_hashes(mut current: Vec<([u8; 32], [u8; 32])>) -> [u8; 32] {
    if current.is_empty() {
        return DEFAULT_HASHES[0];
    }

    current.sort_unstable_by_key(|(left, _)| *left);

    for depth in (1..=256).rev() {
        let mut next = Vec::with_capacity(current.len().saturating_div(2).saturating_add(1));
        let mut index = 0usize;

        while index < current.len() {
            let (node_key, node_hash) = current[index];
            let parent_key = mask_hash_to_depth(node_key, depth - 1);
            let default_sibling = DEFAULT_HASHES[depth];
            let mut consumed_pair = false;

            let (left, right) = if index + 1 < current.len() {
                let (sibling_key, sibling_hash) = current[index + 1];
                if mask_hash_to_depth(sibling_key, depth - 1) == parent_key
                    && bit_is_left_at_depth(&node_key, depth)
                        != bit_is_left_at_depth(&sibling_key, depth)
                {
                    consumed_pair = true;
                    if bit_is_left_at_depth(&node_key, depth) {
                        (node_hash, sibling_hash)
                    } else {
                        (sibling_hash, node_hash)
                    }
                } else if bit_is_left_at_depth(&node_key, depth) {
                    (node_hash, default_sibling)
                } else {
                    (default_sibling, node_hash)
                }
            } else if bit_is_left_at_depth(&node_key, depth) {
                (node_hash, default_sibling)
            } else {
                (default_sibling, node_hash)
            };

            let parent_hash = hash_node(&left, &right);
            next.push((parent_key, parent_hash));
            index += if consumed_pair { 2 } else { 1 };
        }

        current = next;
    }

    current
        .first()
        .map(|(_, hash)| *hash)
        .unwrap_or(DEFAULT_HASHES[0])
}

/// Verify a proof (membership or non-membership) against a given root.
/// `proof` is the tuple returned by `proof()`: `(is_member, leaf_hash, siblings)`.
pub fn verify_proof(root: &[u8; 32], key: &[u8], proof: (bool, [u8; 32], Vec<[u8; 32]>)) -> bool {
    let (_is_member, leaf, siblings) = proof;
    let kh = digest(key);

    let mut cur = leaf;
    for (i, sibling) in siblings.into_iter().enumerate() {
        // siblings vector is bottom-up from leaf (depth=256) upwards
        let depth = 256 - i;
        let (byte_idx, bit_in_byte) = key_bit_position(depth);
        let bit = ((kh[byte_idx] >> bit_in_byte) & 1u8) == 0;

        let (left, right) = if bit { (cur, sibling) } else { (sibling, cur) };
        let p_arr = hash_node(&left, &right);
        cur = p_arr;
    }

    // After folding up, `cur` should equal the root. For non-membership proofs
    // `is_member` should be false and the leaf used is the default leaf.
    &cur == root
}

#[cfg(test)]
#[path = "../tests/unit/sparse_merkle_tests.rs"]
mod tests;
