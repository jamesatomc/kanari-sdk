// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::common::keys::{object_key, owned_objects_key};
use crate::state::StateManager;
use crate::storage::object_storage::StoredObject;
use anyhow::{Result, ensure};
use kanari_types::object::{ObjectID, ObjectMetadata, Owner, compute_object_digest};
use kanari_types::object_effects::ObjectWrite;
use move_core_types::account_address::AccountAddress;

pub(crate) const FEE_SINK_TOTAL: &[u8] = b"system:object_fee_sink_total";

pub(crate) fn metadata_key(id: ObjectID) -> Vec<u8> {
    [
        b"system:object_metadata:".as_slice(),
        id.to_hex_literal().as_bytes(),
    ]
    .concat()
}

fn projected_owner(owner: &Owner) -> AccountAddress {
    match owner {
        Owner::AddressOwner(address) => *address,
        Owner::ObjectOwner(parent) => parent.address(),
        Owner::Shared { .. } | Owner::Immutable => AccountAddress::ZERO,
    }
}

fn address_owner(owner: &Owner) -> Option<AccountAddress> {
    match owner {
        Owner::AddressOwner(address) => Some(*address),
        _ => None,
    }
}

fn update_sorted(ids: &mut Vec<String>, id: &str, add: bool) {
    ids.sort();
    ids.dedup();
    match (ids.binary_search_by(|value| value.as_str().cmp(id)), add) {
        (Err(position), true) => ids.insert(position, id.to_string()),
        (Ok(position), false) => {
            ids.remove(position);
        }
        _ => {}
    }
}

fn verify_write(write: &ObjectWrite) -> Result<()> {
    let digest = compute_object_digest(
        write.object_ref.object_id,
        write.object_ref.version,
        &write.owner,
        &write.type_name,
        &write.contents,
        Some(write.previous_transaction),
    )?;
    ensure!(
        digest == write.object_ref.digest,
        "Invalid object write digest"
    );
    ensure!(
        write.previous_transaction != [0; 32],
        "Missing transaction digest"
    );
    Ok(())
}

impl StateManager {
    fn set_object_address_index(
        &mut self,
        id: ObjectID,
        owner: Option<AccountAddress>,
        add: bool,
    ) -> Result<()> {
        let Some(owner) = owner else {
            return Ok(());
        };
        let key = owned_objects_key(&owner);
        let mut ids: Vec<String> = self.load_internal(&key)?.unwrap_or_default();
        update_sorted(&mut ids, &id.to_hex_literal(), add);
        self.save_internal(&key, &ids)
    }

    pub(crate) fn write_protocol_object(&mut self, write: &ObjectWrite) -> Result<()> {
        verify_write(write)?;
        let old_owner = self.get_object_owner(write.object_ref.object_id)?;
        self.set_object_address_index(
            write.object_ref.object_id,
            old_owner.as_ref().and_then(address_owner),
            false,
        )?;
        self.set_object_address_index(
            write.object_ref.object_id,
            address_owner(&write.owner),
            true,
        )?;

        let id = write.object_ref.object_id.to_hex_literal();
        self.save_internal(
            &object_key(&id),
            &StoredObject {
                id,
                owner: projected_owner(&write.owner),
                type_name: write.type_name.clone(),
                data: write.contents.clone(),
                version: write.object_ref.version,
            },
        )?;
        self.save_object_protocol_metadata(&ObjectMetadata {
            id: write.object_ref.object_id,
            version: write.object_ref.version,
            digest: write.object_ref.digest,
            owner: write.owner.clone(),
            previous_transaction: Some(write.previous_transaction),
        })
    }

    pub(crate) fn remove_protocol_object(&mut self, id: ObjectID) -> Result<()> {
        let old_owner = self.get_object_owner(id)?;
        self.set_object_address_index(id, old_owner.as_ref().and_then(address_owner), false)?;
        self.overlay.insert(object_key(&id.to_hex_literal()), None);
        self.overlay.insert(metadata_key(id), None);
        Ok(())
    }
}
