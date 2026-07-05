// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Helper functions for MoveRuntime resource parsing and object ID generation
use kanari_system_natives::dynamic_field::{DynamicFieldResolver, DynamicFieldStorageExt};
use kanari_types::balance::BalanceModule;
use kanari_types::coin::CoinModule;

use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::{StructTag, TypeTag};
use std::sync::Arc;

struct RuntimeDynamicFieldResolver {
    store: Arc<crate::storage::persistent_store::PersistentStore>,
}

impl RuntimeDynamicFieldResolver {
    fn dynamic_field_key(object_id: &str, name_bytes: &[u8]) -> Vec<u8> {
        let hash = kanari_crypto::hash_data_blake3(name_bytes);
        let mut key = b"df:".to_vec();
        key.extend_from_slice(object_id.as_bytes());
        key.extend_from_slice(b":");
        key.extend_from_slice(hex::encode(&hash[0..16]).as_bytes());
        key
    }
}

impl DynamicFieldResolver for RuntimeDynamicFieldResolver {
    fn get_dynamic_field(&self, object_id: &str, name_bytes: &[u8]) -> Option<Vec<u8>> {
        let key = Self::dynamic_field_key(object_id, name_bytes);
        self.store.load::<Vec<u8>>(&key).ok().flatten()
    }
}

/// Size of a Move object UID in bytes (address)
const UID_SIZE: usize = 32;
/// Size of a u64 field in bytes
const U64_SIZE: usize = 8;

impl super::MoveRuntime {
    pub(crate) fn dynamic_field_storage_ext(&self) -> DynamicFieldStorageExt {
        DynamicFieldStorageExt::new(Arc::new(RuntimeDynamicFieldResolver {
            store: self.state.store(),
        }))
    }

    /// Preload potential object arguments into LoadedObjectsExt before execution
    /// This enables native_borrow_global and borrow_global_mut to resolve objects during VM execution
    pub(crate) fn preload_objects_for_execution(
        &self,
        session: &mut move_vm_runtime::session::Session<
            crate::storage::resolver::KanariMoveResolver,
        >,
        args: &[Vec<u8>],
    ) -> anyhow::Result<()> {
        use kanari_system_natives::object::LoadedObjectsExt;

        // Scan through arguments to find potential object IDs (32-byte addresses)
        for arg in args {
            if arg.len() == 32 {
                let Ok(object_addr) = AccountAddress::from_bytes(arg.as_slice()) else {
                    continue;
                };
                let object_id = object_addr.to_hex_literal();

                let stored_obj = self.object_storage.get_object(&object_id);

                // Try to load object from storage
                if let Some(stored_obj) = stored_obj {
                    // Insert into LoadedObjectsExt so native_borrow_global and borrow_global_mut can find it
                    let exts = session.get_native_extensions();
                    let loaded_ext = exts.get_mut::<LoadedObjectsExt>();
                    loaded_ext.insert(object_id.clone(), stored_obj.type_name, stored_obj.data);
                    log::debug!(
                        "[RUNTIME] Preloaded object {} into LoadedObjectsExt",
                        object_id
                    );
                }
            }
        }

        Ok(())
    }

    /// Check if struct tag represents a balance/coin resource
    pub(crate) fn is_balance_resource(&self, struct_tag: &StructTag) -> bool {
        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();

        (module_name == CoinModule::COIN_MODULE && struct_name == CoinModule::COIN_STRUCT)
            || (module_name == BalanceModule::BALANCE_MODULE
                && struct_name == BalanceModule::BALANCE_STRUCT)
    }

    /// Check if struct tag represents a treasury resource
    pub(crate) fn is_treasury_resource(&self, struct_tag: &StructTag) -> bool {
        struct_tag.name.as_str() == CoinModule::TREASURY_CAP_STRUCT
    }

    /// Extract balance value from bytes for resources that may include UID + Balance
    pub(crate) fn extract_balance_from_bytes(
        &self,
        bytes: &[u8],
        struct_tag: &StructTag,
    ) -> Option<u64> {
        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();

        // Balance<T> (no UID): just 8 bytes
        if module_name == BalanceModule::BALANCE_MODULE
            && struct_name == BalanceModule::BALANCE_STRUCT
            && bytes.len() == U64_SIZE
        {
            let balance_bytes: [u8; U64_SIZE] = bytes.try_into().ok()?;
            return Some(u64::from_le_bytes(balance_bytes));
        }

        // Coin<T> (with UID): [32-byte address][8-byte balance]
        if module_name == CoinModule::COIN_MODULE
            && struct_name == CoinModule::COIN_STRUCT
            && bytes.len() >= (UID_SIZE + U64_SIZE)
        {
            let balance_bytes: [u8; U64_SIZE] =
                bytes[UID_SIZE..(UID_SIZE + U64_SIZE)].try_into().ok()?;
            return Some(u64::from_le_bytes(balance_bytes));
        }

        None
    }

    /// Extract total supply from TreasuryCap bytes
    pub(crate) fn extract_treasury_total_from_bytes(&self, bytes: &[u8]) -> Option<u64> {
        // TreasuryCap: [32-byte address][8-byte total_supply]
        if bytes.len() >= (UID_SIZE + U64_SIZE) {
            let supply_bytes: [u8; U64_SIZE] =
                bytes[UID_SIZE..(UID_SIZE + U64_SIZE)].try_into().ok()?;
            Some(u64::from_le_bytes(supply_bytes))
        } else {
            None
        }
    }

    /// Extract token type string from struct tag's type parameters
    pub(crate) fn token_type_from_struct_tag(&self, struct_tag: &StructTag) -> Option<String> {
        if let Some(TypeTag::Struct(st)) = struct_tag.type_params.first() {
            // Normalize via Move's Display impl so all code paths use one canonical
            // token type key (e.g. `0x2::kanari::KANARI`).
            return Some(format!("{}", st));
        }
        None
    }
}
