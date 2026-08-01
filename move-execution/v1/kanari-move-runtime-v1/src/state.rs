// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::changeset::{ChangeSet, CreatedObject, StateAccessSet};
use crate::common::ids::canonical_object_id;
use crate::common::keys::{metadata_key, object_key, owned_objects_key};
use crate::storage::object_storage::StoredObject;
use crate::storage::persistent_store::PersistentStore;
use anyhow::{Context, Result, ensure};
use kanari_crypto::hash_data_blake3;
use kanari_types::balance::BalanceModule;
use kanari_types::balance::BalanceRecord;
use kanari_types::clock::ClockModule;
use kanari_types::coin::{CoinModule, TreasuryCap};
use kanari_types::error::KanariUnwrapExt;
use kanari_types::event::Event;
use kanari_types::gas_coin::GAS_COIN;
use kanari_types::object::{IDRecord, UIDRecord};
use kanari_types::transaction::ObjectOwnerKind;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::{StructTag, TypeTag};
use rocksdb::WriteBatch;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use smt;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::str::FromStr;
use std::sync::Arc;

mod apply;
mod supply;

const SYSTEM_CLOCK_OBJECT_ID_KEY: &[u8] = b"system:clock_object_id";
const GENESIS_INITIALIZED_KEY: &[u8] = b"system:genesis_initialized";
const OWNER_INDEX_KEY: &[u8] = b"owner_index";
const LEGACY_ACCOUNT_INDEX_KEY: &[u8] = b"account_index";
const OBJECT_LOCKED_COIN_RECORDS_KEY: &[u8] = b"object_locked_coin_records";
const RUNTIME_STATE_SCHEMA_KEY: &[u8] = b"runtime:state_schema_version";
const RUNTIME_STATE_SCHEMA_VERSION: u32 = 1;
const WALLET_SUPPLY_INDEX_VERSION_KEY: &[u8] = b"runtime:wallet_supply_index_version";
const WALLET_SUPPLY_INDEX_VERSION: u32 = 1;
const ACCESS_VERSIONS_KEY: &[u8] = b"runtime:state_access_versions";
const ACCESS_VERSION_PREFIX: &[u8] = b"runtime:state_access_version:";
const UID_SIZE: usize = 32;
const U64_SIZE: usize = 8;

type RawStateKey = Vec<u8>;
type RawStateValue = Vec<u8>;
type RawStateUpdate = (RawStateKey, RawStateValue);
type RawStateDelete = RawStateKey;
type OverlaySmtChanges = (Vec<RawStateUpdate>, Vec<RawStateDelete>);
pub type PrecomputedSmtChanges = (Vec<(Vec<u8>, Vec<u8>)>, Vec<Vec<u8>>, smt::SmtNodeOverlay);
type DerivedIndexes = (
    Vec<String>,
    BTreeMap<AccountAddress, Vec<String>>,
    Vec<String>,
);

/// Owner state in the blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerState {
    pub address: AccountAddress,
    pub nonce: u64,
    pub modules: BTreeSet<String>,
    /// Token balances: token_type -> BalanceRecord
    /// Example: "0x840512ff...::james::JAMES" -> BalanceRecord(1000000000)
    pub token_balances: BTreeMap<String, BalanceRecord>,
}

impl OwnerState {
    pub fn new(address: AccountAddress) -> Self {
        Self {
            address,
            nonce: 0,
            modules: BTreeSet::new(),
            token_balances: BTreeMap::new(),
        }
    }

    pub fn with_native_balance(address: AccountAddress, balance: u64) -> Self {
        let mut account = Self::new(address);
        if balance > 0 {
            account.set_token_balance(GAS_COIN.to_string(), BalanceRecord::new(balance));
        }
        account
    }

    fn add_module(&mut self, module_name: String) {
        self.modules.insert(module_name);
    }

    pub fn set_token_balance(&mut self, token_type: String, amount: BalanceRecord) {
        self.token_balances.insert(token_type, amount);
    }

    fn set_token_balance_value(&mut self, token_type: &str, amount: u64) {
        if amount == 0 {
            self.token_balances.remove(token_type);
        } else {
            self.set_token_balance(token_type.to_string(), BalanceRecord::new(amount));
        }
    }

    pub fn get_token_balance(&self, token_type: &str) -> u64 {
        self.token_balances
            .get(token_type)
            .map(|b| b.value())
            .unwrap_or(0)
    }

    pub fn native_balance(&self) -> u64 {
        self.get_token_balance(GAS_COIN)
    }

    pub fn owner_address(&self) -> AccountAddress {
        self.address
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty() && self.token_balances.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenSupplySummary {
    pub token_type: String,
    pub total_supply: u64,
    pub wallet_visible_supply: u64,
    pub object_locked_supply: u64,
    pub accounted_supply: u64,
    pub untracked_supply: u64,
}

/// Read-only diagnostics for the incremental sparse Merkle tree.
///
/// A full audit is intentionally opt-in because it scans canonical state and
/// the persisted SMT leaves. The cheap status path only reads roots, schema
/// versions, and the pending overlay summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmtDiagnostics {
    pub enabled: bool,
    pub persisted_root: Option<String>,
    pub effective_root: String,
    pub overlay_entries: usize,
    pub overlay_updates: usize,
    pub overlay_deletes: usize,
    pub canonical_membership_changed: bool,
    pub runtime_schema_version: Option<u32>,
    pub expected_runtime_schema_version: u32,
    pub wallet_supply_index_version: Option<u32>,
    pub expected_wallet_supply_index_version: u32,
    pub audit_requested: bool,
    pub audit_performed: bool,
    pub persisted_leaf_count: Option<usize>,
    pub consistent: Option<bool>,
    pub consistency_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ObjectLockedCoinRecord {
    pub holder_object_id: String,
    pub holder_type: String,
    pub owner: AccountAddress,
    pub token_type: String,
    pub amount: u64,
}

/// Global state manager for accounts and balances
/// This is a pure data layer that applies ChangeSet from Move VM execution
///
/// Refactored to use RocksDB via PersistentStore for unlimited capacity.
#[derive(Debug, Clone)]
pub struct StateManager {
    pub store: Arc<PersistentStore>,

    /// Overlay for speculative execution / buffering
    /// Key -> Some(Value) or None (Deleted)
    pub overlay: BTreeMap<Vec<u8>, Option<Vec<u8>>>,

    // Cache for total supply to avoid frequent DB reads
    pub total_supply: u64,

    /// In-memory cache for tracking total token supplies in real-time
    pub global_token_supplies: BTreeMap<String, u64>,

    // SMT for state root calculation (Optional: requires DB backend)
    pub smt: Option<Arc<smt::SparseMerkleTree>>,
    pub events: Vec<Event>,

    /// Monotonic versions for deterministic conflict validation. Runtime metadata
    /// is persisted atomically with the overlay but excluded from the state root.
    access_versions: BTreeMap<Vec<u8>, u64>,
    access_epoch: u64,
}

impl StateManager {
    fn collect_derived_indexes(&self) -> Result<DerivedIndexes> {
        let entries = self
            .store
            .logical_entries()
            .context("Failed to read state entries for derived index rebuild")?;

        let mut owner_ids = BTreeSet::new();
        let mut owned_objects: BTreeMap<AccountAddress, BTreeSet<String>> = BTreeMap::new();
        let mut object_ids = BTreeSet::new();

        for (key, value) in entries {
            if let Some(owner_bytes) = key.strip_prefix(b"account:") {
                if let Ok(owner) = AccountAddress::from_bytes(owner_bytes) {
                    owner_ids.insert(owner.to_hex_literal());
                }
                continue;
            }

            let Some(object_id_bytes) = key.strip_prefix(b"object:") else {
                continue;
            };
            let Ok(mut object_id) = String::from_utf8(object_id_bytes.to_vec()) else {
                continue;
            };
            if let Some(canonical_id) = canonical_object_id(&object_id) {
                object_id = canonical_id;
            }
            object_ids.insert(object_id.clone());

            let Ok(stored) = bcs::from_bytes::<StoredObject>(&value) else {
                continue;
            };
            if matches!(stored.owner_kind, ObjectOwnerKind::AddressOwner(_)) {
                owner_ids.insert(stored.owner.to_hex_literal());
                owned_objects
                    .entry(stored.owner)
                    .or_default()
                    .insert(object_id);
            }
        }

        Ok((
            owner_ids.into_iter().collect(),
            owned_objects
                .into_iter()
                .map(|(owner, ids)| (owner, ids.into_iter().collect()))
                .collect(),
            object_ids.into_iter().collect(),
        ))
    }

    fn load_index_list(&self, key: &[u8]) -> Result<Vec<String>> {
        Ok(self.load_internal(key)?.unwrap_or_default())
    }

    fn save_index_list(&mut self, key: &[u8], ids: &[String]) -> Result<()> {
        self.save_internal(key, ids)
    }

    fn add_to_index_list(&mut self, key: &[u8], value: String) -> Result<()> {
        let mut index = self.load_index_list(key)?;
        let Err(pos) = index.binary_search(&value) else {
            return Ok(());
        };

        index.insert(pos, value);
        self.save_index_list(key, &index)?;
        Ok(())
    }

    fn remove_from_index_list(&mut self, key: &[u8], value: &str) -> Result<()> {
        let mut index = self.load_index_list(key)?;
        let Ok(pos) = index.binary_search_by(|entry| entry.as_str().cmp(value)) else {
            return Ok(());
        };

        index.remove(pos);
        self.save_index_list(key, &index)?;
        Ok(())
    }

    /// Retrieves all Collection IDs from the index
    pub fn try_get_all_collection_ids(&self) -> Result<Vec<String>> {
        self.load_index_list(b"nft_collection_index")
    }

    pub fn get_all_collection_ids(&self) -> Vec<String> {
        self.try_get_all_collection_ids()
            .unwrap_or_else(|error| Self::log_index_fallback("nft_collection_index", error))
    }

    /// Retrieves NFT IDs for the specified collection from the index
    pub fn try_get_collection_nft_ids(&self, collection_id: &str) -> Result<Vec<String>> {
        self.load_index_list(&metadata_key(b"collection_members:", collection_id))
    }

    pub fn get_collection_nft_ids(&self, collection_id: &str) -> Vec<String> {
        self.try_get_collection_nft_ids(collection_id)
            .unwrap_or_else(|error| {
                Self::log_index_fallback(&format!("collection_members:{collection_id}"), error)
            })
    }

    fn log_index_fallback(index_name: &str, error: anyhow::Error) -> Vec<String> {
        log::error!("[StateManager] Failed to load index {index_name}: {error:#}");
        Vec::new()
    }

    fn is_balance_struct(struct_tag: &StructTag) -> bool {
        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();
        (module_name == CoinModule::COIN_MODULE && struct_name == CoinModule::COIN_STRUCT)
            || (module_name == BalanceModule::BALANCE_MODULE
                && struct_name == BalanceModule::BALANCE_STRUCT)
    }

    fn extract_balance_from_object_bytes(data: &[u8], struct_tag: &StructTag) -> Option<u64> {
        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();

        if module_name == CoinModule::COIN_MODULE && struct_name == CoinModule::COIN_STRUCT {
            if data.len() < UID_SIZE + U64_SIZE {
                return None;
            }
            let bytes: [u8; U64_SIZE] = data[UID_SIZE..(UID_SIZE + U64_SIZE)].try_into().ok()?;
            return Some(u64::from_le_bytes(bytes));
        }

        if module_name == BalanceModule::BALANCE_MODULE
            && struct_name == BalanceModule::BALANCE_STRUCT
        {
            if data.len() < U64_SIZE {
                return None;
            }
            let bytes: [u8; U64_SIZE] = data[data.len() - U64_SIZE..].try_into().ok()?;
            return Some(u64::from_le_bytes(bytes));
        }

        None
    }

    fn write_balance_to_object_bytes(data: &mut [u8], struct_tag: &StructTag, amount: u64) -> bool {
        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();
        let bytes = amount.to_le_bytes();

        if module_name == CoinModule::COIN_MODULE && struct_name == CoinModule::COIN_STRUCT {
            if data.len() < UID_SIZE + U64_SIZE {
                return false;
            }
            data[UID_SIZE..(UID_SIZE + U64_SIZE)].copy_from_slice(&bytes);
            return true;
        }

        if module_name == BalanceModule::BALANCE_MODULE
            && struct_name == BalanceModule::BALANCE_STRUCT
        {
            if data.len() < U64_SIZE {
                return false;
            }
            let start = data.len() - U64_SIZE;
            data[start..].copy_from_slice(&bytes);
            return true;
        }

        false
    }
    fn token_type_from_balance_struct(struct_tag: &StructTag) -> Option<String> {
        if let Some(TypeTag::Struct(st)) = struct_tag.type_params.first() {
            return Some(format!("{}", st));
        }
        None
    }

    fn treasury_cap_token_supply(type_name: &str, data: &[u8]) -> Option<(String, u64)> {
        let struct_tag = StructTag::from_str(type_name).ok()?;
        if struct_tag.module.as_str() != CoinModule::COIN_MODULE
            || struct_tag.name.as_str() != CoinModule::TREASURY_CAP_STRUCT
        {
            return None;
        }
        let token_type = Self::token_type_from_balance_struct(&struct_tag)?;
        if data.len() < UID_SIZE + U64_SIZE {
            return None;
        }
        let supply_bytes: [u8; U64_SIZE] = data[UID_SIZE..(UID_SIZE + U64_SIZE)].try_into().ok()?;
        Some((
            Self::normalize_token_type(&token_type),
            u64::from_le_bytes(supply_bytes),
        ))
    }

    /// Create a new in-memory state manager for testing
    pub fn new_in_memory() -> Self {
        Self::try_new_in_memory().invariant("failed to create in-memory state manager")
    }

    /// Create a new in-memory state manager and surface initialization errors.
    pub(crate) fn try_new_in_memory() -> Result<Self> {
        let store = Arc::new(
            PersistentStore::open_in_memory().context("Failed to create in-memory store")?,
        );
        Self::try_new(store)
    }

    pub fn store(&self) -> Arc<PersistentStore> {
        self.store.clone()
    }

    /// Create new state with genesis allocation
    /// Total supply: 11 million KANARI = 11,000,000,000,000,000 Mist
    /// Dev address gets entire supply according to kanari.move
    pub fn new(store: Arc<PersistentStore>) -> Self {
        Self::try_new(store).invariant("failed to create state manager")
    }

    /// Create new state with genesis allocation and return initialization errors.
    pub fn try_new(store: Arc<PersistentStore>) -> Result<Self> {
        // Try to load total supply from DB
        let persisted_total_supply_record = store
            .load::<u64>(b"total_supply")
            .context("Failed to load total_supply")?;
        let persisted_total_supply = persisted_total_supply_record.unwrap_or(0);
        let genesis_marker = store
            .load::<bool>(GENESIS_INITIALIZED_KEY)
            .context("Failed to load genesis initialization marker")?
            .unwrap_or(false);

        // Load existing global token supplies from RocksDB
        let global_token_supplies = store
            .load::<BTreeMap<String, u64>>(b"global_token_supplies")
            .context("Failed to load global_token_supplies")?
            .unwrap_or_default();
        let mut access_versions = store
            .load::<BTreeMap<Vec<u8>, u64>>(ACCESS_VERSIONS_KEY)
            .context("Failed to load state access versions")?
            .unwrap_or_default();
        for (key, value) in store
            .logical_entries()
            .context("Failed to load per-key state access versions")?
        {
            let Some(state_key) = key.strip_prefix(ACCESS_VERSION_PREFIX) else {
                continue;
            };
            let version =
                bcs::from_bytes::<u64>(&value).context("Malformed per-key state access version")?;
            access_versions.insert(state_key.to_vec(), version);
        }

        // Recover native total supply from older databases that persisted the
        // native treasury/global balances but never backfilled `total_supply`.
        let recovered_total_supply = if persisted_total_supply == 0 {
            Self::load_persisted_supply_from_store(store.as_ref(), GAS_COIN)?
                .or_else(|| global_token_supplies.get(GAS_COIN).copied())
                .unwrap_or(0)
        } else {
            persisted_total_supply
        };
        let legacy_zero_supply_evidence = if persisted_total_supply_record == Some(0) {
            store
                .logical_entries()
                .context("Failed to inspect legacy state for genesis evidence")?
                .into_iter()
                .any(|(key, _)| {
                    key != b"total_supply"
                        && (key == b"module_index"
                            || key.starts_with(b"module:")
                            || key.starts_with(b"resource:")
                            || key.starts_with(b"account:")
                            || key.starts_with(b"object:")
                            || key.starts_with(b"system:"))
                })
        } else {
            false
        };
        // Presence of the legacy total-supply record is itself evidence that genesis ran,
        // only when other canonical chain state proves this is not a partial/empty DB.
        let genesis_already_initialized =
            genesis_marker || recovered_total_supply > 0 || legacy_zero_supply_evidence;

        // Initialize SMT if store is backed by RocksDB
        let smt = store
            .get_db()
            .map(|db| Arc::new(smt::SparseMerkleTree::new(db)));

        let mut state = Self {
            store,
            overlay: BTreeMap::new(),
            total_supply: recovered_total_supply,
            global_token_supplies,
            smt,
            events: Vec::new(),
            access_versions,
            access_epoch: 0,
        };

        state
            .ensure_smt_initialized()
            .context("Failed to initialize state SMT")?;

        let mut metadata_migration_pending = false;
        if genesis_already_initialized && !genesis_marker {
            state
                .save_internal(GENESIS_INITIALIZED_KEY, &true)
                .context("Failed to stage genesis initialization marker")?;
            metadata_migration_pending = true;
        }

        if persisted_total_supply_record.is_none() && recovered_total_supply > 0 {
            state
                .save_internal(b"total_supply", &recovered_total_supply)
                .context("Failed to backfill recovered total_supply")?;
            metadata_migration_pending = true;
        }

        if metadata_migration_pending {
            state
                .commit()
                .context("Failed to persist state initialization metadata")?;
        }

        // Genesis identity is explicit. Supply is protocol state and may legitimately be zero.
        // The check, initialization, and commit must remain under the same lock: a
        // second initializer can otherwise observe an empty store before waiting and
        // run genesis after the first one has committed.
        if !genesis_already_initialized {
            let genesis_store = state.store.clone();
            let _genesis_guard = genesis_store.genesis_init_guard();

            let initialized_while_waiting = state
                .store
                .load::<bool>(GENESIS_INITIALIZED_KEY)
                .context("Failed to re-check genesis initialization marker")?
                .unwrap_or(false);
            if initialized_while_waiting {
                state.total_supply = state
                    .store
                    .load::<u64>(b"total_supply")
                    .context("Failed to reload total_supply after genesis initialization")?
                    .unwrap_or(0);
                state.global_token_supplies = state
                    .store
                    .load::<BTreeMap<String, u64>>(b"global_token_supplies")
                    .context("Failed to reload global token supplies after genesis initialization")?
                    .unwrap_or_default();
            } else {
                state
                    .save_internal(GENESIS_INITIALIZED_KEY, &true)
                    .context("Failed to stage genesis initialization marker")?;
                crate::genesis::init_genesis(&mut state)
                    .context("Genesis initialization failed")?;
                // Flush genesis state to DB immediately while still holding the
                // initialization lock, so subsequent constructors see a complete DB.
                state.commit().context("Failed to commit genesis state")?;
                ensure!(
                    state.total_supply > 0,
                    "Genesis initialization completed but total_supply is still 0"
                );
            }
        }

        state
            .migrate_runtime_state_schema()
            .context("Failed to migrate runtime state schema")?;

        if state
            .repair_derived_indexes_on_startup()
            .context("Failed to rebuild derived indexes on startup")?
        {
            state
                .commit()
                .context("Failed to persist rebuilt derived indexes on startup")?;
        }

        let wallet_supply_index_version = state
            .load_internal::<u32>(WALLET_SUPPLY_INDEX_VERSION_KEY)?
            .unwrap_or(0);
        if wallet_supply_index_version < WALLET_SUPPLY_INDEX_VERSION
            && state
                .repair_legacy_native_wallet_overcount()
                .context("Failed to repair native wallet supply on startup")?
        {
            state
                .commit()
                .context("Failed to persist repaired native wallet supply on startup")?;
        }

        if state
            .ensure_wallet_supply_index()
            .context("Failed to initialize wallet supply index")?
        {
            state
                .commit()
                .context("Failed to persist wallet supply index")?;
        }

        if let Err(e) = state.validate_supply_invariants() {
            Self::report_supply_invariant_violation("on startup", &e)?;
        }

        state
            .ensure_smt_consistent()
            .context("Failed to verify state SMT")?;

        Ok(state)
    }

    fn migrate_runtime_state_schema(&mut self) -> Result<()> {
        let persisted = self
            .load_internal::<u32>(RUNTIME_STATE_SCHEMA_KEY)?
            .unwrap_or(0);
        ensure!(
            persisted <= RUNTIME_STATE_SCHEMA_VERSION,
            "State database schema {} is newer than runtime schema {}",
            persisted,
            RUNTIME_STATE_SCHEMA_VERSION
        );
        if persisted < RUNTIME_STATE_SCHEMA_VERSION {
            self.save_internal(RUNTIME_STATE_SCHEMA_KEY, &RUNTIME_STATE_SCHEMA_VERSION)?;
            self.commit()?;
        }
        Ok(())
    }

    fn ensure_smt_initialized(&self) -> Result<()> {
        let Some(smt) = &self.smt else {
            return Ok(());
        };
        if smt.root_hash()? != smt::default_hashes()[0] {
            return Ok(());
        }

        let mut entries: BTreeMap<Vec<u8>, Vec<u8>> = self
            .store
            .logical_entries()
            .context("Failed to read state entries for SMT rebuild")?
            .into_iter()
            .collect();
        Self::retain_canonical_state_root_entries(&mut entries)?;
        if entries.is_empty() {
            return Ok(());
        }

        let updates = entries.into_iter().collect::<Vec<_>>();
        smt.insert(&updates)?;
        Ok(())
    }

    fn canonical_persisted_entries(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let mut entries: BTreeMap<Vec<u8>, Vec<u8>> = self
            .store
            .logical_entries()
            .context("Failed to read canonical state entries")?
            .into_iter()
            .collect();
        Self::retain_canonical_state_root_entries(&mut entries)?;
        Ok(entries.into_iter().collect())
    }

    fn ensure_smt_consistent(&self) -> Result<()> {
        ensure!(
            self.overlay.is_empty(),
            "Cannot verify SMT with a pending overlay"
        );
        self.repair_persisted_smt().map(|_| ())
    }

    /// Reconcile the persisted SMT base after a component writes canonical
    /// Move modules directly to the shared store.
    pub fn repair_persisted_smt(&self) -> Result<bool> {
        let Some(smt) = &self.smt else {
            return Ok(false);
        };
        let entries = self.canonical_persisted_entries()?;
        let expected = smt::compute_sparse_root(&entries);
        if smt.root_hash()? != expected {
            log::warn!("Persisted SMT is stale; rebuilding canonical state tree");
            smt.rebuild(&entries)?;
            ensure!(
                smt.root_hash()? == expected,
                "SMT rebuild produced wrong root"
            );
            return Ok(true);
        }
        Ok(false)
    }

    /// Audit the persisted incremental tree against canonical state. This is a
    /// maintenance/diagnostic operation and intentionally performs a full scan.
    pub fn validate_smt_consistency(&self) -> Result<()> {
        let Some(smt) = &self.smt else {
            return Ok(());
        };
        ensure!(
            self.overlay.is_empty(),
            "Cannot audit SMT with a pending overlay"
        );
        let entries = self.canonical_persisted_entries()?;
        let mut mismatches = Vec::new();
        for (key, expected) in &entries {
            match smt.get(key)? {
                Some(actual) if actual == *expected => {}
                Some(actual) => mismatches.push(format!(
                    "{}(expected_bytes={}, actual_bytes={})",
                    String::from_utf8_lossy(key),
                    expected.len(),
                    actual.len()
                )),
                None => mismatches.push(format!("{}(missing)", String::from_utf8_lossy(key))),
            }
            if mismatches.len() == 8 {
                break;
            }
        }
        ensure!(
            mismatches.is_empty(),
            "SMT leaf mismatch for canonical keys: {}",
            mismatches.join(", ")
        );
        let expected_root = smt::compute_sparse_root(&entries);
        let stale_known_keys = if smt.persisted_leaf_count()? > entries.len() {
            let canonical_keys = entries
                .iter()
                .map(|(key, _)| key.as_slice())
                .collect::<HashSet<_>>();
            let mut stale_keys = Vec::new();
            for (key, _) in self
                .store
                .logical_entries()?
                .into_iter()
                .filter(|(key, _)| !canonical_keys.contains(key.as_slice()))
            {
                if smt.get(&key)?.is_none() {
                    continue;
                }

                let label = String::from_utf8_lossy(&key).into_owned();
                let label = if key.starts_with(b"object:") {
                    self.store
                        .load::<StoredObject>(&key)
                        .with_context(|| format!("Failed to decode stale SMT object leaf {label}"))?
                        .map(|object| {
                            format!(
                                "{}[owner={},kind={:?},type={}]",
                                label,
                                object.owner.to_hex_literal(),
                                object.owner_kind,
                                object.type_name
                            )
                        })
                        .unwrap_or(label)
                } else {
                    label
                };
                stale_keys.push(label);
                if stale_keys.len() == 8 {
                    break;
                }
            }
            stale_keys
        } else {
            Vec::new()
        };
        ensure!(
            smt.root_hash()? == expected_root,
            "SMT root mismatch after canonical leaf audit: canonical_leaves={} persisted_leaves={} stale_known_keys={}",
            entries.len(),
            smt.persisted_leaf_count()?,
            stale_known_keys.join(",")
        );
        Ok(())
    }

    /// Return read-only SMT status. When `audit` is true this also performs the
    /// expensive full canonical leaf/root comparison; it never repairs state.
    pub fn smt_diagnostics(&self, audit: bool) -> Result<SmtDiagnostics> {
        let (updates, deletes) = self.smt_changes_from_overlay()?;
        let enabled = self.smt.is_some();
        let persisted_root = self
            .smt
            .as_ref()
            .map(|tree| tree.root_hash().map(hex::encode))
            .transpose()?;

        let mut persisted_leaf_count = None;
        let mut consistent = None;
        let mut consistency_error = None;
        let audit_performed = audit && enabled;

        if audit_performed {
            persisted_leaf_count = self
                .smt
                .as_ref()
                .map(|tree| tree.persisted_leaf_count())
                .transpose()?;
            match self.validate_smt_consistency() {
                Ok(()) => consistent = Some(true),
                Err(error) => {
                    consistent = Some(false);
                    consistency_error = Some(error.to_string());
                }
            }
        }

        Ok(SmtDiagnostics {
            enabled,
            persisted_root,
            effective_root: hex::encode(self.try_compute_state_root()?),
            overlay_entries: self.overlay.len(),
            overlay_updates: updates.len(),
            overlay_deletes: deletes.len(),
            canonical_membership_changed: self.canonical_membership_changed()?,
            runtime_schema_version: self.load_internal(RUNTIME_STATE_SCHEMA_KEY)?,
            expected_runtime_schema_version: RUNTIME_STATE_SCHEMA_VERSION,
            wallet_supply_index_version: self.load_internal(WALLET_SUPPLY_INDEX_VERSION_KEY)?,
            expected_wallet_supply_index_version: WALLET_SUPPLY_INDEX_VERSION,
            audit_requested: audit,
            audit_performed,
            persisted_leaf_count,
            consistent,
            consistency_error,
        })
    }

    fn repair_derived_indexes_on_startup(&mut self) -> Result<bool> {
        let (expected_owner_ids, expected_owned_objects, expected_object_ids) =
            self.collect_derived_indexes()?;
        let mut changed = false;

        let current_owner_ids = self.load_index_list(OWNER_INDEX_KEY)?;
        if current_owner_ids != expected_owner_ids {
            self.save_index_list(OWNER_INDEX_KEY, &expected_owner_ids)?;
            changed = true;
        }

        let legacy_owner_ids = self.load_index_list(LEGACY_ACCOUNT_INDEX_KEY)?;
        if !legacy_owner_ids.is_empty() {
            self.overlay.insert(LEGACY_ACCOUNT_INDEX_KEY.to_vec(), None);
            changed = true;
        }

        let current_object_ids = self.load_index_list(b"object_index")?;
        if current_object_ids != expected_object_ids {
            self.save_index_list(b"object_index", &expected_object_ids)?;
            changed = true;
        }

        let indexed_owners = current_owner_ids
            .into_iter()
            .chain(expected_owner_ids.iter().cloned())
            .filter_map(|id| AccountAddress::from_hex_literal(&id).ok())
            .collect::<BTreeSet<_>>();

        for owner in indexed_owners {
            let expected_ids = expected_owned_objects
                .get(&owner)
                .cloned()
                .unwrap_or_default();
            let key = owned_objects_key(&owner);
            let current_ids = self.load_index_list(&key)?;
            if current_ids != expected_ids {
                self.save_index_list(&key, &expected_ids)?;
                changed = true;
            }
        }

        Ok(changed)
    }

    /// Commit pending overlay changes to the persistent store and update SMT
    pub fn commit(&mut self) -> Result<()> {
        self.commit_with_raw_updates(Vec::new())
    }

    /// Commit canonical state together with durable metadata owned by the caller.
    /// The extra records share the same RocksDB write batch as the state overlay.
    pub fn commit_with_raw_update(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
        self.commit_with_raw_updates(vec![(key, value)])
    }

    pub fn commit_with_raw_update_and_verified_root(
        &mut self,
        key: Vec<u8>,
        value: Vec<u8>,
        verified_root: &[u8],
    ) -> Result<()> {
        let root: [u8; 32] = verified_root
            .try_into()
            .context("Verified state root must be 32 bytes")?;
        self.commit_with_raw_updates_and_root(vec![(key, value)], Vec::new(), Some(root), None)
    }

    pub fn commit_with_raw_update_verified_root_and_smt_changes(
        &mut self,
        key: Vec<u8>,
        value: Vec<u8>,
        verified_root: &[u8],
        smt_changes: Option<PrecomputedSmtChanges>,
    ) -> Result<()> {
        let root: [u8; 32] = verified_root
            .try_into()
            .context("Verified state root must be 32 bytes")?;
        self.commit_with_raw_updates_and_root(
            vec![(key, value)],
            Vec::new(),
            Some(root),
            smt_changes,
        )
    }

    pub fn commit_with_raw_changes_verified_root_and_smt_changes(
        &mut self,
        updates: Vec<(Vec<u8>, Vec<u8>)>,
        deletes: Vec<Vec<u8>>,
        verified_root: &[u8],
        smt_changes: Option<PrecomputedSmtChanges>,
    ) -> Result<()> {
        let root: [u8; 32] = verified_root
            .try_into()
            .context("Verified state root must be 32 bytes")?;
        self.commit_with_raw_updates_and_root(updates, deletes, Some(root), smt_changes)
    }

    fn commit_with_raw_updates(&mut self, updates: Vec<(Vec<u8>, Vec<u8>)>) -> Result<()> {
        self.commit_with_raw_updates_and_root(updates, Vec::new(), None, None)
    }

    fn commit_with_raw_updates_and_root(
        &mut self,
        mut updates: Vec<(Vec<u8>, Vec<u8>)>,
        mut deletes: Vec<Vec<u8>>,
        verified_root: Option<[u8; 32]>,
        precomputed_smt_changes: Option<PrecomputedSmtChanges>,
    ) -> Result<()> {
        for (key, val_opt) in &self.overlay {
            if let Some(val) = val_opt {
                updates.push((key.clone(), val.clone()));
            } else {
                deletes.push(key.clone());
            }
        }

        // Derive SMT deltas before writing the overlay so index transitions can
        // compare the old persisted membership with the new overlay membership.
        let smt_changes = if self.smt.is_some() {
            match precomputed_smt_changes {
                Some(changes) => Some(changes),
                None => {
                    let (updates, deletes) = self.smt_changes_from_overlay()?;
                    let node_overlay = if let Some(smt) = &self.smt {
                        if let Some(root) = verified_root {
                            let (computed_root, overlay) =
                                smt.root_hash_with_changes_and_overlay(&updates, &deletes)?;
                            ensure!(
                                computed_root == root,
                                "verified SMT root mismatch while precomputing node overlay"
                            );
                            overlay
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };
                    Some((updates, deletes, node_overlay))
                }
            }
        } else {
            None
        };

        // Update SMT if available
        if let (Some(smt), Some((smt_updates, smt_deletes, node_overlay))) =
            (&self.smt, smt_changes)
        {
            if let (Some(root), Some(db)) = (verified_root, self.store.get_db())
                && !node_overlay.is_empty()
            {
                let mut batch = WriteBatch::default();
                for (key, value) in &updates {
                    batch.put(key, value);
                }
                for key in &deletes {
                    batch.delete(key);
                }
                smt.stage_changes_with_precomputed_overlay(
                    &mut batch,
                    &smt_updates,
                    &smt_deletes,
                    root,
                    &node_overlay,
                )?;
                db.write(batch)?;
                smt.apply_precomputed_node_overlay(node_overlay);
                self.overlay.clear();
                return Ok(());
            }

            self.store.apply_raw_changes(&updates, &deletes)?;
            let update_result = (|| -> Result<()> {
                if let Some(root) = verified_root {
                    if node_overlay.is_empty() {
                        smt.apply_changes_with_root(&smt_updates, &smt_deletes, root)?;
                    } else {
                        smt.apply_changes_with_precomputed_overlay(
                            &smt_updates,
                            &smt_deletes,
                            root,
                            node_overlay,
                        )?;
                    }
                } else if !smt_deletes.is_empty() {
                    smt.delete(&smt_deletes)?;
                    if !smt_updates.is_empty() {
                        smt.insert(&smt_updates)?;
                    }
                } else if !smt_updates.is_empty() {
                    smt.insert(&smt_updates)?;
                }
                Ok(())
            })();
            if let Err(update_error) = update_result {
                // Canonical state has already committed. Repair immediately so callers
                // never receive a simple failure while the live DB remains split-brain.
                self.repair_persisted_smt().with_context(|| {
                    format!("SMT delta update failed ({update_error}); full repair also failed")
                })?;
            }
        } else {
            self.store.apply_raw_changes(&updates, &deletes)?;
        }

        self.overlay.clear();
        Ok(())
    }

    // Helper to write to overlay (pub for genesis module)
    pub(crate) fn save_internal<T: Serialize + ?Sized>(
        &mut self,
        key: &[u8],
        value: &T,
    ) -> Result<()> {
        let bytes = bcs::to_bytes(value)?;
        self.overlay.insert(key.to_vec(), Some(bytes));
        Ok(())
    }

    // Helper to read from overlay then store
    pub fn load_internal<T: DeserializeOwned>(&self, key: &[u8]) -> Result<Option<T>> {
        if let Some(val_opt) = self.overlay.get(key) {
            match val_opt {
                Some(bytes) => return Ok(Some(bcs::from_bytes(bytes)?)),
                None => return Ok(None),
            }
        }
        Ok(self.store.load(key)?)
    }

    /// Capture the versions observed by a speculative execution.
    pub fn capture_access_versions(&self, access: &StateAccessSet) -> BTreeMap<Vec<u8>, u64> {
        access
            .reads
            .iter()
            .chain(access.writes.iter())
            .map(|key| {
                (
                    key.clone(),
                    self.access_versions.get(key).copied().unwrap_or(0),
                )
            })
            .collect()
    }

    pub fn access_version_snapshot(&self) -> BTreeMap<Vec<u8>, u64> {
        self.access_versions.clone()
    }

    pub fn access_epoch(&self) -> u64 {
        self.access_epoch
    }

    pub fn validate_access_snapshot(
        &self,
        snapshot: &BTreeMap<Vec<u8>, u64>,
        access: &StateAccessSet,
    ) -> bool {
        access.reads.iter().chain(access.writes.iter()).all(|key| {
            snapshot.get(key).copied().unwrap_or(0)
                == self.access_versions.get(key).copied().unwrap_or(0)
        })
    }

    pub(crate) fn advance_access_versions(&mut self, access: &StateAccessSet) -> Result<()> {
        let persist_access_versions = std::env::var("KANARI_PERSIST_ACCESS_VERSIONS")
            .map(|value| !matches!(value.as_str(), "0" | "false" | "FALSE" | "no" | "NO"))
            .unwrap_or(true);
        if !access.writes.is_empty() {
            self.access_epoch = self
                .access_epoch
                .checked_add(1)
                .context("State access epoch overflow")?;
        }
        for key in &access.writes {
            let next = self
                .access_versions
                .get(key)
                .copied()
                .unwrap_or(0)
                .checked_add(1)
                .context("State access version overflow")?;
            self.access_versions.insert(key.clone(), next);
            if persist_access_versions {
                let mut persisted_key = ACCESS_VERSION_PREFIX.to_vec();
                persisted_key.extend_from_slice(key);
                self.save_internal(&persisted_key, &next)?;
            }
        }
        Ok(())
    }

    // Helper to construct DB key for one owner-state record.
    fn owner_state_key(address: &AccountAddress) -> Vec<u8> {
        let mut key = b"account:".to_vec();
        key.extend_from_slice(address.as_ref());
        key
    }

    // Helper to construct DB key for object

    fn object_lookup_ids(id: &str) -> Vec<String> {
        let mut ids = vec![id.to_string()];
        if let Some(canonical_id) = canonical_object_id(id)
            && canonical_id != id
        {
            ids.push(canonical_id);
        }
        ids
    }

    fn load_stored_object_by_any_id(
        &self,
        object_id: &str,
    ) -> Result<Option<(String, StoredObject)>> {
        for candidate_id in Self::object_lookup_ids(object_id) {
            let obj_key = object_key(&candidate_id);
            if let Some(stored) = self.load_internal::<StoredObject>(&obj_key)? {
                return Ok(Some((candidate_id, stored)));
            }
        }
        Ok(None)
    }

    // helper for generating DB keys for Dynamic Fields
    fn dynamic_field_key(object_id: &str, name_bytes: &[u8]) -> Vec<u8> {
        let hash = hash_data_blake3(name_bytes);
        let mut key = b"df:".to_vec();
        key.extend_from_slice(object_id.as_bytes());
        key.extend_from_slice(b":");
        key.extend_from_slice(hex::encode(&hash[0..16]).as_bytes());
        key
    }

    fn is_canonical_state_root_key(key: &[u8]) -> bool {
        key != GENESIS_INITIALIZED_KEY
            && (key == b"total_supply"
                || key.starts_with(b"resource:")
                || key.starts_with(b"account:")
                || key.starts_with(b"df:")
                || key.starts_with(b"system:")
                || key.starts_with(b"supply:")
                || key.starts_with(b"treasury:")
                || key.starts_with(b"nft:"))
    }

    fn retain_canonical_state_root_entries(entries: &mut BTreeMap<Vec<u8>, Vec<u8>>) -> Result<()> {
        let mut canonical_object_keys = BTreeSet::new();
        let mut canonical_module_keys = BTreeSet::new();

        for (key, value) in entries.iter() {
            if key == b"module_index" {
                let module_ids = bcs::from_bytes::<Vec<String>>(value)
                    .context("Malformed canonical module index")?;
                canonical_module_keys.extend(
                    module_ids
                        .into_iter()
                        .filter(|id| id.starts_with("module:"))
                        .map(String::into_bytes),
                );
            } else if key.starts_with(b"owned_objects:") {
                let object_ids = bcs::from_bytes::<Vec<String>>(value).with_context(|| {
                    format!(
                        "Malformed owned-object index {:?}",
                        String::from_utf8_lossy(key)
                    )
                })?;
                canonical_object_keys.extend(
                    object_ids
                        .into_iter()
                        .map(|id| {
                            canonical_object_id(&id).ok_or_else(|| {
                                anyhow::anyhow!("Invalid object id in owned-object index: {id}")
                            })
                        })
                        .collect::<Result<Vec<_>>>()?
                        .into_iter()
                        .map(|id| {
                            let object_key = object_key(&id);
                            let bytes = entries.get(&object_key).ok_or_else(|| {
                                anyhow::anyhow!("Owned-object index references missing object {id}")
                            })?;
                            bcs::from_bytes::<StoredObject>(bytes)
                                .with_context(|| format!("Malformed object record {id}"))?;
                            Ok(object_key)
                        })
                        .collect::<Result<Vec<_>>>()?,
                );
            }
        }

        entries.retain(|key, _| {
            Self::is_canonical_state_root_key(key)
                || canonical_module_keys.contains(key)
                || canonical_object_keys.contains(key)
        });
        Ok(())
    }

    fn is_canonical_smt_update(&self, key: &[u8], value: &[u8]) -> Result<bool> {
        if Self::is_canonical_state_root_key(key) {
            return Ok(true);
        }
        if let Some(module_id) = key.strip_prefix(b"module:")
            && let Ok(module_id) = std::str::from_utf8(module_id)
        {
            return Ok(self
                .load_index_list(b"module_index")?
                .iter()
                .any(|id| id == &format!("module:{module_id}")));
        }
        if let Some(object_id) = key.strip_prefix(b"object:")
            && let Ok(object_id) = std::str::from_utf8(object_id)
            && let object = bcs::from_bytes::<StoredObject>(value)
                .with_context(|| format!("Malformed object record {object_id}"))?
            && matches!(object.owner_kind, ObjectOwnerKind::AddressOwner(_))
        {
            let canonical_id = canonical_object_id(object_id)
                .ok_or_else(|| anyhow::anyhow!("Invalid canonical object id {object_id}"))?;
            return Ok(self
                .load_index_list(&owned_objects_key(&object.owner))?
                .binary_search(&canonical_id)
                .is_ok());
        }
        Ok(false)
    }

    fn smt_changes_from_overlay(&self) -> Result<OverlaySmtChanges> {
        let mut updates = Vec::new();
        let mut deletes = Vec::new();

        for (key, value_opt) in &self.overlay {
            match value_opt {
                Some(value) => {
                    let canonical = if let Some(object_id) = key.strip_prefix(b"object:")
                        && let Ok(object_id) = std::str::from_utf8(object_id)
                    {
                        let object = bcs::from_bytes::<StoredObject>(value)
                            .with_context(|| format!("Malformed object record {object_id}"))?;
                        if matches!(object.owner_kind, ObjectOwnerKind::AddressOwner(_)) {
                            canonical_object_id(object_id).ok_or_else(|| {
                                anyhow::anyhow!("Invalid canonical object id {object_id}")
                            })? == object_id
                        } else {
                            false
                        }
                    } else {
                        self.is_canonical_smt_update(key, value)?
                    };
                    if canonical {
                        updates.push((key.clone(), value.clone()));
                    } else if key.starts_with(b"module:") || key.starts_with(b"object:") {
                        deletes.push(key.clone());
                    }
                }
                // An object/module may transition out of the canonical root
                // set (for example AddressOwner -> Shared). A non-canonical
                // replacement must remove the previously indexed leaf.
                None if Self::is_canonical_state_root_key(key)
                    || key.starts_with(b"module:")
                    || key.starts_with(b"object:") =>
                {
                    deletes.push(key.clone());
                }
                _ => {}
            }
        }

        // Object membership in the canonical root is driven by owned_objects
        // indexes. An index can drop/add an unchanged object without placing
        // that object's bytes in the overlay, so derive those leaf deltas too.
        for (index_key, value_opt) in self
            .overlay
            .iter()
            .filter(|(key, _)| key.starts_with(b"owned_objects:"))
        {
            let old_ids = self
                .store
                .load::<Vec<String>>(index_key)?
                .unwrap_or_default()
                .into_iter()
                .filter_map(|id| canonical_object_id(&id))
                .collect::<BTreeSet<_>>();
            let new_ids = value_opt
                .as_ref()
                .map(|bytes| {
                    bcs::from_bytes::<Vec<String>>(bytes)
                        .context("Malformed pending owned-object index")
                })
                .transpose()?
                .unwrap_or_default()
                .into_iter()
                .filter_map(|id| canonical_object_id(&id))
                .collect::<BTreeSet<_>>();

            for removed in old_ids.difference(&new_ids) {
                deletes.push(object_key(removed));
            }
            for added in new_ids.difference(&old_ids) {
                let key = object_key(added);
                if let Some(object) = self.load_internal::<StoredObject>(&key)?
                    && matches!(object.owner_kind, ObjectOwnerKind::AddressOwner(_))
                {
                    let value = bcs::to_bytes(&object)?;
                    updates.push((key, value));
                }
            }
        }

        updates.sort_by(|left, right| left.0.cmp(&right.0));
        updates.dedup_by(|left, right| left.0 == right.0);
        let updated_keys = updates
            .iter()
            .map(|(key, _)| key.as_slice())
            .collect::<HashSet<_>>();
        deletes.retain(|key| !updated_keys.contains(key.as_slice()));
        deletes.sort();
        deletes.dedup();

        Ok((updates, deletes))
    }

    fn canonical_membership_changed(&self) -> Result<bool> {
        for (key, value_opt) in &self.overlay {
            if key != b"module_index" && !key.starts_with(b"owned_objects:") {
                continue;
            }
            let old = self.store.load::<Vec<String>>(key)?;
            let new = value_opt
                .as_ref()
                .map(|bytes| {
                    bcs::from_bytes::<Vec<String>>(bytes)
                        .context("Malformed pending canonical membership index")
                })
                .transpose()?;
            if old != new {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn get_system_clock_object_id(&self) -> Result<Option<AccountAddress>> {
        let bytes_opt: Option<Vec<u8>> = self.load_internal(SYSTEM_CLOCK_OBJECT_ID_KEY)?;
        match bytes_opt {
            None => Ok(None),
            Some(bytes) => {
                let addr = AccountAddress::from_bytes(bytes)?;
                Ok(Some(addr))
            }
        }
    }

    pub(crate) fn set_system_clock_object_id(&mut self, id: AccountAddress) -> Result<()> {
        self.save_internal(SYSTEM_CLOCK_OBJECT_ID_KEY, &id.as_ref().to_vec())
    }

    fn save_owner_record(&mut self, owner_state: &OwnerState) -> Result<()> {
        self.save_internal(&Self::owner_state_key(&owner_state.address), owner_state)
    }

    fn add_many_to_index_list<I>(&mut self, key: &[u8], values: I) -> Result<()>
    where
        I: IntoIterator<Item = String>,
    {
        let mut values = values.into_iter().peekable();
        if values.peek().is_none() {
            return Ok(());
        }

        let existing = self.load_index_list(key)?;
        let mut index: BTreeSet<String> = existing.into_iter().collect();
        let mut changed = false;

        for value in values {
            changed |= index.insert(value);
        }

        if changed {
            let sorted = index.into_iter().collect::<Vec<_>>();
            self.save_index_list(key, &sorted)?;
        }

        Ok(())
    }

    pub fn owner_addresses(&self) -> Result<Vec<AccountAddress>> {
        let ids = self.load_owner_index_ids()?;
        ids.into_iter()
            .map(|id| {
                AccountAddress::from_hex_literal(&id)
                    .with_context(|| format!("Owner index contains invalid address {id}"))
            })
            .collect()
    }

    fn load_owner_index_ids(&self) -> Result<Vec<String>> {
        let owner_ids = self.load_index_list(OWNER_INDEX_KEY)?;
        if !owner_ids.is_empty() {
            return Ok(owner_ids);
        }

        self.load_index_list(LEGACY_ACCOUNT_INDEX_KEY)
    }

    fn load_owner_state_or_default(&self, address: AccountAddress) -> Result<OwnerState> {
        Ok(self
            .load_owner_state(&address)?
            .unwrap_or_else(|| OwnerState::new(address)))
    }

    pub fn load_owner_state(&self, owner: &AccountAddress) -> Result<Option<OwnerState>> {
        self.load_internal(&Self::owner_state_key(owner))
    }

    fn save_owner_state_without_supply_index(&mut self, owner_state: &OwnerState) -> Result<()> {
        if owner_state.is_empty() {
            self.overlay
                .insert(Self::owner_state_key(&owner_state.address), None);
            self.remove_from_index_list(OWNER_INDEX_KEY, &owner_state.address.to_hex_literal())
        } else {
            self.save_owner_record(owner_state)?;
            self.add_to_index_list(OWNER_INDEX_KEY, owner_state.address.to_hex_literal())
        }
    }

    pub fn save_owner_state(&mut self, owner_state: &OwnerState) -> Result<()> {
        let old_balances = self
            .load_owner_state(&owner_state.address)?
            .map(|state| state.token_balances)
            .unwrap_or_default();
        self.save_owner_state_without_supply_index(owner_state)?;
        let old_native = old_balances
            .get(GAS_COIN)
            .map(|balance| balance.value())
            .unwrap_or(0);
        let new_native = owner_state.native_balance();
        let cached_native = self
            .global_token_supplies
            .get(GAS_COIN)
            .copied()
            .unwrap_or(0);
        let projected_native =
            u128::from(cached_native.saturating_sub(old_native)) + u128::from(new_native);
        let update_supply_index =
            old_native == new_native || projected_native <= u128::from(self.total_supply);
        if update_supply_index && self.capture_supply_changed(owner_state, &old_balances)? {
            let supplies = self.global_token_supplies.clone();
            self.save_internal(b"global_token_supplies", &supplies)?;
        }
        Ok(())
    }

    pub fn get_owner_state(&self, owner: &AccountAddress) -> Option<OwnerState> {
        match self.load_owner_state(owner) {
            Ok(account) => account,
            Err(e) => {
                log::error!(
                    "[StateManager] Failed to load owner state {}: {}",
                    owner.to_hex_literal(),
                    e
                );
                None
            }
        }
    }

    pub fn try_get_owner_state_by_hex(&self, owner: &str) -> Result<Option<OwnerState>> {
        let addr = kanari_types::address::Address::parse_to_account_address(owner)
            .with_context(|| format!("Failed to parse owner address: {owner}"))?;
        self.load_owner_state(&addr)
            .with_context(|| format!("Failed to load owner state for {owner}"))
    }

    pub fn get_owner_state_by_hex(&self, owner: &str) -> Option<OwnerState> {
        // Use Address::parse_to_account_address which handles tagged addresses,
        // tagged public keys (hashing), and regular 0x addresses.
        match self.try_get_owner_state_by_hex(owner) {
            Ok(state) => state,
            Err(error) => {
                log::warn!("[StateManager] Failed to get owner state by hex: {error:#}");
                None
            }
        }
    }

    fn balance_token_amount(type_name: &str, data: &[u8]) -> Option<(String, u64)> {
        let struct_tag = StructTag::from_str(type_name).ok()?;
        if !Self::is_balance_struct(&struct_tag) {
            return None;
        }
        let token_type = Self::token_type_from_balance_struct(&struct_tag)?;
        let amount = Self::extract_balance_from_object_bytes(data, &struct_tag)?;
        Some((Self::normalize_token_type(&token_type), amount))
    }

    /// Get all object IDs owned by an address
    pub fn get_owned_objects(&self, owner: &AccountAddress) -> Result<Vec<String>> {
        let owner_key = owned_objects_key(owner);
        let mut clean: Vec<String> = self.load_internal(&owner_key)?.unwrap_or_default();
        clean.sort();
        clean.dedup();
        Ok(clean)
    }

    /// Get a specific object by ID
    pub fn get_object(&self, object_id: &str) -> Result<Option<CreatedObject>> {
        let Some((stored_id, stored)) = self.load_stored_object_by_any_id(object_id)? else {
            return Ok(None);
        };
        let normalized_id = canonical_object_id(&stored_id).unwrap_or(stored_id.clone());
        let uid = AccountAddress::from_hex_literal(&normalized_id)
            .ok()
            .map(UIDRecord::new);
        let id = AccountAddress::from_hex_literal(&normalized_id)
            .ok()
            .map(IDRecord::new);

        Ok(Some(CreatedObject {
            owner: stored.owner,
            owner_kind: stored.owner_kind,
            uid,
            id,
            type_: stored.type_name,
            data: stored.data,
            version: stored.version,
        }))
    }

    pub fn get_objects_by_type(&self, object_type: &str) -> Result<Vec<(String, CreatedObject)>> {
        self.query_objects(None, None, Some(object_type), None, None)
    }

    pub fn query_objects(
        &self,
        owner: Option<AccountAddress>,
        owner_kind: Option<&ObjectOwnerKind>,
        object_type: Option<&str>,
        min_version: Option<u64>,
        max_version: Option<u64>,
    ) -> Result<Vec<(String, CreatedObject)>> {
        let object_ids = self.load_index_list(b"object_index")?;
        let mut objects = Vec::new();

        for object_id in object_ids {
            let object = self.get_object(&object_id)?.ok_or_else(|| {
                anyhow::anyhow!("Object index references missing object {object_id}")
            })?;
            if let Some(owner) = owner
                && object.owner != owner
            {
                continue;
            }
            if let Some(owner_kind) = owner_kind {
                let matches = match owner_kind {
                    ObjectOwnerKind::AddressOwner(_) => {
                        matches!(object.owner_kind, ObjectOwnerKind::AddressOwner(_))
                    }
                    ObjectOwnerKind::Shared => matches!(object.owner_kind, ObjectOwnerKind::Shared),
                    ObjectOwnerKind::Immutable => {
                        matches!(object.owner_kind, ObjectOwnerKind::Immutable)
                    }
                };
                if !matches {
                    continue;
                }
            }
            if let Some(object_type) = object_type
                && object.type_ != object_type
            {
                continue;
            }
            if let Some(min_version) = min_version
                && object.version < min_version
            {
                continue;
            }
            if let Some(max_version) = max_version
                && object.version > max_version
            {
                continue;
            }

            objects.push((object_id, object));
        }

        Ok(objects)
    }

    /// Compute a canonical root and propagate storage/index corruption to the caller.
    pub fn try_compute_state_root(&self) -> Result<Vec<u8>> {
        self.try_compute_state_root_with_smt_changes()
            .map(|(root, _)| root)
    }

    /// Compute a canonical root and return reusable SMT overlay deltas when the
    /// production SMT backend is active. Passing these deltas into commit avoids
    /// walking and decoding the same overlay twice on the checkpoint hot path.
    pub fn try_compute_state_root_with_smt_changes(
        &self,
    ) -> Result<(Vec<u8>, Option<PrecomputedSmtChanges>)> {
        let profile_root = matches!(
            std::env::var("KANARI_STATE_ROOT_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let started_at = std::time::Instant::now();
        if let Some(smt) = &self.smt {
            let (updates, deletes) = self.smt_changes_from_overlay()?;
            let (root, node_overlay) =
                smt.root_hash_with_changes_and_overlay(&updates, &deletes)?;
            let root = root.to_vec();
            if profile_root {
                eprintln!(
                    "state root profile: smt updates={} deletes={} total={:.6}s",
                    updates.len(),
                    deletes.len(),
                    started_at.elapsed().as_secs_f64()
                );
            }
            return Ok((root, Some((updates, deletes, node_overlay))));
        }
        let logical_started_at = std::time::Instant::now();
        let mut entries: BTreeMap<Vec<u8>, Vec<u8>> =
            self.store.logical_entries()?.into_iter().collect();
        let logical_count = entries.len();
        let logical_at = std::time::Instant::now();

        for (key, value_opt) in &self.overlay {
            if let Some(value) = value_opt {
                entries.insert(key.clone(), value.clone());
            } else {
                entries.remove(key);
            }
        }
        let overlay_at = std::time::Instant::now();

        Self::retain_canonical_state_root_entries(&mut entries)?;
        let retain_at = std::time::Instant::now();

        let canonical_count = entries.len();
        let root = smt::compute_sparse_root(&entries.into_iter().collect::<Vec<_>>()).to_vec();
        let rooted_at = std::time::Instant::now();
        if profile_root {
            eprintln!(
                "state root profile: logical_entries={} overlay_entries={} canonical_entries={} logical={:.6}s overlay={:.6}s retain={:.6}s compute={:.6}s total={:.6}s",
                logical_count,
                self.overlay.len(),
                canonical_count,
                logical_at.duration_since(logical_started_at).as_secs_f64(),
                overlay_at.duration_since(logical_at).as_secs_f64(),
                retain_at.duration_since(overlay_at).as_secs_f64(),
                rooted_at.duration_since(retain_at).as_secs_f64(),
                rooted_at.duration_since(started_at).as_secs_f64(),
            );
        }
        Ok((root, None))
    }

    /// Legacy convenience API. Consensus and RPC paths must use
    /// `try_compute_state_root` so persistent-state faults are returned rather
    /// than converted into a different root.
    pub fn compute_state_root(&self) -> Vec<u8> {
        self.try_compute_state_root()
            .expect("canonical state root requires readable, well-formed persistent state")
    }

    pub fn try_canonical_state_snapshot(&self) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
        let mut entries: BTreeMap<Vec<u8>, Vec<u8>> =
            self.store.logical_entries()?.into_iter().collect();

        for (key, value_opt) in &self.overlay {
            if let Some(value) = value_opt {
                entries.insert(key.clone(), value.clone());
            } else {
                entries.remove(key);
            }
        }

        Self::retain_canonical_state_root_entries(&mut entries)?;
        Ok(entries)
    }

    pub fn canonical_state_snapshot(&self) -> BTreeMap<Vec<u8>, Vec<u8>> {
        self.try_canonical_state_snapshot()
            .expect("canonical snapshot requires readable, well-formed persistent state")
    }

    /// Get the total number of owners with persisted owner state.
    pub fn try_owner_count(&self) -> Result<usize> {
        Ok(self.load_owner_index_ids()?.len())
    }

    /// Legacy convenience API for callers that cannot return errors. Consensus,
    /// diagnostics, and RPC paths should prefer `try_owner_count`.
    pub fn owner_count(&self) -> usize {
        match self.try_owner_count() {
            Ok(count) => count,
            Err(error) => {
                log::error!("[StateManager] Failed to load owner count: {error:#}");
                0
            }
        }
    }
}

pub(crate) fn default_owner_kind_for_type(
    type_name: &str,
    owner: AccountAddress,
) -> ObjectOwnerKind {
    if type_name.contains("::clock::Clock") {
        ObjectOwnerKind::Shared
    } else if type_name.contains("::coin::CoinMetadata<") {
        ObjectOwnerKind::Immutable
    } else {
        ObjectOwnerKind::AddressOwner(owner.to_hex_literal())
    }
}

#[cfg(test)]
#[path = "../tests/unit/state_tests.rs"]
mod tests;
