// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
#[cfg(test)]
use kanari_types::error::KanariUnwrapExt;
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::ModuleId;
use std::path::PathBuf;
use std::sync::Arc;

use crate::storage::object_storage::StoredObject;
use crate::storage::persistent_store::PersistentStore;

/// Persistent storage wrapper for Move modules, resources, and framework metadata.
#[derive(Clone)]
#[allow(clippy::upper_case_acronyms)]
pub(crate) struct MoveVMState {
    store: Arc<PersistentStore>,
    overlay: Option<crate::StateOverlay>,
}

impl MoveVMState {
    const MODULE_INDEX_KEY: &'static [u8] = b"module_index";

    /// Use an already-open persistent store shared with the chain state.
    pub(crate) fn new(store: Arc<PersistentStore>) -> Self {
        MoveVMState {
            store,
            overlay: None,
        }
    }

    /// Return the shared backing store so callers can create isolated runtime caches
    /// over the same canonical module/resource database.
    pub(crate) fn store(&self) -> Arc<PersistentStore> {
        self.store.clone()
    }

    /// Create an in-memory MoveVMState for testing or Miri (no filesystem ops).
    pub(crate) fn new_in_memory() -> Result<Self> {
        let store = PersistentStore::open_in_memory()?;
        Ok(MoveVMState {
            store: Arc::new(store),
            overlay: None,
        })
    }

    /// Open default store for Move VM state.
    pub(crate) fn open_default() -> Result<Self> {
        // Honor legacy env var for Move VM DB path
        let db_path = std::env::var("KANARI_MOVE_VM_DB").ok().map(PathBuf::from);
        let store = PersistentStore::open_with_path(db_path)?;
        Ok(MoveVMState {
            store: Arc::new(store),
            overlay: None,
        })
    }

    pub(crate) fn with_overlay(&self, overlay: Option<crate::StateOverlay>) -> Self {
        Self {
            store: self.store.clone(),
            overlay,
        }
    }

    fn load_with_overlay<T: serde::de::DeserializeOwned>(&self, key: &[u8]) -> Result<Option<T>> {
        if let Some(overlay) = &self.overlay
            && let Some(value) = overlay.get(key)
        {
            return value
                .as_ref()
                .map(|bytes| bcs::from_bytes(bytes).map_err(Into::into))
                .transpose();
        }
        Ok(self.store.load(key)?)
    }

    fn module_key(module_id: &ModuleId) -> String {
        format!(
            "module:{}:{}",
            module_id.address().to_hex_literal(),
            module_id.name().as_str()
        )
    }

    fn resource_key(
        address: &AccountAddress,
        tag: &move_core_types::language_storage::StructTag,
    ) -> String {
        format!("resource:{}:{}", address.to_hex_literal(), tag)
    }

    fn framework_manifest_key(name: &str) -> String {
        format!("framework_manifest:{name}")
    }

    fn framework_hash_key(name: &str) -> String {
        format!("framework_hash:{name}")
    }

    fn object_key(object_id: &AccountAddress) -> String {
        format!("object:{}", object_id.to_hex_literal())
    }

    fn load_string_index(&self, key: &[u8]) -> Result<Vec<String>> {
        Ok(self.store.load::<Vec<String>>(key)?.unwrap_or_default())
    }

    fn add_to_string_index(&self, key: &[u8], value: String) -> Result<()> {
        let mut index = self.load_string_index(key)?;
        if !index.iter().any(|entry| entry == &value) {
            index.push(value);
            self.store.save(key, &index)?;
        }
        Ok(())
    }

    #[cfg(feature = "framework-pruning")]
    fn remove_from_string_index(&self, key: &[u8], value: &str) -> Result<()> {
        let mut index = self.load_string_index(key)?;
        let old_len = index.len();
        index.retain(|entry| entry != value);
        if index.len() != old_len {
            self.store.save(key, &index)?;
        }
        Ok(())
    }

    fn parse_module_key(key: &str) -> Option<ModuleId> {
        let parts: Vec<&str> = key.splitn(3, ':').collect();
        if parts.len() != 3 {
            return None;
        }
        let addr = AccountAddress::from_hex_literal(parts[1]).ok()?;
        let ident = Identifier::new(parts[2]).ok()?;
        Some(ModuleId::new(addr, ident))
    }

    /// Save a module blob keyed by module id.
    pub(crate) fn save_module(&self, module_id: &ModuleId, blob: &[u8]) -> Result<()> {
        let key = Self::module_key(module_id);
        self.store.save(key.as_bytes(), blob)?;
        self.add_to_string_index(Self::MODULE_INDEX_KEY, key)?;
        Ok(())
    }

    /// Delete a module blob keyed by module id and remove it from the persistent index.
    #[cfg(feature = "framework-pruning")]
    pub(crate) fn delete_module(&self, module_id: &ModuleId) -> Result<()> {
        let key = Self::module_key(module_id);
        self.store.delete(key.as_bytes())?;
        self.remove_from_string_index(Self::MODULE_INDEX_KEY, &key)?;
        Ok(())
    }

    /// Persist framework manifest + hash for operational safety / debugging.
    pub(crate) fn save_framework_manifest(
        &self,
        name: &str,
        manifest: &Vec<(String, String)>,
        hash_hex: &str,
    ) -> Result<()> {
        let manifest_key = Self::framework_manifest_key(name);
        let hash_key = Self::framework_hash_key(name);
        self.store.save(manifest_key.as_bytes(), manifest)?;
        self.store
            .save(hash_key.as_bytes(), &hash_hex.to_string())?;
        Ok(())
    }

    /// Load a previously persisted framework hash (if any).
    pub(crate) fn try_get_framework_hash(&self, name: &str) -> Result<Option<String>> {
        let hash_key = Self::framework_hash_key(name);
        Ok(self.store.load::<String>(hash_key.as_bytes())?)
    }

    /// Get all module IDs from the persistent index.
    pub(crate) fn get_all_module_ids(&self) -> Result<Vec<ModuleId>> {
        let mut modules = Vec::new();
        for module_key in self.load_string_index(Self::MODULE_INDEX_KEY)? {
            if let Some(module_id) = Self::parse_module_key(&module_key) {
                modules.push(module_id);
            }
        }
        Ok(modules)
    }

    /// Get module bytecode from persistent storage without hiding backend errors.
    pub(crate) fn try_get_module(&self, module_id: &ModuleId) -> Result<Option<Vec<u8>>> {
        let key = Self::module_key(module_id);
        self.load_with_overlay(key.as_bytes())
    }

    /// Save a resource blob keyed by address and struct tag.
    /// Coin resources also update the mirrored object payload stored under the same address.
    #[cfg(test)]
    pub(crate) fn save_resource(
        &self,
        address: &AccountAddress,
        tag: &move_core_types::language_storage::StructTag,
        blob: &[u8],
    ) -> Result<()> {
        let key = Self::resource_key(address, tag);

        // Persist the resource blob in Move VM storage.
        self.store
            .save(key.as_bytes(), blob)
            .require("Failed to persist Move VM resource")?;

        // Keep coin objects in sync with the latest resource bytes.
        if tag.module.as_str() == "coin" && tag.name.as_str() == "Coin" {
            let obj_key = Self::object_key(address);

            if let Some(mut stored_object) = self.store.load::<StoredObject>(obj_key.as_bytes())? {
                // Keep the mirrored payload current for VM resource reads, but leave
                // object versioning to StateManager so authorities stay deterministic.
                stored_object.data = blob.to_vec();
                self.store.save(obj_key.as_bytes(), &stored_object)?;
            }
        }

        Ok(())
    }

    /// Get resource blob from persistent storage
    #[cfg(test)]
    pub(crate) fn get_resource(
        &self,
        address: &AccountAddress,
        tag: &move_core_types::language_storage::StructTag,
    ) -> Option<Vec<u8>> {
        self.try_get_resource(address, tag).ok().flatten()
    }

    pub(crate) fn try_get_resource(
        &self,
        address: &AccountAddress,
        tag: &move_core_types::language_storage::StructTag,
    ) -> Result<Option<Vec<u8>>> {
        let key = Self::resource_key(address, tag);
        self.load_with_overlay(key.as_bytes())
    }

    /// Load object payload bytes from the stored `CreatedObject` wrapper.
    pub(crate) fn try_get_object(&self, object_id: &AccountAddress) -> Result<Option<Vec<u8>>> {
        Ok(self
            .try_get_stored_object(object_id)?
            .map(|object| object.data))
    }

    /// Load the full stored object metadata and payload for object-native execution.
    pub(crate) fn try_get_stored_object(
        &self,
        object_id: &AccountAddress,
    ) -> Result<Option<StoredObject>> {
        let obj_key = Self::object_key(object_id);
        self.load_with_overlay::<StoredObject>(obj_key.as_bytes())
    }

    /// Delete a resource blob keyed by address and struct tag.
    #[cfg(test)]
    pub(crate) fn delete_resource(
        &self,
        address: &AccountAddress,
        tag: &move_core_types::language_storage::StructTag,
    ) -> Result<()> {
        let key = Self::resource_key(address, tag);
        self.store
            .delete(key.as_bytes())
            .require("Failed to delete Move VM resource")
    }
}

#[cfg(test)]
#[path = "../../tests/unit/move_vm_state_tests.rs"]
mod tests;
