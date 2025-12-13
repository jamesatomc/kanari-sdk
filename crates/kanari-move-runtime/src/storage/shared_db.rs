use anyhow::{Context, Result};
use once_cell::sync::OnceCell;
use rocksdb::{DB, Options};
use std::path::PathBuf;
use std::sync::Arc;

static GLOBAL_DB: OnceCell<Arc<DB>> = OnceCell::new();
static GLOBAL_DB_PATH: OnceCell<PathBuf> = OnceCell::new();

/// Open (once) a RocksDB instance at the given path (or default) and return Arc<DB>.
/// Subsequent calls will return the same Arc. If a different path is provided after
/// the DB was opened, an error is returned to avoid multiple opens to different paths.
pub fn get_or_open_db(path_opt: Option<PathBuf>) -> Result<Arc<DB>> {
    if let Some(db) = GLOBAL_DB.get() {
        // Already opened. If a path was provided, ensure it matches the existing one.
        if let Some(p) = path_opt {
            if let Some(existing) = GLOBAL_DB_PATH.get() {
                if existing != &p {
                    anyhow::bail!("RocksDB already opened with a different path");
                }
            }
        }
        return Ok(db.clone());
    }

    // Determine path
    let path = if let Some(p) = path_opt {
        p
    } else if let Ok(dir) = std::env::var("KANARI_DB") {
        let mut pb = PathBuf::from(dir);
        if pb.is_dir() {
            pb.push("kanari_db");
        }
        pb
    } else {
        let mut pb = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        pb.push(".kanari");
        pb.push("kanari-db");
        std::fs::create_dir_all(&pb).context("Failed to create kanari-db directory")?;
        pb.push("kanari_db");
        pb
    };

    std::fs::create_dir_all(path.parent().unwrap_or_else(|| std::path::Path::new(".")))
        .context("Failed to create RocksDB parent directory")?;

    let mut opts = Options::default();
    opts.create_if_missing(true);
    let db = DB::open(&opts, &path).context("Failed to open RocksDB for kanari")?;

    GLOBAL_DB_PATH.set(path.clone()).ok();
    let arc = Arc::new(db);
    GLOBAL_DB
        .set(arc.clone())
        .map_err(|_| anyhow::anyhow!("Failed to set global RocksDB"))?;
    Ok(arc)
}
