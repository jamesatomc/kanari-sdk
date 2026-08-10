// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use fs2::FileExt;

use super::KeystoreError;

const LOCK_RETRIES: usize = 50;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(100);

pub(super) struct LockFileGuard {
    file: File,
    path: std::path::PathBuf,
}

impl Drop for LockFileGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
        let _ = fs::remove_file(&self.path);
    }
}

pub(super) fn read_to_string(path: &Path) -> Result<String, KeystoreError> {
    Ok(fs::read_to_string(path)?)
}

pub(super) fn ensure_parent_dir(path: &Path) -> Result<(), KeystoreError> {
    let Some(dir) = path.parent() else {
        return Err(KeystoreError::InvalidPath(
            "Invalid keystore path".to_string(),
        ));
    };

    if !dir.exists() {
        fs::create_dir_all(dir)?;
    }

    Ok(())
}

pub(super) fn acquire_shared_lock(path: &Path) -> Result<LockFileGuard, KeystoreError> {
    let lock_path = path.with_extension("lock");
    let lock_file = open_lock_file(&lock_path)?;
    retry_lock(|| lock_file.try_lock_shared())?;

    Ok(LockFileGuard {
        file: lock_file,
        path: lock_path,
    })
}

pub(super) fn acquire_exclusive_lock(path: &Path) -> Result<LockFileGuard, KeystoreError> {
    let lock_path = path.with_extension("lock");
    let lock_file = open_lock_file(&lock_path)?;
    retry_lock(|| lock_file.try_lock_exclusive())?;

    Ok(LockFileGuard {
        file: lock_file,
        path: lock_path,
    })
}

pub(super) fn atomic_write_string(path: &Path, data: &str) -> Result<(), KeystoreError> {
    let temp_path = path.with_extension("tmp");
    let mut file = File::create(&temp_path)?;
    file.write_all(data.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp_path, path)?;
    Ok(())
}

fn open_lock_file(lock_path: &Path) -> Result<File, KeystoreError> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(KeystoreError::IoError)
}

fn retry_lock<E>(lock: impl Fn() -> Result<(), E>) -> Result<(), KeystoreError> {
    for _ in 0..LOCK_RETRIES {
        if lock().is_ok() {
            return Ok(());
        }
        std::thread::sleep(LOCK_RETRY_DELAY);
    }

    Err(KeystoreError::Locked)
}
