// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::common::keys::owned_objects_key;
use crate::storage::persistent_store::{PersistentStore, PersistentStoreError};
use anyhow::Result;
use kanari_types::transaction::ObjectOwnerKind;
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

#[derive(Debug)]
pub enum ObjectStorageError {
    PersistenceError(anyhow::Error),
}

impl std::fmt::Display for ObjectStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectStorageError::PersistenceError(e) => write!(f, "PersistenceError: {}", e),
        }
    }
}

impl std::error::Error for ObjectStorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ObjectStorageError::PersistenceError(e) => e.source(),
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

/// Trait abstraction for object storage backends.
pub trait ObjectStore: Send + Sync {
    fn store_object(&self, obj: StoredObject) -> Result<(), ObjectStorageError>;
    fn get_object(&self, id: &str) -> Result<Option<StoredObject>, ObjectStorageError>;
    fn delete_object(&self, id: &str) -> Result<(), ObjectStorageError>;
    #[cfg(test)]
    fn count(&self) -> usize;
    fn clear(&self) -> Result<(), ObjectStorageError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredObject {
    pub id: String,
    pub owner: AccountAddress,
    pub owner_kind: ObjectOwnerKind,
    pub type_name: String,
    pub data: Vec<u8>,
    pub version: u64,
}

impl StoredObject {
    #[cfg(test)]
    pub fn owner_address(&self) -> Option<AccountAddress> {
        match self.owner_kind {
            ObjectOwnerKind::AddressOwner(_) => Some(self.owner),
            ObjectOwnerKind::Shared | ObjectOwnerKind::Immutable => None,
        }
    }
}

struct InnerState {
    objects: BTreeMap<String, StoredObject>,
}

pub struct ObjectStorage {
    state: Arc<RwLock<InnerState>>,
    persistent: Option<Arc<PersistentStore>>,
}

impl ObjectStorage {
    const OBJECT_INDEX_KEY: &'static str = "object_index";

    fn legacy_owner_key(owner: &AccountAddress) -> Vec<u8> {
        let mut key = b"owner_index:".to_vec();
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

    #[cfg(test)]
    fn save_id_index(
        store: &PersistentStore,
        key: &[u8],
        ids: &[String],
    ) -> Result<(), ObjectStorageError> {
        store.save(key, ids)?;
        Ok(())
    }

    fn encode_update<T: Serialize + ?Sized>(
        key: Vec<u8>,
        value: &T,
    ) -> Result<(Vec<u8>, Vec<u8>), ObjectStorageError> {
        let bytes = bcs::to_bytes(value)
            .map_err(|error| ObjectStorageError::PersistenceError(error.into()))?;
        Ok((key, bytes))
    }

    fn load_owned_object_ids_for_batch(
        store: &PersistentStore,
        owner: &AccountAddress,
        deletes: &mut Vec<Vec<u8>>,
    ) -> Result<Vec<String>, ObjectStorageError> {
        let canonical_key = owned_objects_key(owner);
        let canonical_ids = Self::load_id_index(store, &canonical_key)?;
        if !canonical_ids.is_empty() {
            return Ok(canonical_ids);
        }

        let legacy_key = Self::legacy_owner_key(owner);
        let legacy_ids = Self::load_id_index(store, &legacy_key)?;
        if !legacy_ids.is_empty() {
            deletes.push(legacy_key);
        }
        Ok(legacy_ids)
    }

    #[cfg(test)]
    fn load_owned_object_ids(
        store: &PersistentStore,
        owner: &AccountAddress,
    ) -> Result<Vec<String>, ObjectStorageError> {
        let canonical_key = owned_objects_key(owner);
        let canonical_ids = Self::load_id_index(store, &canonical_key)?;
        if !canonical_ids.is_empty() {
            return Ok(canonical_ids);
        }

        let legacy_key = Self::legacy_owner_key(owner);
        let legacy_ids = Self::load_id_index(store, &legacy_key)?;
        if legacy_ids.is_empty() {
            return Ok(legacy_ids);
        }

        // One-time lazy migration from the old owner index format.
        Self::save_id_index(store, &canonical_key, &legacy_ids)?;
        store.delete(&legacy_key)?;
        Ok(legacy_ids)
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
    pub(crate) fn new_with_store(store: Arc<PersistentStore>) -> Result<Self> {
        let mut objects_map: BTreeMap<String, StoredObject> = BTreeMap::new();

        if let Some(ids) = store.load::<Vec<String>>(Self::OBJECT_INDEX_KEY.as_bytes())? {
            for id in ids {
                let object_key = format!("object:{id}");
                let object = store.load::<StoredObject>(object_key.as_bytes())?.ok_or_else(|| {
                    anyhow::anyhow!(
                        "object index references missing object {id}; refusing to start with a partial object cache"
                    )
                })?;
                objects_map.insert(id, object);
            }
        }

        Ok(Self {
            state: Arc::new(RwLock::new(InnerState {
                objects: objects_map,
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
        let mut old_owner_kind = None;
        let transaction_guard = self
            .persistent
            .as_ref()
            .map(|store| store.transaction_guard());

        {
            let state = self.state.read().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = state.objects.get(&id) {
                old_owner = Some(existing.owner);
                old_owner_kind = Some(existing.owner_kind.clone());
            }
        }

        if old_owner.is_none()
            && let Some(store) = &self.persistent
            && let Some(existing) =
                store.load::<StoredObject>(format!("object:{}", id).as_bytes())?
        {
            old_owner = Some(existing.owner);
            old_owner_kind = Some(existing.owner_kind);
        }

        if let Some(store) = &self.persistent {
            let mut updates = vec![Self::encode_update(
                format!("object:{}", id).into_bytes(),
                &obj,
            )?];
            let mut deletes = Vec::new();

            let new_is_owned = matches!(obj.owner_kind, ObjectOwnerKind::AddressOwner(_));
            if let Some(old) = old_owner {
                let old_is_owned = matches!(old_owner_kind, Some(ObjectOwnerKind::AddressOwner(_)));
                if old != owner || old_is_owned != new_is_owned {
                    if old_is_owned {
                        let old_key = owned_objects_key(&old);
                        let mut old_ids = Self::load_id_index(store, &old_key)?;
                        if Self::remove_index_id(&mut old_ids, &id) {
                            updates.push(Self::encode_update(old_key, &old_ids)?);
                        }
                    }

                    if new_is_owned {
                        let new_key = owned_objects_key(&owner);
                        let mut new_ids =
                            Self::load_owned_object_ids_for_batch(store, &owner, &mut deletes)?;
                        if Self::add_index_id(&mut new_ids, &id) {
                            updates.push(Self::encode_update(new_key, &new_ids)?);
                        }
                    }
                }
            } else {
                if new_is_owned {
                    let new_key = owned_objects_key(&owner);
                    let mut new_ids =
                        Self::load_owned_object_ids_for_batch(store, &owner, &mut deletes)?;
                    if Self::add_index_id(&mut new_ids, &id) {
                        updates.push(Self::encode_update(new_key, &new_ids)?);
                    }
                }
            }

            let mut ids = Self::load_id_index(store, Self::OBJECT_INDEX_KEY.as_bytes())?;
            if Self::add_index_id(&mut ids, &id) {
                updates.push(Self::encode_update(
                    Self::OBJECT_INDEX_KEY.as_bytes().to_vec(),
                    &ids,
                )?);
            }
            store.apply_raw_changes(&updates, &deletes)?;
        }

        // Publish to the live cache only after every persistent write succeeded.
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .objects
            .insert(id, obj);

        drop(transaction_guard);
        Ok(())
    }

    fn get_object(&self, id: &str) -> Result<Option<StoredObject>, ObjectStorageError> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        if let Some(obj) = state.objects.get(id) {
            return Ok(Some(obj.clone()));
        }
        drop(state);

        if let Some(store) = &self.persistent
            && let Some(obj) = store.load::<StoredObject>(format!("object:{}", id).as_bytes())?
        {
            let mut write_state = self.state.write().unwrap_or_else(|e| e.into_inner());
            write_state.objects.insert(id.to_string(), obj.clone());
            return Ok(Some(obj));
        }
        Ok(None)
    }

    #[cfg(test)]
    fn get_objects_by_owner(&self, owner: &AccountAddress) -> Vec<StoredObject> {
        if let Some(store) = &self.persistent {
            let ids = Self::load_owned_object_ids(store, owner).unwrap_or_default();
            let mut results = Vec::with_capacity(ids.len());
            for id in ids {
                if let Ok(Some(obj)) = self.get_object(&id) {
                    results.push(obj);
                }
            }
            results.sort_by(|a, b| a.id.cmp(&b.id));
            return results;
        }

        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        let mut results: Vec<_> = state
            .objects
            .values()
            .filter(|obj| obj.owner_address() == Some(*owner))
            .cloned()
            .collect();
        results.sort_by(|a, b| a.id.cmp(&b.id));
        results
    }

    fn delete_object(&self, id: &str) -> Result<(), ObjectStorageError> {
        let mut old_object = None;
        let transaction_guard = self
            .persistent
            .as_ref()
            .map(|store| store.transaction_guard());

        {
            let state = self.state.read().unwrap_or_else(|e| e.into_inner());
            if let Some(obj) = state.objects.get(id) {
                old_object = Some(obj.clone());
            }
        }

        if let Some(store) = &self.persistent {
            if old_object.is_none() {
                old_object = store.load::<StoredObject>(format!("object:{}", id).as_bytes())?;
            }
            let mut updates = Vec::new();
            let deletes = vec![format!("object:{}", id).into_bytes()];

            if let Some(object) = &old_object
                && matches!(object.owner_kind, ObjectOwnerKind::AddressOwner(_))
            {
                let owner_key = owned_objects_key(&object.owner);
                let mut ids = Self::load_id_index(store, &owner_key)?;
                if Self::remove_index_id(&mut ids, id) {
                    updates.push(Self::encode_update(owner_key, &ids)?);
                }
            }

            let mut ids = Self::load_id_index(store, Self::OBJECT_INDEX_KEY.as_bytes())?;
            if Self::remove_index_id(&mut ids, id) {
                updates.push(Self::encode_update(
                    Self::OBJECT_INDEX_KEY.as_bytes().to_vec(),
                    &ids,
                )?);
            }
            store.apply_raw_changes(&updates, &deletes)?;
        }

        // Keep the old cache entry available if persistence failed above.
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .objects
            .remove(id);

        drop(transaction_guard);
        Ok(())
    }

    #[cfg(test)]
    fn count(&self) -> usize {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .objects
            .len()
    }

    fn clear(&self) -> Result<(), ObjectStorageError> {
        let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
        state.objects.clear();
        Ok(())
    }
}

impl ObjectStore for ObjectStorage {
    fn store_object(&self, obj: StoredObject) -> Result<(), ObjectStorageError> {
        ObjectStorage::store_object(self, obj)
    }

    fn get_object(&self, id: &str) -> Result<Option<StoredObject>, ObjectStorageError> {
        ObjectStorage::get_object(self, id)
    }

    fn delete_object(&self, id: &str) -> Result<(), ObjectStorageError> {
        ObjectStorage::delete_object(self, id)
    }

    #[cfg(test)]
    fn count(&self) -> usize {
        ObjectStorage::count(self)
    }

    fn clear(&self) -> Result<(), ObjectStorageError> {
        ObjectStorage::clear(self)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/object_storage_tests.rs"]
mod tests;
