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

    fn supply_key(token_type: &str) -> Vec<u8> {
        let mut key = b"supply:".to_vec();
        key.extend_from_slice(token_type.as_bytes());
        key
    }

    fn load_persisted_supply_from_store(store: &PersistentStore, token_type: &str) -> Option<u64> {
        let key = Self::supply_key(token_type);
        store
            .load::<TreasuryCap>(&key)
            .ok()
            .flatten()
            .map(|cap| cap.total_supply)
            .or_else(|| store.load::<u64>(&key).ok().flatten())
    }

    fn issued_supply_for_token(&self, token_type: &str) -> u64 {
        if token_type == KANARI_TOKEN_TYPE {
            return self.total_supply;
        }

        let supply_key = Self::supply_key(token_type);
        self.load_internal::<TreasuryCap>(&supply_key)
            .ok()
            .flatten()
            .map(|cap| cap.total_supply)
            .or_else(|| self.load_internal::<u64>(&supply_key).ok().flatten())
            .or_else(|| self.global_token_supplies.get(token_type).copied())
            .unwrap_or(0)
    }

    fn indexed_wallet_supply(&self, token_type: &str) -> Result<u64> {
        let token_type = Self::normalize_token_type(token_type);
        Ok(self
            .load_account_addresses()?
            .into_iter()
            .filter_map(|address| self.load_account(&address).ok().flatten())
            .map(|account| account.get_token_balance(&token_type))
            .fold(0u64, |acc, balance| acc.saturating_add(balance)))
    }

    fn load_object_locked_coin_records(&self) -> Result<Vec<ObjectLockedCoinRecord>> {
        Ok(self
            .load_internal(OBJECT_LOCKED_COIN_RECORDS_KEY)?
            .unwrap_or_default())
    }

    fn save_object_locked_coin_records(
        &mut self,
        records: &[ObjectLockedCoinRecord],
    ) -> Result<()> {
        self.save_internal(OBJECT_LOCKED_COIN_RECORDS_KEY, records)
    }

    fn object_locked_supply_for_token(&self, token_type: &str) -> Result<u64> {
        let token_type = Self::normalize_token_type(token_type);
        Ok(self
            .load_object_locked_coin_records()?
            .into_iter()
            .filter(|record| record.token_type == token_type)
            .map(|record| record.amount)
            .fold(0u64, |acc, amount| acc.saturating_add(amount)))
    }

    pub fn token_supply_summary(&self, token_type: &str) -> Result<TokenSupplySummary> {
        let token_type = Self::normalize_token_type(token_type);
        let total_supply = self.issued_supply_for_token(&token_type);
        let cached_visible = self
            .global_token_supplies
            .get(&token_type)
            .copied()
            .unwrap_or(0);
        let indexed_visible = self.indexed_wallet_supply(&token_type)?;
        let wallet_visible_supply = cached_visible.max(indexed_visible);
        let ledger_locked_supply = self.object_locked_supply_for_token(&token_type)?;
        let inferred_locked_supply = total_supply.saturating_sub(wallet_visible_supply);
        let object_locked_supply = ledger_locked_supply.max(inferred_locked_supply);
        let accounted_supply = wallet_visible_supply.saturating_add(object_locked_supply);

        Ok(TokenSupplySummary {
            token_type,
            total_supply,
            wallet_visible_supply,
            object_locked_supply,
            accounted_supply,
            untracked_supply: total_supply.saturating_sub(accounted_supply),
        })
    }

    pub fn supply_invariant_fail_fast_enabled() -> bool {
        std::env::var("KANARI_FAIL_FAST_ON_SUPPLY_MISMATCH")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or_else(|_| {
                matches!(
                    std::env::var("KANARI_NETWORK")
                        .unwrap_or_else(|_| "testnet".to_string())
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "mainnet"
                )
            })
    }

    fn report_supply_invariant_violation(context: &str, error: &anyhow::Error) {
        log::error!(
            "[StateManager] Supply invariant check failed {}: {}",
            context,
            error
        );

        assert!(
            !Self::supply_invariant_fail_fast_enabled(),
            "Supply invariant check failed {}: {}",
            context,
            error
        );
    }

    fn save_token_metadata_field<T: Serialize + ?Sized>(
        &mut self,
        prefix: &[u8],
        token_type: &str,
        value: &T,
    ) -> Result<()> {
        let key = metadata_key(prefix, token_type);
        self.save_internal(&key, value)
    }

    fn load_token_metadata_field<T: DeserializeOwned>(
        &self,
        prefix: &[u8],
        token_type: &str,
    ) -> Result<Option<T>> {
        let key = metadata_key(prefix, token_type);
        self.load_internal(&key)
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
    fn normalize_token_type(token_type: &str) -> String {
        if let Ok(TypeTag::Struct(st)) = TypeTag::from_str(token_type) {
            return format!("{}", st);
        }
        token_type.to_string()
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

    fn token_type_from_balance_struct(struct_tag: &StructTag) -> Option<String> {
        if let Some(TypeTag::Struct(st)) = struct_tag.type_params.first() {
            return Some(format!("{}", st));
        }
        None
    }

    fn persist_coin_metadata(&mut self, token_type: &str, data: &[u8]) -> Result<()> {
        #[derive(Deserialize)]
        struct MoveString {
            bytes: Vec<u8>,
        }
        #[derive(Deserialize)]
        struct MoveUrl {
            inner: MoveString,
        }
        #[derive(Deserialize)]
        struct MoveOption<T> {
            vec: Vec<T>,
        }
        #[derive(Deserialize)]
        struct ParsedCoinMetadata {
            _id: AccountAddress,
            decimals: u8,
            symbol: MoveString,
            name: MoveString,
            description: MoveString,
            icon_url: MoveOption<MoveUrl>,
        }

        if let Ok(meta) = bcs::from_bytes::<ParsedCoinMetadata>(data) {
            self.save_token_metadata_field(b"metadata_decimals:", token_type, &meta.decimals)?;

            if let Ok(name) = String::from_utf8(meta.name.bytes) {
                self.save_token_metadata_field(b"metadata_name:", token_type, &name)?;
            }
            if let Ok(symbol) = String::from_utf8(meta.symbol.bytes) {
                self.save_token_metadata_field(b"metadata_symbol:", token_type, &symbol)?;
            }
            if let Ok(description) = String::from_utf8(meta.description.bytes) {
                self.save_token_metadata_field(b"metadata_description:", token_type, &description)?;
            }
            if let Some(url_obj) = meta.icon_url.vec.into_iter().next()
                && let Ok(url) = String::from_utf8(url_obj.inner.bytes)
            {
                self.save_token_metadata_field(b"metadata_icon_url:", token_type, &url)?;
            }
        } else if data.len() > 32 {
            self.save_token_metadata_field(b"metadata_decimals:", token_type, &data[32])?;
        }
        Ok(())
    }

    fn adjust_global_supplies_for_account_delta(
        &mut self,
        old_balances: &BTreeMap<String, BalanceRecord>,
        new_balances: &BTreeMap<String, BalanceRecord>,
    ) -> bool {
        let mut changed = false;
        let mut tokens = BTreeSet::new();
        tokens.extend(old_balances.keys().cloned());
        tokens.extend(new_balances.keys().cloned());

        for token_type in tokens {
            let old_amount = old_balances
                .get(&token_type)
                .map(|x| x.value())
                .unwrap_or(0);
            let new_amount = new_balances
                .get(&token_type)
                .map(|x| x.value())
                .unwrap_or(0);

            if old_amount == new_amount {
                continue;
            }

            changed = true;
            let current_supply = self
                .global_token_supplies
                .get(&token_type)
                .copied()
                .unwrap_or(0);

            let updated_supply = if new_amount >= old_amount {
                current_supply.saturating_add(new_amount - old_amount)
            } else {
                current_supply.saturating_sub(old_amount - new_amount)
            };

            if updated_supply == 0 {
                self.global_token_supplies.remove(&token_type);
            } else {
                self.global_token_supplies
                    .insert(token_type, updated_supply);
            }
        }

        changed
    }

    fn recompute_token_balances_for_owner(&mut self, owner: AccountAddress) -> Result<bool> {
        let mut account = self.load_account_or_default(owner)?;
        let old_balances = account.token_balances.clone();
        let native_balance = account.native_balance();
        let mut aggregated: BTreeMap<String, u64> = BTreeMap::new();

        for object_id in self.get_owned_objects(&owner)? {
            let Some(obj) = self.get_object(&object_id)? else {
                continue;
            };

            let Ok(struct_tag) = StructTag::from_str(&obj.type_) else {
                continue;
            };

            if !Self::is_balance_struct(&struct_tag) {
                continue;
            }

            let Some(amount) = Self::extract_balance_from_object_bytes(&obj.data, &struct_tag)
            else {
                continue;
            };

            let Some(token_type) = Self::token_type_from_balance_struct(&struct_tag) else {
                continue;
            };

            let token_type = Self::normalize_token_type(&token_type);
            let entry = aggregated.entry(token_type).or_insert(0);
            *entry = entry.saturating_add(amount);
        }

        account.token_balances = aggregated
            .into_iter()
            .map(|(token_type, amount)| (token_type, BalanceRecord::new(amount)))
            .collect();
        if native_balance > 0 {
            account.set_token_balance(
                KANARI_TOKEN_TYPE.to_string(),
                BalanceRecord::new(native_balance),
            );
        }
        self.save_account(&account)?;

        Ok(self.adjust_global_supplies_for_account_delta(&old_balances, &account.token_balances))
    }

    /// Create a new in-memory state manager for testing
    pub fn new_in_memory() -> Self {
        Self::try_new_in_memory().expect("Failed to create in-memory state manager")
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
        Self::try_new(store).expect("Failed to create state manager")
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

        if let Err(e) = state.validate_supply_invariants() {
            Self::report_supply_invariant_violation("on startup", &e);
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
                || (key.starts_with(b"object:") && canonical_object_keys.contains(key))
        });
    }

    fn is_canonical_smt_update(&self, key: &[u8], value: &[u8]) -> bool {
        if Self::is_canonical_state_root_key(key) {
            return true;
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
                None if Self::is_canonical_state_root_key(key) || key.starts_with(b"object:") => {
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

    fn is_object_locked_coin_holder_type(type_name: &str) -> bool {
        let Ok(struct_tag) = StructTag::from_str(type_name) else {
            return false;
        };

        if Self::is_balance_struct(&struct_tag) {
            return false;
        }

        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();
        !(module_name == CoinModule::COIN_MODULE
            && (struct_name == CoinModule::TREASURY_CAP_STRUCT || struct_name == "CoinMetadata"))
    }

    fn supply_tracking_token_types(&self, changeset: &ChangeSet) -> BTreeSet<String> {
        let mut token_types: BTreeSet<String> =
            self.global_token_supplies.keys().cloned().collect();
        token_types.insert(KANARI_TOKEN_TYPE.to_string());

        for (_, token_type, _) in &changeset.treasuries {
            token_types.insert(Self::normalize_token_type(token_type));
        }
        for (_, token_type, _) in &changeset.token_balance_sets {
            token_types.insert(Self::normalize_token_type(token_type));
        }
        for (_, created) in &changeset.created_objects {
            if let Some((token_type, _)) = Self::balance_token_amount(&created.type_, &created.data)
            {
                token_types.insert(token_type);
            }
        }

        token_types
    }

    fn visible_supply_snapshot(&self, token_type: &str) -> Result<u64> {
        let token_type = Self::normalize_token_type(token_type);
        let cached = self
            .global_token_supplies
            .get(&token_type)
            .copied()
            .unwrap_or(0);
        Ok(cached.max(self.indexed_wallet_supply(&token_type)?))
    }

    fn add_locked_coin_record(
        records: &mut Vec<ObjectLockedCoinRecord>,
        holder: &(String, CreatedObject),
        token_type: &str,
        amount: u64,
    ) {
        if amount == 0 {
            return;
        }

        if let Some(existing) = records
            .iter_mut()
            .find(|record| record.holder_object_id == holder.0 && record.token_type == token_type)
        {
            existing.amount = existing.amount.saturating_add(amount);
            existing.holder_type = holder.1.type_.clone();
            existing.owner = holder.1.owner;
            return;
        }

        records.push(ObjectLockedCoinRecord {
            holder_object_id: holder.0.clone(),
            holder_type: holder.1.type_.clone(),
            owner: holder.1.owner,
            token_type: token_type.to_string(),
            amount,
        });
    }

    fn release_locked_coin_records(
        records: &mut Vec<ObjectLockedCoinRecord>,
        holder_ids: &HashSet<String>,
        token_type: &str,
        amount: u64,
    ) {
        let mut remaining = amount;

        for prefer_holder in [true, false] {
            if remaining == 0 {
                break;
            }

            for record in records.iter_mut() {
                if remaining == 0 {
                    break;
                }
                if record.token_type != token_type {
                    continue;
                }
                if prefer_holder && !holder_ids.contains(&record.holder_object_id) {
                    continue;
                }

                let release = record.amount.min(remaining);
                record.amount -= release;
                remaining -= release;
            }
        }

        records.retain(|record| record.amount > 0);
    }

    fn reconcile_object_locked_coin_records(
        &mut self,
        changeset: &ChangeSet,
        issued_before: &BTreeMap<String, u64>,
        visible_before: &BTreeMap<String, u64>,
    ) -> Result<()> {
        let holder_candidates: Vec<(String, CreatedObject)> = changeset
            .created_objects
            .iter()
            .filter(|(_, created)| Self::is_object_locked_coin_holder_type(&created.type_))
            .map(|(id, created)| (id.clone(), created.clone()))
            .collect();

        if holder_candidates.is_empty() && issued_before.is_empty() {
            return Ok(());
        }

        let holder_ids: HashSet<String> =
            holder_candidates.iter().map(|(id, _)| id.clone()).collect();
        let deleted_ids: HashSet<String> = changeset.deleted_objects.iter().cloned().collect();
        let mut records = self.load_object_locked_coin_records()?;
        let original_records = records.clone();

        if !deleted_ids.is_empty() {
            records.retain(|record| !deleted_ids.contains(&record.holder_object_id));
        }

        let token_types: BTreeSet<String> = issued_before
            .keys()
            .chain(visible_before.keys())
            .cloned()
            .collect();

        for token_type in token_types {
            let issued_before_value = issued_before.get(&token_type).copied().unwrap_or(0);
            let visible_before_value = visible_before.get(&token_type).copied().unwrap_or(0);
            let issued_after_value = self.issued_supply_for_token(&token_type);
            let visible_after_value = self.visible_supply_snapshot(&token_type)?;

            let issued_delta = issued_after_value as i128 - issued_before_value as i128;
            let visible_delta = visible_after_value as i128 - visible_before_value as i128;
            let locked_delta = issued_delta - visible_delta;

            if locked_delta > 0 {
                if let Some(holder) = holder_candidates.first() {
                    Self::add_locked_coin_record(
                        &mut records,
                        holder,
                        &token_type,
                        locked_delta as u64,
                    );
                }
            } else if locked_delta < 0 {
                Self::release_locked_coin_records(
                    &mut records,
                    &holder_ids,
                    &token_type,
                    (-locked_delta) as u64,
                );
            }
        }

        if records != original_records {
            self.save_object_locked_coin_records(&records)?;
        }

        Ok(())
    }

    /// Apply ChangeSet from Move VM execution
    /// This is the ONLY way to modify state - all changes must come from Move VM
    pub fn apply_changeset(&mut self, changeset: &ChangeSet) -> Result<()> {
        self.apply_changeset_with_options(changeset, true)
    }

    pub fn apply_changeset_without_supply_validation(
        &mut self,
        changeset: &ChangeSet,
    ) -> Result<()> {
        self.apply_changeset_with_options(changeset, false)
    }

    fn needs_object_locked_reconciliation(changeset: &ChangeSet) -> bool {
        !changeset.treasuries.is_empty()
            || !changeset.token_balance_sets.is_empty()
            || !changeset.created_objects.is_empty()
            || !changeset.deleted_objects.is_empty()
    }

    fn apply_changeset_with_options(
        &mut self,
        changeset: &ChangeSet,
        validate_supply: bool,
    ) -> Result<()> {
        let supply_delta = changeset
            .account_changes
            .values()
            .try_fold(0i64, |total, change| {
                total
                    .checked_add(change.balance_delta)
                    .ok_or_else(|| anyhow::anyhow!("Native supply delta overflow"))
            })?;
        let next_total_supply = if supply_delta > 0 {
            Some(
                self.total_supply
                    .checked_add(supply_delta as u64)
                    .ok_or_else(|| anyhow::anyhow!("Native total supply overflow"))?,
            )
        } else if supply_delta < 0 {
            let burn_amount = supply_delta.unsigned_abs();
            ensure!(
                self.total_supply >= burn_amount,
                "Native total supply underflow: tried to burn {} from {}",
                burn_amount,
                self.total_supply
            );
            Some(self.total_supply - burn_amount)
        } else {
            None
        };

        for (address, change) in &changeset.account_changes {
            if change.balance_delta >= 0 {
                continue;
            }
            let debit = change.balance_delta.unsigned_abs();
            let balance = self.load_account_or_default(*address)?.native_balance();
            ensure!(
                balance >= debit,
                "Insufficient native balance for {}: need {}, have {}",
                address.to_hex_literal(),
                debit,
                balance
            );
        }

        let mut supplies_dirty = false;
        let mut account_index_additions = Vec::with_capacity(changeset.account_changes.len());
        let reconcile_object_locked = Self::needs_object_locked_reconciliation(changeset);
        let token_types_before = if reconcile_object_locked {
            self.supply_tracking_token_types(changeset)
        } else {
            BTreeSet::new()
        };
        let mut issued_before = BTreeMap::new();
        let mut visible_before = BTreeMap::new();
        for token_type in token_types_before {
            issued_before.insert(
                token_type.clone(),
                self.issued_supply_for_token(&token_type),
            );
            visible_before.insert(
                token_type.clone(),
                self.visible_supply_snapshot(&token_type)?,
            );
        }

        for (address, change) in &changeset.account_changes {
            let mut account = self.load_account_or_default(*address)?;
            let old_balances = account.token_balances.clone();
            let native_token = KANARI_TOKEN_TYPE.to_string();

            if change.balance_delta > 0 {
                let amount = change.balance_delta as u64;
                let next = account
                    .native_balance()
                    .checked_add(amount)
                    .ok_or_else(|| anyhow::anyhow!("Native account balance overflow"))?;
                account.set_token_balance(native_token.clone(), BalanceRecord::new(next));
            } else if change.balance_delta < 0 {
                let debit = change.balance_delta.unsigned_abs();
                let next = account.native_balance() - debit;
                if next == 0 {
                    account.token_balances.remove(KANARI_TOKEN_TYPE);
                } else {
                    account.set_token_balance(native_token.clone(), BalanceRecord::new(next));
                }
            }
            account.sequence_number = account
                .sequence_number
                .checked_add(change.sequence_increment)
                .ok_or_else(|| anyhow::anyhow!("Account sequence number overflow"))?;
            for module_name in &change.modules_added {
                account.add_module(module_name.clone());
            }
            self.save_account_record(&account)?;
            account_index_additions.push(account.address.to_hex_literal());
            if self.adjust_global_supplies_for_account_delta(&old_balances, &account.token_balances)
            {
                supplies_dirty = true;
            }
        }

        self.add_many_to_index_list(ACCOUNT_INDEX_KEY, account_index_additions)?;

        // Update total supply if there was mint/burn (supply_delta != 0)
        if let Some(next_total_supply) = next_total_supply {
            self.total_supply = next_total_supply;
            self.save_internal(b"total_supply", &next_total_supply)?;
        }

        // Apply treasury creations/updates
        for (owner, token_type, total_supply) in &changeset.treasuries {
            let key = Self::supply_key(token_type);
            self.save_internal(&key, total_supply)?;

            let mut key_owner = b"treasury:".to_vec();
            key_owner.extend_from_slice(token_type.as_bytes());
            self.save_internal(&key_owner, owner)?;

            self.add_to_index_list(b"treasury_index", format!("treasury:{}", token_type))?;

            if token_type == KANARI_TOKEN_TYPE {
                self.total_supply = total_supply.total_supply;
                let supply = self.total_supply;
                self.save_internal(b"total_supply", &supply)?;
            }
        }

        // Apply NFT capability creations/updates
        for (owner, token_type, nft_cap) in &changeset.nft_caps {
            let mut key = b"nft:".to_vec();
            key.extend_from_slice(token_type.as_bytes());
            self.save_internal(&key, &(*owner, nft_cap.clone()))?;
        }

        // Apply incremental token balance hints first; exact owner totals are recomputed
        // from owned coin objects after object mutations are applied.
        for (owner, token_type, amount) in &changeset.token_balance_sets {
            let mut account = self.load_account_or_default(*owner)?;
            let normalized_token_type = Self::normalize_token_type(token_type);

            let old_balances = account.token_balances.clone();
            let current = account.get_token_balance(&normalized_token_type);
            let next = current.saturating_add(amount.value());
            account.set_token_balance(normalized_token_type, BalanceRecord::new(next));
            self.save_account(&account)?;

            if self.adjust_global_supplies_for_account_delta(&old_balances, &account.token_balances)
            {
                supplies_dirty = true;
            }
        }

        // Record Global Token Supplies to database only once after all processing
        let mut owners_to_recompute: BTreeSet<AccountAddress> = BTreeSet::new();

        for obj_id in &changeset.deleted_objects {
            if let Some((stored_id, existing)) = self.load_stored_object_by_any_id(obj_id)? {
                let obj_key = object_key(&stored_id);
                if Self::balance_token_amount(&existing.type_name, &existing.data).is_some() {
                    owners_to_recompute.insert(existing.owner);
                }
                let owner_key = owned_objects_key(&existing.owner);
                self.remove_from_index_list(&owner_key, &stored_id)?;
                self.overlay.insert(obj_key, None);
            } else {
                let obj_key = object_key(obj_id);
                self.overlay.insert(obj_key, None);
            }
        }

        // 1. Check for newly created Objects to index Collections
        for (obj_id, created) in &changeset.created_objects {
            // If this is a Collection type Object, record it in the global index
            if created.type_.contains("::collection::Collection") {
                self.add_to_index_list(b"nft_collection_index", obj_id.clone())?;
            }
        }

        // 2. Check Events to index NFT <-> Collection relationships
        for event in &changeset.events {
            // Check if this is a MintLog from james::nft
            if event.type_tag.to_string().contains("::nft::MintLog") {
                // MintLog data in nft.move contains: object_id(32), creator(32), collection_id(32)
                if event.event_data.len() >= 96 {
                    let nft_id_bytes = &event.event_data[0..32];
                    let coll_id_bytes = &event.event_data[64..96];

                    let (Ok(nft_id), Ok(coll_id)) = (
                        AccountAddress::from_bytes(nft_id_bytes).map(|addr| addr.to_hex_literal()),
                        AccountAddress::from_bytes(coll_id_bytes).map(|addr| addr.to_hex_literal()),
                    ) else {
                        log::warn!(
                            "Skipping malformed MintLog object ids while indexing collection members"
                        );
                        continue;
                    };

                    // Record in Collection member index (O(1) Access)
                    let key = metadata_key(b"collection_members:", &coll_id);
                    self.add_to_index_list(&key, nft_id)?;
                }
            }
        }

        for (obj_id, created) in &changeset.created_objects {
            let mut new_obj = created.clone();
            let existing_obj = self.load_stored_object_by_any_id(obj_id)?;
            let obj_key = object_key(obj_id);

            if let Some((stored_id, existing)) = existing_obj {
                let existing_affects_balances =
                    Self::balance_token_amount(&existing.type_name, &existing.data).is_some();
                // Use the version from the ChangeSet (already calculated by MoveRuntime)
                // Only recalculate if the ChangeSet version seems wrong (0 or less than existing)
                if new_obj.version == 0 || new_obj.version <= existing.version {
                    new_obj.version = existing.version + 1;
                }
                if new_obj.owner.to_hex_literal() == *obj_id {
                    new_obj.owner = existing.owner;
                }
                if existing.owner != new_obj.owner {
                    let old_owner_key = owned_objects_key(&existing.owner);
                    self.remove_from_index_list(&old_owner_key, &stored_id)?;
                }
                if stored_id != *obj_id {
                    self.overlay.insert(object_key(&stored_id), None);
                }
                if existing_affects_balances {
                    owners_to_recompute.insert(existing.owner);
                }
            } else {
                // For new objects, use version from ChangeSet or default to 1
                if new_obj.version == 0 {
                    new_obj.version = 1;
                }
            }
            if Self::balance_token_amount(&new_obj.type_, &new_obj.data).is_some() {
                owners_to_recompute.insert(new_obj.owner);
            }

            let stored_obj = StoredObject {
                id: obj_id.clone(),
                owner: new_obj.owner,
                type_name: new_obj.type_.clone(),
                data: new_obj.data.clone(),
                version: new_obj.version,
            };
            self.save_internal(&obj_key, &stored_obj)?;

            let owner_key = owned_objects_key(&new_obj.owner);
            self.remove_from_index_list(&owner_key, obj_id)?;
            self.add_to_index_list(&owner_key, obj_id.clone())?;

            if new_obj.type_.contains("::coin::CoinMetadata<")
                && let Some(start) = new_obj.type_.find('<')
                && let Some(end) = new_obj.type_.rfind('>')
            {
                let token_type = &new_obj.type_[start + 1..end];
                self.persist_coin_metadata(token_type, &new_obj.data)?;
            }
        }

        for owner in owners_to_recompute {
            if self.recompute_token_balances_for_owner(owner)? {
                supplies_dirty = true;
            }
        }

        if reconcile_object_locked {
            self.reconcile_object_locked_coin_records(changeset, &issued_before, &visible_before)?;
        }

        if supplies_dirty {
            let supplies_clone = self.global_token_supplies.clone();
            self.save_internal(b"global_token_supplies", &supplies_clone)?;
        }

        // =====================================================================
        // Process Dynamic Fields into State Overlay.
        // =====================================================================
        for (object_id, name_bytes, value_bytes) in &changeset.added_dynamic_fields {
            let df_key = Self::dynamic_field_key(object_id, name_bytes);
            self.save_internal(&df_key, value_bytes)?;
        }

        for (object_id, name_bytes) in &changeset.removed_dynamic_fields {
            let df_key = Self::dynamic_field_key(object_id, name_bytes);
            // Record as None so commit() will delete it from RocksDB
            self.overlay.insert(df_key, None);
        }

        if validate_supply && let Err(e) = self.validate_supply_invariants() {
            Self::report_supply_invariant_violation("after apply_changeset", &e);
        }

        Ok(())
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

    /// Get token decimals for a specific token type
    pub fn get_token_decimals(&self, token_type: &str) -> Result<Option<u8>> {
        self.load_token_metadata_field(b"metadata_decimals:", token_type)
    }

    ///  Get token name for a specific token type
    pub fn get_token_name(&self, token_type: &str) -> Result<Option<String>> {
        self.load_token_metadata_field(b"metadata_name:", token_type)
    }

    ///  Get token symbol for a specific token type
    pub fn get_token_symbol(&self, token_type: &str) -> Result<Option<String>> {
        self.load_token_metadata_field(b"metadata_symbol:", token_type)
    }

    /// Get token description for a specific token type
    pub fn get_token_description(&self, token_type: &str) -> Result<Option<String>> {
        self.load_token_metadata_field(b"metadata_description:", token_type)
    }

    /// Get token icon URL for a specific token type
    pub fn get_token_icon_url(&self, token_type: &str) -> Result<Option<String>> {
        self.load_token_metadata_field(b"metadata_icon_url:", token_type)
    }

    pub fn validate_supply_invariants(&self) -> Result<()> {
        let persisted_native_supply =
            Self::load_persisted_supply_from_store(self.store.as_ref(), KANARI_TOKEN_TYPE);
        if let Some(persisted) = persisted_native_supply
            && persisted != self.total_supply
        {
            anyhow::bail!(
                "native total supply mismatch: state.total_supply={} persisted_treasury={}",
                self.total_supply,
                persisted
            );
        }

        let native_supply = self.token_supply_summary(KANARI_TOKEN_TYPE)?;
        // Wallet-visible balance caches only reflect top-level wallet-owned
        // coin objects. Coins can also be held inside DeFi objects (for
        // example escrow funds), so visible supply may be lower than issued
        // supply without implying a burn. It must never exceed total supply.
        if native_supply.wallet_visible_supply > native_supply.total_supply {
            anyhow::bail!(
                "native supply overcount: total_supply={} wallet_visible_supply={} object_locked_supply={}",
                native_supply.total_supply,
                native_supply.wallet_visible_supply,
                native_supply.object_locked_supply
            );
        }
        if native_supply.accounted_supply > native_supply.total_supply {
            anyhow::bail!(
                "native supply overcount: total_supply={} accounted_supply={} wallet_visible_supply={} object_locked_supply={}",
                native_supply.total_supply,
                native_supply.accounted_supply,
                native_supply.wallet_visible_supply,
                native_supply.object_locked_supply
            );
        }

        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/state_tests.rs"]
mod tests;
