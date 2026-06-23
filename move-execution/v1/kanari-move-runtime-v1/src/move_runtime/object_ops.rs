// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Object storage operations
use crate::{changeset::ChangeSet, storage::object_storage::StoredObject};
use kanari_system_natives::transfer_natives::TransferredObject;
use kanari_types::object::{IDRecord, UIDRecord};
use log::debug;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::StructTag;
use std::str::FromStr;

impl super::MoveRuntime {
    /// Reserved owner marker for immutable objects. Shared objects use address zero.
    pub(crate) fn immutable_object_owner() -> AccountAddress {
        AccountAddress::new([u8::MAX; AccountAddress::LENGTH])
    }

    fn canonical_object_id_str(object_id: &str) -> Option<String> {
        let trimmed = object_id.trim();
        let normalized = if trimmed.starts_with("0x") {
            trimmed.to_string()
        } else {
            format!("0x{}", trimmed)
        };

        AccountAddress::from_hex_literal(&normalized)
            .ok()
            .map(|addr| addr.to_hex_literal())
    }

    /// Add transferred objects from native function tracking to changeset
    /// Also persists objects to ObjectStorage for later retrieval
    pub(crate) fn add_transferred_objects_to_changeset(
        &self,
        cs: &mut ChangeSet,
        transferred: Vec<TransferredObject>,
        persist_runtime_state: bool,
    ) {
        let count = transferred.len();
        debug!("Processing {} transferred objects", count);

        for obj in transferred {
            // take ownership of fields to avoid unnecessary clones where possible
            let id = obj.object_id;
            let obj_type = obj.object_type;
            let owner = if obj.is_frozen {
                Self::immutable_object_owner()
            } else {
                obj.recipient
            };
            let data = obj.data;
            let should_persist = obj.should_persist;

            debug!(
                "Adding transferred object: id={} type={} owner={} data_len={}",
                id,
                obj_type,
                owner,
                data.len()
            );

            if !should_persist {
                debug!("Skipping non-persistable transferred object {}", id);
                continue;
            }

            let Some(canonical_id) = Self::canonical_object_id_str(&id) else {
                debug!("Skipping transferred object with invalid object id: {}", id);
                continue;
            };

            let next_version = self
                .object_storage
                .get_object(&canonical_id)
                .map(|existing| existing.version.saturating_add(1))
                .unwrap_or(1);

            if persist_runtime_state {
                let stored_obj = StoredObject {
                    id: canonical_id.clone(),
                    owner,
                    type_name: obj_type.clone(),
                    data: data.clone(),
                    version: next_version,
                };

                match self.object_storage.store_object(stored_obj) {
                    Ok(_) => debug!("Object {} persisted to ObjectStorage", canonical_id),
                    Err(e) => {
                        debug!(
                            "WARNING: Failed to persist object {} to storage: {}. Object remains in changeset.",
                            canonical_id, e
                        );
                    }
                }
            }

            // Upsert by object_id: a transferred object should represent the final owner/state.
            // This avoids duplicate entries for the same id from mixed paths
            // (e.g. mutable writeback + native transfer capture in the same tx).
            cs.created_objects
                .retain(|(existing_id, _)| existing_id != &canonical_id);

            // Add to created_objects in changeset (after storage to avoid double clone).
            // Pass the explicit ID to ensure ChangeSet uses the same ID as ObjectStorage.
            let uid = if let Ok(addr) = AccountAddress::from_hex_literal(&canonical_id) {
                Some(UIDRecord::new(addr))
            } else {
                None
            };

            // For DEX/DeFi objects, also create IDRecord for copyable ID tracking
            let id_record = if let Ok(addr) = AccountAddress::from_hex_literal(&canonical_id) {
                Some(IDRecord::new(addr))
            } else {
                None
            };

            // Detect special objects (TreasuryCap, Coin) and add them to changeset
            if let Ok(struct_tag) = StructTag::from_str(&obj_type) {
                // Parse TreasuryCap resources
                if self.is_treasury_resource(&struct_tag)
                    && let Some(total) = self.extract_treasury_total_from_bytes(&data)
                    && let Some(token_type) = self.token_type_from_struct_tag(&struct_tag)
                {
                    cs.add_treasury(owner, token_type, total);
                    debug!("Detected TreasuryCap object: supply={}", total);
                }

                // Parse Coin resources (Balance)
                // Each Coin object detected adds to token_balance_sets which will be ACCUMULATED
                // in StateManager. Multiple coins of the same type to same owner in one transaction
                // are consolidated by add_token_balance_set, and separate transactions accumulate.
                if self.is_balance_resource(&struct_tag)
                    && let Some(amount) = self.extract_balance_from_bytes(&data, &struct_tag)
                    && let Some(token_type) = self.token_type_from_struct_tag(&struct_tag)
                {
                    cs.add_token_balance_set(owner, token_type, amount);
                    debug!("Detected Coin object: amount={}", amount);
                }
            }

            // Log object type for debugging
            if id_record.is_some() {
                debug!(
                    "Object {} - UID: {:?}, ID: {:?}",
                    canonical_id,
                    uid.as_ref().map(|u| u.address()),
                    id_record.as_ref().map(|i| i.address())
                );
            }

            cs.add_created_object(
                owner,
                obj_type,
                data,
                next_version,
                uid,
                id_record,
                Some(canonical_id),
            );
        }

        if count > 0 {
            debug!("Total {} objects added to changeset", count);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/move_runtime/object_ops_tests.rs"]
mod tests;
