// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::common::ids::canonical_object_id;
use crate::{changeset::ChangeSet, storage::object_storage::StoredObject};
use kanari_system_natives::transfer_natives::TransferredObject;
use log::debug;
use move_core_types::language_storage::StructTag;
use std::str::FromStr;

impl super::MoveRuntime {
    pub(crate) fn add_transferred_objects_to_changeset(
        &self,
        cs: &mut ChangeSet,
        transferred: Vec<TransferredObject>,
        persist_runtime_state: bool,
    ) {
        let count = transferred.len();
        debug!("Processing {} transferred objects", count);

        for obj in transferred {
            let id = obj.object_id;
            let obj_type = obj.object_type;
            let owner = obj.recipient;
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

            let normalized_id = if id.trim().starts_with("0x") {
                id.clone()
            } else {
                format!("0x{}", id.trim())
            };
            let Some(canonical_id) = canonical_object_id(&normalized_id) else {
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
                    Err(e) => debug!(
                        "WARNING: Failed to persist object {} to storage: {}. Object remains in changeset.",
                        canonical_id, e
                    ),
                }
            }

            cs.created_objects
                .retain(|(existing_id, _)| existing_id != &canonical_id);

            if let Ok(struct_tag) = StructTag::from_str(&obj_type) {
                if self.is_treasury_resource(&struct_tag)
                    && let Some(total) = self.extract_treasury_total_from_bytes(&data)
                    && let Some(token_type) = self.token_type_from_struct_tag(&struct_tag)
                {
                    cs.add_treasury(owner, token_type, total);
                    debug!("Detected TreasuryCap object: supply={}", total);
                }

                if self.is_balance_resource(&struct_tag)
                    && let Some(amount) = self.extract_balance_from_bytes(&data, &struct_tag)
                    && let Some(token_type) = self.token_type_from_struct_tag(&struct_tag)
                {
                    cs.add_token_balance_set(owner, token_type, amount);
                    debug!("Detected Coin object: amount={}", amount);
                }
            }

            let created =
                Self::build_created_object(owner, &canonical_id, &obj_type, data, next_version);

            debug!(
                "Object {} - UID: {:?}, ID: {:?}",
                canonical_id,
                created.uid.as_ref().map(|u| u.address()),
                created.id.as_ref().map(|i| i.address())
            );

            cs.created_objects.push((canonical_id, created));
        }

        if count > 0 {
            debug!("Total {} objects added to changeset", count);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/object_ops_tests.rs"]
mod tests;
