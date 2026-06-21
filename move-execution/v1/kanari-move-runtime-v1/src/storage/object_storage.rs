// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::storage::persistent_store::{PersistentStore, PersistentStoreError};
use anyhow::Result;
use kanari_crypto::hash_data_blake3;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::TypeTag;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

/// Create a unique key for a dynamic field in RocksDB
/// Format: df_{object_id}_{hash(name_bytes)}
fn derive_dynamic_field_key(object_id: &str, name_bytes: &[u8]) -> String {
    let hash = hash_data_blake3(name_bytes);
    format!("df_{}_{}", object_id, hex::encode(&hash[0..16]))
}

/// Error types that can occur during `ObjectStorage` operations.
#[derive(Debug)]
pub enum ObjectStorageError {
    /// Failed to acquire or interact with an in-memory lock.
    LockError(String),

    /// Failure writing to or reading from the persistent backend.
    PersistenceError(anyhow::Error),

    /// The requested object was not found in the store.
    NotFound,
}

impl std::fmt::Display for ObjectStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectStorageError::LockError(s) => write!(f, "LockError: {}", s),
            ObjectStorageError::PersistenceError(e) => write!(f, "PersistenceError: {}", e),
            ObjectStorageError::NotFound => write!(f, "NotFound"),
        }
    }
}

impl std::error::Error for ObjectStorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ObjectStorageError::PersistenceError(e) => e.source(),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for ObjectStorageError {
    fn from(e: anyhow::Error) -> Self {
        ObjectStorageError::PersistenceError(e)
    }
}

impl From<PersistentStoreError> for ObjectStorageError {
    fn from(e: PersistentStoreError) -> Self {
        ObjectStorageError::PersistenceError(e.into())
    }
}

/// Trait abstraction for object storage backends. Allows swapping in-memory and
/// persistent implementations without changing the runtime.
pub trait ObjectStore: Send + Sync {
    fn store_object(&self, obj: StoredObject) -> Result<(), ObjectStorageError>;
    fn get_object(&self, id: &str) -> Option<StoredObject>;
    fn get_objects_by_owner(&self, owner: &AccountAddress) -> Vec<StoredObject>;
    fn transfer_object(
        &self,
        id: &str,
        new_owner: AccountAddress,
    ) -> Result<(), ObjectStorageError>;
    fn delete_object(&self, id: &str) -> Result<(), ObjectStorageError>;
    fn count(&self) -> usize;
    fn clear(&self) -> Result<(), ObjectStorageError>;

    fn get_coins_by_type_and_owner(
        &self,
        owner: AccountAddress,
        coin_type: &TypeTag,
    ) -> Vec<StoredObject>;

    // --- 🟢 Dynamic Field Methods ---
    fn put_dynamic_field(
        &self,
        object_id: &str,
        name_bytes: &[u8],
        value_bytes: &[u8],
    ) -> Result<()>;
    fn get_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Option<Vec<u8>>;
    fn remove_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Result<()>;
}

/// Stored object with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredObject {
    pub id: String,
    pub owner: AccountAddress,
    pub type_name: String,
    pub data: Vec<u8>,
    pub version: u64,
}

struct InnerState {
    objects: BTreeMap<String, StoredObject>,
    dynamic_fields: BTreeMap<String, Vec<u8>>,
}

pub struct ObjectStorage {
    state: Arc<RwLock<InnerState>>,
    persistent: Option<Arc<PersistentStore>>,
}

impl ObjectStorage {
    const OBJECT_INDEX_KEY: &'static str = "object_index";

    // Helper to create Key for fetching Owner Index directly from RocksDB database
    fn owner_key(owner: &AccountAddress) -> Vec<u8> {
        let mut key = b"owner_index:".to_vec();
        key.extend_from_slice(owner.as_ref());
        key
    }

    fn canonical_owned_objects_key(owner: &AccountAddress) -> Vec<u8> {
        let mut key = b"owned_objects:".to_vec();
        key.extend_from_slice(owner.as_ref());
        key
    }

    fn load_id_index(
        store: &PersistentStore,
        key: &[u8],
    ) -> Result<Vec<String>, ObjectStorageError> {
        let mut ids: Vec<String> = store.load(key)?.unwrap_or_default();
        ids.sort();
        ids.dedup();
        Ok(ids)
    }

    fn save_id_index(
        store: &PersistentStore,
        key: &[u8],
        ids: &[String],
    ) -> Result<(), ObjectStorageError> {
        store.save(key, ids)?;
        Ok(())
    }

    fn add_index_id(ids: &mut Vec<String>, id: &str) -> bool {
        match ids.binary_search_by(|existing| existing.as_str().cmp(id)) {
            Ok(_) => false,
            Err(pos) => {
                ids.insert(pos, id.to_string());
                true
            }
        }
    }

    fn remove_index_id(ids: &mut Vec<String>, id: &str) -> bool {
        match ids.binary_search_by(|existing| existing.as_str().cmp(id)) {
            Ok(pos) => {
                ids.remove(pos);
                true
            }
            Err(_) => false,
        }
    }

    fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(InnerState {
                objects: BTreeMap::new(),
                dynamic_fields: BTreeMap::new(),
            })),
            persistent: None,
        }
    }

    pub(crate) fn boxed_inmemory() -> Box<dyn ObjectStore> {
        Box::new(Self::new())
    }
}

impl Default for ObjectStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectStorage {
    fn get_coins_by_type_and_owner(
        &self,
        owner: AccountAddress,
        coin_type: &move_core_types::language_storage::TypeTag,
    ) -> Vec<StoredObject> {
        let mut coins: Vec<_> = self
            .get_objects_by_owner(&owner)
            .into_iter()
            .filter(|obj| {
                if let Ok(struct_tag) = obj
                    .type_name
                    .parse::<move_core_types::language_storage::StructTag>()
                    && struct_tag.module.as_str() == "coin"
                    && struct_tag.name.as_str() == "Coin"
                    && let Some(tag) = struct_tag.type_params.first()
                {
                    return tag == coin_type;
                }
                false
            })
            .collect();
        coins.sort_by(|a, b| a.id.cmp(&b.id));
        coins
    }

    /// Create a new ObjectStorage backed by an already-open persistent store.
    pub(crate) fn new_with_store(store: Arc<PersistentStore>) -> Result<Self> {
        let mut objects_map: BTreeMap<String, StoredObject> = BTreeMap::new();

        if let Ok(Some(ids)) = store.load::<Vec<String>>(Self::OBJECT_INDEX_KEY.as_bytes()) {
            for id in ids.into_iter() {
                if let Ok(Some(obj)) =
                    store.load::<StoredObject>(format!("object:{}", id).as_bytes())
                {
                    objects_map.insert(id, obj);
                }
            }
        }

        Ok(Self {
            state: Arc::new(RwLock::new(InnerState {
                objects: objects_map,
                dynamic_fields: BTreeMap::new(),
            })),
            persistent: Some(store),
        })
    }

    pub(crate) fn boxed_with_store(store: Arc<PersistentStore>) -> Result<Box<dyn ObjectStore>> {
        if cfg!(miri) {
            return Ok(Self::boxed_inmemory());
        }
        Ok(Box::new(Self::new_with_store(store)?))
    }

    fn store_object(&self, obj: StoredObject) -> Result<(), ObjectStorageError> {
        let id = obj.id.clone();
        let owner = obj.owner;
        let mut old_owner = None;

        // 🚨 Lock Poisoning Fix: Always use unwrap_or_else
        {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = state.objects.get(&id) {
                old_owner = Some(existing.owner);
            }
            state.objects.insert(id.clone(), obj.clone());
        }

        // 🚨 Persist owner index directly to DB instead of keeping in memory
        if let Some(store) = &self.persistent {
            store.save(format!("object:{}", id).as_bytes(), &obj)?;

            if let Some(old) = old_owner {
                if old != owner {
                    let old_key = Self::owner_key(&old);
                    let mut old_ids = Self::load_id_index(store, &old_key)?;
                    if Self::remove_index_id(&mut old_ids, &id) {
                        Self::save_id_index(store, &old_key, &old_ids)?;
                    }

                    let new_key = Self::owner_key(&owner);
                    let mut new_ids = Self::load_id_index(store, &new_key)?;
                    if Self::add_index_id(&mut new_ids, &id) {
                        Self::save_id_index(store, &new_key, &new_ids)?;
                    }
                }
            } else {
                let new_key = Self::owner_key(&owner);
                let mut new_ids = Self::load_id_index(store, &new_key)?;
                if Self::add_index_id(&mut new_ids, &id) {
                    Self::save_id_index(store, &new_key, &new_ids)?;
                }
            }

            let mut ids = Self::load_id_index(store, Self::OBJECT_INDEX_KEY.as_bytes())?;
            if Self::add_index_id(&mut ids, &id) {
                Self::save_id_index(store, Self::OBJECT_INDEX_KEY.as_bytes(), &ids)?;
            }
        }

        Ok(())
    }

    /// Get object by ID
    pub fn get_object(&self, id: &str) -> Option<StoredObject> {
        // 🚨 Lock Poisoning Fix
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        if let Some(obj) = state.objects.get(id) {
            return Some(obj.clone());
        }
        drop(state); // Drop Lock before accessing database

        if let Some(store) = &self.persistent
            && let Ok(Some(obj)) = store.load::<StoredObject>(format!("object:{}", id).as_bytes())
        {
            let mut write_state = self.state.write().unwrap_or_else(|e| e.into_inner());
            write_state.objects.insert(id.to_string(), obj.clone());
            return Some(obj);
        }
        None
    }

    /// Get all objects owned by an address
    fn get_objects_by_owner(&self, owner: &AccountAddress) -> Vec<StoredObject> {
        // 🚨 Read Owner Index from DB directly if persistent
        if let Some(store) = &self.persistent {
            let canonical_key = Self::canonical_owned_objects_key(owner);
            let mut ids = Self::load_id_index(store, &canonical_key).unwrap_or_default();
            if ids.is_empty() {
                let legacy_key = Self::owner_key(owner);
                ids = Self::load_id_index(store, &legacy_key).unwrap_or_default();
            }

            let mut results = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(obj) = self.get_object(&id) {
                    results.push(obj);
                }
            }
            results.sort_by(|a, b| a.id.cmp(&b.id));
            return results;
        }

        // 🚨 Fallback: in-memory calculation (used in testing environments)
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .objects
            .values()
            .filter(|obj| obj.owner == *owner)
            .cloned()
            .collect();
        results.sort_by(|a, b| a.id.cmp(&b.id));
        results
    }

    // Transfer object ownership
    fn transfer_object(
        &self,
        id: &str,
        new_owner: AccountAddress,
    ) -> Result<(), ObjectStorageError> {
        if self.get_object(id).is_none() {
            return Err(ObjectStorageError::NotFound);
        }

        // 🚨 FIX: Fetch value directly from RwLock scope without declaring dummy variables
        let (old_owner, obj_to_persist) = {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            if let Some(obj) = state.objects.get_mut(id) {
                let old = obj.owner;
                obj.owner = new_owner;
                (old, obj.clone()) // Return value as Tuple
            } else {
                return Err(ObjectStorageError::NotFound);
            }
        };

        if let Some(store) = &self.persistent {
            store.save(format!("object:{}", id).as_bytes(), &obj_to_persist)?;

            let old_key = Self::owner_key(&old_owner);
            let mut old_ids = Self::load_id_index(store, &old_key)?;
            if Self::remove_index_id(&mut old_ids, id) {
                Self::save_id_index(store, &old_key, &old_ids)?;
            }

            let new_key = Self::owner_key(&new_owner);
            let mut new_ids = Self::load_id_index(store, &new_key)?;
            if Self::add_index_id(&mut new_ids, id) {
                Self::save_id_index(store, &new_key, &new_ids)?;
            }
        }

        Ok(())
    }

    /// Delete object
    fn delete_object(&self, id: &str) -> Result<(), ObjectStorageError> {
        let mut old_owner = None;

        {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            if let Some(obj) = state.objects.remove(id) {
                old_owner = Some(obj.owner);
            }
        }

        if let Some(store) = &self.persistent {
            store.delete(format!("object:{}", id).as_bytes())?;

            if let Some(owner) = old_owner {
                let owner_key = Self::owner_key(&owner);
                let mut ids = Self::load_id_index(store, &owner_key)?;
                if Self::remove_index_id(&mut ids, id) {
                    Self::save_id_index(store, &owner_key, &ids)?;
                }
            }

            let mut ids = Self::load_id_index(store, Self::OBJECT_INDEX_KEY.as_bytes())?;
            if Self::remove_index_id(&mut ids, id) {
                Self::save_id_index(store, Self::OBJECT_INDEX_KEY.as_bytes(), &ids)?;
            }
        }

        Ok(())
    }

    /// Get total number of objects
    fn count(&self) -> usize {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .objects
            .len()
    }

    /// Clear all objects
    fn clear(&self) -> Result<(), ObjectStorageError> {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.objects.clear();
        state.dynamic_fields.clear(); // Clear Cache
        Ok(())
    }

    // =====================================================================
    // 🟢 Dynamic Field Implementations
    // =====================================================================

    fn put_dynamic_field(
        &self,
        object_id: &str,
        name_bytes: &[u8],
        value_bytes: &[u8],
    ) -> Result<()> {
        let key = derive_dynamic_field_key(object_id, name_bytes);
        let value = value_bytes.to_vec();

        {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            state.dynamic_fields.insert(key.clone(), value.clone());
        }

        if let Some(store) = &self.persistent {
            // Use vector load/save to comply with PersistentStore BCS requirements
            store
                .save(key.as_bytes(), &value)
                .map_err(|e| anyhow::anyhow!("RocksDB Error (put_dynamic_field): {}", e))?;
        }

        Ok(())
    }

    fn get_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Option<Vec<u8>> {
        let key = derive_dynamic_field_key(object_id, name_bytes);

        {
            let state = self.state.read().unwrap_or_else(|e| e.into_inner());
            if let Some(val) = state.dynamic_fields.get(&key) {
                return Some(val.clone());
            }
        }

        if let Some(store) = &self.persistent
            && let Ok(Some(val)) = store.load::<Vec<u8>>(key.as_bytes())
        {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            state.dynamic_fields.insert(key, val.clone());
            return Some(val);
        }

        None
    }

    fn remove_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Result<()> {
        let key = derive_dynamic_field_key(object_id, name_bytes);

        {
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            state.dynamic_fields.remove(&key);
        }

        if let Some(store) = &self.persistent {
            store
                .delete(key.as_bytes())
                .map_err(|e| anyhow::anyhow!("RocksDB Error (remove_dynamic_field): {}", e))?;
        }

        Ok(())
    }
}

// Implement the ObjectStore trait for the in-memory ObjectStorage
impl ObjectStore for ObjectStorage {
    fn store_object(&self, obj: StoredObject) -> Result<(), ObjectStorageError> {
        ObjectStorage::store_object(self, obj)
    }

    fn get_object(&self, id: &str) -> Option<StoredObject> {
        ObjectStorage::get_object(self, id)
    }

    fn get_objects_by_owner(&self, owner: &AccountAddress) -> Vec<StoredObject> {
        ObjectStorage::get_objects_by_owner(self, owner)
    }

    fn transfer_object(
        &self,
        id: &str,
        new_owner: AccountAddress,
    ) -> Result<(), ObjectStorageError> {
        ObjectStorage::transfer_object(self, id, new_owner)
    }

    fn delete_object(&self, id: &str) -> Result<(), ObjectStorageError> {
        ObjectStorage::delete_object(self, id)
    }

    fn count(&self) -> usize {
        ObjectStorage::count(self)
    }

    fn clear(&self) -> Result<(), ObjectStorageError> {
        ObjectStorage::clear(self)
    }

    fn get_coins_by_type_and_owner(
        &self,
        owner: AccountAddress,
        coin_type: &TypeTag,
    ) -> Vec<StoredObject> {
        ObjectStorage::get_coins_by_type_and_owner(self, owner, coin_type)
    }

    // Dynamic Field methods
    fn put_dynamic_field(
        &self,
        object_id: &str,
        name_bytes: &[u8],
        value_bytes: &[u8],
    ) -> Result<()> {
        ObjectStorage::put_dynamic_field(self, object_id, name_bytes, value_bytes)
    }

    fn get_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Option<Vec<u8>> {
        ObjectStorage::get_dynamic_field(self, object_id, name_bytes)
    }

    fn remove_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Result<()> {
        ObjectStorage::remove_dynamic_field(self, object_id, name_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persistent_owner_lookup_prefers_canonical_owned_objects_index() -> Result<()> {
        let store = Arc::new(PersistentStore::open_in_memory()?);
        let owner = AccountAddress::from_hex_literal("0x1")?;
        let stale_id = "0xaaaa".to_string();
        let canonical_id = "0xbbbb".to_string();

        store.save(
            format!("object:{}", stale_id).as_bytes(),
            &StoredObject {
                id: stale_id.clone(),
                owner,
                type_name: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
                data: vec![1],
                version: 1,
            },
        )?;
        store.save(
            format!("object:{}", canonical_id).as_bytes(),
            &StoredObject {
                id: canonical_id.clone(),
                owner,
                type_name: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
                data: vec![2],
                version: 1,
            },
        )?;
        store.save(&ObjectStorage::owner_key(&owner), &vec![stale_id.clone()])?;
        store.save(
            &ObjectStorage::canonical_owned_objects_key(&owner),
            &vec![canonical_id.clone()],
        )?;

        let storage = ObjectStorage::new_with_store(store)?;
        let objects = storage.get_objects_by_owner(&owner);

        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].id, canonical_id);

        Ok(())
    }
}
