// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

const FILE_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const FILE_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(25);

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn lock_path(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{name}.lock"))
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    if !destination.exists() {
        match fs::rename(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => return Err(error),
            Err(_) => {}
        }
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *const core::ffi::c_void,
            reserved: *const core::ffi::c_void,
        ) -> i32;
    }

    const REPLACEFILE_WRITE_THROUGH: u32 = 0x0000_0001;
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let destination = wide(destination);
    let source = wide(source);
    if unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            source.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn sync_parent_directory(parent: &Path) -> std::io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(windows)]
fn sync_parent_directory(_: &Path) -> std::io::Result<()> {
    Ok(())
}

fn lock_exclusively(lock_file: &File, path: &Path) -> Result<()> {
    let started = Instant::now();
    loop {
        match lock_file.try_lock_exclusive() {
            Ok(()) => {
                let waited = started.elapsed();
                if !waited.is_zero() {
                    tracing::debug!(path = %path.display(), wait_ms = waited.as_millis(), "Acquired file write lock after waiting");
                }
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if started.elapsed() >= FILE_LOCK_TIMEOUT {
                    anyhow::bail!(
                        "Timed out waiting {} seconds for write lock on {}",
                        FILE_LOCK_TIMEOUT.as_secs(),
                        path.display()
                    );
                }
                thread::sleep(FILE_LOCK_RETRY_INTERVAL);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to lock {}", path.display()));
            }
        }
    }
}

fn write_file_atomically_with(
    path: &Path,
    write_contents: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    let parent = parent_directory(path);
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create directory {}", parent.display()))?;

    let lock_file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path(path))
        .with_context(|| format!("Failed to open write lock for {}", path.display()))?;
    lock_exclusively(&lock_file, path)?;

    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("Failed to create temporary file for {}", path.display()))?;
    write_contents(temporary.as_file_mut())
        .with_context(|| format!("Failed to write temporary file for {}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .with_context(|| format!("Failed to sync temporary file for {}", path.display()))?;

    let temporary_path = temporary.into_temp_path();
    replace_file(temporary_path.as_ref(), path)
        .with_context(|| format!("Failed to replace {}", path.display()))?;
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .with_context(|| format!("Failed to sync replaced file {}", path.display()))?;
    sync_parent_directory(parent)
        .with_context(|| format!("Failed to sync directory {}", parent.display()))?;
    Ok(())
}

/// Durably replaces a file without exposing a partially written destination.
///
/// Calls targeting the same path are serialized across processes with a
/// sidecar lock. The temporary file is unique and created in the destination
/// directory so replacement stays on the same filesystem.
pub fn write_file_atomically(path: &Path, contents: &[u8]) -> Result<()> {
    write_file_atomically_with(path, |file| file.write_all(contents).map_err(Into::into))
}

/// Serializes a value as readable JSON and stores it atomically.
pub fn write_json_pretty_atomically<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    write_file_atomically_with(path, |file| {
        serde_json::to_writer_pretty(file, value).map_err(Into::into)
    })
}

/// Reads and deserializes a JSON file with consistent file-path context.
pub fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let file =
        File::open(path).with_context(|| format!("Failed to read JSON file {}", path.display()))?;
    serde_json::from_reader(BufReader::new(file))
        .with_context(|| format!("Invalid JSON file {}", path.display()))
}
