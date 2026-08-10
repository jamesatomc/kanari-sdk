// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::hash::{digest, hash_leaf, hash_node};
use crate::open_or_get_db;
use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use rayon::prelude::*;
use rocksdb::IteratorMode;
use rocksdb::WriteBatch;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub type SmtNodeOverlay = Vec<(Vec<u8>, Option<[u8; 32]>)>;

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
///   Stores only non-default branch nodes in RocksDB under keys `n:<depth>:<prefix>`.
#[derive(Debug)]
pub struct SparseMerkleTree {
    db: Arc<rocksdb::DB>,
    default_hashes: &'static [[u8; 32]],
    node_cache: Mutex<HashMap<Vec<u8>, [u8; 32]>>,
    node_cache_ready: AtomicBool,
}

const ROOT_NODE_KEY: [u8; 4] = [b'n', b':', 0, 0];
const NODE_INDEX_VERSION_KEY: &[u8] = b"meta:node_index_version";

fn key_bit_position(depth: usize) -> (usize, usize) {
    let bit_index = depth - 1;
    (bit_index / 8, 7 - (bit_index % 8))
}

fn top_bits(key_hash: &[u8; 32], depth: usize) -> usize {
    debug_assert!(depth <= 16);
    let mut value = usize::from(key_hash[0]);
    if depth > 8 {
        value = (value << 8) | usize::from(key_hash[1]);
    }
    value >> (16 - depth)
}

fn prefix_from_top_bits(bucket_index: usize, depth: usize) -> [u8; 32] {
    debug_assert!(depth <= 16);
    let value = bucket_index << (16 - depth);
    let mut prefix = [0u8; 32];
    prefix[0] = (value >> 8) as u8;
    if depth > 8 {
        prefix[1] = value as u8;
    }
    prefix
}

fn data_key(key_hash: &[u8; 32]) -> [u8; 34] {
    let mut out = [0u8; 34];
    out[0] = b'd';
    out[1] = b':';
    out[2..].copy_from_slice(key_hash);
    out
}

fn node_prefix_len(depth: usize) -> usize {
    depth.div_ceil(8)
}

fn node_key(prefix_hash: &[u8; 32], depth: usize) -> Vec<u8> {
    debug_assert!(depth <= 256);
    if depth == 0 {
        return ROOT_NODE_KEY.to_vec();
    }

    let prefix_len = node_prefix_len(depth);
    let mut out = Vec::with_capacity(4 + prefix_len);
    out.extend_from_slice(b"n:");
    out.extend_from_slice(&(depth as u16).to_be_bytes());
    out.extend_from_slice(&prefix_hash[..prefix_len]);

    let remaining_bits = depth % 8;
    if remaining_bits != 0 {
        let mask = 0xFF_u8 << (8 - remaining_bits);
        let last = out.last_mut().expect("prefix length is non-zero");
        *last &= mask;
    }

    out
}

fn hash_from_slice(bytes: &[u8]) -> Result<[u8; 32]> {
    anyhow::ensure!(
        bytes.len() == 32,
        "invalid SMT node hash length: {}",
        bytes.len()
    );
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    Ok(out)
}

impl SparseMerkleTree {
    pub fn new(db: Arc<rocksdb::DB>) -> Self {
        Self {
            db,
            default_hashes: &DEFAULT_HASHES,
            node_cache: Mutex::new(HashMap::new()),
            node_cache_ready: AtomicBool::new(false),
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
        let iter = self.db.iterator(IteratorMode::Start);
        for item in iter {
            let (k, v) = item?;
            let key = k.to_vec();
            if key.starts_with(b"n:") || key.starts_with(b"d:") {
                out.push((key, v.to_vec()));
            }
        }
        Ok(out)
    }

    pub fn persisted_leaf_count(&self) -> Result<usize> {
        let mut count = 0;
        for item in self.db.iterator(IteratorMode::Start) {
            let (key, _) = item?;
            if key.starts_with(b"d:") {
                count += 1;
            }
        }
        Ok(count)
    }

    fn node_hash_from_cache(
        &self,
        cache: &HashMap<Vec<u8>, [u8; 32]>,
        prefix_hash: &[u8; 32],
        depth: usize,
    ) -> [u8; 32] {
        cache
            .get(&node_key(prefix_hash, depth))
            .copied()
            .unwrap_or(self.default_hashes[depth])
    }

    fn node_hash_from_overlay(
        &self,
        cache: &HashMap<Vec<u8>, [u8; 32]>,
        overlay: &HashMap<Vec<u8>, Option<[u8; 32]>>,
        prefix_hash: &[u8; 32],
        depth: usize,
    ) -> [u8; 32] {
        let key = node_key(prefix_hash, depth);
        if let Some(value) = overlay.get(&key) {
            return value.unwrap_or(self.default_hashes[depth]);
        }
        self.node_hash_from_cache(cache, prefix_hash, depth)
    }

    fn stage_node(
        &self,
        overlay: &mut HashMap<Vec<u8>, Option<[u8; 32]>>,
        prefix_hash: &[u8; 32],
        depth: usize,
        hash: [u8; 32],
    ) {
        let key = node_key(prefix_hash, depth);
        if hash == self.default_hashes[depth] {
            overlay.insert(key, None);
        } else {
            overlay.insert(key, Some(hash));
        }
    }

    fn stage_leaf_node_update(
        &self,
        cache: &HashMap<Vec<u8>, [u8; 32]>,
        overlay: &mut HashMap<Vec<u8>, Option<[u8; 32]>>,
        key_hash: [u8; 32],
        leaf_hash: [u8; 32],
    ) {
        self.stage_node_update_from_depth(cache, overlay, key_hash, 256, leaf_hash);
    }

    fn stage_node_update_from_depth(
        &self,
        cache: &HashMap<Vec<u8>, [u8; 32]>,
        overlay: &mut HashMap<Vec<u8>, Option<[u8; 32]>>,
        key_hash: [u8; 32],
        node_depth: usize,
        node_hash: [u8; 32],
    ) {
        debug_assert!(node_depth <= 256);
        if node_depth == 0 {
            self.stage_node(overlay, &key_hash, 0, node_hash);
            return;
        }

        let mut current = node_hash;
        for depth in (1..=node_depth).rev() {
            let mut sibling_prefix = key_hash;
            let (byte_idx, bit_in_byte) = key_bit_position(depth);
            let bit_mask = 1u8 << bit_in_byte;
            sibling_prefix[byte_idx] ^= bit_mask;
            let sibling = self.node_hash_from_overlay(cache, overlay, &sibling_prefix, depth);
            let current_is_left = (key_hash[byte_idx] & bit_mask) == 0;
            let parent = if current_is_left {
                hash_node(&current, &sibling)
            } else {
                hash_node(&sibling, &current)
            };
            self.stage_node(overlay, &key_hash, depth - 1, parent);
            current = parent;
        }
    }

    fn stage_leaf_node_update_to_depth(
        &self,
        cache: &HashMap<Vec<u8>, [u8; 32]>,
        overlay: &mut HashMap<Vec<u8>, Option<[u8; 32]>>,
        key_hash: [u8; 32],
        leaf_hash: [u8; 32],
        stop_depth: usize,
    ) -> [u8; 32] {
        debug_assert!(stop_depth <= 256);
        if stop_depth == 256 {
            self.stage_node(overlay, &key_hash, 256, leaf_hash);
            return leaf_hash;
        }

        let mut current = leaf_hash;
        for depth in ((stop_depth + 1)..=256).rev() {
            let mut sibling_prefix = key_hash;
            let (byte_idx, bit_in_byte) = key_bit_position(depth);
            let bit_mask = 1u8 << bit_in_byte;
            sibling_prefix[byte_idx] ^= bit_mask;
            let sibling = self.node_hash_from_overlay(cache, overlay, &sibling_prefix, depth);
            let current_is_left = (key_hash[byte_idx] & bit_mask) == 0;
            let parent = if current_is_left {
                hash_node(&current, &sibling)
            } else {
                hash_node(&sibling, &current)
            };
            self.stage_node(overlay, &key_hash, depth - 1, parent);
            current = parent;
        }
        current
    }

    fn apply_node_overlay_to_cache(
        &self,
        cache: &mut HashMap<Vec<u8>, [u8; 32]>,
        overlay: HashMap<Vec<u8>, Option<[u8; 32]>>,
    ) {
        Self::apply_node_overlay_entries_to_cache(cache, overlay);
    }

    fn apply_node_overlay_entries_to_cache<I>(cache: &mut HashMap<Vec<u8>, [u8; 32]>, overlay: I)
    where
        I: IntoIterator<Item = (Vec<u8>, Option<[u8; 32]>)>,
    {
        for (key, value) in overlay {
            match value {
                Some(hash) => {
                    cache.insert(key, hash);
                }
                None => {
                    cache.remove(&key);
                }
            }
        }
    }

    fn rebuild_node_cache_from_persisted_data(&self) -> Result<()> {
        let mut cache = HashMap::new();
        let base = HashMap::new();
        let mut overlay = HashMap::new();
        for (key_hash, value) in self.persisted_data_entries()? {
            self.stage_leaf_node_update(
                &base,
                &mut overlay,
                key_hash,
                hash_leaf(&key_hash, &value),
            );
        }
        self.apply_node_overlay_to_cache(&mut cache, overlay);
        *self
            .node_cache
            .lock()
            .expect("SMT node cache mutex poisoned") = cache;
        self.node_cache_ready.store(true, Ordering::Release);
        self.purge_legacy_persisted_node_index()?;
        Ok(())
    }

    fn purge_legacy_persisted_node_index(&self) -> Result<()> {
        if self.db.get(NODE_INDEX_VERSION_KEY)?.is_none() {
            return Ok(());
        }

        let mut batch = WriteBatch::default();
        for item in self.db.iterator(IteratorMode::Start) {
            let (key, _) = item?;
            if key.starts_with(b"n:") && key.as_ref() != ROOT_NODE_KEY {
                batch.delete(key);
            }
        }
        batch.delete(NODE_INDEX_VERSION_KEY);
        self.db.write(batch)?;
        Ok(())
    }

    fn ensure_node_index(&self) -> Result<()> {
        if self.node_cache_ready.load(Ordering::Acquire) {
            return Ok(());
        }
        self.rebuild_node_cache_from_persisted_data()
            .context("Failed to rebuild SMT node cache")
    }

    pub fn root_hash(&self) -> Result<[u8; 32]> {
        if self.node_cache_ready.load(Ordering::Acquire) {
            let cache = self
                .node_cache
                .lock()
                .expect("SMT node cache mutex poisoned");
            return Ok(self.node_hash_from_cache(&cache, &[0u8; 32], 0));
        }
        // root is stored at depth 0 with empty prefix
        if let Ok(Some(v)) = self.db.get(ROOT_NODE_KEY) {
            hash_from_slice(&v)
        } else {
            Ok(self.default_hashes[0])
        }
    }

    fn persisted_data_entries(&self) -> Result<BTreeMap<[u8; 32], Vec<u8>>> {
        let mut entries = BTreeMap::new();
        for item in self.db.iterator(IteratorMode::Start) {
            let (key, value) = item?;
            let Some(hash_bytes) = key.strip_prefix(b"d:") else {
                continue;
            };
            if hash_bytes.len() != 32 {
                continue;
            }
            let mut key_hash = [0u8; 32];
            key_hash.copy_from_slice(hash_bytes);
            entries.insert(key_hash, value.to_vec());
        }
        Ok(entries)
    }

    /// Replace the persisted tree with a tree built from the supplied canonical
    /// entries. This is intended for startup repair and schema migrations, not
    /// the transaction hot path.
    pub fn rebuild(&self, entries: &[(Vec<u8>, Vec<u8>)]) -> Result<()> {
        let mut batch = WriteBatch::default();
        for item in self.db.iterator(IteratorMode::Start) {
            let (key, _) = item?;
            if key.starts_with(b"n:") || key.starts_with(b"d:") {
                batch.delete(key);
            }
        }
        batch.delete(NODE_INDEX_VERSION_KEY);
        self.db.write(batch)?;
        self.node_cache
            .lock()
            .expect("SMT node cache mutex poisoned")
            .clear();
        self.node_cache_ready.store(false, Ordering::Release);
        if !entries.is_empty() {
            self.insert(entries)?;
        } else {
            let mut batch = WriteBatch::default();
            batch.put(ROOT_NODE_KEY, self.default_hashes[0]);
            self.db.write(batch)?;
            self.node_cache_ready.store(true, Ordering::Release);
        }
        Ok(())
    }

    pub fn root_hash_with_changes(
        &self,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
    ) -> Result<[u8; 32]> {
        self.root_hash_with_changes_and_overlay(updates, deletes)
            .map(|(root, _)| root)
    }

    pub fn root_hash_with_changes_and_overlay(
        &self,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
    ) -> Result<([u8; 32], SmtNodeOverlay)> {
        if updates.is_empty() && deletes.is_empty() {
            return Ok((self.root_hash()?, Vec::new()));
        }

        self.ensure_node_index()?;
        let cache = self
            .node_cache
            .lock()
            .expect("SMT node cache mutex poisoned");
        let mut changes = BTreeMap::new();
        for key in deletes {
            changes.insert(digest(key), None);
        }
        for (key, value) in updates {
            changes.insert(digest(key), Some(value));
        }

        let mut overlay = HashMap::new();
        if changes.len() >= 1024 {
            return self.root_hash_with_sorted_changes_parallel(&cache, changes);
        }
        for (key_hash, value) in changes {
            let leaf = value
                .map(|value| hash_leaf(&key_hash, value))
                .unwrap_or(self.default_hashes[256]);
            self.stage_leaf_node_update(&cache, &mut overlay, key_hash, leaf);
        }
        let root = self.node_hash_from_overlay(&cache, &overlay, &[0u8; 32], 0);
        Ok((root, overlay.into_iter().collect()))
    }

    fn root_hash_with_sorted_changes_parallel(
        &self,
        cache: &HashMap<Vec<u8>, [u8; 32]>,
        changes: BTreeMap<[u8; 32], Option<&Vec<u8>>>,
    ) -> Result<([u8; 32], SmtNodeOverlay)> {
        const PARALLEL_BUCKET_DEPTH: usize = 12;
        let mut buckets = (0..(1usize << PARALLEL_BUCKET_DEPTH))
            .map(|_| Vec::new())
            .collect::<Vec<_>>();
        for (key_hash, value) in changes {
            buckets[top_bits(&key_hash, PARALLEL_BUCKET_DEPTH)].push((key_hash, value));
        }

        let bucket_results = buckets
            .par_iter()
            .enumerate()
            .filter(|(_, bucket)| !bucket.is_empty())
            .map(|(bucket_index, bucket)| {
                let mut overlay = HashMap::new();
                let mut bucket_root = self.default_hashes[PARALLEL_BUCKET_DEPTH];
                let bucket_prefix = prefix_from_top_bits(bucket_index, PARALLEL_BUCKET_DEPTH);
                for (key_hash, value) in bucket {
                    let leaf = value
                        .map(|value| hash_leaf(key_hash, value))
                        .unwrap_or(self.default_hashes[256]);
                    bucket_root = self.stage_leaf_node_update_to_depth(
                        cache,
                        &mut overlay,
                        *key_hash,
                        leaf,
                        PARALLEL_BUCKET_DEPTH,
                    );
                }
                self.stage_node(
                    &mut overlay,
                    &bucket_prefix,
                    PARALLEL_BUCKET_DEPTH,
                    bucket_root,
                );
                (bucket_prefix, bucket_root, overlay)
            })
            .collect::<Vec<_>>();

        let mut overlay = HashMap::new();
        for (bucket_prefix, bucket_root, bucket_overlay) in bucket_results {
            overlay.extend(bucket_overlay);
            self.stage_node_update_from_depth(
                cache,
                &mut overlay,
                bucket_prefix,
                PARALLEL_BUCKET_DEPTH,
                bucket_root,
            );
        }

        let root = self.node_hash_from_overlay(cache, &overlay, &[0u8; 32], 0);
        Ok((root, overlay.into_iter().collect()))
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let kh = digest(key);
        let data_key = data_key(&kh);
        if let Some(v) = self.db.get(data_key)? {
            Ok(Some(v.to_vec()))
        } else {
            Ok(None)
        }
    }

    /// Produce a proof for `key`. Returns (is_member, leaf_hash, siblings bottom-up)
    pub fn proof(&self, key: &[u8]) -> Result<(bool, [u8; 32], Vec<[u8; 32]>)> {
        let kh = digest(key);
        let data_key = data_key(&kh);

        // check membership
        let value = self.db.get(data_key)?;
        let is_member = value.is_some();

        // leaf hash
        let leaf_hash = if let Some(val) = value {
            hash_leaf(&kh, &val)
        } else {
            self.default_hashes[256]
        };

        self.ensure_node_index()?;
        let cache = self
            .node_cache
            .lock()
            .expect("SMT node cache mutex poisoned");
        let mut siblings: Vec<[u8; 32]> = Vec::with_capacity(256);

        // traverse from leaf depth down to 1 and collect sibling at each level
        for depth in (1..=256).rev() {
            let mut sibling_prefix = kh;
            let (byte_idx, bit_in_byte) = key_bit_position(depth);
            sibling_prefix[byte_idx] ^= 1u8 << bit_in_byte;
            siblings.push(self.node_hash_from_cache(&cache, &sibling_prefix, depth));
        }

        Ok((is_member, leaf_hash, siblings))
    }

    /// Insert a batch of key/value pairs. Simple implementation that calls
    /// `insert` per entry. This can be optimized later to build a single
    /// write batch updating multiple leaves/parents.
    pub fn insert(&self, kvs: &[(Vec<u8>, Vec<u8>)]) -> Result<()> {
        if kvs.is_empty() {
            return Ok(());
        }
        self.ensure_node_index()?;
        let mut batch = WriteBatch::default();
        let mut cache = self
            .node_cache
            .lock()
            .expect("SMT node cache mutex poisoned");
        let mut keyed = kvs
            .iter()
            .map(|(key, value)| (digest(key), key.as_slice(), value.as_slice()))
            .collect::<Vec<_>>();
        keyed.sort_by_key(|entry| entry.0);
        keyed.dedup_by(|left, right| left.0 == right.0);

        let mut overlay = HashMap::new();
        for (kh, _key, value) in keyed {
            let data_key = data_key(&kh);
            batch.put(data_key, value);
            self.stage_leaf_node_update(&cache, &mut overlay, kh, hash_leaf(&kh, value));
        }

        let root = self.node_hash_from_overlay(&cache, &overlay, &[0u8; 32], 0);
        batch.put(ROOT_NODE_KEY, root);
        batch.delete(NODE_INDEX_VERSION_KEY);
        self.db.write(batch)?;
        self.apply_node_overlay_to_cache(&mut cache, overlay);
        Ok(())
    }

    /// Apply data-leaf updates/deletes and install a caller-verified root.
    ///
    /// Checkpoint execution already computes and verifies the canonical root
    /// before finalization. Reusing that root keeps the RocksDB commit path from
    /// materializing the whole SMT a second time.
    pub fn apply_changes_with_root(
        &self,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
        root: [u8; 32],
    ) -> Result<()> {
        self.ensure_node_index()?;
        let mut batch = WriteBatch::default();
        let mut cache = self
            .node_cache
            .lock()
            .expect("SMT node cache mutex poisoned");
        let mut changes = BTreeMap::new();
        for key in deletes {
            batch.delete(data_key(&digest(key)));
            changes.insert(digest(key), None);
        }
        for (key, value) in updates {
            batch.put(data_key(&digest(key)), value);
            changes.insert(digest(key), Some(value.as_slice()));
        }

        let mut overlay = HashMap::new();
        for (key_hash, value) in changes {
            let leaf = value
                .map(|value| hash_leaf(&key_hash, value))
                .unwrap_or(self.default_hashes[256]);
            self.stage_leaf_node_update(&cache, &mut overlay, key_hash, leaf);
        }
        let computed_root = self.node_hash_from_overlay(&cache, &overlay, &[0u8; 32], 0);
        anyhow::ensure!(
            computed_root == root,
            "verified SMT root mismatch while applying incremental node changes"
        );
        batch.put(ROOT_NODE_KEY, root);
        batch.delete(NODE_INDEX_VERSION_KEY);
        self.db.write(batch)?;
        self.apply_node_overlay_to_cache(&mut cache, overlay);
        Ok(())
    }

    /// Apply data-leaf updates/deletes and install a root whose node overlay was
    /// already computed by `root_hash_with_changes_and_overlay`.
    ///
    /// This is the checkpoint hot path: prepare computes and verifies the root,
    /// finalize only persists the canonical leaves/root and updates the in-memory
    /// cache from the already-computed overlay. The root node in the overlay is
    /// checked cheaply to catch mismatched callers without re-hashing the tree.
    pub fn apply_changes_with_precomputed_overlay(
        &self,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
        root: [u8; 32],
        node_overlay: SmtNodeOverlay,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();
        self.stage_changes_with_precomputed_overlay(
            &mut batch,
            updates,
            deletes,
            root,
            &node_overlay,
        )?;
        self.db.write(batch)?;
        self.apply_precomputed_node_overlay(node_overlay);
        Ok(())
    }

    /// Stage already-verified SMT data/root writes into a caller-owned
    /// RocksDB batch. Call `apply_precomputed_node_overlay` only after the
    /// caller successfully writes that batch.
    pub fn stage_changes_with_precomputed_overlay(
        &self,
        batch: &mut WriteBatch,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
        root: [u8; 32],
        node_overlay: &SmtNodeOverlay,
    ) -> Result<()> {
        let overlay_root = node_overlay
            .iter()
            .find(|(key, _)| key.as_slice() == ROOT_NODE_KEY)
            .map(|(_, value)| value.unwrap_or(self.default_hashes[0]))
            .unwrap_or_else(|| self.root_hash().unwrap_or(self.default_hashes[0]));
        anyhow::ensure!(
            overlay_root == root,
            "precomputed SMT overlay root does not match verified root"
        );

        self.ensure_node_index()?;
        for key in deletes {
            batch.delete(data_key(&digest(key)));
        }
        for (key, value) in updates {
            batch.put(data_key(&digest(key)), value);
        }
        batch.put(ROOT_NODE_KEY, root);
        batch.delete(NODE_INDEX_VERSION_KEY);
        Ok(())
    }

    pub fn apply_precomputed_node_overlay(&self, node_overlay: SmtNodeOverlay) {
        let mut cache = self
            .node_cache
            .lock()
            .expect("SMT node cache mutex poisoned");
        Self::apply_node_overlay_entries_to_cache(&mut cache, node_overlay);
    }

    /// Delete a single key from the tree, updating parent nodes. This will
    /// remove the stored data entry and propagate default hashes upward,
    /// deleting node records when they equal the default value.
    /// Delete a batch of keys
    pub fn delete(&self, keys: &[Vec<u8>]) -> Result<()> {
        // Optimize deletes by sorting/deduping keys by their key-hash, and
        // applying all deletions in a single WriteBatch while caching node
        // reads/writes to avoid repeated DB access for shared prefixes.
        if keys.is_empty() {
            return Ok(());
        }
        self.ensure_node_index()?;
        let mut cache = self
            .node_cache
            .lock()
            .expect("SMT node cache mutex poisoned");

        // prepare key hashes
        let mut keyed: Vec<[u8; 32]> = Vec::with_capacity(keys.len());
        for k in keys.iter() {
            let khd = digest(k.as_slice());
            keyed.push(khd);
        }

        // sort and dedup by hash to coalesce shared prefixes
        keyed.sort();
        keyed.dedup();

        let mut batch = WriteBatch::default();
        let mut overlay = HashMap::new();

        for kh in keyed.into_iter() {
            let data_key = data_key(&kh);
            batch.delete(data_key);
            self.stage_leaf_node_update(&cache, &mut overlay, kh, self.default_hashes[256]);
        }

        let root = self.node_hash_from_overlay(&cache, &overlay, &[0u8; 32], 0);
        batch.put(ROOT_NODE_KEY, root);
        batch.delete(NODE_INDEX_VERSION_KEY);
        self.db.write(batch)?;
        self.apply_node_overlay_to_cache(&mut cache, overlay);
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

    if entries.len() >= 2_048 {
        return compute_sparse_root_parallel_byte(entries);
    }

    compute_sparse_subtree_root(
        &entries
            .iter()
            .map(|(key, value)| (digest(key), value))
            .collect::<Vec<_>>(),
        0,
    )
}

fn compute_sparse_root_parallel_byte(entries: &[(Vec<u8>, Vec<u8>)]) -> [u8; 32] {
    const PARALLEL_BUCKET_DEPTH: usize = 12;
    let mut buckets = (0..(1usize << PARALLEL_BUCKET_DEPTH))
        .map(|_| Vec::new())
        .collect::<Vec<_>>();
    for (key, value) in entries {
        let key_hash = digest(key);
        buckets[top_bits(&key_hash, PARALLEL_BUCKET_DEPTH)].push((key_hash, value));
    }

    let mut roots = buckets
        .par_iter()
        .map(|bucket| compute_sparse_subtree_root(bucket, PARALLEL_BUCKET_DEPTH))
        .collect::<Vec<_>>();

    for depth in (1..=PARALLEL_BUCKET_DEPTH).rev() {
        roots = roots
            .chunks_exact(2)
            .map(|pair| hash_node(&pair[0], &pair[1]))
            .collect();
        debug_assert_eq!(roots.len(), 1usize << (depth - 1));
    }

    roots[0]
}

fn compute_sparse_subtree_root(entries: &[([u8; 32], &Vec<u8>)], subtree_depth: usize) -> [u8; 32] {
    if entries.is_empty() {
        return DEFAULT_HASHES[subtree_depth];
    }
    if entries.windows(2).all(|pair| pair[0].0 <= pair[1].0) {
        return compute_sparse_subtree_root_sorted(entries, subtree_depth);
    }

    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|entry| entry.0);
    compute_sparse_subtree_root_sorted(&sorted, subtree_depth)
}

fn compute_sparse_subtree_root_sorted(
    entries: &[([u8; 32], &Vec<u8>)],
    subtree_depth: usize,
) -> [u8; 32] {
    debug_assert!(!entries.is_empty());
    if entries.len() == 1 {
        let (key_hash, value) = entries[0];
        let mut current = hash_leaf(&key_hash, value);
        for depth in ((subtree_depth + 1)..=256).rev() {
            let (byte_idx, bit_in_byte) = key_bit_position(depth);
            let bit_is_left = ((key_hash[byte_idx] >> bit_in_byte) & 1u8) == 0;
            current = if bit_is_left {
                hash_node(&current, &DEFAULT_HASHES[depth])
            } else {
                hash_node(&DEFAULT_HASHES[depth], &current)
            };
        }
        return current;
    }

    if subtree_depth == 256 {
        return hash_leaf(&entries[0].0, entries[0].1);
    }

    let next_depth = subtree_depth + 1;
    let (byte_idx, bit_in_byte) = key_bit_position(next_depth);
    let split =
        entries.partition_point(|(key_hash, _)| ((key_hash[byte_idx] >> bit_in_byte) & 1u8) == 0);

    let left = if split == 0 {
        DEFAULT_HASHES[next_depth]
    } else {
        compute_sparse_subtree_root_sorted(&entries[..split], next_depth)
    };
    let right = if split == entries.len() {
        DEFAULT_HASHES[next_depth]
    } else {
        compute_sparse_subtree_root_sorted(&entries[split..], next_depth)
    };
    hash_node(&left, &right)
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
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rocksdb::{DB, Options};
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn open_test_db(path: &std::path::Path) -> SparseMerkleTree {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, path).unwrap();
        SparseMerkleTree::new(Arc::new(db))
    }

    #[test]
    fn test_smt_basic_membership() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());

        let key = b"test-key";
        let value = b"test-value";

        // Insert
        smt.insert(&[(key.to_vec(), value.to_vec())])?;

        // Proof
        let (is_member, leaf_hash, siblings) = smt.proof(key)?;
        assert!(is_member);

        // Root
        let root = smt.root_hash()?;

        // Verify
        assert!(verify_proof(&root, key, (is_member, leaf_hash, siblings)));

        Ok(())
    }

    #[test]
    fn test_smt_non_membership() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());

        let key = b"non-existent";

        // Proof
        let (is_member, leaf_hash, siblings) = smt.proof(key)?;
        assert!(!is_member);
        assert_eq!(leaf_hash, default_hashes()[256]);

        // Root (should be empty tree root)
        let root = smt.root_hash()?;
        assert_eq!(root, default_hashes()[0]);

        // Verify
        assert!(verify_proof(&root, key, (is_member, leaf_hash, siblings)));

        Ok(())
    }

    #[test]
    fn test_smt_multi_keys() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());

        let kvs = vec![
            (b"key1".to_vec(), b"val1".to_vec()),
            (b"key2".to_vec(), b"val2".to_vec()),
            (b"key3".to_vec(), b"val3".to_vec()),
        ];

        smt.insert(&kvs)?;

        let root = smt.root_hash()?;

        for (k, _v) in kvs {
            let (is_member, leaf_hash, siblings) = smt.proof(&k)?;
            assert!(is_member);
            assert!(verify_proof(&root, &k, (is_member, leaf_hash, siblings)));
        }

        // Test non-membership of another key
        let other_key = b"key4";
        let (is_member, leaf_hash, siblings) = smt.proof(other_key)?;
        assert!(!is_member);
        assert!(verify_proof(
            &root,
            other_key,
            (is_member, leaf_hash, siblings)
        ));

        Ok(())
    }

    #[test]
    fn test_smt_update() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());

        let key = b"key";
        let val1 = b"val1";
        let val2 = b"val2";

        smt.insert(&[(key.to_vec(), val1.to_vec())])?;
        let root1 = smt.root_hash()?;

        smt.insert(&[(key.to_vec(), val2.to_vec())])?;
        let root2 = smt.root_hash()?;

        assert_ne!(root1, root2);

        let (is_member, leaf_hash, siblings) = smt.proof(key)?;
        assert!(is_member);
        assert!(verify_proof(&root2, key, (is_member, leaf_hash, siblings)));

        Ok(())
    }

    #[test]
    fn test_smt_delete() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());

        let key = b"delete-me";
        let value = b"val";

        smt.insert(&[(key.to_vec(), value.to_vec())])?;
        let root_after_insert = smt.root_hash()?;
        assert_ne!(root_after_insert, default_hashes()[0]);

        smt.delete(&[key.to_vec()])?;
        let root_after_delete = smt.root_hash()?;
        assert_eq!(root_after_delete, default_hashes()[0]);

        let (is_member, _, _) = smt.proof(key)?;
        assert!(!is_member);

        Ok(())
    }

    #[test]
    fn test_root_hash_with_changes_matches_applied_batch() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());

        smt.insert(&[
            (b"keep".to_vec(), b"old".to_vec()),
            (b"delete".to_vec(), b"value".to_vec()),
        ])?;

        let updates = vec![
            (b"keep".to_vec(), b"new".to_vec()),
            (b"add".to_vec(), b"value".to_vec()),
        ];
        let deletes = vec![b"delete".to_vec()];
        let speculative_root = smt.root_hash_with_changes(&updates, &deletes)?;

        smt.delete(&deletes)?;
        smt.insert(&updates)?;

        assert_eq!(speculative_root, smt.root_hash()?);
        Ok(())
    }

    #[test]
    fn test_compute_sparse_root_matches_persisted_smt_insert() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());
        let entries = vec![
            (b"account:a".to_vec(), b"one".to_vec()),
            (b"account:b".to_vec(), b"two".to_vec()),
            (b"system:clock".to_vec(), b"three".to_vec()),
        ];

        smt.insert(&entries)?;

        assert_eq!(compute_sparse_root(&entries), smt.root_hash()?);
        Ok(())
    }

    #[test]
    fn test_rebuild_removes_stale_entries() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());
        smt.insert(&[
            (b"stale".to_vec(), b"old".to_vec()),
            (b"keep".to_vec(), b"old".to_vec()),
        ])?;

        let replacement = vec![(b"keep".to_vec(), b"new".to_vec())];
        smt.rebuild(&replacement)?;

        assert_eq!(compute_sparse_root(&replacement), smt.root_hash()?);
        assert!(!smt.proof(b"stale")?.0);
        assert!(smt.proof(b"keep")?.0);
        Ok(())
    }

    #[test]
    fn test_cache_rebuild_purges_legacy_persisted_node_index() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());
        smt.insert(&[(b"keep".to_vec(), b"value".to_vec())])?;
        let root = smt.root_hash()?;

        smt.db.put(b"n:legacy-node", [7u8; 32])?;
        smt.db.put(NODE_INDEX_VERSION_KEY, b"1")?;
        smt.node_cache_ready.store(false, Ordering::Release);
        smt.node_cache
            .lock()
            .expect("SMT node cache mutex poisoned")
            .clear();

        assert!(smt.proof(b"keep")?.0);
        assert_eq!(smt.db.get(ROOT_NODE_KEY)?.as_deref(), Some(root.as_slice()));
        assert!(smt.db.get(b"n:legacy-node")?.is_none());
        assert!(smt.db.get(NODE_INDEX_VERSION_KEY)?.is_none());
        Ok(())
    }

    #[test]
    fn test_apply_precomputed_overlay_matches_verified_root() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());
        smt.insert(&[
            (b"keep".to_vec(), b"old".to_vec()),
            (b"delete".to_vec(), b"value".to_vec()),
        ])?;

        let updates = vec![
            (b"keep".to_vec(), b"new".to_vec()),
            (b"add".to_vec(), b"value".to_vec()),
        ];
        let deletes = vec![b"delete".to_vec()];
        let (root, overlay) = smt.root_hash_with_changes_and_overlay(&updates, &deletes)?;
        smt.apply_changes_with_precomputed_overlay(&updates, &deletes, root, overlay)?;

        assert_eq!(root, smt.root_hash()?);
        assert!(smt.proof(b"keep")?.0);
        assert!(smt.proof(b"add")?.0);
        assert!(!smt.proof(b"delete")?.0);

        drop(smt);
        let reopened = open_test_db(dir.path());
        assert_eq!(root, reopened.root_hash()?);
        assert!(reopened.proof(b"keep")?.0);
        assert!(reopened.proof(b"add")?.0);
        assert!(!reopened.proof(b"delete")?.0);
        Ok(())
    }

    #[test]
    fn test_incremental_batches_match_full_materialization() -> Result<()> {
        use std::collections::BTreeMap;

        let dir = tempdir()?;
        let smt = open_test_db(dir.path());
        let mut materialized = BTreeMap::new();
        for i in 0..64u64 {
            materialized.insert(
                format!("object:{i:064x}").into_bytes(),
                i.to_le_bytes().to_vec(),
            );
        }
        let initial = materialized
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        smt.insert(&initial)?;
        assert_eq!(compute_sparse_root(&initial), smt.root_hash()?);

        let deletes = materialized.keys().step_by(3).cloned().collect::<Vec<_>>();
        for key in &deletes {
            materialized.remove(key);
        }
        let updates = (32..96u64)
            .map(|i| {
                (
                    format!("object:{i:064x}").into_bytes(),
                    i.wrapping_mul(17).to_le_bytes().to_vec(),
                )
            })
            .collect::<Vec<_>>();
        for (key, value) in &updates {
            materialized.insert(key.clone(), value.clone());
        }

        let expected = compute_sparse_root(
            &materialized
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect::<Vec<_>>(),
        );
        assert_eq!(expected, smt.root_hash_with_changes(&updates, &deletes)?);
        smt.delete(&deletes)?;
        smt.insert(&updates)?;
        assert_eq!(expected, smt.root_hash()?);
        Ok(())
    }

    #[test]
    fn test_parallel_large_overlay_matches_applied_root() -> Result<()> {
        let dir = tempdir()?;
        let smt = open_test_db(dir.path());
        let base = (0..1500)
            .map(|index| {
                (
                    format!("base-key-{index:04}").into_bytes(),
                    format!("base-value-{index:04}").into_bytes(),
                )
            })
            .collect::<Vec<_>>();
        smt.insert(&base)?;

        let updates = (0..4096)
            .map(|index| {
                (
                    format!("large-key-{index:04}").into_bytes(),
                    format!("large-value-{index:04}").into_bytes(),
                )
            })
            .collect::<Vec<_>>();
        let deletes = base
            .iter()
            .take(256)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();

        let speculative = smt.root_hash_with_changes(&updates, &deletes)?;
        smt.delete(&deletes)?;
        smt.insert(&updates)?;
        assert_eq!(speculative, smt.root_hash()?);
        Ok(())
    }

    fn deterministic_kv(seed: u64, namespace: &str, index: usize) -> (Vec<u8>, Vec<u8>) {
        let mixed = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(index as u64)
            .rotate_left((index % 63) as u32);
        (
            format!("{namespace}:{mixed:016x}:{index:05}").into_bytes(),
            mixed
                .wrapping_mul(0xD6E8_FD9D_50D8_3845)
                .to_le_bytes()
                .to_vec(),
        )
    }

    fn smt_prop_cases(default_cases: u32) -> u32 {
        std::env::var("KANARI_SMT_PROPTEST_CASES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default_cases)
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: smt_prop_cases(24),
            max_shrink_iters: 32,
            .. ProptestConfig::default()
        })]

        #[test]
        fn prop_speculative_parallel_root_matches_full_materialization(
            seed in any::<u64>(),
            base_len in 0usize..192,
            update_len in 1024usize..2304,
            delete_stride in 2usize..11,
        ) {
            let dir = tempdir().expect("tempdir");
            let smt = open_test_db(dir.path());
            let mut materialized = BTreeMap::new();

            let base = (0..base_len)
                .map(|index| deterministic_kv(seed, "base", index))
                .collect::<Vec<_>>();
            smt.insert(&base).expect("insert base");
            materialized.extend(base.iter().cloned());

            let deletes = materialized
                .keys()
                .enumerate()
                .filter(|(index, _)| index % delete_stride == 0)
                .map(|(_, key)| key.clone())
                .collect::<Vec<_>>();
            for key in &deletes {
                materialized.remove(key);
            }

            let updates = (0..update_len)
                .map(|index| deterministic_kv(seed ^ 0xA5A5_A5A5_A5A5_A5A5, "update", index))
                .collect::<Vec<_>>();
            materialized.extend(updates.iter().cloned());

            let expected = compute_sparse_root(
                &materialized
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect::<Vec<_>>(),
            );
            let speculative = smt
                .root_hash_with_changes(&updates, &deletes)
                .expect("speculative root");
            prop_assert_eq!(speculative, expected);

            smt.delete(&deletes).expect("delete");
            smt.insert(&updates).expect("insert updates");
            prop_assert_eq!(smt.root_hash().expect("root after apply"), expected);

            drop(smt);
            let reopened = open_test_db(dir.path());
            prop_assert_eq!(reopened.root_hash().expect("reopened root"), expected);
        }
    }
}
