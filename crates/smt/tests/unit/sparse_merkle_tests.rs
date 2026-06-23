use super::*;
use rocksdb::{DB, Options};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tempfile::tempdir;

fn open_test_db(path: &std::path::Path) -> SparseMerkleTree {
    let mut opts = Options::default();
    opts.create_if_missing(true);
    let db = DB::open(&opts, path).unwrap();
    SparseMerkleTree::new(Arc::new(db))
}

fn open_test_memory_smt() -> SparseMerkleTree {
    SparseMerkleTree::new_in_memory(Arc::new(RwLock::new(HashMap::new())))
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
fn test_in_memory_smt_insert_delete_and_proof() -> Result<()> {
    let smt = open_test_memory_smt();
    let entries = vec![
        (b"account:a".to_vec(), b"one".to_vec()),
        (b"account:b".to_vec(), b"two".to_vec()),
    ];

    smt.insert(&entries)?;
    let root = smt.root_hash()?;
    assert_eq!(compute_sparse_root(&entries), root);

    let (is_member, leaf_hash, siblings) = smt.proof(b"account:a")?;
    assert!(is_member);
    assert!(verify_proof(
        &root,
        b"account:a",
        (is_member, leaf_hash, siblings)
    ));

    smt.delete(&[b"account:a".to_vec()])?;
    assert_eq!(
        smt.root_hash()?,
        compute_sparse_root(&[(b"account:b".to_vec(), b"two".to_vec())])
    );
    Ok(())
}
