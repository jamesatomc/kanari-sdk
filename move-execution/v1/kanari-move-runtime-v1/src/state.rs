// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::changeset::{ChangeSet, CreatedObject};
use crate::common::ids::canonical_object_id;
use crate::common::keys::{metadata_key, object_key, owned_objects_key};
use crate::storage::object_storage::StoredObject;
use crate::storage::persistent_store::PersistentStore;
use anyhow::{Context, Result, ensure};
use kanari_crypto::hash_data_blake3;
use kanari_types::balance::BalanceModule;
use kanari_types::balance::BalanceRecord;
use kanari_types::coin::{CoinModule, TreasuryCap};
use kanari_types::error::KanariUnwrapExt;
use kanari_types::event::Event;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use kanari_types::object::{IDRecord, UIDRecord};
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::{StructTag, TypeTag};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use smt;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::str::FromStr;
use std::sync::Arc;

mod apply;
mod supply;

const SYSTEM_CLOCK_OBJECT_ID_KEY: &[u8] = b"system:clock_object_id";
const ACCOUNT_INDEX_KEY: &[u8] = b"account_index";
const OBJECT_LOCKED_COIN_RECORDS_KEY: &[u8] = b"object_locked_coin_records";
const UID_SIZE: usize = 32;
const U64_SIZE: usize = 8;

type RawStateKey = Vec<u8>;
type RawStateValue = Vec<u8>;
type RawStateUpdate = (RawStateKey, RawStateValue);
type RawStateDelete = RawStateKey;
type OverlaySmtChanges = (Vec<RawStateUpdate>, Vec<RawStateDelete>);

/// Account state in the blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub address: AccountAddress,
    pub sequence_number: u64,
    pub modules: BTreeSet<String>,
    /// Token balances: token_type -> BalanceRecord
    /// Example: "0x840512ff...::james::JAMES" -> BalanceRecord(1000000000)
    pub token_balances: BTreeMap<String, BalanceRecord>,
}

impl Account {
    pub fn new(address: AccountAddress) -> Self {
        Self {
            address,
            sequence_number: 0,
            modules: BTreeSet::new(),
            token_balances: BTreeMap::new(),
        }
    }

    pub fn with_native_balance(address: AccountAddress, balance: u64) -> Self {
        let mut account = Self::new(address);
        if balance > 0 {
            account.set_token_balance(KANARI_TOKEN_TYPE.to_string(), BalanceRecord::new(balance));
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
        self.get_token_balance(KANARI_TOKEN_TYPE)
    }

    pub fn increment_sequence(&mut self) {
        self.sequence_number += 1;
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
}

impl StateManager {
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
    pub fn get_all_collection_ids(&self) -> Vec<String> {
        self.load_index_list(b"nft_collection_index")
            .unwrap_or_default()
    }

    /// Retrieves NFT IDs for the specified collection from the index
    pub fn get_collection_nft_ids(&self, collection_id: &str) -> Vec<String> {
        self.load_index_list(&metadata_key(b"collection_members:", collection_id))
            .unwrap_or_default()
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

    /// Create new state with genesis allocation
    /// Total supply: 11 million KANARI = 11,000,000,000,000,000 Mist
    /// Dev address gets entire supply according to kanari.move
    pub fn new(store: Arc<PersistentStore>) -> Self {
        Self::try_new(store).invariant("failed to create state manager")
    }

    /// Create new state with genesis allocation and return initialization errors.
    pub fn try_new(store: Arc<PersistentStore>) -> Result<Self> {
        // Try to load total supply from DB
        let persisted_total_supply = store
            .load::<u64>(b"total_supply")
            .context("Failed to load total_supply")?
            .unwrap_or(0);

        // Load existing global token supplies from RocksDB
        let global_token_supplies = store
            .load::<BTreeMap<String, u64>>(b"global_token_supplies")
            .context("Failed to load global_token_supplies")?
            .unwrap_or_default();

        // Recover native total supply from older databases that persisted the
        // native treasury/global balances but never backfilled `total_supply`.
        let recovered_total_supply = if persisted_total_supply == 0 {
            Self::load_persisted_supply_from_store(store.as_ref(), KANARI_TOKEN_TYPE)
                .or_else(|| global_token_supplies.get(KANARI_TOKEN_TYPE).copied())
                .unwrap_or(0)
        } else {
            persisted_total_supply
        };

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
        };

        state
            .ensure_smt_initialized()
            .context("Failed to initialize state SMT")?;

        if persisted_total_supply == 0 && recovered_total_supply > 0 {
            state
                .save_internal(b"total_supply", &recovered_total_supply)
                .context("Failed to backfill recovered total_supply")?;
            state
                .commit()
                .context("Failed to persist recovered total_supply")?;
        }

        // If total supply is 0, initialize genesis
        if state.total_supply == 0 {
            crate::genesis::init_genesis(&mut state).context("Genesis initialization failed")?;
            // Flush genesis state to DB immediately
            state.commit().context("Failed to commit genesis state")?;
            ensure!(
                state.total_supply > 0,
                "Genesis initialization completed but total_supply is still 0"
            );
        }

        state
            .repair_legacy_native_wallet_overcount()
            .context("Failed to repair native wallet supply on startup")?;

        if let Err(e) = state.validate_supply_invariants() {
            Self::report_supply_invariant_violation("on startup", &e)?;
        }

        Ok(state)
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
        Self::retain_canonical_state_root_entries(&mut entries);
        if entries.is_empty() {
            return Ok(());
        }

        let updates = entries.into_iter().collect::<Vec<_>>();
        smt.insert(&updates)?;
        Ok(())
    }

    /// Commit pending overlay changes to the persistent store and update SMT
    pub fn commit(&mut self) -> Result<()> {
        let mut updates = Vec::new();
        let mut deletes = Vec::new();

        for (key, val_opt) in &self.overlay {
            if let Some(val) = val_opt {
                updates.push((key.clone(), val.clone()));
            } else {
                deletes.push(key.clone());
            }
        }

        self.store.apply_raw_changes(&updates, &deletes)?;

        // Update SMT if available
        if let Some(smt) = &self.smt {
            let (smt_updates, smt_deletes) = self.smt_changes_from_overlay();
            if !smt_updates.is_empty() {
                smt.insert(&smt_updates)?;
            }
            if !smt_deletes.is_empty() {
                smt.delete(&smt_deletes)?;
            }
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
    pub(crate) fn load_internal<T: DeserializeOwned>(&self, key: &[u8]) -> Result<Option<T>> {
        if let Some(val_opt) = self.overlay.get(key) {
            match val_opt {
                Some(bytes) => return Ok(Some(bcs::from_bytes(bytes)?)),
                None => return Ok(None),
            }
        }
        Ok(self.store.load(key)?)
    }

    // Helper to construct DB key for account
    fn account_key(address: &AccountAddress) -> Vec<u8> {
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
        key == ACCOUNT_INDEX_KEY
            || key == OBJECT_LOCKED_COIN_RECORDS_KEY
            || key == b"total_supply"
            || key == b"global_token_supplies"
            || key == b"treasury_index"
            || key == b"nft_collection_index"
            || key.starts_with(b"resource:")
            || key.starts_with(b"account:")
            || key.starts_with(b"owned_objects:")
            || key.starts_with(b"df:")
            || key.starts_with(b"system:")
            || key.starts_with(b"supply:")
            || key.starts_with(b"treasury:")
            || key.starts_with(b"nft:")
            || key.starts_with(b"collection_members:")
            || key.starts_with(b"metadata_decimals:")
            || key.starts_with(b"metadata_name:")
            || key.starts_with(b"metadata_symbol:")
            || key.starts_with(b"metadata_description:")
            || key.starts_with(b"metadata_icon_url:")
    }

    fn retain_canonical_state_root_entries(entries: &mut BTreeMap<Vec<u8>, Vec<u8>>) {
        let mut canonical_object_keys = BTreeSet::new();
        let mut canonical_module_keys = BTreeSet::new();

        if let Some(module_index_bytes) = entries.get(b"module_index".as_slice())
            && let Ok(module_keys) = bcs::from_bytes::<Vec<String>>(module_index_bytes)
        {
            for module_key in module_keys {
                let module_key = module_key.into_bytes();
                if module_key.starts_with(b"module:") && entries.contains_key(&module_key) {
                    canonical_module_keys.insert(module_key);
                }
            }
        }

        for (key, value) in entries.iter() {
            if !key.starts_with(b"owned_objects:") {
                continue;
            }

            let Ok(object_ids) = bcs::from_bytes::<Vec<String>>(value) else {
                log::warn!(
                    "[StateManager] Skipping malformed owned object index while computing state root"
                );
                continue;
            };

            for object_id in object_ids {
                canonical_object_keys.insert(object_key(&object_id));
            }
        }

        entries.retain(|key, _| {
            Self::is_canonical_state_root_key(key)
                || (key.starts_with(b"module:") && canonical_module_keys.contains(key))
                || (key.starts_with(b"object:") && canonical_object_keys.contains(key))
        });
    }

    fn is_canonical_smt_update(&self, key: &[u8], value: &[u8]) -> bool {
        if Self::is_canonical_state_root_key(key) {
            return true;
        }

        if key.starts_with(b"module:") {
            return self
                .load_internal::<Vec<String>>(b"module_index")
                .ok()
                .flatten()
                .map(|modules| modules.iter().any(|module| module.as_bytes() == key))
                .unwrap_or(false);
        }

        let Some(object_id) = key
            .strip_prefix(b"object:")
            .and_then(|id| std::str::from_utf8(id).ok())
        else {
            return false;
        };

        bcs::from_bytes::<StoredObject>(value)
            .map(|stored| {
                let owner_key = owned_objects_key(&stored.owner);
                self.load_internal::<Vec<String>>(&owner_key)
                    .ok()
                    .flatten()
                    .map(|owned| owned.iter().any(|id| id == object_id))
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    fn smt_changes_from_overlay(&self) -> OverlaySmtChanges {
        let mut updates = Vec::new();
        let mut deletes = Vec::new();

        for (key, value_opt) in &self.overlay {
            match value_opt {
                Some(value) if self.is_canonical_smt_update(key, value) => {
                    updates.push((key.clone(), value.clone()));
                }
                None
                    if Self::is_canonical_state_root_key(key)
                        || key.starts_with(b"module:")
                        || key.starts_with(b"object:") =>
                {
                    deletes.push(key.clone());
                }
                _ => {}
            }
        }

        (updates, deletes)
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

    pub fn load_account(&self, address: &AccountAddress) -> Result<Option<Account>> {
        self.load_internal(&Self::account_key(address))
    }

    pub fn save_account(&mut self, account: &Account) -> Result<()> {
        self.save_account_record(account)?;
        self.add_to_index_list(ACCOUNT_INDEX_KEY, account.address.to_hex_literal())
    }

    pub fn apply_zero_effect_sequence_batch<I>(&mut self, sequence_increments: I) -> Result<()>
    where
        I: IntoIterator<Item = (AccountAddress, u64)>,
    {
        let mut account_index_additions = Vec::new();

        for (address, increment) in sequence_increments {
            if increment == 0 {
                continue;
            }

            let mut account = self.load_account_or_default(address)?;
            account.sequence_number = account.sequence_number.saturating_add(increment);
            self.save_account_record(&account)?;
            account_index_additions.push(account.address.to_hex_literal());
        }

        self.add_many_to_index_list(ACCOUNT_INDEX_KEY, account_index_additions)
    }

    fn save_account_record(&mut self, account: &Account) -> Result<()> {
        self.save_internal(&Self::account_key(&account.address), account)
    }

    fn add_many_to_index_list<I>(&mut self, key: &[u8], values: I) -> Result<()>
    where
        I: IntoIterator<Item = String>,
    {
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

    fn load_account_addresses(&self) -> Result<Vec<AccountAddress>> {
        let ids = self.load_index_list(ACCOUNT_INDEX_KEY)?;
        Ok(ids
            .into_iter()
            .filter_map(|id| AccountAddress::from_hex_literal(&id).ok())
            .collect())
    }

    fn load_account_or_default(&self, address: AccountAddress) -> Result<Account> {
        Ok(self
            .load_account(&address)?
            .unwrap_or_else(|| Account::new(address)))
    }

    pub fn get_account(&self, address: &AccountAddress) -> Option<Account> {
        match self.load_account(address) {
            Ok(account) => account,
            Err(e) => {
                log::error!(
                    "[StateManager] Failed to load account {}: {}",
                    address.to_hex_literal(),
                    e
                );
                None
            }
        }
    }

    pub fn get_account_by_hex(&self, hex_address: &str) -> Option<Account> {
        // Use Address::parse_to_account_address which handles tagged addresses,
        // tagged public keys (hashing), and regular 0x addresses.
        if let Ok(addr) = kanari_types::address::Address::parse_to_account_address(hex_address) {
            self.get_account(&addr)
        } else {
            log::warn!(
                "[StateManager] Failed to parse address from hex: {}",
                hex_address
            );
            None
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
            uid,
            id,
            type_: stored.type_name,
            data: stored.data,
            version: stored.version,
        }))
    }

    pub fn compute_state_root(&self) -> Vec<u8> {
        let mut entries: BTreeMap<Vec<u8>, Vec<u8>> = match self.store.logical_entries() {
            Ok(entries) => entries.into_iter().collect(),
            Err(e) => {
                log::error!("Failed to materialize state root snapshot: {}", e);
                BTreeMap::new()
            }
        };

        for (key, value_opt) in &self.overlay {
            if let Some(value) = value_opt {
                entries.insert(key.clone(), value.clone());
            } else {
                entries.remove(key);
            }
        }

        Self::retain_canonical_state_root_entries(&mut entries);

        smt::compute_sparse_root(&entries.into_iter().collect::<Vec<_>>()).to_vec()
    }

    /// Validate sequence number for an account
    pub fn validate_sequence(&self, addr: &AccountAddress, expected_seq: u64) -> Result<()> {
        let account_key = Self::account_key(addr);
        if let Some(account) = self.load_internal::<Account>(&account_key)? {
            if account.sequence_number != expected_seq {
                anyhow::bail!("Invalid sequence number");
            }
            Ok(())
        } else {
            if expected_seq != 0 {
                anyhow::bail!("Account does not exist, sequence number must be 0");
            }
            Ok(())
        }
    }

    /// Get the total number of accounts
    pub fn account_count(&self) -> usize {
        self.load_index_list(ACCOUNT_INDEX_KEY)
            .map(|accounts| accounts.len())
            .unwrap_or(0)
    }
}

#[cfg(test)]
#[path = "../tests/unit/state_tests.rs"]
mod tests;
