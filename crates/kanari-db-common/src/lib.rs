// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use rocksdb::{DB, Options};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

static GLOBAL_DBS: Lazy<Mutex<HashMap<PathBuf, Weak<DB>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn resolve_db_path(path_opt: Option<PathBuf>) -> Result<PathBuf> {
    let path = if let Some(p) = path_opt {
        p
    } else if let Ok(dir) = std::env::var("KANARI_DB") {
        let mut pb = PathBuf::from(dir);
        if pb.is_dir() {
            pb.push("kanari_db");
        }
        pb
    } else {
        let mut pb = if cfg!(miri) {
            PathBuf::from(".")
        } else {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
        };
        pb.push(".kanari");
        pb.push("kanari-db");
        std::fs::create_dir_all(&pb).context("Failed to create kanari-db directory")?;
        pb.push("kanari_db");
        pb
    };

    Ok(path)
}

/// Open or reuse a RocksDB instance keyed by its resolved path.
///
/// Calls for the same path share one live `Arc<DB>` to avoid multi-open corruption,
/// while different paths may coexist in the same process (important for tests).
pub fn open_or_get_db(path_opt: Option<PathBuf>) -> Result<Arc<DB>> {
    let path = resolve_db_path(path_opt)?;

    {
        let mut dbs = GLOBAL_DBS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = dbs.get(&path).and_then(|weak| weak.upgrade()) {
            return Ok(existing);
        }
        dbs.retain(|_, weak| weak.strong_count() > 0);
    }

    std::fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")))
        .context("Failed to create RocksDB parent directory")?;

    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let parallelism = std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4);
    opts.increase_parallelism(parallelism);
    opts.set_max_background_jobs(std::cmp::max(4, parallelism));

    let mut block_opts = rocksdb::BlockBasedOptions::default();
    block_opts.set_block_cache(&rocksdb::Cache::new_lru_cache(512 * 1024 * 1024));
    block_opts.set_bloom_filter(10.0, false);
    block_opts.set_cache_index_and_filter_blocks(true);
    block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
    opts.set_block_based_table_factory(&block_opts);

    opts.set_write_buffer_size(64 * 1024 * 1024);
    opts.set_max_write_buffer_number(4);
    opts.set_target_file_size_base(64 * 1024 * 1024);
    opts.set_max_bytes_for_level_base(256 * 1024 * 1024);
    opts.set_compression_type(rocksdb::DBCompressionType::Lz4);
    opts.set_bytes_per_sync(1024 * 1024);

    let db = Arc::new(DB::open(&opts, &path).context("Failed to open RocksDB for kanari")?);

    let mut dbs = GLOBAL_DBS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = dbs.get(&path).and_then(|weak| weak.upgrade()) {
        return Ok(existing);
    }
    dbs.insert(path, Arc::downgrade(&db));
    Ok(db)
}