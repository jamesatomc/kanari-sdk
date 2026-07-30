// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::hash::{digest, hash_leaf, hash_node};
use crate::open_or_get_db;
use anyhow::Result;
use once_cell::sync::Lazy;
use rayon::prelude::*;
use rocksdb::IteratorMode;
use rocksdb::WriteBatch;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::RwLock;

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
    db: std::sync::Arc<rocksdb::DB>,
    default_hashes: &'static [[u8; 32]],
    data_cache: RwLock<Option<BTreeMap<[u8; 32], Vec<u8>>>>,
}

const ROOT_NODE_KEY: [u8; 4] = [b'n', b':', 0, 0];

fn key_bit_position(depth: usize) -> (usize, usize) {
    let bit_index = depth - 1;
    (bit_index / 8, 7 - (bit_index % 8))
}

fn top_byte(key_hash: &[u8; 32]) -> usize {
    key_hash[0] as usize
}

fn data_key(key_hash: &[u8; 32]) -> [u8; 34] {
    let mut out = [0u8; 34];
    out[0] = b'd';
    out[1] = b':';
    out[2..].copy_from_slice(key_hash);
    out
}

fn hashed_entries_root(entries: &BTreeMap<[u8; 32], Vec<u8>>) -> [u8; 32] {
    if entries.is_empty() {
        return DEFAULT_HASHES[0];
    }
    if entries.len() >= 2_048 {
        return hashed_entries_root_parallel_byte(entries);
    }

    compute_sparse_subtree_root(
        &entries
            .iter()
            .map(|(key_hash, value)| (*key_hash, value))
            .collect::<Vec<_>>(),
        0,
    )
}

fn hashed_entry_refs_root(entries: &[([u8; 32], &Vec<u8>)]) -> [u8; 32] {
    if entries.is_empty() {
        return DEFAULT_HASHES[0];
    }
    if entries.len() >= 2_048 {
        let mut buckets = (0..256).map(|_| Vec::new()).collect::<Vec<_>>();
        for (key_hash, value) in entries {
            buckets[top_byte(key_hash)].push((*key_hash, *value));
        }

        let mut roots = buckets
            .par_iter()
            .map(|bucket| compute_sparse_subtree_root(bucket, 8))
            .collect::<Vec<_>>();

        for depth in (1..=8).rev() {
            roots = roots
                .chunks_exact(2)
                .map(|pair| hash_node(&pair[0], &pair[1]))
                .collect();
            debug_assert_eq!(roots.len(), 1usize << (depth - 1));
        }

        return roots[0];
    }

    compute_sparse_subtree_root(entries, 0)
}

fn hashed_entries_root_parallel_byte(entries: &BTreeMap<[u8; 32], Vec<u8>>) -> [u8; 32] {
    let mut buckets = (0..256).map(|_| Vec::new()).collect::<Vec<_>>();
    for (key_hash, value) in entries {
        buckets[top_byte(key_hash)].push((*key_hash, value));
    }

    let mut roots = buckets
        .par_iter()
        .map(|bucket| compute_sparse_subtree_root(bucket, 8))
        .collect::<Vec<_>>();

    for depth in (1..=8).rev() {
        roots = roots
            .chunks_exact(2)
            .map(|pair| hash_node(&pair[0], &pair[1]))
            .collect();
        debug_assert_eq!(roots.len(), 1usize << (depth - 1));
    }

    roots[0]
}

fn prefix_matches(key_hash: &[u8; 32], prefix_hash: &[u8; 32], depth: usize) -> bool {
    if depth == 0 {
        return true;
    }
    let full_bytes = depth / 8;
    if key_hash[..full_bytes] != prefix_hash[..full_bytes] {
        return false;
    }
    let remaining_bits = depth % 8;
    if remaining_bits == 0 {
        return true;
    }
    let mask = 0xFF_u8 << (8 - remaining_bits);
    (key_hash[full_bytes] & mask) == (prefix_hash[full_bytes] & mask)
}

impl SparseMerkleTree {
    pub fn new(db: std::sync::Arc<rocksdb::DB>) -> Self {
        Self {
            db,
            default_hashes: &DEFAULT_HASHES,
            data_cache: RwLock::new(None),
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

    pub fn root_hash(&self) -> Result<[u8; 32]> {
        // root is stored at depth 0 with empty prefix
        if let Ok(Some(v)) = self.db.get(ROOT_NODE_KEY) {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&v);
            Ok(arr)
        } else {
            Ok(self.default_hashes[0])
        }
    }

    fn persisted_data_entries(&self) -> Result<BTreeMap<[u8; 32], Vec<u8>>> {
        if let Some(entries) = self
            .data_cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
        {
            return Ok(entries.clone());
        }

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
        *self
            .data_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(entries.clone());
        Ok(entries)
    }

    fn replace_data_cache(&self, entries: BTreeMap<[u8; 32], Vec<u8>>) {
        *self
            .data_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(entries);
    }

    fn update_data_cache(&self, updates: &[(Vec<u8>, Vec<u8>)], deletes: &[Vec<u8>]) {
        let mut guard = self
            .data_cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(entries) = guard.as_mut() else {
            return;
        };
        for key in deletes {
            entries.remove(&digest(key));
        }
        for (key, value) in updates {
            entries.insert(digest(key), value.clone());
        }
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
        self.db.write(batch)?;
        self.replace_data_cache(BTreeMap::new());
        if !entries.is_empty() {
            self.insert(entries)?;
        }
        Ok(())
    }

    pub fn root_hash_with_changes(
        &self,
        updates: &[(Vec<u8>, Vec<u8>)],
        deletes: &[Vec<u8>],
    ) -> Result<[u8; 32]> {
        if updates.is_empty() && deletes.is_empty() {
            return self.root_hash();
        }

        if self
            .data_cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none()
        {
            let _ = self.persisted_data_entries()?;
        }

        let guard = self
            .data_cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entries = guard
            .as_ref()
            .expect("SMT data cache must be initialized before root preview");
        let delete_hashes = deletes
            .iter()
            .map(|key| digest(key))
            .collect::<BTreeSet<_>>();
        let update_hashes = updates
            .iter()
            .map(|(key, value)| (digest(key), value))
            .collect::<BTreeMap<_, _>>();
        let mut root_entries = Vec::with_capacity(entries.len().saturating_add(updates.len()));
        for (key_hash, value) in entries {
            if delete_hashes.contains(key_hash) {
                continue;
            }
            root_entries.push((
                *key_hash,
                update_hashes.get(key_hash).copied().unwrap_or(value),
            ));
        }
        for (key_hash, value) in update_hashes {
            if delete_hashes.contains(&key_hash) || !entries.contains_key(&key_hash) {
                root_entries.push((key_hash, value));
            }
        }

        Ok(hashed_entry_refs_root(&root_entries))
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

        let entries = self.persisted_data_entries()?;
        let mut siblings: Vec<[u8; 32]> = Vec::with_capacity(256);

        // traverse from leaf depth down to 1 and collect sibling at each level
        for depth in (1..=256).rev() {
            let mut sibling_prefix = kh;
            let (byte_idx, bit_in_byte) = key_bit_position(depth);
            sibling_prefix[byte_idx] ^= 1u8 << bit_in_byte;
            let sibling_entries = entries
                .iter()
                .filter(|(entry_hash, _)| prefix_matches(entry_hash, &sibling_prefix, depth))
                .map(|(entry_hash, value)| (*entry_hash, value))
                .collect::<Vec<_>>();
            siblings.push(compute_sparse_subtree_root(&sibling_entries, depth));
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
        let mut batch = WriteBatch::default();
        let mut entries = self.persisted_data_entries()?;
        let mut keyed = kvs
            .iter()
            .map(|(key, value)| (digest(key), key.as_slice(), value.as_slice()))
            .collect::<Vec<_>>();
        keyed.sort_by_key(|entry| entry.0);
        keyed.dedup_by(|left, right| left.0 == right.0);

        for (kh, _key, value) in keyed {
            let data_key = data_key(&kh);
            batch.put(data_key, value);
            entries.insert(kh, value.to_vec());
        }

        batch.put(ROOT_NODE_KEY, hashed_entries_root(&entries));
        self.db.write(batch)?;
        self.replace_data_cache(entries);
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
        let mut batch = WriteBatch::default();
        for key in deletes {
            batch.delete(data_key(&digest(key)));
        }
        for (key, value) in updates {
            batch.put(data_key(&digest(key)), value);
        }
        if root == self.default_hashes[0] {
            batch.delete(ROOT_NODE_KEY);
        } else {
            batch.put(ROOT_NODE_KEY, root);
        }
        self.db.write(batch)?;
        self.update_data_cache(updates, deletes);
        Ok(())
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
        let mut entries = self.persisted_data_entries()?;

        for kh in keyed.into_iter() {
            let data_key = data_key(&kh);
            batch.delete(data_key);
            entries.remove(&kh);
        }

        let root = hashed_entries_root(&entries);
        if root == self.default_hashes[0] {
            batch.delete(ROOT_NODE_KEY);
        } else {
            batch.put(ROOT_NODE_KEY, root);
        }
        self.db.write(batch)?;
        self.replace_data_cache(entries);
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
    let mut buckets = (0..256).map(|_| Vec::new()).collect::<Vec<_>>();
    for (key, value) in entries {
        let key_hash = digest(key);
        buckets[top_byte(&key_hash)].push((key_hash, value));
    }

    let mut roots = buckets
        .par_iter()
        .map(|bucket| compute_sparse_subtree_root(bucket, 8))
        .collect::<Vec<_>>();

    for depth in (1..=8).rev() {
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
    use rocksdb::{DB, Options};
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
}
