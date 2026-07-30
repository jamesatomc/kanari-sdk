// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use kanari_core::{
    BlockchainEngine, engine::StateSnapshot, read_json_file, write_json_pretty_atomically,
};
use serde::{Deserialize, Serialize};
use sha3::{Digest, Sha3_256};
use tracing::info;
use zeroize::Zeroizing;

const BACKUP_FORMAT_VERSION: u32 = 1;
const BACKUP_FORMAT_VERSION_V2: u32 = 2;
const BACKUP_PASSWORD_ENV: &str = "KANARI_VALIDATOR_BACKUP_PASSWORD";
const BACKUP_V2_MAGIC: &[u8] = b"KANARI_VALIDATOR_BACKUP_V2\n";
const BACKUP_V2_PAYLOAD_MAGIC: &[u8] = b"KANARI_VALIDATOR_BACKUP_PAYLOAD_V2\n";
const BACKUP_COPY_BUFFER_SIZE: usize = 128 * 1024;
const MAX_BACKUP_V2_COMPRESSED_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_BACKUP_V2_ENTRY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const MAX_BACKUP_V2_RECORDS: usize = 100_000;

#[derive(Debug, Serialize, Deserialize)]
struct ValidatorBackupPayload {
    format_version: u32,
    network: String,
    created_unix_seconds: u64,
    state_snapshot: Vec<u8>,
    files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedValidatorBackup {
    format_version: u32,
    payload_sha3_256: String,
    encrypted_payload: kanari_crypto::EncryptedData,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedValidatorBackupV2Header {
    format_version: u32,
    payload_sha3_256: String,
    encrypted_payload: kanari_crypto::StreamEncryptionHeader,
}

#[derive(Debug, Serialize, Deserialize)]
struct ValidatorBackupV2Manifest {
    format_version: u32,
    network: String,
    created_unix_seconds: u64,
    checkpoint_height: u64,
    state_root: String,
    included_files: usize,
}

#[derive(Debug)]
pub struct ValidatorBackupSummary {
    pub checkpoint_height: u64,
    pub state_root: String,
    pub included_files: usize,
}

struct PendingValidatorRestore {
    data_staging: tempfile::TempDir,
    recovery_staging: tempfile::TempDir,
    data_dir: PathBuf,
    recovery_dir: PathBuf,
    summary: ValidatorBackupSummary,
}

impl PendingValidatorRestore {
    fn promote(self) -> Result<ValidatorBackupSummary> {
        promote_restore_staging(self.data_staging, &self.data_dir)?;
        promote_restore_staging(self.recovery_staging, &self.recovery_dir)?;
        Ok(self.summary)
    }
}

fn backup_password() -> Result<Zeroizing<String>> {
    let password = Zeroizing::new(std::env::var(BACKUP_PASSWORD_ENV).map_err(|_| {
        anyhow::anyhow!("{BACKUP_PASSWORD_ENV} is required for validator backup and restore")
    })?);
    if password.len() < 12 {
        anyhow::bail!("{BACKUP_PASSWORD_ENV} must contain at least 12 characters");
    }
    Ok(password)
}

fn add_required_file(
    files: &mut BTreeMap<String, Vec<u8>>,
    archive_name: &str,
    source: &Path,
) -> Result<()> {
    let bytes = fs::read(source)
        .with_context(|| format!("Failed to read required backup file {}", source.display()))?;
    files.insert(archive_name.to_string(), bytes);
    Ok(())
}

struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha3_256,
    bytes: u64,
    limit: u64,
}

impl<W: Write> HashingWriter<W> {
    fn new(inner: W, limit: u64) -> Self {
        Self {
            inner,
            hasher: Sha3_256::new(),
            bytes: 0,
            limit,
        }
    }

    fn finish(self) -> (W, String, u64) {
        (self.inner, hex::encode(self.hasher.finalize()), self.bytes)
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let next = self
            .bytes
            .checked_add(buf.len() as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "backup size overflow"))?;
        if next > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "validator backup compressed payload exceeds configured limit",
            ));
        }
        let written = self.inner.write(buf)?;
        self.hasher.update(&buf[..written]);
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct HashingReader<R: Read> {
    inner: R,
    hasher: Sha3_256,
    bytes: u64,
    limit: u64,
}

impl<R: Read> HashingReader<R> {
    fn new(inner: R, limit: u64) -> Self {
        Self {
            inner,
            hasher: Sha3_256::new(),
            bytes: 0,
            limit,
        }
    }

    fn digest_hex(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        let next = self
            .bytes
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "backup size overflow"))?;
        if next > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "validator backup compressed payload exceeds configured limit",
            ));
        }
        self.hasher.update(&buf[..read]);
        self.bytes = next;
        Ok(read)
    }
}

fn required_backup_files(
    data_dir: &Path,
    consensus_private_key: &Path,
    consensus_public_keys: &Path,
    genesis: &Path,
) -> Result<Vec<(String, PathBuf)>> {
    let mut files = vec![
        (
            "identity/p2p-identity.key".to_string(),
            data_dir.join("p2p-identity.key"),
        ),
        (
            "consensus/private-key.key".to_string(),
            consensus_private_key.to_path_buf(),
        ),
        (
            "consensus/public-keys.json".to_string(),
            consensus_public_keys.to_path_buf(),
        ),
        ("network/genesis.json".to_string(), genesis.to_path_buf()),
    ];

    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("mysticeti-") && name.ends_with(".wal") {
            files.push((format!("wal/{name}"), entry.path()));
        }
    }
    Ok(files)
}

pub fn export_validator_backup(
    engine: &BlockchainEngine,
    network: &str,
    data_dir: &Path,
    consensus_private_key: &Path,
    consensus_public_keys: &Path,
    genesis: &Path,
    output: &Path,
) -> Result<ValidatorBackupSummary> {
    if output.exists() {
        anyhow::bail!(
            "Refusing to overwrite validator backup: {}",
            output.display()
        );
    }
    let password = backup_password()?;
    let started = std::time::Instant::now();
    let temp_dir = tempfile::tempdir()?;
    let snapshot_path = temp_dir.path().join("state-snapshot.json");
    let snapshot = engine.export_state_snapshot(&snapshot_path, network)?;
    let files = required_backup_files(
        data_dir,
        consensus_private_key,
        consensus_public_keys,
        genesis,
    )?;
    let included_files = files.len();

    export_validator_backup_v2(
        network,
        output,
        password.as_str(),
        &snapshot,
        &snapshot_path,
        &files,
        started,
    )?;

    Ok(ValidatorBackupSummary {
        checkpoint_height: snapshot.checkpoint_height,
        state_root: snapshot.state_root,
        included_files,
    })
}

#[allow(dead_code)]
fn export_validator_backup_v1(
    network: &str,
    output: &Path,
    password: &str,
    _snapshot: &StateSnapshot,
    snapshot_path: &Path,
    files: &[(String, PathBuf)],
) -> Result<()> {
    let state_snapshot = fs::read(snapshot_path)?;
    let mut payload_files = BTreeMap::new();
    for (archive_name, source) in files {
        add_required_file(&mut payload_files, archive_name, source)?;
    }

    let payload = ValidatorBackupPayload {
        format_version: BACKUP_FORMAT_VERSION,
        network: network.to_string(),
        created_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        state_snapshot,
        files: payload_files,
    };
    let encoded = bincode::serde::encode_to_vec(&payload, bincode::config::standard())?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&encoded)?;
    let compressed = Zeroizing::new(encoder.finish()?);
    let payload_sha3_256 = hex::encode(Sha3_256::digest(&compressed));
    let encrypted_payload = kanari_crypto::encrypt_data(&compressed, password)
        .map_err(|error| anyhow::anyhow!("Failed to encrypt validator backup: {error}"))?;
    let archive = EncryptedValidatorBackup {
        format_version: BACKUP_FORMAT_VERSION,
        payload_sha3_256,
        encrypted_payload,
    };

    write_json_pretty_atomically(output, &archive)?;
    Ok(())
}

fn export_validator_backup_v2(
    network: &str,
    output: &Path,
    password: &str,
    snapshot: &StateSnapshot,
    snapshot_path: &Path,
    files: &[(String, PathBuf)],
    started: std::time::Instant,
) -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let encrypted_path = temp_dir.path().join("validator-backup-v2.ciphertext");

    let output_file = File::create(&encrypted_path)?;
    let output_writer = BufWriter::new(output_file);
    let (encrypted_payload, encrypting_writer) = kanari_crypto::stream_encrypting_writer(
        output_writer,
        password,
        kanari_crypto::DEFAULT_STREAM_CHUNK_SIZE,
    )
    .map_err(|error| {
        anyhow::anyhow!("Failed to initialize streaming backup encryption: {error}")
    })?;
    let hashing_writer = HashingWriter::new(encrypting_writer, MAX_BACKUP_V2_COMPRESSED_BYTES);
    let mut encoder = GzEncoder::new(hashing_writer, Compression::default());
    write_payload_v2(&mut encoder, network, snapshot, snapshot_path, files)?;
    let hashing_writer = encoder.finish()?;
    let (encrypting_writer, payload_sha3_256, payload_bytes) = hashing_writer.finish();
    let output_writer = encrypting_writer.finish().map_err(|error| {
        anyhow::anyhow!("Failed to finish streaming backup encryption: {error}")
    })?;
    output_writer.into_inner()?.sync_all()?;

    let encrypted_bytes = fs::metadata(&encrypted_path)?.len();
    write_backup_v2_envelope(
        output,
        &EncryptedValidatorBackupV2Header {
            format_version: BACKUP_FORMAT_VERSION_V2,
            payload_sha3_256,
            encrypted_payload,
        },
        &encrypted_path,
    )?;
    info!(
        payload_bytes,
        encrypted_bytes,
        included_files = files.len(),
        elapsed_ms = started.elapsed().as_millis(),
        "Exported streaming validator backup v2"
    );
    Ok(())
}

fn safe_restore_file(base: &Path, relative_name: &str, bytes: &[u8]) -> Result<PathBuf> {
    let relative = Path::new(relative_name);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        anyhow::bail!("Unsafe validator backup path: {relative_name}");
    }
    let target = base.join(relative);
    if target.exists() {
        anyhow::bail!("Restore target already exists: {}", target.display());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&target, bytes)?;
    Ok(target)
}

fn safe_restore_stream<R: Read>(
    base: &Path,
    relative_name: &str,
    mut reader: R,
    expected_len: u64,
    expected_hash: [u8; 32],
) -> Result<PathBuf> {
    let relative = Path::new(relative_name);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        anyhow::bail!("Unsafe validator backup path: {relative_name}");
    }
    let target = base.join(relative);
    if target.exists() {
        anyhow::bail!("Restore target already exists: {}", target.display());
    }
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Restore target has no parent: {}", target.display()))?;
    fs::create_dir_all(parent)?;

    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    let mut hasher = Sha3_256::new();
    let mut remaining = expected_len;
    let mut copied = 0u64;
    let mut buffer = vec![0u8; BACKUP_COPY_BUFFER_SIZE];
    while remaining > 0 {
        let to_read = usize::try_from(remaining.min(buffer.len() as u64))?;
        reader.read_exact(&mut buffer[..to_read])?;
        temp.write_all(&buffer[..to_read])?;
        hasher.update(&buffer[..to_read]);
        remaining -= to_read as u64;
        copied += to_read as u64;
    }
    let actual_hash: [u8; 32] = hasher.finalize().into();
    if actual_hash != expected_hash {
        anyhow::bail!("Validator backup entry checksum mismatch: {relative_name}");
    }
    temp.as_file().sync_all()?;
    temp.persist_noclobber(&target)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to persist restored file {}", target.display()))?;
    sync_parent_dir(parent)?;
    if copied != expected_len {
        anyhow::bail!("Validator backup entry length mismatch: {relative_name}");
    }
    Ok(target)
}

fn persist_staged_tree(staged_base: &Path, target_base: &Path) -> Result<()> {
    if !staged_base.exists() {
        return Ok(());
    }
    persist_staged_tree_inner(staged_base, staged_base, target_base)
}

fn remove_restore_subtree(staging_root: &Path, subtree: &Path) -> Result<()> {
    if !subtree.exists() {
        return Ok(());
    }
    subtree.strip_prefix(staging_root).with_context(|| {
        format!(
            "Restore staging subtree {} is outside staging root {}",
            subtree.display(),
            staging_root.display()
        )
    })?;
    fs::remove_dir_all(subtree).with_context(|| {
        format!(
            "Failed to remove restore staging subtree {}",
            subtree.display()
        )
    })
}

fn persist_staged_tree_inner(staged_base: &Path, current: &Path, target_base: &Path) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            persist_staged_tree_inner(staged_base, &path, target_base)?;
            continue;
        }
        let relative = path.strip_prefix(staged_base)?;
        let relative_name = relative.to_string_lossy().replace('\\', "/");
        validate_archive_name(&relative_name)?;
        let (len, hash) = hash_file(&path)?;
        let reader = BufReader::new(File::open(&path)?);
        safe_restore_stream(target_base, &relative_name, reader, len, hash)?;
    }
    Ok(())
}

fn create_restore_staging_dir(target: &Path, prefix: &str) -> Result<tempfile::TempDir> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(parent)
        .with_context(|| {
            format!(
                "Failed to create restore staging directory near {}",
                target.display()
            )
        })
}

fn promote_restore_staging(staging: tempfile::TempDir, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if target.exists() {
        if fs::read_dir(target)?.next().is_some() {
            anyhow::bail!("Restore target is no longer empty: {}", target.display());
        }
        fs::remove_dir(target).with_context(|| {
            format!("Failed to remove empty restore target {}", target.display())
        })?;
    }
    let staging_path = staging.keep();
    fs::rename(&staging_path, target).with_context(|| {
        format!(
            "Failed to promote restore staging directory {} to {}",
            staging_path.display(),
            target.display()
        )
    })?;
    sync_parent_dir(parent)?;
    Ok(())
}

fn write_payload_v2<W: Write>(
    writer: &mut W,
    network: &str,
    snapshot: &StateSnapshot,
    snapshot_path: &Path,
    files: &[(String, PathBuf)],
) -> Result<()> {
    writer.write_all(BACKUP_V2_PAYLOAD_MAGIC)?;
    let manifest = ValidatorBackupV2Manifest {
        format_version: BACKUP_FORMAT_VERSION_V2,
        network: network.to_string(),
        created_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        checkpoint_height: snapshot.checkpoint_height,
        state_root: snapshot.state_root.clone(),
        included_files: files.len(),
    };
    let manifest_json = serde_json::to_vec(&manifest)?;
    write_len_prefixed(writer, &manifest_json)?;
    write_payload_record(writer, "state/state-snapshot.json", snapshot_path)?;
    for (archive_name, source) in files {
        write_payload_record(writer, archive_name, source)?;
    }
    writer.write_all(&[0])?;
    Ok(())
}

fn write_payload_record<W: Write>(writer: &mut W, archive_name: &str, source: &Path) -> Result<()> {
    validate_archive_name(archive_name)?;
    let (len, hash) = hash_file(source)
        .with_context(|| format!("Failed to hash backup source {}", source.display()))?;
    writer.write_all(&[1])?;
    write_len_prefixed(writer, archive_name.as_bytes())?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&hash)?;
    let mut reader = BufReader::new(
        File::open(source)
            .with_context(|| format!("Failed to read backup source {}", source.display()))?,
    );
    io::copy(&mut reader, writer)?;
    Ok(())
}

fn validate_archive_name(archive_name: &str) -> Result<()> {
    if archive_name.is_empty() || archive_name.len() > u16::MAX as usize {
        anyhow::bail!("Invalid validator backup entry name length");
    }
    if archive_name.starts_with('/')
        || archive_name.ends_with('/')
        || archive_name.contains('\\')
        || archive_name.contains('\0')
    {
        anyhow::bail!("Unsafe validator backup entry name: {archive_name}");
    }
    for segment in archive_name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains(':') {
            anyhow::bail!("Unsafe validator backup entry name: {archive_name}");
        }
    }
    Ok(())
}

fn write_len_prefixed<W: Write>(writer: &mut W, value: &[u8]) -> Result<()> {
    let len = u32::try_from(value.len())?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(value)?;
    Ok(())
}

fn read_len_prefixed<R: Read>(reader: &mut R, limit: usize) -> Result<Vec<u8>> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_be_bytes(len_bytes) as usize;
    if len > limit {
        anyhow::bail!("Validator backup length-prefixed field exceeds limit");
    }
    let mut value = vec![0u8; len];
    reader.read_exact(&mut value)?;
    Ok(value)
}

fn hash_file(path: &Path) -> Result<(u64, [u8; 32])> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha3_256::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; BACKUP_COPY_BUFFER_SIZE];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((total, hasher.finalize().into()))
}

fn write_backup_v2_envelope(
    output: &Path,
    header: &EncryptedValidatorBackupV2Header,
    encrypted_path: &Path,
) -> Result<()> {
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(BACKUP_V2_MAGIC)?;
    let header_json = serde_json::to_vec(header)?;
    write_len_prefixed(&mut temp, &header_json)?;
    let mut encrypted = BufReader::new(File::open(encrypted_path)?);
    io::copy(&mut encrypted, &mut temp)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(output)
        .map_err(|error| error.error)
        .with_context(|| format!("Failed to persist validator backup {}", output.display()))?;
    sync_parent_dir(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> Result<()> {
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> Result<()> {
    Ok(())
}

pub fn import_validator_backup(
    backup: &Path,
    expected_network: &str,
    data_dir: &Path,
    recovery_dir: &Path,
) -> Result<ValidatorBackupSummary> {
    if data_dir.exists() && fs::read_dir(data_dir)?.next().is_some() {
        anyhow::bail!(
            "Validator restore data directory must be empty: {}",
            data_dir.display()
        );
    }
    if recovery_dir.exists() && fs::read_dir(recovery_dir)?.next().is_some() {
        anyhow::bail!(
            "Validator recovery directory must be empty: {}",
            recovery_dir.display()
        );
    }

    let password = backup_password()?;
    let mut reader = BufReader::new(File::open(backup)?);
    let mut magic = vec![0u8; BACKUP_V2_MAGIC.len()];
    match reader.read_exact(&mut magic) {
        Ok(()) if magic == BACKUP_V2_MAGIC => import_validator_backup_v2_reader(
            reader,
            expected_network,
            data_dir,
            recovery_dir,
            password.as_str(),
        ),
        Ok(()) | Err(_) => import_validator_backup_v1(
            backup,
            expected_network,
            data_dir,
            recovery_dir,
            password.as_str(),
        ),
    }
}

fn import_validator_backup_v1(
    backup: &Path,
    expected_network: &str,
    data_dir: &Path,
    recovery_dir: &Path,
    password: &str,
) -> Result<ValidatorBackupSummary> {
    let archive: EncryptedValidatorBackup = read_json_file(backup)?;
    if archive.format_version != BACKUP_FORMAT_VERSION {
        anyhow::bail!(
            "Unsupported validator backup format {}",
            archive.format_version
        );
    }
    let compressed = Zeroizing::new(
        kanari_crypto::decrypt_data(&archive.encrypted_payload, password)
            .map_err(|error| anyhow::anyhow!("Failed to decrypt validator backup: {error}"))?,
    );
    let actual_hash = hex::encode(Sha3_256::digest(&compressed));
    if actual_hash != archive.payload_sha3_256 {
        anyhow::bail!("Validator backup integrity checksum mismatch");
    }

    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut decoded = Zeroizing::new(Vec::new());
    decoder.read_to_end(&mut decoded)?;
    let (payload, consumed): (ValidatorBackupPayload, usize) =
        bincode::serde::decode_from_slice(&decoded, bincode::config::standard())?;
    if consumed != decoded.len() {
        anyhow::bail!("Validator backup payload contains trailing data");
    }
    if payload.format_version != BACKUP_FORMAT_VERSION {
        anyhow::bail!(
            "Unsupported validator payload format {}",
            payload.format_version
        );
    }
    if payload.network != expected_network {
        anyhow::bail!(
            "Validator backup network mismatch: expected {}, got {}",
            expected_network,
            payload.network
        );
    }

    fs::create_dir_all(data_dir)?;
    fs::create_dir_all(recovery_dir)?;
    let temp_dir = tempfile::tempdir()?;
    let snapshot_path = temp_dir.path().join("state-snapshot.json");
    fs::write(&snapshot_path, &payload.state_snapshot)?;
    let snapshot: StateSnapshot = serde_json::from_slice(&payload.state_snapshot)?;
    let imported = BlockchainEngine::import_trusted_state_snapshot(
        &snapshot_path,
        data_dir,
        expected_network,
    )?;

    for (name, bytes) in &payload.files {
        if let Some(wal_name) = name.strip_prefix("wal/") {
            safe_restore_file(data_dir, wal_name, bytes)?;
        } else if name == "identity/p2p-identity.key" {
            safe_restore_file(data_dir, "p2p-identity.key", bytes)?;
        } else if let Some(recovery_name) = name.strip_prefix("consensus/") {
            safe_restore_file(recovery_dir, recovery_name, bytes)?;
        } else if name == "network/genesis.json" {
            safe_restore_file(recovery_dir, "genesis.json", bytes)?;
        } else {
            anyhow::bail!("Unknown validator backup entry: {name}");
        }
    }

    if imported.checkpoint_height != snapshot.checkpoint_height
        || imported.state_root != snapshot.state_root
    {
        anyhow::bail!("Restored validator state does not match backup manifest");
    }

    Ok(ValidatorBackupSummary {
        checkpoint_height: imported.checkpoint_height,
        state_root: imported.state_root,
        included_files: payload.files.len(),
    })
}

fn import_validator_backup_v2_reader<R: Read>(
    mut reader: R,
    expected_network: &str,
    data_dir: &Path,
    recovery_dir: &Path,
    password: &str,
) -> Result<ValidatorBackupSummary> {
    let started = std::time::Instant::now();
    let header_json = read_len_prefixed(&mut reader, 1024 * 1024)?;
    let header: EncryptedValidatorBackupV2Header = serde_json::from_slice(&header_json)?;
    if header.format_version != BACKUP_FORMAT_VERSION_V2 {
        anyhow::bail!(
            "Unsupported validator backup format {}",
            header.format_version
        );
    }

    let decrypting_reader =
        kanari_crypto::StreamDecryptingReader::new(&header.encrypted_payload, reader, password)
            .map_err(|error| {
                anyhow::anyhow!("Failed to initialize streaming backup decryption: {error}")
            })?;
    let mut hashing_reader = HashingReader::new(decrypting_reader, MAX_BACKUP_V2_COMPRESSED_BYTES);
    let pending_restore = {
        let mut decoder = GzDecoder::new(&mut hashing_reader);
        let pending_restore =
            restore_payload_v2(&mut decoder, expected_network, data_dir, recovery_dir)?;
        io::copy(&mut decoder, &mut io::sink())?;
        pending_restore
    };
    let actual_hash = hashing_reader.digest_hex();
    if actual_hash != header.payload_sha3_256 {
        anyhow::bail!("Validator backup integrity checksum mismatch");
    }
    let summary = pending_restore.promote()?;
    info!(
        checkpoint_height = summary.checkpoint_height,
        included_files = summary.included_files,
        elapsed_ms = started.elapsed().as_millis(),
        "Imported streaming validator backup v2"
    );
    Ok(summary)
}

fn restore_payload_v2<R: Read>(
    reader: &mut R,
    expected_network: &str,
    data_dir: &Path,
    recovery_dir: &Path,
) -> Result<PendingValidatorRestore> {
    let mut magic = vec![0u8; BACKUP_V2_PAYLOAD_MAGIC.len()];
    reader.read_exact(&mut magic)?;
    if magic != BACKUP_V2_PAYLOAD_MAGIC {
        anyhow::bail!("Invalid validator backup v2 payload magic");
    }
    let manifest_json = read_len_prefixed(reader, 1024 * 1024)?;
    let manifest: ValidatorBackupV2Manifest = serde_json::from_slice(&manifest_json)?;
    if manifest.format_version != BACKUP_FORMAT_VERSION_V2 {
        anyhow::bail!(
            "Unsupported validator payload format {}",
            manifest.format_version
        );
    }
    if manifest.network != expected_network {
        anyhow::bail!(
            "Validator backup network mismatch: expected {}, got {}",
            expected_network,
            manifest.network
        );
    }
    if manifest.included_files > MAX_BACKUP_V2_RECORDS {
        anyhow::bail!("Validator backup v2 manifest entry count exceeds configured limit");
    }

    let data_staging = create_restore_staging_dir(data_dir, ".kanari-restore-data-")?;
    let recovery_staging = create_restore_staging_dir(recovery_dir, ".kanari-restore-recovery-")?;
    let snapshot_temp = tempfile::tempdir()?;
    let snapshot_path = snapshot_temp.path().join("state-snapshot.json");
    let staged_data_dir = data_staging.path().join("data-files");
    let staged_recovery_dir = recovery_staging.path().join("recovery-files");
    let mut restored_entries = 0usize;
    let mut restored_snapshot = false;
    let mut restored_records = 0usize;

    loop {
        let mut tag = [0u8; 1];
        reader.read_exact(&mut tag)?;
        if tag[0] == 0 {
            break;
        }
        if tag[0] != 1 {
            anyhow::bail!("Unknown validator backup v2 record tag {}", tag[0]);
        }
        restored_records += 1;
        if restored_records > MAX_BACKUP_V2_RECORDS {
            anyhow::bail!("Validator backup v2 record count exceeds configured limit");
        }
        let name_bytes = read_len_prefixed(reader, u16::MAX as usize)?;
        let name = String::from_utf8(name_bytes)?;
        validate_archive_name(&name)?;
        let mut len_bytes = [0u8; 8];
        reader.read_exact(&mut len_bytes)?;
        let entry_len = u64::from_be_bytes(len_bytes);
        if entry_len > MAX_BACKUP_V2_ENTRY_BYTES {
            anyhow::bail!("Validator backup v2 entry exceeds configured size limit: {name}");
        }
        let mut expected_hash = [0u8; 32];
        reader.read_exact(&mut expected_hash)?;

        if name == "state/state-snapshot.json" {
            safe_restore_stream(
                snapshot_temp.path(),
                "state-snapshot.json",
                reader.by_ref(),
                entry_len,
                expected_hash,
            )?;
            restored_snapshot = true;
        } else if let Some(wal_name) = name.strip_prefix("wal/") {
            safe_restore_stream(
                &staged_data_dir,
                wal_name,
                reader.by_ref(),
                entry_len,
                expected_hash,
            )?;
            restored_entries += 1;
        } else if name == "identity/p2p-identity.key" {
            safe_restore_stream(
                &staged_data_dir,
                "p2p-identity.key",
                reader.by_ref(),
                entry_len,
                expected_hash,
            )?;
            restored_entries += 1;
        } else if let Some(recovery_name) = name.strip_prefix("consensus/") {
            safe_restore_stream(
                &staged_recovery_dir,
                recovery_name,
                reader.by_ref(),
                entry_len,
                expected_hash,
            )?;
            restored_entries += 1;
        } else if name == "network/genesis.json" {
            safe_restore_stream(
                &staged_recovery_dir,
                "genesis.json",
                reader.by_ref(),
                entry_len,
                expected_hash,
            )?;
            restored_entries += 1;
        } else {
            anyhow::bail!("Unknown validator backup entry: {name}");
        }
    }

    if !restored_snapshot {
        anyhow::bail!("Validator backup v2 payload is missing state snapshot");
    }
    if restored_entries != manifest.included_files {
        anyhow::bail!(
            "Validator backup v2 entry count mismatch: expected {}, restored {}",
            manifest.included_files,
            restored_entries
        );
    }

    let snapshot: StateSnapshot = read_json_file(&snapshot_path)?;
    let imported = BlockchainEngine::import_trusted_state_snapshot(
        &snapshot_path,
        data_staging.path(),
        expected_network,
    )?;
    persist_staged_tree(&staged_data_dir, data_staging.path())?;
    persist_staged_tree(&staged_recovery_dir, recovery_staging.path())?;
    remove_restore_subtree(data_staging.path(), &staged_data_dir)?;
    remove_restore_subtree(recovery_staging.path(), &staged_recovery_dir)?;

    if imported.checkpoint_height != manifest.checkpoint_height
        || imported.state_root != manifest.state_root
        || imported.checkpoint_height != snapshot.checkpoint_height
        || imported.state_root != snapshot.state_root
    {
        anyhow::bail!("Restored validator state does not match backup manifest");
    }
    Ok(PendingValidatorRestore {
        data_staging,
        recovery_staging,
        data_dir: data_dir.to_path_buf(),
        recovery_dir: recovery_dir.to_path_buf(),
        summary: ValidatorBackupSummary {
            checkpoint_height: imported.checkpoint_height,
            state_root: imported.state_root,
            included_files: restored_entries,
        },
    })
}

#[cfg(test)]
#[path = "../tests/unit/validator_backup_tests.rs"]
mod tests;
