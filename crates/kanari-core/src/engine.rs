// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::blockchain::Blockchain;
use crate::consensus::Checkpoint;
use crate::file_io::{read_json_file, write_json_pretty_atomically};
use ahash::AHashMap;
use anyhow::{Context, Result, ensure};
use kanari_move_runtime_v1::changeset::{ChangeSet, CreatedObject, StateAccessSet};
use kanari_move_runtime_v1::move_runtime::{EntryFunctionObjectContext, MoveRuntime};
use kanari_move_runtime_v1::state::StateManager;
use kanari_move_runtime_v1::storage::persistent_store::PersistentStore;
use kanari_rpc_api::ObjectInfo;
use kanari_types::address::Address as KanariAddress;
use kanari_types::coin::CoinModule;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::gas_coin::GAS_COIN;

use kanari_types::transaction::{
    ObjectChange, ObjectChangeKind, ObjectOwnerKind, ObjectRef, SignedTransaction, Transaction,
    TransactionEffects,
};
use kanari_types::{GasMeter, GasOperation, effective_gas_price, gas_price_is_valid};
use log::{error, info};
use lru::LruCache;
use move_core_types::{
    account_address::AccountAddress,
    identifier::Identifier,
    language_storage::{ModuleId, StructTag, TypeTag},
};
use num_cpus;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::env;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

type ProofCache = LruCache<(u64, usize), (String, Vec<Vec<u8>>)>;
type RawCheckpointUpdates = Vec<(Vec<u8>, Vec<u8>)>;
type RawCheckpointDeletes = Vec<Vec<u8>>;

#[derive(Debug, Clone, serde::Deserialize)]
struct LegacyCheckpointMetadata {
    sequence: u64,
    vertices: Vec<[u8; 32]>,
    transactions: Vec<SignedTransaction>,
    state_root: Vec<u8>,
    timestamp: u64,
    prev_checkpoint_hash: Vec<u8>,
}

/// Portable identity shared by every node in a network.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct GenesisManifest {
    pub format_version: u32,
    pub network: String,
    pub protocol_version: String,
    pub state_schema_version: String,
    pub genesis_checkpoint_hash: String,
    pub genesis_state_root: String,
}

fn compatible_protocol_version(manifest: &str, local: &str) -> bool {
    let parse = |version: &str| {
        let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
        Some((parts.next()??, parts.next()??, parts.next()??))
    };
    match (parse(manifest), parse(local)) {
        (
            Some((manifest_major, manifest_minor, manifest_patch)),
            Some((local_major, local_minor, local_patch)),
        ) => {
            (manifest_major, manifest_minor) == (local_major, local_minor)
                && local_patch >= manifest_patch
        }
        _ => manifest == local,
    }
}

pub const GENESIS_MANIFEST_FORMAT_VERSION: u32 = 1;
pub const STATE_SCHEMA_VERSION: &str = "canonical-state-root-v1";

/// Returns the canonical representation used for a consensus authority ID.
///
/// Configuration files and command-line inputs may omit the `0x` prefix, but
/// the engine stores and compares authority IDs with it. Keeping this rule in
/// core means every caller uses the same identity when configuring consensus.
pub fn normalize_consensus_authority_id(authority_id: impl AsRef<str>) -> String {
    let authority_id = authority_id.as_ref();
    if authority_id.starts_with("0x") {
        authority_id.to_owned()
    } else {
        format!("0x{authority_id}")
    }
}

/// Decodes a hexadecimal value, accepting an optional `0x` prefix, and
/// verifies its byte length before the value is used as key material.
pub fn decode_hex_exact(label: &str, value: &str, expected_len: usize) -> Result<Vec<u8>> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    let bytes =
        hex::decode(value).map_err(|error| anyhow::anyhow!("Invalid {label} hex: {error}"))?;
    if bytes.len() != expected_len {
        anyhow::bail!(
            "Invalid {label} length: expected {expected_len} bytes, got {}",
            bytes.len()
        );
    }
    Ok(bytes)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StateSnapshot {
    pub format_version: u32,
    pub network: String,
    pub genesis: GenesisManifest,
    pub checkpoint_height: u64,
    pub checkpoint_hash: String,
    /// Root recorded in the historical checkpoint, before any explicit migration.
    pub checkpoint_state_root: String,
    pub state_root: String,
    /// True only when export was explicitly authorized to migrate a legacy root.
    pub state_root_migrated: bool,
    pub entries_hash: String,
    pub entries: Vec<SnapshotEntry>,
}

pub const STATE_SNAPSHOT_FORMAT_VERSION: u32 = 2;
/// Keeps a DAG vertex comfortably below the node's bounded gossip frame limit.
pub const MAX_DAG_VERTEX_TRANSACTIONS: usize = 512;
// Vertex JSON/hex encoding can approach 2x the BCS transaction bytes.
pub const MAX_DAG_VERTEX_TRANSACTION_BYTES: usize = 350 * 1024;
const DEFAULT_MAX_DAG_VERTEX_TXS_PER_HOT_OBJECT: usize = 64;
const DEFAULT_MAX_OWNED_FAST_CHECKPOINT_TRANSACTIONS: usize = 65_536;
pub const MAX_TRANSACTION_BYTES: usize = 256 * 1024;
pub const MAX_TRANSACTION_GAS_LIMIT: u64 = 10_000_000;

mod apply_checkpoint;
mod bootstrap;
mod mempool;
mod produce_dag_vertex;
mod queries;
mod runtime_guards;
pub use produce_dag_vertex::{
    CheckpointInfo, CheckpointProductionInfo, DagEngine, DagProductionPolicy, DagVertex,
};
pub use runtime_guards::{RuntimeGuardConfig, RuntimeHealthReport};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckpointSyncData {
    pub checkpoint: Checkpoint,
    /// Signed native Mysticeti vertices needed to reproduce the local commit.
    #[serde(default)]
    pub dag_vertices: Vec<DagVertex>,
}

const MAX_MEMPOOL_SIZE: usize = 100_000;
const MAX_MEMPOOL_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const MAX_PENDING_PER_PRIMARY_ACCESS_LANE: u64 = 64;
const MAX_PERSISTED_RECENT_TX_HASHES: usize = 100_000;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedTransactionLocation {
    checkpoint_sequence: u64,
    state_root: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct PendingTransactionMetadata {
    pub previewed: bool,
    pub preview_gas_used: Option<u64>,
    pub preview_effects: Option<TransactionEffects>,
}

#[derive(Debug, Clone)]
pub struct PendingTransactionRecord {
    pub signed_tx: SignedTransaction,
    pub metadata: PendingTransactionMetadata,
    tx_hash: Vec<u8>,
    normalized_sender: String,
    primary_access_key: String,
    congestion_access_key: String,
    access_keys: Vec<String>,
    size_bytes: usize,
}

#[derive(Debug, Default)]
pub(crate) struct MempoolState {
    pending_txs: Vec<PendingTransactionRecord>,
    pending_bytes: usize,
    pending_tx_hashes: HashSet<Vec<u8>>,
    pending_sender_counts: AHashMap<String, u64>,
    pending_access_counts: AHashMap<String, u64>,
    pending_primary_access_counts: AHashMap<String, u64>,
    pending_congestion_access_counts: AHashMap<String, u64>,
}

/// Complete blockchain engine with Move VM integration
pub struct BlockchainEngine {
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub state: Arc<RwLock<StateManager>>,
    mempool: Arc<RwLock<MempoolState>>,
    invalid_pending_drop_count: Arc<AtomicU64>,
    pub persistent_store: Option<Arc<PersistentStore>>,
    // Reusable pool of MoveRuntime instances for parallel execution
    pub runtime_pool: Vec<kanari_move_runtime_v1::move_runtime::MoveRuntime>,
    // LRU cache for frequently requested merkle proofs
    // Cache key: (block_height, tx_index), Value: (tx_hash, proof)
    pub proof_cache: Arc<RwLock<ProofCache>>,
    // DAG engine for high-throughput consensus (lazy-initialized)
    dag_engine: Arc<RwLock<Option<DagEngine>>>,
    // Authority ID for this node (used in DAG mode)
    authority_id: String,
    // List of all authorities (validators) in the network
    authorities: Vec<String>,
    // Optional production-safe DAG signing key. When absent, DAG mode uses
    // deterministic demo keys for tests/local development only.
    consensus_signing_key: Option<ed25519_dalek::SigningKey>,
    consensus_public_keys: BTreeMap<String, Vec<u8>>,
}

// Basic recursive parser for simple type-argument strings used by RPC/tests.
fn parse_type_tag(s: &str) -> Option<TypeTag> {
    fn split_top_level_commas(s: &str) -> Vec<&str> {
        let mut parts = Vec::new();
        let mut depth: usize = 0;
        let mut start = 0usize;
        for (i, ch) in s.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth = depth.saturating_sub(1);
                }
                ',' if depth == 0 => {
                    parts.push(s[start..i].trim());
                    start = i + 1;
                }
                _ => {}
            }
        }
        parts.push(s[start..].trim());
        parts
    }

    let s = s.trim();
    match s {
        "bool" => return Some(TypeTag::Bool),
        "u8" => return Some(TypeTag::U8),
        "u64" => return Some(TypeTag::U64),
        "u128" => return Some(TypeTag::U128),
        "address" => return Some(TypeTag::Address),
        _ => {}
    }

    if let Some(inner) = s.strip_prefix("vector<")
        && let Some(inner) = inner.strip_suffix('>')
    {
        return parse_type_tag(inner).map(|t| TypeTag::Vector(Box::new(t)));
    }

    if s.contains("::") {
        let parts = s.split("::").collect::<Vec<_>>();
        if parts.len() >= 3 {
            let addr_str = parts[0].trim();
            let module_str = parts[1].trim();
            let name_and_generics = parts[2..].join("::").trim().to_string();

            let (name_str, generics_opt) = if let Some(idx) = name_and_generics.find('<') {
                if !name_and_generics.ends_with('>') || idx + 1 >= name_and_generics.len() {
                    return None;
                }
                let name = &name_and_generics[..idx];
                let generics = &name_and_generics[idx + 1..name_and_generics.len() - 1];
                (name.trim(), Some(generics))
            } else {
                (name_and_generics.as_str(), None)
            };

            let addr = KanariAddress::parse_to_account_address(addr_str).ok()?;
            let module_id = Identifier::new(module_str).ok()?;
            let name_id = Identifier::new(name_str).ok()?;

            let mut type_params = Vec::new();
            if let Some(r#gen) = generics_opt {
                for g in split_top_level_commas(r#gen) {
                    if g.is_empty() {
                        continue;
                    }
                    let parsed = parse_type_tag(g)?;
                    type_params.push(parsed);
                }
            }

            let st = StructTag {
                address: addr,
                module: module_id,
                name: name_id,
                type_params,
            };
            return Some(TypeTag::Struct(Box::new(st)));
        }
    }

    None
}

impl BlockchainEngine {
    fn snapshot_entries_hash(entries: &[SnapshotEntry]) -> Result<String> {
        Ok(hex::encode(
            blake3::hash(&bcs::to_bytes(entries)?).as_bytes(),
        ))
    }

    pub fn export_state_snapshot(
        &self,
        path: &std::path::Path,
        network: impl Into<String>,
    ) -> Result<StateSnapshot> {
        self.export_state_snapshot_with_options(path, network, false)
    }

    pub fn export_state_snapshot_with_options(
        &self,
        path: &std::path::Path,
        network: impl Into<String>,
        allow_state_root_migration: bool,
    ) -> Result<StateSnapshot> {
        let network = network.into();
        let genesis = self.genesis_manifest(network.clone())?;
        let checkpoint = self
            .blockchain
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .latest_checkpoint()
            .clone();
        let state = self.state_read();
        ensure!(
            state.overlay.is_empty(),
            "Cannot export snapshot while state has uncommitted changes"
        );
        state
            .store
            .flush()
            .context("Failed to flush state before snapshot")?;
        let current_state_root = state.try_compute_state_root()?;
        let state_root_migrated =
            checkpoint.sequence > 0 && checkpoint.state_root != current_state_root;
        ensure!(
            !state_root_migrated || allow_state_root_migration,
            "Cannot export snapshot: checkpoint state root does not match committed state (height={}, checkpoint={}, computed={}); rerun with --allow-state-root-migration only after auditing the legacy database",
            checkpoint.sequence,
            hex::encode(&checkpoint.state_root),
            hex::encode(&current_state_root)
        );
        if state_root_migrated {
            log::warn!(
                "[SNAPSHOT] Explicitly migrating legacy state root at height {}: checkpoint={}, computed={}",
                checkpoint.sequence,
                hex::encode(&checkpoint.state_root),
                hex::encode(&current_state_root)
            );
        }
        let entries = state
            .store
            .logical_entries()
            .context("Failed to read state entries for snapshot")?
            .into_iter()
            .map(|(key, value)| SnapshotEntry {
                key: hex::encode(key),
                value: hex::encode(value),
            })
            .collect::<Vec<_>>();
        let snapshot = StateSnapshot {
            format_version: STATE_SNAPSHOT_FORMAT_VERSION,
            network,
            genesis,
            checkpoint_height: checkpoint.sequence,
            checkpoint_hash: hex::encode(checkpoint.hash()?),
            checkpoint_state_root: hex::encode(&checkpoint.state_root),
            state_root: hex::encode(current_state_root),
            state_root_migrated,
            entries_hash: Self::snapshot_entries_hash(&entries)?,
            entries,
        };
        write_json_pretty_atomically(path, &snapshot)
            .with_context(|| format!("Failed to write state snapshot {}", path.display()))?;
        Ok(snapshot)
    }

    pub fn read_state_snapshot(path: &std::path::Path) -> Result<StateSnapshot> {
        read_json_file(path).with_context(|| format!("Invalid state snapshot {}", path.display()))
    }

    pub fn import_state_snapshot(
        snapshot_path: &std::path::Path,
        target_dir: &std::path::Path,
        network: &str,
        expected_checkpoint_hash: &str,
    ) -> Result<StateSnapshot> {
        Self::import_state_snapshot_inner(
            snapshot_path,
            target_dir,
            network,
            Some(expected_checkpoint_hash),
        )
    }

    /// Import a snapshot whose transport authenticity was already established
    /// by an authenticated container (for example validator backup AEAD) or a
    /// local trusted workflow. Public snapshot import must use
    /// `import_state_snapshot` and pin the checkpoint hash out of band.
    pub fn import_trusted_state_snapshot(
        snapshot_path: &std::path::Path,
        target_dir: &std::path::Path,
        network: &str,
    ) -> Result<StateSnapshot> {
        Self::import_state_snapshot_inner(snapshot_path, target_dir, network, None)
    }

    fn import_state_snapshot_inner(
        snapshot_path: &std::path::Path,
        target_dir: &std::path::Path,
        network: &str,
        expected_checkpoint_hash: Option<&str>,
    ) -> Result<StateSnapshot> {
        let snapshot = Self::read_state_snapshot(snapshot_path)?;
        ensure!(
            snapshot.format_version == STATE_SNAPSHOT_FORMAT_VERSION,
            "Unsupported state snapshot format: {}",
            snapshot.format_version
        );
        ensure!(
            snapshot.network == network,
            "Snapshot network mismatch: snapshot={}, local={}",
            snapshot.network,
            network
        );
        if let Some(expected) = expected_checkpoint_hash {
            let normalize = |hash: &str| hash.trim().trim_start_matches("0x").to_ascii_lowercase();
            ensure!(
                normalize(&snapshot.checkpoint_hash) == normalize(expected),
                "Snapshot checkpoint hash does not match the externally trusted hash"
            );
        }
        let verifier = Self::new_in_memory()?;
        verifier.validate_genesis_manifest(&snapshot.genesis, network)?;
        ensure!(
            snapshot.entries_hash == Self::snapshot_entries_hash(&snapshot.entries)?,
            "State snapshot entries hash mismatch"
        );
        let checkpoint_state_root = hex::decode(&snapshot.checkpoint_state_root)
            .with_context(|| "Invalid checkpoint state root in snapshot")?;
        let state_root =
            hex::decode(&snapshot.state_root).with_context(|| "Invalid state root in snapshot")?;
        ensure!(
            snapshot.checkpoint_height == 0
                || snapshot.state_root_migrated
                || checkpoint_state_root == state_root,
            "Snapshot root metadata is inconsistent"
        );

        let mut updates = Vec::with_capacity(snapshot.entries.len());
        for entry in &snapshot.entries {
            let key = hex::decode(&entry.key)
                .with_context(|| format!("Invalid snapshot key {}", entry.key))?;
            let value = hex::decode(&entry.value)
                .with_context(|| format!("Invalid snapshot value for key {}", entry.key))?;
            updates.push((key, value));
        }
        let store = PersistentStore::open_with_path(Some(target_dir.to_path_buf()))?;
        ensure!(
            store.logical_entries()?.is_empty(),
            "Refusing to import snapshot into a non-empty data directory"
        );
        store.apply_raw_changes(&updates, &[])?;
        drop(store);

        let imported = Self::new_dir(
            target_dir
                .to_str()
                .context("Invalid target data directory")?,
        )?;
        let chain = imported
            .blockchain
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let checkpoint = chain.latest_checkpoint();
        ensure!(
            checkpoint.sequence == snapshot.checkpoint_height,
            "Imported snapshot height mismatch"
        );
        ensure!(
            hex::encode(checkpoint.hash()?) == snapshot.checkpoint_hash,
            "Imported snapshot checkpoint hash mismatch"
        );
        ensure!(
            checkpoint.state_root == checkpoint_state_root,
            "Imported snapshot checkpoint state root mismatch"
        );
        let imported_state_root = hex::encode(imported.state_read().try_compute_state_root()?);
        ensure!(
            imported_state_root == snapshot.state_root,
            "Imported snapshot state root mismatch: snapshot={}, imported={}",
            snapshot.state_root,
            imported_state_root
        );
        Ok(snapshot)
    }

    pub fn genesis_manifest(&self, network: impl Into<String>) -> Result<GenesisManifest> {
        let genesis = self.get_block(0).context("Genesis checkpoint is missing")?;
        Ok(GenesisManifest {
            format_version: GENESIS_MANIFEST_FORMAT_VERSION,
            network: network.into(),
            protocol_version: env!("CARGO_PKG_VERSION").to_string(),
            state_schema_version: STATE_SCHEMA_VERSION.to_string(),
            genesis_checkpoint_hash: genesis.hash,
            genesis_state_root: genesis.state_root,
        })
    }

    pub fn validate_genesis_manifest(
        &self,
        manifest: &GenesisManifest,
        network: &str,
    ) -> Result<()> {
        ensure!(
            manifest.format_version == GENESIS_MANIFEST_FORMAT_VERSION,
            "Unsupported genesis manifest format: {}",
            manifest.format_version
        );
        ensure!(
            manifest.network == network,
            "Genesis network mismatch: manifest={}, local={}",
            manifest.network,
            network
        );
        ensure!(
            compatible_protocol_version(&manifest.protocol_version, env!("CARGO_PKG_VERSION")),
            "Genesis protocol version incompatible: manifest={}, local={}",
            manifest.protocol_version,
            env!("CARGO_PKG_VERSION")
        );
        ensure!(
            manifest.state_schema_version == STATE_SCHEMA_VERSION,
            "Genesis state schema mismatch: manifest={}, local={}",
            manifest.state_schema_version,
            STATE_SCHEMA_VERSION
        );
        let local = self.genesis_manifest(network)?;
        ensure!(
            manifest.genesis_checkpoint_hash == local.genesis_checkpoint_hash,
            "Genesis checkpoint hash mismatch: manifest={}, local={}",
            manifest.genesis_checkpoint_hash,
            local.genesis_checkpoint_hash
        );
        ensure!(
            manifest.genesis_state_root == local.genesis_state_root,
            "Genesis state root mismatch: manifest={}, local={}",
            manifest.genesis_state_root,
            local.genesis_state_root
        );
        Ok(())
    }

    pub fn write_genesis_manifest(
        &self,
        path: &std::path::Path,
        network: impl Into<String>,
    ) -> Result<()> {
        let manifest = self.genesis_manifest(network)?;
        write_json_pretty_atomically(path, &manifest)
            .with_context(|| format!("Failed to write genesis manifest {}", path.display()))?;
        Ok(())
    }

    pub fn read_genesis_manifest(path: &std::path::Path) -> Result<GenesisManifest> {
        read_json_file(path).with_context(|| format!("Invalid genesis manifest {}", path.display()))
    }

    fn is_native_gas_coin_type(object_type: &str) -> bool {
        CoinModule::is_coin_type_for(object_type, GAS_COIN)
    }

    fn execute_backend_native_burn(
        state: &StateManager,
        tx: &Transaction,
        sender_addr: AccountAddress,
        amount: u64,
        gas_cost: u64,
        changeset: &mut ChangeSet,
    ) -> Result<()> {
        let gas_payment = tx
            .gas_payment()
            .context("Native burn requires prepared gas payment")?;
        let gas_object_ref = gas_payment
            .payment_objects
            .first()
            .context("Native burn requires one prepared gas payment object")?;
        let gas_object = state
            .get_object(&gas_object_ref.object_id)?
            .with_context(|| {
                format!(
                    "Native burn gas object {} was not found in state",
                    gas_object_ref.object_id
                )
            })?;

        ensure!(
            Self::is_native_gas_coin_type(&gas_object.type_),
            "Native burn gas object {} must be Coin<{}>, found {}",
            gas_object_ref.object_id,
            GAS_COIN,
            gas_object.type_
        );
        ensure!(
            gas_object.owner == sender_addr,
            "Native burn gas object {} is not owned by sender {}",
            gas_object_ref.object_id,
            sender_addr.to_hex_literal()
        );

        let object_balance = CoinModule::read_balance(&gas_object.data).with_context(|| {
            format!(
                "Native burn gas object {} has invalid coin bytes",
                gas_object_ref.object_id
            )
        })?;
        let total_required = amount
            .checked_add(gas_cost)
            .context("Native burn amount + gas overflowed u64")?;
        ensure!(
            object_balance >= total_required,
            "Native burn requires one Coin<{}> object with at least {} Mist for burn + gas; selected object {} only has {} Mist",
            GAS_COIN,
            total_required,
            gas_object_ref.object_id,
            object_balance
        );

        changeset.burn(sender_addr, amount);

        if object_balance == total_required {
            changeset
                .deleted_objects
                .push(gas_object_ref.object_id.clone());
        } else {
            changeset.created_objects.push((
                gas_object_ref.object_id.clone(),
                CreatedObject {
                    owner: gas_object.owner,
                    owner_kind: gas_object.owner_kind,
                    uid: gas_object.uid,
                    id: gas_object.id,
                    type_: gas_object.type_,
                    data: gas_object.data,
                    version: gas_object.version,
                },
            ));
        }

        Ok(())
    }

    fn checkpoint_transactions_key(sequence: u64) -> Vec<u8> {
        format!("checkpoint_txs/{sequence:020}").into_bytes()
    }

    fn checkpoint_metadata_key(sequence: u64) -> Vec<u8> {
        format!("checkpoint_meta/{sequence:020}").into_bytes()
    }

    fn checkpoint_metadata_json_key(sequence: u64) -> Vec<u8> {
        format!("checkpoint_meta_json_v1/{sequence:020}").into_bytes()
    }

    pub(crate) fn blockchain_json_key() -> &'static [u8] {
        b"blockchain_json_v1"
    }

    pub(crate) fn pending_checkpoint_commit_key() -> &'static [u8] {
        b"runtime:pending_checkpoint_commit_v1"
    }

    fn transaction_payload_key(tx_hash: &[u8]) -> Vec<u8> {
        let mut key = b"tx_payload/".to_vec();
        key.extend_from_slice(hex::encode(tx_hash).as_bytes());
        key
    }

    fn transaction_index_key(tx_hash: &[u8]) -> Vec<u8> {
        let mut key = b"tx_index/".to_vec();
        key.extend_from_slice(hex::encode(tx_hash).as_bytes());
        key
    }

    fn recent_transaction_hashes_key() -> &'static [u8] {
        b"tx_recent"
    }

    fn history_pruned_through_key() -> &'static [u8] {
        b"runtime:history_pruned_through"
    }

    fn history_persisted_through_key() -> &'static [u8] {
        b"runtime:history_persisted_through"
    }

    fn history_retention_checkpoints() -> Option<u64> {
        std::env::var("KANARI_HISTORY_RETENTION_CHECKPOINTS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
    }

    fn persist_transaction_hash_index_enabled(store: &PersistentStore) -> bool {
        if let Ok(value) = std::env::var("KANARI_PERSIST_TX_HASH_INDEX") {
            return matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES");
        }

        // Retention/pruning deletes checkpoint transaction payloads, so keep the
        // permanent hash index when pruning is configured. In-memory tests also
        // retain the old behavior. RocksDB production without pruning can use
        // checkpoint transaction batches plus the in-memory blockchain index on
        // restart, avoiding 1 write per transaction in the checkpoint critical path.
        Self::history_retention_checkpoints().is_some() || store.get_db().is_none()
    }

    fn persist_blockchain_json_snapshot_enabled(store: &PersistentStore) -> bool {
        if let Ok(value) = std::env::var("KANARI_PERSIST_BLOCKCHAIN_JSON") {
            return matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES");
        }

        // RocksDB production already persists per-checkpoint metadata and tx
        // batches. Rewriting the whole retained chain snapshot every checkpoint
        // duplicates large effect/object metadata on the hot path. In-memory
        // stores keep the legacy behavior for unit tests and simple embedding.
        store.get_db().is_none()
    }

    fn checkpoint_without_transactions(checkpoint: &Checkpoint) -> Checkpoint {
        let mut stripped = Checkpoint::new(
            checkpoint.sequence,
            checkpoint.vertices.clone(),
            Vec::new(),
            checkpoint.state_root.clone(),
            checkpoint.timestamp,
            checkpoint.prev_checkpoint_hash.clone(),
        )
        .with_transaction_effects(checkpoint.transaction_effects.clone())
        .with_object_changes(checkpoint.object_changes.clone())
        .with_object_graph_edges(checkpoint.object_graph_edges.clone());
        stripped.hash_version = checkpoint.hash_version;
        stripped
    }

    fn persist_checkpoint_transactions(
        store: &PersistentStore,
        checkpoint: &Checkpoint,
    ) -> Result<()> {
        let (updates, deletes) =
            Self::checkpoint_persistence_raw_changes(store, checkpoint, false)?;
        store
            .apply_raw_changes(&updates, &deletes)
            .context("Failed to persist checkpoint transaction metadata")?;
        Ok(())
    }

    pub(crate) fn checkpoint_persistence_raw_changes(
        store: &PersistentStore,
        checkpoint: &Checkpoint,
        mark_history_persisted: bool,
    ) -> Result<(RawCheckpointUpdates, RawCheckpointDeletes)> {
        let mut updates = RawCheckpointUpdates::new();
        updates.push((
            Self::checkpoint_metadata_key(checkpoint.sequence),
            bcs::to_bytes(&Self::checkpoint_without_transactions(checkpoint))
                .context("Failed to encode checkpoint metadata")?,
        ));
        let deletes = vec![Self::checkpoint_metadata_json_key(checkpoint.sequence)];

        if checkpoint.transactions.is_empty() || checkpoint.sequence == 0 {
            if mark_history_persisted {
                updates.push((
                    Self::history_persisted_through_key().to_vec(),
                    bcs::to_bytes(&checkpoint.sequence)
                        .context("Failed to encode history persisted marker")?,
                ));
            }
            return Ok((updates, deletes));
        }

        let mut recent_hashes = Self::load_recent_transaction_hashes_for_update(store)?;
        let mut recent_set: HashSet<Vec<u8>> = recent_hashes.iter().cloned().collect();
        let persist_hash_index = Self::persist_transaction_hash_index_enabled(store);

        for tx in checkpoint.transactions.iter() {
            let tx_hash = tx.transaction_hash().to_vec();
            if persist_hash_index {
                updates.push((
                    Self::transaction_index_key(&tx_hash),
                    bcs::to_bytes(&PersistedTransactionLocation {
                        checkpoint_sequence: checkpoint.sequence,
                        state_root: checkpoint.state_root.clone(),
                    })
                    .context("Failed to encode transaction hash index")?,
                ));
            }

            if recent_set.insert(tx_hash.clone()) {
                recent_hashes.push(tx_hash);
            }
        }

        if recent_hashes.len() > MAX_PERSISTED_RECENT_TX_HASHES {
            let trim = recent_hashes.len() - MAX_PERSISTED_RECENT_TX_HASHES;
            recent_hashes.drain(0..trim);
        }
        updates.push((
            Self::recent_transaction_hashes_key().to_vec(),
            bcs::to_bytes(&recent_hashes).context("Failed to encode recent transaction index")?,
        ));
        updates.push((
            Self::checkpoint_transactions_key(checkpoint.sequence),
            bcs::to_bytes(&checkpoint.transactions)
                .context("Failed to encode checkpoint transaction payload")?,
        ));
        if mark_history_persisted {
            updates.push((
                Self::history_persisted_through_key().to_vec(),
                bcs::to_bytes(&checkpoint.sequence)
                    .context("Failed to encode history persisted marker")?,
            ));
        }

        Ok((updates, deletes))
    }

    fn load_recent_transaction_hashes_for_update(store: &PersistentStore) -> Result<Vec<Vec<u8>>> {
        store
            .load::<Vec<Vec<u8>>>(Self::recent_transaction_hashes_key())
            .context("Failed to load recent transaction index")?
            .map(|hashes| {
                hashes
                    .into_iter()
                    .map(|hash| {
                        anyhow::ensure!(
                            hash.len() == 32,
                            "Recent transaction index contains invalid hash length {}",
                            hash.len()
                        );
                        Ok(hash)
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()
            .map(|hashes| hashes.unwrap_or_default())
    }

    fn load_checkpoint_metadata(store: &PersistentStore, sequence: u64) -> Option<Checkpoint> {
        if let Ok(Some(json)) = store.load::<Vec<u8>>(&Self::checkpoint_metadata_json_key(sequence))
        {
            match serde_json::from_slice::<Checkpoint>(&json) {
                Ok(checkpoint) => return Some(checkpoint),
                Err(error) => tracing::warn!(
                    checkpoint = sequence,
                    "Failed to decode JSON checkpoint metadata: {}",
                    error
                ),
            }
        }

        let key = Self::checkpoint_metadata_key(sequence);
        match store.load::<Checkpoint>(&key) {
            Ok(Some(checkpoint)) => Some(checkpoint),
            Ok(None) => None,
            Err(current_error) => {
                tracing::warn!(
                    checkpoint = sequence,
                    "Current checkpoint metadata schema failed; trying legacy schema: {}",
                    current_error
                );
                match store.load::<LegacyCheckpointMetadata>(&key) {
                    Ok(Some(legacy)) => Some(Checkpoint::new(
                        legacy.sequence,
                        legacy.vertices,
                        legacy.transactions,
                        legacy.state_root,
                        legacy.timestamp,
                        legacy.prev_checkpoint_hash,
                    )),
                    Ok(None) => None,
                    Err(legacy_error) => {
                        tracing::warn!(
                            checkpoint = sequence,
                            "Failed to load legacy checkpoint metadata: {}",
                            legacy_error
                        );
                        None
                    }
                }
            }
        }
    }

    fn load_checkpoint_transactions(
        store: &PersistentStore,
        sequence: u64,
    ) -> Option<Arc<[SignedTransaction]>> {
        store
            .load(&Self::checkpoint_transactions_key(sequence))
            .map_err(|e| {
                tracing::warn!(
                    checkpoint = sequence,
                    "Failed to load checkpoint transaction payload: {}",
                    e
                );
                e
            })
            .ok()
            .flatten()
    }

    fn load_transaction_by_hash_from_index(
        store: &PersistentStore,
        tx_hash: &[u8],
    ) -> Option<(SignedTransaction, PersistedTransactionLocation)> {
        let location = store
            .load::<PersistedTransactionLocation>(&Self::transaction_index_key(tx_hash))
            .map_err(|e| {
                tracing::warn!(
                    tx_hash = %hex::encode(tx_hash),
                    "Failed to load transaction index: {}",
                    e
                );
                e
            })
            .ok()
            .flatten()?;
        let tx = store
            .load::<SignedTransaction>(&Self::transaction_payload_key(tx_hash))
            .map_err(|e| {
                tracing::warn!(
                    tx_hash = %hex::encode(tx_hash),
                    "Failed to load transaction payload: {}",
                    e
                );
                e
            })
            .ok()
            .flatten()
            .or_else(|| {
                Self::load_checkpoint_transactions(store, location.checkpoint_sequence).and_then(
                    |transactions| {
                        transactions
                            .iter()
                            .find(|tx| tx.transaction_hash() == tx_hash)
                            .cloned()
                    },
                )
            })?;
        Some((tx, location))
    }

    fn persist_blockchain_snapshot_to_store(
        store: &PersistentStore,
        chain: &Blockchain,
    ) -> Result<()> {
        let persisted_through = store
            .load::<u64>(Self::history_persisted_through_key())?
            .map(|sequence| sequence.min(chain.height()));
        for checkpoint in chain.dag_checkpoints.iter().filter(|checkpoint| {
            persisted_through.is_none_or(|sequence| checkpoint.sequence > sequence)
        }) {
            Self::persist_checkpoint_transactions(store, checkpoint)?;
        }

        store.save(Self::history_persisted_through_key(), &chain.height())?;

        if Self::persist_blockchain_json_snapshot_enabled(store) {
            let mut slim = chain.clone();
            for checkpoint in &mut slim.dag_checkpoints {
                if !checkpoint.transactions.is_empty() {
                    *checkpoint = Self::checkpoint_without_transactions(checkpoint);
                }
            }
            let blockchain_json = serde_json::to_vec(&slim)
                .context("Failed to encode blockchain metadata as JSON")?;
            store
                .save(Self::blockchain_json_key(), &blockchain_json)
                .context("Failed to persist blockchain metadata")?;
        } else {
            store
                .delete(Self::blockchain_json_key())
                .context("Failed to clear stale blockchain JSON snapshot")?;
        }
        if let Some(retention) = Self::history_retention_checkpoints() {
            Self::prune_transaction_payloads(store, chain.height().saturating_sub(retention))?;
        }
        Ok(())
    }

    /// Delete old transaction payloads while retaining checkpoint metadata and
    /// transaction hash indexes. The retained hash index remains the permanent
    /// replay guard even when an archival node no longer serves the payload.
    fn prune_transaction_payloads(store: &PersistentStore, through: u64) -> Result<()> {
        if through == 0 {
            return Ok(());
        }
        let previous = store
            .load::<u64>(Self::history_pruned_through_key())?
            .unwrap_or(0);
        if through <= previous {
            return Ok(());
        }

        let mut pruned_hashes = HashSet::new();
        for sequence in previous.saturating_add(1)..=through {
            if let Some(transactions) = Self::load_checkpoint_transactions(store, sequence) {
                for tx in transactions.iter() {
                    let hash = tx.transaction_hash().to_vec();
                    store.delete(&Self::transaction_payload_key(&hash))?;
                    pruned_hashes.insert(hash);
                }
            }
            store.delete(&Self::checkpoint_transactions_key(sequence))?;
        }

        if !pruned_hashes.is_empty() {
            let recent = Self::load_recent_transaction_hashes_for_update(store)?
                .into_iter()
                .filter(|hash| !pruned_hashes.contains(hash))
                .collect::<Vec<_>>();
            store.save(Self::recent_transaction_hashes_key(), &recent)?;
        }
        store.save(Self::history_pruned_through_key(), &through)?;
        Ok(())
    }

    pub(crate) fn persist_blockchain_snapshot(&self, chain: &Blockchain) -> Result<()> {
        let Some(store) = &self.persistent_store else {
            return Ok(());
        };
        Self::persist_blockchain_snapshot_to_store(store, chain)
    }

    fn hydrate_blockchain_transactions(store: &PersistentStore, chain: &mut Blockchain) {
        for checkpoint in &mut chain.dag_checkpoints {
            if !checkpoint.transactions.is_empty() || checkpoint.sequence == 0 {
                continue;
            }
            if let Some(transactions) =
                Self::load_checkpoint_transactions(store, checkpoint.sequence)
            {
                checkpoint.transactions = transactions;
            }
        }
    }

    pub fn get_committed_transaction_from_history(
        &self,
        tx_hash: &[u8],
    ) -> Option<(SignedTransaction, u64, Vec<u8>)> {
        let store = self.persistent_store.as_ref()?;
        if let Some((tx, location)) = Self::load_transaction_by_hash_from_index(store, tx_hash) {
            return Some((tx, location.checkpoint_sequence, location.state_root));
        }

        let height = self.get_stats().height;

        for sequence in (1..=height).rev() {
            let Some(transactions) = Self::load_checkpoint_transactions(store, sequence) else {
                continue;
            };
            for tx in transactions.iter().rev() {
                if tx.transaction_hash() == tx_hash {
                    let state_root = Self::load_checkpoint_metadata(store, sequence)
                        .map(|checkpoint| checkpoint.state_root)
                        .unwrap_or_default();
                    return Some((tx.clone(), sequence, state_root));
                }
            }
        }

        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        for checkpoint in chain.dag_checkpoints.iter().rev() {
            if checkpoint.sequence == 0 {
                break;
            }
            for tx in checkpoint.transactions.iter().rev() {
                if tx.transaction_hash() == tx_hash {
                    return Some((
                        tx.clone(),
                        checkpoint.sequence,
                        checkpoint.state_root.clone(),
                    ));
                }
            }
        }

        None
    }

    pub fn get_committed_transaction_effect_from_history(
        &self,
        tx_hash: &[u8],
    ) -> Option<kanari_types::transaction::TransactionEffects> {
        if let Some(store) = self.persistent_store.as_ref()
            && let Some((_, location)) = Self::load_transaction_by_hash_from_index(store, tx_hash)
            && let Some(transactions) =
                Self::load_checkpoint_transactions(store, location.checkpoint_sequence)
            && let Some(index) = transactions
                .iter()
                .position(|tx| tx.transaction_hash() == tx_hash)
            && let Some(checkpoint) =
                Self::load_checkpoint_metadata(store, location.checkpoint_sequence)
            && let Some(effect) = checkpoint.transaction_effects.get(index)
        {
            return Some(effect.clone());
        }

        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain
            .get_transaction_location_with_effect(tx_hash)
            .and_then(|(_, _, _, effect)| effect.cloned())
    }

    pub(crate) fn try_is_transaction_committed(&self, tx_hash: &[u8]) -> Result<bool> {
        if self
            .blockchain
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_transaction_hash_executed(tx_hash)
        {
            return Ok(true);
        }
        let Some(store) = self.persistent_store.as_ref() else {
            return Ok(false);
        };
        Ok(store
            .load::<PersistedTransactionLocation>(&Self::transaction_index_key(tx_hash))
            .with_context(|| {
                format!(
                    "Failed to load committed transaction index for {}",
                    hex::encode(tx_hash)
                )
            })?
            .is_some())
    }

    pub fn list_committed_transactions_from_history<F>(
        &self,
        limit: usize,
        mut matches: F,
    ) -> Vec<(SignedTransaction, u64, Vec<u8>)>
    where
        F: FnMut(&Transaction) -> bool,
    {
        let Some(store) = self.persistent_store.as_ref() else {
            return Vec::new();
        };
        let mut results = Vec::with_capacity(limit);
        let mut seen_hashes = HashSet::new();

        if let Ok(Some(recent_hashes)) =
            store.load::<Vec<Vec<u8>>>(Self::recent_transaction_hashes_key())
        {
            for tx_hash in recent_hashes.iter().rev() {
                if results.len() >= limit {
                    break;
                }
                let Some((tx, location)) =
                    Self::load_transaction_by_hash_from_index(store, tx_hash)
                else {
                    continue;
                };
                if !seen_hashes.insert(tx_hash.clone()) {
                    continue;
                }
                if matches(&tx.transaction) {
                    results.push((tx, location.checkpoint_sequence, location.state_root));
                }
            }
        }

        if results.len() >= limit {
            return results;
        }

        let height = self.get_stats().height;

        for sequence in (1..=height).rev() {
            if results.len() >= limit {
                break;
            }

            let Some(transactions) = Self::load_checkpoint_transactions(store, sequence) else {
                continue;
            };
            let state_root = Self::load_checkpoint_metadata(store, sequence)
                .map(|checkpoint| checkpoint.state_root)
                .unwrap_or_default();

            for tx in transactions.iter().rev() {
                if results.len() >= limit {
                    break;
                }
                if !seen_hashes.insert(tx.transaction_hash().to_vec()) {
                    continue;
                }
                if matches(&tx.transaction) {
                    results.push((tx.clone(), sequence, state_root.clone()));
                }
            }
        }

        if results.len() < limit {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            for checkpoint in chain.dag_checkpoints.iter().rev() {
                if checkpoint.sequence == 0 || results.len() >= limit {
                    break;
                }

                for tx in checkpoint.transactions.iter().rev() {
                    if results.len() >= limit {
                        break;
                    }

                    let tx_hash = tx.transaction_hash().to_vec();
                    if !seen_hashes.insert(tx_hash) {
                        continue;
                    }

                    if matches(&tx.transaction) {
                        results.push((
                            tx.clone(),
                            checkpoint.sequence,
                            checkpoint.state_root.clone(),
                        ));
                    }
                }
            }
        }

        results
    }

    pub fn state_read(&self) -> RwLockReadGuard<'_, StateManager> {
        self.state.read().unwrap_or_else(|poisoned| {
            error!("State lock poisoned while reading runtime state; recovering...");
            poisoned.into_inner()
        })
    }

    pub fn state_write(&self) -> RwLockWriteGuard<'_, StateManager> {
        self.state.write().unwrap_or_else(|poisoned| {
            error!("State lock poisoned while writing runtime state; recovering...");
            poisoned.into_inner()
        })
    }

    pub(crate) fn mempool_read(&self) -> RwLockReadGuard<'_, MempoolState> {
        self.mempool.read().unwrap_or_else(|poisoned| {
            error!("Mempool lock poisoned while reading pending state; recovering...");
            poisoned.into_inner()
        })
    }

    pub(crate) fn mempool_write(&self) -> RwLockWriteGuard<'_, MempoolState> {
        self.mempool.write().unwrap_or_else(|poisoned| {
            error!("Mempool lock poisoned while writing pending state; recovering...");
            poisoned.into_inner()
        })
    }

    pub fn find_pending_transaction(
        &self,
        transaction_hash: &[u8],
    ) -> Option<PendingTransactionRecord> {
        self.mempool_read()
            .pending_txs
            .iter()
            .find(|record| record.signed_tx.transaction_hash() == transaction_hash)
            .cloned()
    }

    pub fn filter_pending_transaction_records<F>(
        &self,
        limit: usize,
        mut predicate: F,
    ) -> Vec<PendingTransactionRecord>
    where
        F: FnMut(&PendingTransactionRecord) -> bool,
    {
        self.mempool_read()
            .pending_txs
            .iter()
            .rev()
            .filter(|record| predicate(record))
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn pending_transactions_snapshot(&self) -> Vec<SignedTransaction> {
        self.pending_signed_transactions()
    }

    fn pending_signed_transactions(&self) -> Vec<SignedTransaction> {
        self.mempool_read()
            .pending_txs
            .iter()
            .map(|record| record.signed_tx.clone())
            .collect()
    }

    pub(crate) fn select_conflict_free_transactions(
        transactions: Vec<SignedTransaction>,
    ) -> Vec<SignedTransaction> {
        let mut selected = Vec::with_capacity(transactions.len());
        let mut reserved_keys = HashSet::new();

        for signed_tx in transactions {
            let mut conflict_keys = signed_tx.transaction.get_conflict_keys();
            conflict_keys.sort();
            conflict_keys.dedup();

            if conflict_keys
                .iter()
                .any(|conflict_key| reserved_keys.contains(conflict_key))
            {
                continue;
            }

            reserved_keys.extend(conflict_keys);
            selected.push(signed_tx);
        }

        selected
    }

    fn transaction_lane_rank(tx: &SignedTransaction) -> u8 {
        let object_inputs = tx.transaction.object_inputs();
        if object_inputs
            .iter()
            .any(|input| matches!(input.owner, Some(ObjectOwnerKind::Shared)))
        {
            return 2;
        }
        if object_inputs.is_empty() {
            return 0;
        }
        1
    }

    pub(crate) fn congestion_access_key(tx: &Transaction) -> String {
        if let Some(input) = tx.object_inputs().into_iter().find(|input| input.mutable) {
            return format!("object:{}", input.object_ref.object_id);
        }

        if let Some(input) = tx.object_inputs().into_iter().next() {
            return format!("object:{}", input.object_ref.object_id);
        }

        if let Some(payment) = tx
            .gas_payment()
            .and_then(|gas_payment| gas_payment.payment_objects.into_iter().next())
        {
            return format!("object:{}", payment.object_id);
        }

        format!("owner:{}", tx.sender())
    }

    fn dag_hot_object_tx_cap() -> usize {
        std::env::var("KANARI_DAG_HOT_OBJECT_TX_CAP")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_DAG_VERTEX_TXS_PER_HOT_OBJECT)
    }

    fn pending_sort_key(tx: &SignedTransaction) -> (u8, String, String, u64, Vec<u8>) {
        (
            Self::transaction_lane_rank(tx),
            tx.transaction.primary_access_key(),
            tx.transaction.sender_address().to_string(),
            tx.transaction.nonce(),
            tx.transaction_hash().to_vec(),
        )
    }

    fn dag_lane_aware_selection_enabled() -> bool {
        !matches!(
            std::env::var("KANARI_DAG_LANE_AWARE").as_deref(),
            Ok("0") | Ok("false") | Ok("FALSE") | Ok("no") | Ok("NO")
        )
    }

    fn owned_fast_checkpoint_tx_cap() -> usize {
        std::env::var("KANARI_OWNED_FAST_CHECKPOINT_MAX_TXS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_OWNED_FAST_CHECKPOINT_TRANSACTIONS)
    }

    fn owned_fastpath_audit_interval() -> Option<u64> {
        std::env::var("KANARI_OWNED_FASTPATH_AUDIT_INTERVAL")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
    }

    fn owned_fastpath_smt_audit_enabled() -> bool {
        std::env::var("KANARI_OWNED_FASTPATH_AUDIT_SMT")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    }

    fn is_owned_fast_path_eligible(tx: &SignedTransaction) -> bool {
        tx.transaction
            .object_inputs()
            .iter()
            .all(|input| !matches!(input.owner, Some(ObjectOwnerKind::Shared)))
    }

    pub fn pending_owned_fast_path_transactions_snapshot(&self) -> Vec<SignedTransaction> {
        let mut transactions = self
            .pending_signed_transactions()
            .into_iter()
            .filter(Self::is_owned_fast_path_eligible)
            .collect::<Vec<_>>();
        if transactions.is_empty() {
            return Vec::new();
        }
        transactions.sort_by(|a, b| {
            a.transaction
                .primary_access_key()
                .cmp(&b.transaction.primary_access_key())
                .then_with(|| {
                    a.transaction
                        .sender_address()
                        .cmp(b.transaction.sender_address())
                })
                .then_with(|| a.transaction.nonce().cmp(&b.transaction.nonce()))
                .then_with(|| a.transaction_hash().cmp(b.transaction_hash()))
        });
        Self::select_conflict_free_transactions(transactions)
            .into_iter()
            .take(Self::owned_fast_checkpoint_tx_cap())
            .collect()
    }

    pub fn pending_conflict_free_transactions_snapshot(&self) -> Vec<SignedTransaction> {
        let mut transactions = self.pending_signed_transactions();
        if transactions.is_empty() {
            return Vec::new();
        }
        if Self::dag_lane_aware_selection_enabled() {
            transactions.sort_by_cached_key(Self::pending_sort_key);
        } else {
            transactions.sort_by(|a, b| {
                a.transaction
                    .primary_access_key()
                    .cmp(&b.transaction.primary_access_key())
                    .then_with(|| {
                        a.transaction
                            .sender_address()
                            .cmp(b.transaction.sender_address())
                    })
                    .then_with(|| a.transaction.nonce().cmp(&b.transaction.nonce()))
                    .then_with(|| a.transaction_hash().cmp(b.transaction_hash()))
            });
        }
        let selected = Self::select_conflict_free_transactions(transactions);
        let hot_object_tx_cap = Self::dag_hot_object_tx_cap();
        let mut hot_object_counts = AHashMap::<String, usize>::new();
        let mut total_bytes = 0usize;
        let mut ready = Vec::with_capacity(selected.len().min(MAX_DAG_VERTEX_TRANSACTIONS));
        for signed_tx in selected {
            if ready.len() >= MAX_DAG_VERTEX_TRANSACTIONS {
                break;
            }
            let primary_access_key = signed_tx.transaction.primary_access_key();
            if hot_object_counts
                .get(&primary_access_key)
                .copied()
                .unwrap_or(0)
                >= hot_object_tx_cap
            {
                continue;
            }
            let Ok(size) = bcs::serialized_size(&signed_tx) else {
                break;
            };
            let Some(next_total) = total_bytes.checked_add(size) else {
                break;
            };
            if next_total > MAX_DAG_VERTEX_TRANSACTION_BYTES {
                break;
            }
            total_bytes = next_total;
            *hot_object_counts.entry(primary_access_key).or_insert(0) += 1;
            ready.push(signed_tx);
        }
        ready
    }

    pub fn pending_transaction_len(&self) -> usize {
        self.mempool_read().pending_txs.len()
    }

    pub(crate) fn get_expected_nonce(&self, address_hex: &str) -> u64 {
        // Legacy wire field only. Account sequence is no longer a state/consensus rule;
        // keep this as a best-effort transaction nonce so older clients get distinct hashes.
        self.pending_tx_count_for_sender(address_hex)
    }

    fn resolve_account_objects(
        &self,
        state: &StateManager,
        owner_addr: &AccountAddress,
    ) -> Result<Vec<ObjectInfo>> {
        let mut unique_ids = state
            .get_owned_objects(owner_addr)
            .with_context(|| format!("Failed to load owned-object index for {owner_addr:#x}"))?;
        unique_ids.sort();
        unique_ids.dedup();

        let mut coins = Vec::new();
        let mut others = Vec::new();

        for id in unique_ids {
            let obj = state
                .get_object(&id)
                .with_context(|| format!("Failed to load indexed object {id}"))?
                .ok_or_else(|| {
                    anyhow::anyhow!("Owned-object index references missing object {id}")
                })?;
            let coin_amount = if obj.type_.contains("::coin::Coin<") && obj.data.len() >= 40 {
                let mut arr = [0u8; 8];
                arr.copy_from_slice(&obj.data[32..40]);
                Some(u64::from_le_bytes(arr))
            } else {
                None
            };
            let info = Self::object_info_from_created_object(id.clone(), obj);

            if let Some(amount) = coin_amount
                && amount > 0
            {
                coins.push((amount, info));
                continue;
            }
            others.push(info);
        }

        coins.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        others.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(coins
            .into_iter()
            .map(|(_, info)| info)
            .chain(others)
            .collect())
    }

    #[cfg(test)]
    fn execute_tx_waves_parallel(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
        strict_mode: bool,
    ) -> Result<(usize, usize)> {
        let (executed, failed, _) = self.execute_tx_waves_parallel_inner(
            transactions,
            state_arc,
            timestamp,
            persist_objects,
            strict_mode,
            strict_mode,
        )?;
        Ok((executed, failed))
    }

    pub(crate) fn execute_tx_waves_deterministic_parallel_with_effects(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
    ) -> Result<(usize, usize, Vec<TransactionEffects>)> {
        self.execute_tx_waves_parallel_inner(
            transactions,
            state_arc,
            timestamp,
            persist_objects,
            false,
            false,
        )
    }

    fn can_trust_owned_native_conflict_keys(
        transactions: &[SignedTransaction],
        changesets: &[ChangeSet],
    ) -> bool {
        if transactions.len() != changesets.len() {
            return false;
        }

        let mut reserved_keys = HashSet::new();
        for signed_tx in transactions {
            if !Self::is_owned_fast_path_eligible(signed_tx) {
                return false;
            }

            let mut conflict_keys = signed_tx.transaction.get_conflict_keys();
            conflict_keys.sort();
            conflict_keys.dedup();
            if conflict_keys.is_empty()
                || conflict_keys
                    .iter()
                    .any(|conflict_key| !reserved_keys.insert(conflict_key.clone()))
            {
                return false;
            }
        }

        changesets.iter().all(|changeset| {
            changeset.success
                && changeset.events.is_empty()
                && changeset.treasuries.is_empty()
                && changeset.nft_caps.is_empty()
                && changeset.token_balance_sets.is_empty()
                && changeset.shared_inputs.is_empty()
                && changeset.immutable_inputs.is_empty()
                && changeset.deleted_objects.is_empty()
                && changeset.added_dynamic_fields.is_empty()
                && changeset.removed_dynamic_fields.is_empty()
                && changeset.move_writes.is_empty()
                && changeset.resolver_reads.is_empty()
        })
    }

    fn dependency_apply_batch_cap() -> usize {
        std::env::var("KANARI_DEPENDENCY_APPLY_BATCH_CAP")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            // Keep the default above the current owned-fastpath checkpoint cap.
            // Splitting a purely disjoint owned-object workload too eagerly can
            // force repeated state-index and balance reconciliation passes. The
            // scheduler still cuts a new batch on real dependencies, and ops can
            // lower this cap explicitly for memory-bound workloads.
            .unwrap_or(65_536)
    }

    fn dependency_apply_scheduler_enabled() -> bool {
        !matches!(
            std::env::var("KANARI_DEPENDENCY_APPLY_SCHEDULER").as_deref(),
            Ok("0") | Ok("false") | Ok("FALSE") | Ok("no") | Ok("NO")
        )
    }

    fn extend_access_set(target: &mut StateAccessSet, source: &StateAccessSet) {
        target.reads.extend(source.reads.iter().cloned());
        target.writes.extend(source.writes.iter().cloned());
    }

    fn schedule_dependency_aware_apply_batches(changesets: &[ChangeSet]) -> Vec<Vec<usize>> {
        if changesets.is_empty() {
            return Vec::new();
        }
        if !Self::dependency_apply_scheduler_enabled() {
            return vec![(0..changesets.len()).collect()];
        }

        let batch_cap = Self::dependency_apply_batch_cap();
        let mut batches = Vec::new();
        let mut current = Vec::with_capacity(batch_cap.min(changesets.len()));
        let mut current_access = StateAccessSet::default();

        for (index, changeset) in changesets.iter().enumerate() {
            let access = changeset.deterministic_access_set();
            let batch_full = current.len() >= batch_cap;
            let has_dependency = !current.is_empty() && access.conflicts_with(&current_access);
            if !current.is_empty() && (batch_full || has_dependency) {
                batches.push(std::mem::take(&mut current));
                current_access = StateAccessSet::default();
            }

            Self::extend_access_set(&mut current_access, &access);
            current.push(index);
        }

        if !current.is_empty() {
            batches.push(current);
        }
        batches
    }

    fn apply_changeset_batch_without_supply_validation(
        state: &mut StateManager,
        changesets: &[ChangeSet],
        indices: &[usize],
    ) -> Result<()> {
        let profile = matches!(
            std::env::var("KANARI_DEPENDENCY_APPLY_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let started_at = std::time::Instant::now();
        match indices {
            [] => Ok(()),
            [index] => state.apply_changeset_without_supply_validation(&changesets[*index]),
            _ => {
                let mut merged = ChangeSet::new();
                for index in indices {
                    merged.merge_from(&changesets[*index]);
                }
                let merged_at = std::time::Instant::now();
                let result = state.apply_changeset_without_supply_validation(&merged);
                let applied_at = std::time::Instant::now();
                if profile {
                    eprintln!(
                        "dependency apply batch profile: txs={} merge={:.6}s state_apply={:.6}s total={:.6}s",
                        indices.len(),
                        merged_at.duration_since(started_at).as_secs_f64(),
                        applied_at.duration_since(merged_at).as_secs_f64(),
                        applied_at.duration_since(started_at).as_secs_f64(),
                    );
                }
                result
            }
        }
    }

    fn apply_dependency_aware_changesets_without_supply_validation(
        state: &mut StateManager,
        changesets: &[ChangeSet],
    ) -> Result<(usize, usize)> {
        let profile = matches!(
            std::env::var("KANARI_DEPENDENCY_APPLY_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let started_at = std::time::Instant::now();
        let batches = Self::schedule_dependency_aware_apply_batches(changesets);
        let scheduled_at = std::time::Instant::now();
        let batch_count = batches.len();
        for batch in &batches {
            Self::apply_changeset_batch_without_supply_validation(state, changesets, batch)?;
        }
        let applied_at = std::time::Instant::now();
        if profile {
            eprintln!(
                "dependency apply profile: changesets={} batches={} schedule={:.6}s apply_batches={:.6}s total={:.6}s",
                changesets.len(),
                batch_count,
                scheduled_at.duration_since(started_at).as_secs_f64(),
                applied_at.duration_since(scheduled_at).as_secs_f64(),
                applied_at.duration_since(started_at).as_secs_f64(),
            );
        }
        Ok((batch_count, changesets.len()))
    }

    fn apply_single_changeset_batch_without_supply_validation(
        state: &mut StateManager,
        changesets: &[ChangeSet],
    ) -> Result<(usize, usize)> {
        let indices = (0..changesets.len()).collect::<Vec<_>>();
        Self::apply_changeset_batch_without_supply_validation(state, changesets, &indices)?;
        Ok((usize::from(!changesets.is_empty()), changesets.len()))
    }

    pub(crate) fn execute_conflict_free_transactions_parallel_with_effects(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
    ) -> Result<(usize, usize, Vec<TransactionEffects>)> {
        let profile = matches!(
            std::env::var("KANARI_CONFLICT_FREE_EXECUTOR_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let started_at = std::time::Instant::now();
        if transactions.is_empty() {
            return Ok((0, 0, Vec::new()));
        }
        let (access_epoch, state_overlay) = {
            let state = state_arc
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            (
                state.access_epoch(),
                (!state.overlay.is_empty()).then(|| Arc::new(state.overlay.clone())),
            )
        };
        let results = transactions
            .par_iter()
            .enumerate()
            .map(|(i, signed_tx)| {
                let runtime = &self.runtime_pool[i % self.runtime_pool.len()];
                self.execute_transaction_with_runtime_overlay(
                    &signed_tx.transaction,
                    runtime,
                    state_arc,
                    false,
                    timestamp,
                    persist_objects,
                    state_overlay.clone(),
                )
            })
            .collect::<Vec<_>>();
        let executed_at = std::time::Instant::now();

        if results.iter().any(Result::is_err) {
            return self.execute_tx_waves_deterministic_parallel_with_effects(
                transactions,
                state_arc,
                timestamp,
                persist_objects,
            );
        }

        let changesets = results
            .into_iter()
            .collect::<Result<Vec<_>>>()
            .context("Failed to collect conflict-free execution results")?;
        let trusted_owned_native_conflict_keys =
            Self::can_trust_owned_native_conflict_keys(&transactions, &changesets);
        if !trusted_owned_native_conflict_keys {
            let mut reserved_reads = BTreeSet::new();
            let mut reserved_writes = BTreeSet::new();
            for changeset in &changesets {
                let access_set = changeset.deterministic_access_set();
                let has_conflict = access_set
                    .writes
                    .iter()
                    .any(|key| reserved_reads.contains(key) || reserved_writes.contains(key))
                    || access_set
                        .reads
                        .iter()
                        .any(|key| reserved_writes.contains(key));
                if has_conflict {
                    return self.execute_tx_waves_deterministic_parallel_with_effects(
                        transactions,
                        state_arc,
                        timestamp,
                        persist_objects,
                    );
                }
                reserved_reads.extend(access_set.reads);
                reserved_writes.extend(access_set.writes);
            }
        }
        let access_checked_at = std::time::Instant::now();

        let mut state_write = state_arc
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state_write.access_epoch() != access_epoch {
            return self.execute_tx_waves_deterministic_parallel_with_effects(
                transactions,
                state_arc,
                timestamp,
                persist_objects,
            );
        }

        let apply_started_at = std::time::Instant::now();
        let mut candidate = state_write.clone();
        let candidate_cloned_at = std::time::Instant::now();
        let apply_result = if trusted_owned_native_conflict_keys {
            match candidate.try_apply_owned_native_burn_batch_without_supply_validation(&changesets)
            {
                Ok(true) => Ok((usize::from(!changesets.is_empty()), changesets.len())),
                Ok(false) => Self::apply_single_changeset_batch_without_supply_validation(
                    &mut candidate,
                    &changesets,
                ),
                Err(error) => Err(error),
            }
        } else {
            Self::apply_dependency_aware_changesets_without_supply_validation(
                &mut candidate,
                &changesets,
            )
        };
        let merged_applied_at = std::time::Instant::now();
        let apply_batches = apply_result
            .as_ref()
            .map(|(batches, _)| *batches)
            .unwrap_or(0);
        let validate_result = if trusted_owned_native_conflict_keys {
            let validate_owned_fastpath = std::env::var("KANARI_VALIDATE_OWNED_FASTPATH_SUPPLY")
                .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
                .unwrap_or(false);
            if validate_owned_fastpath {
                apply_result
                    .map(|_| ())
                    .and_then(|_| candidate.repair_cached_native_wallet_overcount())
                    .map(|_| ())
                    .and_then(|_| candidate.validate_cached_supply_invariants())
            } else {
                apply_result.map(|_| ())
            }
        } else {
            apply_result
                .map(|_| ())
                .and_then(|_| candidate.repair_legacy_native_wallet_overcount())
                .and_then(|_| candidate.validate_supply_invariants())
        };
        let validated_at = std::time::Instant::now();
        if validate_result.is_err() {
            drop(state_write);
            return self.execute_tx_waves_deterministic_parallel_with_effects(
                transactions,
                state_arc,
                timestamp,
                persist_objects,
            );
        }
        *state_write = candidate;
        drop(state_write);
        let applied_at = std::time::Instant::now();

        let mut executed = 0usize;
        let mut failed = 0usize;
        let mut effects = Vec::with_capacity(changesets.len());
        for (signed_tx, changeset) in transactions.iter().zip(changesets) {
            if changeset.success {
                executed += 1;
            } else {
                failed += 1;
            }
            effects.push(changeset.effects(signed_tx.transaction.gas_payment()));
        }
        let effects_at = std::time::Instant::now();
        if profile {
            eprintln!(
                "conflict-free executor profile: txs={} apply_batches={} execute_parallel={:.6}s access={:.6}s clone_state={:.6}s apply={:.6}s validate={:.6}s effects={:.6}s total={:.6}s",
                transactions.len(),
                apply_batches,
                executed_at.duration_since(started_at).as_secs_f64(),
                access_checked_at.duration_since(executed_at).as_secs_f64(),
                candidate_cloned_at
                    .duration_since(apply_started_at)
                    .as_secs_f64(),
                merged_applied_at
                    .duration_since(candidate_cloned_at)
                    .as_secs_f64(),
                validated_at.duration_since(merged_applied_at).as_secs_f64(),
                effects_at.duration_since(applied_at).as_secs_f64(),
                effects_at.duration_since(started_at).as_secs_f64(),
            );
        }
        Ok((executed, failed, effects))
    }

    fn execute_tx_waves_parallel_inner(
        &self,
        transactions: Vec<SignedTransaction>,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
        persist_objects: bool,
        serial_execution: bool,
        fail_hard: bool,
    ) -> Result<(usize, usize, Vec<TransactionEffects>)> {
        let profile = matches!(
            std::env::var("KANARI_EXECUTION_WAVES_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let execution_started_at = std::time::Instant::now();
        let mut executed_count = 0;
        let mut failed_count = 0;
        let mut transaction_effects = Vec::with_capacity(transactions.len());
        let has_module_publish = transactions.iter().any(|tx| {
            matches!(
                tx.transaction,
                Transaction::PublishModule { .. }
                    | Transaction::PublishPackage { .. }
                    | Transaction::UpgradeModule { .. }
                    | Transaction::UpgradePackage { .. }
            )
        });
        if serial_execution
            || has_module_publish
            || kanari_move_runtime_v1::TransactionScheduler::requires_serial_execution()
        {
            if has_module_publish {
                self.runtime_pool[0].reload_vm_cache()?;
            }

            let mut executed_count = 0;
            let mut failed_count = 0;
            for signed_tx in transactions {
                let persist_runtime_state = persist_objects;
                let changeset = self.execute_transaction_with_runtime_internal(
                    &signed_tx.transaction,
                    &self.runtime_pool[0],
                    state_arc,
                    false,
                    timestamp,
                    persist_runtime_state,
                );

                let changeset = match changeset {
                    Ok(changeset) => changeset,
                    Err(error) if fail_hard => {
                        return Err(error).with_context(|| {
                            format!(
                                "Execution failed for tx {} (sender={}, nonce={})",
                                hex::encode(signed_tx.transaction_hash()),
                                signed_tx.transaction.sender_address(),
                                signed_tx.transaction.nonce()
                            )
                        });
                    }
                    Err(error) => {
                        log::warn!("Serial execution failed: {error}");
                        let failed = self.failed_changeset_for_error(
                            &signed_tx.transaction,
                            state_arc,
                            error.to_string(),
                        )?;
                        let mut state_write = state_arc
                            .write()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Err(apply_error) = state_write.apply_changeset(&failed) {
                            return Err(apply_error)
                                .context("Failed to apply deterministic failure gas changeset");
                        } else {
                            transaction_effects
                                .push(failed.effects(signed_tx.transaction.gas_payment()));
                        }
                        failed_count += 1;
                        continue;
                    }
                };

                let mut state_write = match state_arc.write() {
                    Ok(guard) => guard,
                    Err(poisoned) => {
                        log::error!("State lock poisoned during strict execution, recovering...");
                        poisoned.into_inner()
                    }
                };

                match state_write.apply_changeset(&changeset) {
                    Ok(()) => {
                        transaction_effects
                            .push(changeset.effects(signed_tx.transaction.gas_payment()));
                        executed_count += 1;
                    }
                    Err(error) if fail_hard => {
                        return Err(error).with_context(|| {
                            format!(
                                "Execution failed for tx {} (sender={}, nonce={}): Failed to apply changeset",
                                hex::encode(signed_tx.transaction_hash()),
                                signed_tx.transaction.sender_address(),
                                signed_tx.transaction.nonce()
                            )
                        });
                    }
                    Err(error) => {
                        return Err(error).context("Failed to apply serial transaction changeset");
                    }
                }
            }

            return Ok((executed_count, failed_count, transaction_effects));
        }

        let waves = kanari_move_runtime_v1::TransactionScheduler::schedule(transactions);
        let wave_count = waves.len();
        let mut speculative_committed_waves = 0usize;
        let mut speculative_committed_txs = 0usize;
        let mut serial_retry_txs = 0usize;
        let mut retry_execution_error_txs = 0usize;
        let mut retry_conflict_txs = 0usize;
        let mut retry_epoch_txs = 0usize;
        let mut retry_apply_txs = 0usize;
        let mut retry_supply_txs = 0usize;
        let mut result_secs = 0.0f64;
        let mut conflict_secs = 0.0f64;
        let mut apply_secs = 0.0f64;
        let mut effects_secs = 0.0f64;

        if has_module_publish {
            // Keep speculative publish execution deterministic across authorities.
            // PublishModule depends on VM/module cache state more heavily than regular
            // user transactions, so we reset the shared cache and execute on one
            // runtime in a fixed serial order for DAG production / validation.
            self.runtime_pool[0].reload_vm_cache()?;
        }

        for wave in waves {
            let wave_started_at = std::time::Instant::now();
            let (access_epoch, wave_overlay) = match state_arc.read() {
                Ok(state) => (
                    state.access_epoch(),
                    (!state.overlay.is_empty()).then(|| Arc::new(state.overlay.clone())),
                ),
                Err(poisoned) => {
                    let state = poisoned.into_inner();
                    (
                        state.access_epoch(),
                        (!state.overlay.is_empty()).then(|| Arc::new(state.overlay.clone())),
                    )
                }
            };
            let results: Vec<Result<ChangeSet>> = if has_module_publish {
                wave.iter()
                    .map(|signed_tx| {
                        self.execute_transaction_with_runtime_overlay(
                            &signed_tx.transaction,
                            &self.runtime_pool[0],
                            state_arc,
                            false,
                            timestamp,
                            false,
                            wave_overlay.clone(),
                        )
                    })
                    .collect()
            } else {
                wave.par_iter()
                    .enumerate()
                    .map(|(i, signed_tx)| {
                        let runtime = &self.runtime_pool[i % self.runtime_pool.len()];
                        self.execute_transaction_with_runtime_overlay(
                            &signed_tx.transaction,
                            runtime,
                            state_arc,
                            false,
                            timestamp,
                            false,
                            wave_overlay.clone(),
                        )
                    })
                    .collect()
            };
            let results_at = std::time::Instant::now();
            result_secs += results_at.duration_since(wave_started_at).as_secs_f64();

            let access_sets = results
                .iter()
                .map(|result| {
                    result
                        .as_ref()
                        .ok()
                        .map(ChangeSet::deterministic_access_set)
                })
                .collect::<Vec<_>>();
            let mut retry_reason = results
                .iter()
                .any(Result::is_err)
                .then_some("execution_error");
            let mut retry_serial = retry_reason.is_some();
            for i in 0..access_sets.len() {
                let Some(left) = &access_sets[i] else {
                    continue;
                };
                let conflicts = access_sets[..i]
                    .iter()
                    .flatten()
                    .any(|right| left.conflicts_with(right));
                if conflicts {
                    retry_reason.get_or_insert("intra_wave_conflict");
                    retry_serial = true;
                    break;
                }
            }
            let conflicts_at = std::time::Instant::now();
            conflict_secs += conflicts_at.duration_since(results_at).as_secs_f64();

            if !retry_serial {
                let mut state_write = match state_arc.write() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if state_write.access_epoch() != access_epoch {
                    retry_reason = Some("epoch_changed");
                    retry_serial = true;
                }

                if !retry_serial {
                    // Apply the conflict-free wave to a candidate through deterministic
                    // dependency-aware batches. The live state changes exactly once, so
                    // a partial wave is never observable, while large owned-object waves
                    // avoid constructing one giant ChangeSet clone.
                    let mut candidate = state_write.clone();
                    let mut wave_changesets = Vec::with_capacity(results.len());
                    for result in results.iter() {
                        let Ok(cs) = result else {
                            log::warn!(
                                "Speculative wave had an unexpected failed result; retrying serially"
                            );
                            retry_reason = Some("execution_error");
                            retry_serial = true;
                            break;
                        };
                        wave_changesets.push(cs.clone());
                    }
                    if !retry_serial
                        && let Err(error) =
                            Self::apply_dependency_aware_changesets_without_supply_validation(
                                &mut candidate,
                                &wave_changesets,
                            )
                            .map(|_| ())
                    {
                        log::warn!("Speculative wave apply rejected; retrying serially: {error}");
                        retry_reason = Some("apply_rejected");
                        retry_serial = true;
                    }
                    if !retry_serial {
                        if let Err(error) = candidate
                            .repair_legacy_native_wallet_overcount()
                            .and_then(|_| candidate.validate_supply_invariants())
                        {
                            log::warn!(
                                "Speculative wave supply validation rejected; retrying serially: {error}"
                            );
                            retry_reason = Some("supply_rejected");
                            retry_serial = true;
                        } else {
                            *state_write = candidate;
                        }
                    }
                }
            }
            let applied_at = std::time::Instant::now();
            apply_secs += applied_at.duration_since(conflicts_at).as_secs_f64();

            if !retry_serial {
                speculative_committed_waves += 1;
                speculative_committed_txs += wave.len();
                for (signed_tx, result) in wave.iter().zip(&results) {
                    let cs = result.as_ref().map_err(|error| {
                        anyhow::anyhow!(
                            "Speculative wave committed with an unexpected failed result: {error}"
                        )
                    })?;
                    transaction_effects.push(cs.effects(signed_tx.transaction.gas_payment()));
                    executed_count += 1;
                }
                effects_secs += std::time::Instant::now()
                    .duration_since(applied_at)
                    .as_secs_f64();
                continue;
            }

            // Any incomplete trace, execution error, version change, or intra-wave
            // conflict discards the entire speculative wave and replays canonical order.
            serial_retry_txs += wave.len();
            match retry_reason.unwrap_or("unknown") {
                "execution_error" => retry_execution_error_txs += wave.len(),
                "intra_wave_conflict" => retry_conflict_txs += wave.len(),
                "epoch_changed" => retry_epoch_txs += wave.len(),
                "apply_rejected" => retry_apply_txs += wave.len(),
                "supply_rejected" => retry_supply_txs += wave.len(),
                _ => {}
            }
            for signed_tx in &wave {
                let result = self.execute_transaction_with_runtime_internal(
                    &signed_tx.transaction,
                    &self.runtime_pool[0],
                    state_arc,
                    false,
                    timestamp,
                    persist_objects,
                );
                let cs = match result {
                    Ok(cs) => cs,
                    Err(error) if fail_hard => {
                        return Err(error).with_context(|| {
                            format!(
                                "Serial retry failed for tx {}",
                                hex::encode(signed_tx.transaction_hash())
                            )
                        });
                    }
                    Err(error) => {
                        let failed = self.failed_changeset_for_error(
                            &signed_tx.transaction,
                            state_arc,
                            error.to_string(),
                        )?;
                        let mut state_write = state_arc
                            .write()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Err(apply_error) = state_write.apply_changeset(&failed) {
                            return Err(apply_error)
                                .context("Failed to apply deterministic failure gas changeset");
                        } else {
                            transaction_effects
                                .push(failed.effects(signed_tx.transaction.gas_payment()));
                        }
                        failed_count += 1;
                        continue;
                    }
                };
                let mut state_write = match state_arc.write() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                match state_write.apply_changeset(&cs) {
                    Ok(()) => {
                        transaction_effects.push(cs.effects(signed_tx.transaction.gas_payment()));
                        executed_count += 1;
                    }
                    Err(error) if fail_hard => {
                        return Err(error).with_context(|| {
                            format!(
                                "Serial retry apply failed for tx {}",
                                hex::encode(signed_tx.transaction_hash())
                            )
                        });
                    }
                    Err(error) => {
                        return Err(error).context("Failed to apply serial retry changeset");
                    }
                }
            }
        }

        if wave_count > 0 {
            log::info!(
                "[parallel execution] waves={} speculative_committed_waves={} speculative_txs={} serial_retry_txs={} retry_execution_error_txs={} retry_conflict_txs={} retry_epoch_txs={} retry_apply_txs={} retry_supply_txs={} executed={} failed={}",
                wave_count,
                speculative_committed_waves,
                speculative_committed_txs,
                serial_retry_txs,
                retry_execution_error_txs,
                retry_conflict_txs,
                retry_epoch_txs,
                retry_apply_txs,
                retry_supply_txs,
                executed_count,
                failed_count
            );
        }
        if profile {
            let total_secs = execution_started_at.elapsed().as_secs_f64();
            eprintln!(
                "execution waves profile: waves={} txs={} results={:.6}s conflicts={:.6}s apply_validate={:.6}s effects={:.6}s total={:.6}s speculative_txs={} serial_retry_txs={}",
                wave_count,
                executed_count + failed_count,
                result_secs,
                conflict_secs,
                apply_secs,
                effects_secs,
                total_secs,
                speculative_committed_txs,
                serial_retry_txs,
            );
        }

        Ok((executed_count, failed_count, transaction_effects))
    }

    /// Build the canonical failure result for errors that happen before the VM
    /// can return a `ChangeSet` (for example a stale object reference). Such a
    /// transaction is still ordered by Mysticeti and must pay its deterministic
    /// admission/base gas; otherwise an attacker can commit unlimited signed,
    /// guaranteed-to-fail transactions at zero cost.
    fn failed_changeset_for_error(
        &self,
        tx: &Transaction,
        state_arc: &Arc<RwLock<StateManager>>,
        error_message: String,
    ) -> Result<ChangeSet> {
        let sender_addr = KanariAddress::parse_to_account_address(tx.sender_address())?;
        let gas_op = match tx {
            Transaction::PublishModule { module_bytes, .. }
            | Transaction::UpgradeModule { module_bytes, .. } => GasOperation::PublishModule {
                module_size: module_bytes.len(),
            },
            Transaction::PublishPackage { modules, .. }
            | Transaction::UpgradePackage { modules, .. } => GasOperation::PublishModule {
                module_size: modules.iter().map(|module| module.module_bytes.len()).sum(),
            },
            Transaction::ExecuteFunction { .. } => {
                if tx.native_call().is_some() {
                    GasOperation::Transfer
                } else {
                    GasOperation::ExecuteFunction { complexity: 1 }
                }
            }
        };
        let base_units = gas_op.gas_units().min(tx.gas_limit());
        let requested_cost = base_units
            .checked_mul(effective_gas_price(tx.gas_price()))
            .context("Failed to calculate failure gas cost")?;

        let balance = {
            let state = state_arc.read().unwrap_or_else(|error| error.into_inner());
            state.resolve_owner_native_balance(sender_addr).unwrap_or(0)
        };
        let gas_cost = requested_cost.min(balance);
        let mut changeset = ChangeSet::new();
        changeset.set_transaction_context(tx.object_inputs(), tx.gas_payment());
        changeset.mark_failed(error_message);
        Self::apply_gas_and_sequence(&mut changeset, sender_addr, gas_cost, base_units)?;
        let state = state_arc.read().unwrap_or_else(|error| error.into_inner());
        Self::annotate_changeset_object_effects(&state, &mut changeset)?;
        Ok(changeset)
    }

    pub(crate) fn checkpoint_root_matches(
        &self,
        checkpoint_sequence: u64,
        computed_root: &[u8],
        checkpoint_root: &[u8],
    ) -> Result<bool> {
        if computed_root == checkpoint_root {
            return Ok(true);
        }

        if Self::strict_checkpoint_roots_required() {
            anyhow::bail!(
                "Checkpoint #{} state root mismatch: strict verification failed",
                checkpoint_sequence
            );
        }

        Ok(false)
    }

    fn apply_gas_and_sequence(
        changeset: &mut ChangeSet,
        sender: AccountAddress,
        gas_cost: u64,
        gas_used: u64,
    ) -> Result<()> {
        let sender_owner_delta = changeset.get_or_create_owner_delta(sender);
        sender_owner_delta.debit(gas_cost);

        let dao_addr = AccountAddress::from_hex_literal(KanariAddress::DAO_ADDRESS)?;
        let dao_delta_before = changeset
            .owner_deltas
            .get(&dao_addr)
            .map(|delta| delta.balance_delta)
            .unwrap_or(0);
        let dao_gas_credit_before = changeset
            .native_gas_credits
            .get(&dao_addr)
            .copied()
            .unwrap_or(0);
        changeset.collect_gas(dao_addr, gas_cost);
        let dao_delta_after = changeset
            .owner_deltas
            .get(&dao_addr)
            .map(|delta| delta.balance_delta)
            .unwrap_or(0);
        ensure!(
            dao_delta_after
                .checked_sub(dao_delta_before)
                .is_some_and(|delta| delta == i128::from(gas_cost)),
            "DAO gas credit mismatch: expected {} Mist",
            gas_cost
        );
        ensure!(
            changeset
                .native_gas_credits
                .get(&dao_addr)
                .copied()
                .and_then(|credit| credit.checked_sub(dao_gas_credit_before))
                == Some(gas_cost),
            "DAO gas-credit metadata mismatch: expected {} Mist",
            gas_cost
        );
        changeset.set_gas_used(gas_used);
        Ok(())
    }

    /// Reject malformed payloads at admission so they cannot occupy a mempool
    /// lane or later become zero-gas failed checkpoint effects.
    pub(crate) fn validate_transaction_admission_shape(tx: &Transaction) -> Result<()> {
        ensure!(
            bcs::serialized_size(tx)? <= MAX_TRANSACTION_BYTES,
            "Transaction exceeds {} byte admission limit",
            MAX_TRANSACTION_BYTES
        );
        ensure!(
            tx.gas_limit() <= MAX_TRANSACTION_GAS_LIMIT,
            "Transaction gas limit {} exceeds protocol maximum {}",
            tx.gas_limit(),
            MAX_TRANSACTION_GAS_LIMIT
        );
        KanariAddress::parse_to_account_address(tx.sender_address())
            .context("Invalid transaction sender address")?;

        if let Transaction::ExecuteFunction {
            module, type_args, ..
        } = tx
        {
            let mut parts = module.split("::");
            let address = parts
                .next()
                .filter(|part| !part.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid module format. Expected: address::module")
                })?;
            let module_name = parts
                .next()
                .filter(|part| !part.is_empty())
                .ok_or_else(|| {
                    anyhow::anyhow!("Invalid module format. Expected: address::module")
                })?;
            anyhow::ensure!(
                parts.next().is_none(),
                "Invalid module format. Expected: address::module"
            );
            KanariAddress::parse_to_account_address(address)
                .context("Invalid entry-function module address")?;
            move_core_types::identifier::Identifier::new(module_name)
                .context("Invalid entry-function module name")?;
            for type_arg in type_args {
                parse_type_tag(type_arg).ok_or_else(|| anyhow::anyhow!("Invalid type argument"))?;
            }
        }
        ensure!(tx.gas_limit() > 0, "Transaction gas limit must be non-zero");
        ensure!(
            gas_price_is_valid(tx.gas_price()),
            "Transaction gas price is not valid for the active gas model"
        );
        Ok(())
    }

    fn execute_transaction_with_runtime(
        &self,
        tx: &Transaction,
        runtime: &kanari_move_runtime_v1::move_runtime::MoveRuntime,
        state_arc: &Arc<RwLock<StateManager>>,
        timestamp: Option<u64>,
    ) -> Result<ChangeSet> {
        self.execute_transaction_with_runtime_internal(
            tx, runtime, state_arc, true, timestamp, false,
        )
    }

    pub(crate) fn execute_transaction_with_runtime_internal(
        &self,
        tx: &Transaction,
        runtime: &kanari_move_runtime_v1::move_runtime::MoveRuntime,
        state_arc: &Arc<RwLock<StateManager>>,
        _validate_sequence: bool,
        timestamp: Option<u64>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        let state_overlay = {
            let state = match state_arc.read() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            (!state.overlay.is_empty()).then(|| Arc::new(state.overlay.clone()))
        };
        self.execute_transaction_with_runtime_overlay(
            tx,
            runtime,
            state_arc,
            _validate_sequence,
            timestamp,
            persist_runtime_state,
            state_overlay,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_transaction_with_runtime_overlay(
        &self,
        tx: &Transaction,
        runtime: &kanari_move_runtime_v1::move_runtime::MoveRuntime,
        state_arc: &Arc<RwLock<StateManager>>,
        _validate_sequence: bool,
        timestamp: Option<u64>,
        persist_runtime_state: bool,
        state_overlay: Option<kanari_move_runtime_v1::StateOverlay>,
    ) -> Result<ChangeSet> {
        let sender_addr = KanariAddress::parse_to_account_address(tx.sender_address())?;
        let mut gas_meter = GasMeter::new(tx.gas_limit(), effective_gas_price(tx.gas_price()));
        let mut changeset = ChangeSet::new();
        changeset.set_transaction_context(tx.object_inputs(), tx.gas_payment());

        let native_call = tx.native_call();

        let gas_op = match tx {
            Transaction::PublishModule { module_bytes, .. }
            | Transaction::UpgradeModule { module_bytes, .. } => GasOperation::PublishModule {
                module_size: module_bytes.len(),
            },
            Transaction::PublishPackage { modules, .. }
            | Transaction::UpgradePackage { modules, .. } => GasOperation::PublishModule {
                module_size: modules.iter().map(|module| module.module_bytes.len()).sum(),
            },
            Transaction::ExecuteFunction { .. } => {
                if native_call.is_some() {
                    GasOperation::Transfer
                } else {
                    GasOperation::ExecuteFunction { complexity: 1 }
                }
            }
        };

        gas_meter.consume(gas_op.gas_units())?;
        let base_gas_cost = gas_meter.total_cost();
        let max_gas_cost = tx
            .gas_limit()
            .checked_mul(effective_gas_price(tx.gas_price()))
            .context("Transaction maximum gas cost overflow")?;

        let state = match state_arc.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("State arc lock poisoned in pre-execution checks, recovering...");
                poisoned.into_inner()
            }
        };
        Self::validate_transaction_object_access(&state, tx, sender_addr)?;
        if max_gas_cost > 0 {
            let balance = state.resolve_owner_native_balance(sender_addr).unwrap_or(0);
            if balance < max_gas_cost {
                let msg = format!(
                    "Insufficient balance for maximum gas: need {}, have {}",
                    max_gas_cost, balance
                );
                changeset.mark_failed(msg);
                Self::apply_gas_and_sequence(
                    &mut changeset,
                    sender_addr,
                    base_gas_cost.min(balance),
                    gas_meter.gas_used,
                )?;
                Self::annotate_changeset_object_effects(&state, &mut changeset)?;
                return Ok(changeset);
            }
        }

        match tx {
            Transaction::PublishModule {
                sender,
                module_bytes,
                ..
            } => {
                match runtime.publish_module_with_context_and_persistence(
                    module_bytes.clone(),
                    KanariAddress::parse_to_account_address(sender)?,
                    Some((tx.gas_limit(), effective_gas_price(tx.gas_price()))),
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,
                ) {
                    Ok(move_cs) => changeset.merge(move_cs),
                    Err(e) => {
                        changeset.mark_failed(format!("Publish failed: {}", e));
                    }
                }
            }
            Transaction::UpgradeModule {
                sender,
                module_bytes,
                ..
            } => {
                match runtime.upgrade_module_with_context_and_persistence(
                    module_bytes.clone(),
                    KanariAddress::parse_to_account_address(sender)?,
                    Some((tx.gas_limit(), effective_gas_price(tx.gas_price()))),
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,
                ) {
                    Ok(move_cs) => changeset.merge(move_cs),
                    Err(e) => {
                        changeset.mark_failed(format!("Upgrade failed: {}", e));
                    }
                }
            }
            Transaction::PublishPackage {
                sender, modules, ..
            } => {
                let package_modules = modules
                    .iter()
                    .map(|module| (module.module_name.clone(), module.module_bytes.clone()))
                    .collect();
                match runtime.publish_package_with_context_and_persistence(
                    package_modules,
                    KanariAddress::parse_to_account_address(sender)?,
                    Some((tx.gas_limit(), effective_gas_price(tx.gas_price()))),
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,
                ) {
                    Ok(move_cs) => changeset.merge(move_cs),
                    Err(e) => {
                        changeset.mark_failed(format!("Publish package failed: {}", e));
                    }
                }
            }
            Transaction::UpgradePackage {
                sender, modules, ..
            } => {
                let package_modules = modules
                    .iter()
                    .map(|module| (module.module_name.clone(), module.module_bytes.clone()))
                    .collect();
                match runtime.upgrade_package_with_context_and_persistence(
                    package_modules,
                    KanariAddress::parse_to_account_address(sender)?,
                    Some((tx.gas_limit(), effective_gas_price(tx.gas_price()))),
                    timestamp,
                    Some(tx.hash()),
                    persist_runtime_state,
                ) {
                    Ok(move_cs) => changeset.merge(move_cs),
                    Err(e) => {
                        changeset.mark_failed(format!("Upgrade package failed: {}", e));
                    }
                }
            }

            Transaction::ExecuteFunction {
                module,
                function,
                type_args,
                args,
                ..
            } => {
                if let Some(kanari_types::transaction::NativeCall::Burn { amount }) =
                    native_call.clone()
                {
                    let state = state_arc.read().unwrap_or_else(|e| e.into_inner());
                    match Self::execute_backend_native_burn(
                        &state,
                        tx,
                        sender_addr,
                        amount,
                        base_gas_cost,
                        &mut changeset,
                    ) {
                        Ok(()) => {}
                        Err(e) => {
                            changeset.mark_failed(format!("Execution failed: {}", e));
                        }
                    }
                    Self::apply_gas_and_sequence(
                        &mut changeset,
                        sender_addr,
                        base_gas_cost,
                        gas_meter.gas_used,
                    )?;
                    Self::annotate_changeset_object_effects(&state, &mut changeset)?;
                    return Ok(changeset);
                }

                let parts: Vec<&str> = module.split("::").collect();
                if parts.len() != 2 {
                    anyhow::bail!("Invalid module format. Expected: address::module");
                } else {
                    let addr = KanariAddress::parse_to_account_address(parts[0])?;
                    let module_id = ModuleId::new(
                        addr,
                        move_core_types::identifier::Identifier::new(parts[1])?,
                    );
                    changeset.record_resolver_reads([format!(
                        "module:{}:{}",
                        module_id.address().to_hex_literal(),
                        module_id.name()
                    )
                    .into_bytes()]);
                    let type_tags = type_args
                        .iter()
                        .map(|s| parse_type_tag(s.as_str()).require("Invalid type argument"))
                        .collect::<Result<Vec<_>>>();
                    let type_tags = type_tags?;
                    match runtime.execute_entry_function_with_object_context_and_persistence(
                        &module_id,
                        function,
                        type_tags,
                        args.clone(),
                        EntryFunctionObjectContext {
                            object_inputs: tx.object_inputs(),
                            sender: Some(sender_addr),
                            gas_info: Some((tx.gas_limit(), effective_gas_price(tx.gas_price()))),
                            timestamp,
                            tx_hash: Some(tx.hash()),
                            persist_runtime_state,
                            state_overlay: state_overlay.clone(),
                        },
                    ) {
                        Ok(move_cs) => changeset.merge(move_cs),
                        Err(e) => {
                            changeset.mark_failed(format!("Execution failed: {}", e));
                            changeset.set_gas_used(tx.gas_limit());
                        }
                    }
                }
            }
        }

        let charged_gas_units = changeset.gas_used.max(gas_meter.gas_used);
        ensure!(
            charged_gas_units <= tx.gas_limit(),
            "Runtime gas usage {} exceeds transaction gas limit {}",
            charged_gas_units,
            tx.gas_limit()
        );
        let charged_gas_cost = charged_gas_units
            .checked_mul(effective_gas_price(tx.gas_price()))
            .context("Transaction gas cost overflow")?;
        Self::apply_gas_and_sequence(
            &mut changeset,
            sender_addr,
            charged_gas_cost,
            charged_gas_units,
        )?;
        let state = state_arc.read().unwrap_or_else(|e| e.into_inner());
        Self::annotate_changeset_object_effects(&state, &mut changeset)?;
        Ok(changeset)
    }

    fn annotate_changeset_object_effects(
        state: &StateManager,
        changeset: &mut ChangeSet,
    ) -> Result<()> {
        let mut object_changes = Vec::new();

        for (object_id, created) in &changeset.created_objects {
            let next_ref = created.object_ref(object_id);
            if let Some(existing) = state.get_object(object_id)? {
                let previous_owner = existing.owner_kind();
                let next_owner = created.owner_kind();
                let previous_ref = existing.object_ref(object_id);
                let change_type = if existing.owner != created.owner {
                    ObjectChangeKind::Transferred
                } else {
                    ObjectChangeKind::Mutated
                };
                object_changes.push(ObjectChange {
                    change_type,
                    object_ref: next_ref,
                    previous_object_ref: Some(previous_ref),
                    type_: Some(created.type_.clone()),
                    owner: Some(next_owner),
                    previous_owner: Some(previous_owner),
                    previous_version: Some(existing.version),
                });
            } else {
                object_changes.push(ObjectChange {
                    change_type: ObjectChangeKind::Created,
                    object_ref: next_ref,
                    previous_object_ref: None,
                    type_: Some(created.type_.clone()),
                    owner: Some(created.owner_kind()),
                    previous_owner: None,
                    previous_version: None,
                });
            }
        }

        for object_id in &changeset.deleted_objects {
            let (previous_owner, previous_version, previous_object_ref) =
                if let Some(existing) = state.get_object(object_id)? {
                    (
                        Some(existing.owner_kind()),
                        Some(existing.version),
                        Some(existing.object_ref(object_id)),
                    )
                } else {
                    (None, None, None)
                };
            object_changes.push(ObjectChange {
                change_type: ObjectChangeKind::Deleted,
                object_ref: previous_object_ref
                    .clone()
                    .unwrap_or_else(|| ObjectRef::new(object_id.clone(), previous_version, None)),
                previous_object_ref,
                type_: None,
                owner: None,
                previous_owner,
                previous_version,
            });
        }

        changeset.set_explicit_object_changes(object_changes);
        Ok(())
    }

    fn validate_transaction_object_access(
        state: &StateManager,
        tx: &Transaction,
        sender_addr: AccountAddress,
    ) -> Result<()> {
        let strict_metadata = tx.requires_strict_object_metadata();
        let mutable_input_ids = tx
            .object_inputs()
            .into_iter()
            .filter(|input| input.mutable)
            .map(|input| input.object_ref.object_id)
            .collect::<HashSet<_>>();

        for input in tx.object_inputs() {
            if strict_metadata {
                ensure!(
                    input.object_ref.version.is_some() && input.object_ref.digest.is_some(),
                    "ExecuteFunction object input {} must include (object_id, version, digest)",
                    input.object_ref.object_id
                );
                ensure!(
                    input.owner.is_some(),
                    "ExecuteFunction object input {} must declare owner semantics",
                    input.object_ref.object_id
                );
            }
            if matches!(input.owner, Some(ObjectOwnerKind::Immutable)) && input.mutable {
                anyhow::bail!(
                    "Immutable object input {} cannot be declared mutable",
                    input.object_ref.object_id
                );
            }

            let stored = state
                .get_object(&input.object_ref.object_id)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Referenced object {} does not exist",
                        input.object_ref.object_id
                    )
                })?;

            if let Some(version) = input.object_ref.version
                && stored.version != version
            {
                anyhow::bail!(
                    "Object version mismatch for {}: expected {}, found {}",
                    input.object_ref.object_id,
                    version,
                    stored.version
                );
            }

            if let Some(digest) = &input.object_ref.digest
                && stored.digest() != *digest
            {
                anyhow::bail!("Object digest mismatch for {}", input.object_ref.object_id);
            }

            if let Some(owner) = input.owner {
                match owner {
                    ObjectOwnerKind::AddressOwner(expected_owner) => {
                        ensure!(
                            matches!(stored.owner_kind, ObjectOwnerKind::AddressOwner(_)),
                            "Object {} is not canonically address-owned",
                            input.object_ref.object_id
                        );
                        let expected = KanariAddress::parse_to_account_address(&expected_owner)?;
                        if stored.owner != expected {
                            anyhow::bail!(
                                "Owned object {} is not owned by declared owner {}",
                                input.object_ref.object_id,
                                expected_owner
                            );
                        }
                    }
                    ObjectOwnerKind::Shared => {
                        ensure!(
                            matches!(stored.owner_kind, ObjectOwnerKind::Shared),
                            "Object {} is not canonically shared",
                            input.object_ref.object_id
                        );
                    }
                    ObjectOwnerKind::Immutable => {
                        ensure!(
                            matches!(stored.owner_kind, ObjectOwnerKind::Immutable),
                            "Object {} is not canonically immutable",
                            input.object_ref.object_id
                        );
                    }
                }
            }
        }

        if let Some(gas_payment) = tx.gas_payment() {
            let expected_owner = KanariAddress::parse_to_account_address(&gas_payment.owner)?;
            if expected_owner != sender_addr {
                anyhow::bail!("Gas payment owner must match sender");
            }
            if strict_metadata {
                ensure!(
                    !gas_payment.payment_objects.is_empty(),
                    "ExecuteFunction gas payment must declare at least one payment object"
                );
            }

            for payment in gas_payment.payment_objects {
                if strict_metadata {
                    ensure!(
                        payment.version.is_some() && payment.digest.is_some(),
                        "ExecuteFunction gas payment {} must include (object_id, version, digest)",
                        payment.object_id
                    );
                }
                let stored = state.get_object(&payment.object_id)?.ok_or_else(|| {
                    anyhow::anyhow!("Gas payment object {} does not exist", payment.object_id)
                })?;
                ensure!(
                    Self::is_native_gas_coin_type(&stored.type_),
                    "Gas payment object {} must be Coin<{}>, found {}",
                    payment.object_id,
                    GAS_COIN,
                    stored.type_
                );
                if stored.owner != expected_owner {
                    anyhow::bail!(
                        "Gas payment object {} is not owned by sender",
                        payment.object_id
                    );
                }
                if let Some(version) = payment.version
                    && stored.version != version
                {
                    anyhow::bail!(
                        "Gas payment version mismatch for {}: expected {}, found {}",
                        payment.object_id,
                        version,
                        stored.version
                    );
                }
                if let Some(digest) = &payment.digest
                    && stored.digest() != *digest
                {
                    anyhow::bail!("Gas payment digest mismatch for {}", payment.object_id);
                }
                ensure!(
                    !mutable_input_ids.contains(&payment.object_id),
                    "Gas payment object {} cannot overlap with a mutable object input",
                    payment.object_id
                );
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn collect_transaction_effects_strict(
        &self,
        transactions: &[SignedTransaction],
        timestamp: Option<u64>,
        allow_stale_object_versions: bool,
    ) -> Result<Vec<TransactionEffects>> {
        if transactions.is_empty() {
            return Ok(Vec::new());
        }

        let mut state_snapshot = self.state_read().clone();
        state_snapshot
            .repair_legacy_native_wallet_overcount()
            .context("Failed to repair legacy native wallet overcount before effect collection")?;
        let state_arc = Arc::new(RwLock::new(state_snapshot));

        if let Some(ts) = timestamp {
            self.apply_system_prologue_to_state(&state_arc, ts, false)?;
        }

        let mut effects = Vec::with_capacity(transactions.len());
        for signed_tx in transactions {
            let changeset = match self.execute_transaction_with_runtime_internal(
                &signed_tx.transaction,
                &self.runtime_pool[0],
                &state_arc,
                false,
                timestamp,
                false,
            ) {
                Ok(changeset) => changeset,
                Err(error)
                    if allow_stale_object_versions
                        && error.to_string().contains("Object version mismatch") =>
                {
                    effects.push(TransactionEffects {
                        status: "failed".to_string(),
                        gas_used: 0,
                        gas_payment: signed_tx.transaction.gas_payment(),
                        input_objects: signed_tx
                            .transaction
                            .object_inputs()
                            .into_iter()
                            .map(|input| input.object_ref)
                            .collect(),
                        shared_inputs: Vec::new(),
                        immutable_inputs: Vec::new(),
                        gas_object_refs: signed_tx
                            .transaction
                            .gas_payment()
                            .map(|payment| payment.payment_objects)
                            .unwrap_or_default(),
                        object_changes: Vec::new(),
                        created: Vec::new(),
                        mutated: Vec::new(),
                        deleted: Vec::new(),
                        transferred: Vec::new(),
                        causal_edges: Vec::new(),
                        error_message: Some(error.to_string()),
                    });
                    continue;
                }
                Err(error) => return Err(error),
            };
            effects.push(changeset.effects(signed_tx.transaction.gas_payment()));

            let mut state_write = state_arc.write().unwrap_or_else(|e| e.into_inner());
            state_write.apply_changeset(&changeset)?;
        }

        Ok(effects)
    }

    fn dag_engine_instance(&self) -> Result<DagEngine> {
        let mut dag_engine_guard = match self.dag_engine.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("DAG engine lock poisoned while initializing, recovering...");
                poisoned.into_inner()
            }
        };
        if dag_engine_guard.is_none() {
            let signing_key = self.consensus_signing_key.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "DAG consensus requires an explicit signing key. Call set_consensus_signing_key() before producing or syncing DAG vertices."
                )
            })?;
            let engine = DagEngine::new_secure(
                Arc::new(self.clone_for_dag()),
                self.authority_id.clone(),
                self.authorities.clone(),
                signing_key,
                self.consensus_public_keys.clone(),
            )?;
            *dag_engine_guard = Some(engine);
        }

        dag_engine_guard
            .as_ref()
            .cloned()
            .require("Failed to initialize DAG engine")
    }

    pub fn produce_checkpoint(&self) -> Result<CheckpointProductionInfo> {
        if self.authorities.len() <= 1 {
            let started_at = std::time::Instant::now();
            let transactions = self.pending_owned_fast_path_transactions_snapshot();
            let selected_at = std::time::Instant::now();
            if !transactions.is_empty() {
                return self.produce_owned_fast_checkpoint_from_selection(
                    transactions,
                    started_at,
                    selected_at,
                );
            }
        }
        let dag_engine = self.dag_engine_instance()?;
        dag_engine.produce_vertex()
    }

    pub fn produce_owned_fast_checkpoint(&self) -> Result<CheckpointProductionInfo> {
        let started_at = std::time::Instant::now();
        let transactions = self.pending_owned_fast_path_transactions_snapshot();
        let selected_at = std::time::Instant::now();
        self.produce_owned_fast_checkpoint_from_selection(transactions, started_at, selected_at)
    }

    fn produce_owned_fast_checkpoint_from_selection(
        &self,
        transactions: Vec<SignedTransaction>,
        started_at: std::time::Instant,
        selected_at: std::time::Instant,
    ) -> Result<CheckpointProductionInfo> {
        let profile = matches!(
            std::env::var("KANARI_OWNED_FASTPATH_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        if transactions.is_empty() {
            return Ok(CheckpointProductionInfo {
                vertex_id: "owned-fastpath-empty".to_string(),
                round: 0,
                tx_count: 0,
                executed: 0,
                failed: 0,
                events: Vec::new(),
                checkpoint: None,
                vertex: None,
            });
        }

        let (sequence, prev_hash, timestamp) = {
            let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
            (
                chain.latest_checkpoint().sequence.saturating_add(1),
                chain.latest_checkpoint().hash()?,
                chain
                    .latest_checkpoint()
                    .timestamp
                    .saturating_add(1)
                    .max(chain.height().saturating_add(1)),
            )
        };
        let mut checkpoint = Checkpoint::new(
            sequence,
            Vec::new(),
            transactions,
            Vec::new(),
            timestamp,
            prev_hash,
        );
        let prepared = self.prepare_conflict_free_checkpoint_state(&checkpoint)?;
        let prepared_at = std::time::Instant::now();
        checkpoint.state_root = prepared.state_root;
        checkpoint.transaction_effects = prepared.effects.into();
        let aggregated_at = std::time::Instant::now();

        let executed = checkpoint
            .transaction_effects
            .iter()
            .filter(|effects| effects.status == "success")
            .count();
        let failed = checkpoint
            .transaction_effects
            .len()
            .saturating_sub(executed);
        let tx_count = checkpoint.transactions.len();
        let checkpoint_info = CheckpointInfo {
            sequence: checkpoint.sequence,
            vertex_count: checkpoint.vertices.len(),
            tx_count,
        };

        self.apply_prepared_checkpoint(
            checkpoint,
            prepared.state,
            prepared.smt_changes,
            prepared.transactions,
            false,
        )?;
        self.maybe_audit_owned_fastpath_checkpoint(checkpoint_info.sequence)?;
        let applied_at = std::time::Instant::now();
        if profile {
            eprintln!(
                "owned-fastpath profile: txs={} select={:.6}s prepare={:.6}s aggregate={:.6}s apply={:.6}s total={:.6}s",
                tx_count,
                selected_at.duration_since(started_at).as_secs_f64(),
                prepared_at.duration_since(selected_at).as_secs_f64(),
                aggregated_at.duration_since(prepared_at).as_secs_f64(),
                applied_at.duration_since(aggregated_at).as_secs_f64(),
                applied_at.duration_since(started_at).as_secs_f64(),
            );
        }

        Ok(CheckpointProductionInfo {
            vertex_id: format!("owned-fastpath:{}", checkpoint_info.sequence),
            round: checkpoint_info.sequence,
            tx_count,
            executed,
            failed,
            events: Vec::new(),
            checkpoint: Some(checkpoint_info),
            vertex: None,
        })
    }

    fn maybe_audit_owned_fastpath_checkpoint(&self, sequence: u64) -> Result<()> {
        let Some(interval) = Self::owned_fastpath_audit_interval() else {
            return Ok(());
        };
        if !sequence.is_multiple_of(interval) {
            return Ok(());
        }

        let state = self.state_read();
        state
            .validate_cached_supply_invariants()
            .context("Owned-fastpath periodic cached supply audit failed")?;
        if Self::owned_fastpath_smt_audit_enabled() {
            state
                .validate_smt_consistency()
                .context("Owned-fastpath periodic SMT audit failed")?;
        }
        Ok(())
    }

    pub fn dag_production_policy(&self) -> Result<DagProductionPolicy> {
        let dag_engine = self.dag_engine_instance()?;
        Ok(dag_engine.production_policy())
    }

    pub fn latest_own_dag_vertices(&self, limit: usize) -> Result<Vec<DagVertex>> {
        self.dag_engine_instance()?.latest_own_vertices(limit)
    }

    pub fn dag_vertices_through_round_for_sync(
        &self,
        target_round: u64,
        limit: usize,
    ) -> Result<Vec<DagVertex>> {
        self.dag_engine_instance()?
            .vertices_through_round_for_sync(target_round, limit)
    }

    pub fn dag_missing_parent_rounds_for_sync(&self, vertices: &[DagVertex]) -> Result<Vec<u64>> {
        Ok(self
            .dag_engine_instance()?
            .missing_parent_rounds_for_sync(vertices))
    }

    pub fn dag_vertices_for_checkpoint_sync(
        &self,
        checkpoint_vertices: &[[u8; 32]],
        limit: usize,
    ) -> Result<Vec<DagVertex>> {
        self.dag_engine_instance()?
            .checkpoint_vertices_for_sync(checkpoint_vertices, limit)
    }

    pub fn add_network_dag_vertex(&self, vertex: DagVertex) -> Result<()> {
        self.dag_engine_instance()?.add_network_vertex(vertex)
    }

    fn clone_for_dag(&self) -> BlockchainEngine {
        BlockchainEngine {
            blockchain: self.blockchain.clone(),
            state: self.state.clone(),
            mempool: self.mempool.clone(),
            invalid_pending_drop_count: self.invalid_pending_drop_count.clone(),
            persistent_store: self.persistent_store.clone(),
            runtime_pool: self.runtime_pool.clone(),
            proof_cache: self.proof_cache.clone(),
            dag_engine: Arc::new(RwLock::new(None)),
            authority_id: self.authority_id.clone(),
            authorities: self.authorities.clone(),
            consensus_signing_key: self.consensus_signing_key.clone(),
            consensus_public_keys: self.consensus_public_keys.clone(),
        }
    }

    pub fn set_authorities(&mut self, authority_id: String, authorities: Vec<String>) {
        self.authority_id = normalize_consensus_authority_id(authority_id);
        self.authorities = authorities
            .into_iter()
            .map(normalize_consensus_authority_id)
            .collect();
        self.consensus_signing_key = None;
        self.consensus_public_keys.clear();
        match self.dag_engine.write() {
            Ok(mut guard) => *guard = None,
            Err(poisoned) => {
                log::error!("DAG engine lock poisoned in set_authorities, recovering...");
                *poisoned.into_inner() = None;
            }
        }
    }

    pub fn authority_id(&self) -> &str {
        &self.authority_id
    }

    pub fn authorities(&self) -> &[String] {
        &self.authorities
    }

    pub fn set_consensus_signing_key(
        &mut self,
        local_signing_key: ed25519_dalek::SigningKey,
        authority_public_keys: BTreeMap<String, Vec<u8>>,
    ) -> Result<()> {
        let authority_public_keys: BTreeMap<String, Vec<u8>> = authority_public_keys
            .into_iter()
            .map(|(authority, key)| (normalize_consensus_authority_id(authority), key))
            .collect();
        let local_public_key = local_signing_key.verifying_key().to_bytes().to_vec();
        let expected_public_key =
            authority_public_keys
                .get(&self.authority_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Missing consensus public key for local authority {}",
                        self.authority_id
                    )
                })?;
        if *expected_public_key != local_public_key {
            anyhow::bail!("Consensus signing key does not match local authority public key");
        }
        for authority in &self.authorities {
            let key = authority_public_keys.get(authority).ok_or_else(|| {
                anyhow::anyhow!("Missing consensus public key for authority {}", authority)
            })?;
            let key_bytes: [u8; 32] = key.as_slice().try_into().map_err(|_| {
                anyhow::anyhow!("Invalid consensus public key length for {}", authority)
            })?;
            ed25519_dalek::VerifyingKey::from_bytes(&key_bytes).map_err(|e| {
                anyhow::anyhow!("Invalid consensus public key for {}: {}", authority, e)
            })?;
        }

        self.consensus_signing_key = Some(local_signing_key);
        self.consensus_public_keys = authority_public_keys;
        match self.dag_engine.write() {
            Ok(mut guard) => *guard = None,
            Err(poisoned) => {
                log::error!("DAG engine lock poisoned while replacing consensus key");
                *poisoned.into_inner() = None;
            }
        }

        Ok(())
    }

    pub fn export_consensus_metrics_prometheus(&self) -> Result<String> {
        let invalid_pending_drop_count = self.invalid_pending_drop_count.load(Ordering::Relaxed);
        let dag_engine_guard = match self.dag_engine.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("DAG engine lock poisoned in export_consensus_metrics_prometheus");
                poisoned.into_inner()
            }
        };

        if let Some(dag_engine) = dag_engine_guard.as_ref() {
            return Ok(format!(
                "{}# HELP dropped_invalid_pending_tx_total Total invalid pending transactions dropped after deterministic execution failure\n\
                 # TYPE dropped_invalid_pending_tx_total counter\n\
                 dropped_invalid_pending_tx_total {}\n",
                dag_engine.metrics_prometheus(),
                invalid_pending_drop_count,
            ));
        }

        Ok(format!(
            "# HELP dropped_invalid_pending_tx_total Total invalid pending transactions dropped after deterministic execution failure\n\
             # TYPE dropped_invalid_pending_tx_total counter\n\
             dropped_invalid_pending_tx_total {}\n",
            invalid_pending_drop_count,
        ))
    }

    pub(crate) fn record_invalid_pending_drop(&self, removed: usize) {
        self.invalid_pending_drop_count
            .fetch_add(removed as u64, Ordering::Relaxed);
    }
}

#[cfg(test)]
#[path = "../tests/unit/engine_tests.rs"]
mod tests;
