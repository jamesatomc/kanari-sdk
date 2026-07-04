// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Object-reference and protocol-metadata support for `StateManager`.
//!
//! Legacy object rows are interpreted as address-owned objects. New
//! object-centric writes persist an `ObjectMetadata` sidecar under the
//! canonical `system:` prefix, so owner variants, digest and previous
//! transaction survive restart and participate in the state root.

use crate::changeset::CreatedObject;
use crate::state::StateManager;
use anyhow::{Result, ensure};
use kanari_types::object::{
    ObjectID, ObjectMetadata, ObjectRef, Owner, compute_object_digest,
};
use move_core_types::account_address::AccountAddress;

fn metadata_key(object_id: ObjectID) -> Vec<u8> {
    let mut key = b"system:object_metadata:".to_vec();
    key.extend_from_slice(object_id.to_hex_literal().as_bytes());
    key
}

impl StateManager {
    pub fn get_object_protocol_metadata(
        &self,
        object_id: ObjectID,
    ) -> Result<Option<ObjectMetadata>> {
        self.load_internal(&metadata_key(object_id))
    }

    pub fn save_object_protocol_metadata(
        &mut self,
        metadata: &ObjectMetadata,
    ) -> Result<()> {
        self.save_internal(&metadata_key(metadata.id), metadata)
    }

    pub fn delete_object_protocol_metadata(&mut self, object_id: ObjectID) {
        self.overlay.insert(metadata_key(object_id), None);
    }

    pub fn get_object_owner(&self, object_id: ObjectID) -> Result<Option<Owner>> {
        if let Some(metadata) = self.get_object_protocol_metadata(object_id)? {
            return Ok(Some(metadata.owner));
        }
        Ok(self
            .get_object(&object_id.to_hex_literal())?
            .map(|object| Owner::AddressOwner(object.owner)))
    }

    pub fn get_object_ref_exact(&self, object_id: ObjectID) -> Result<Option<ObjectRef>> {
        let id = object_id.to_hex_literal();
        let Some(object) = self.get_object(&id)? else {
            return Ok(None);
        };

        if let Some(metadata) = self.get_object_protocol_metadata(object_id)? {
            ensure!(metadata.id == object_id, "Object metadata ID mismatch");
            ensure!(
                metadata.version == object.version,
                "Object metadata version does not match stored object"
            );
            let digest = compute_object_digest(
                object_id,
                object.version,
                &metadata.owner,
                &object.type_,
                &object.data,
                metadata.previous_transaction,
            )?;
            ensure!(
                digest == metadata.digest,
                "Stored object digest does not match canonical contents"
            );
            return Ok(Some(metadata.object_ref()));
        }

        let owner = Owner::AddressOwner(object.owner);
        let digest = compute_object_digest(
            object_id,
            object.version,
            &owner,
            &object.type_,
            &object.data,
            None,
        )?;
        Ok(Some(ObjectRef::new(object_id, object.version, digest)))
    }

    pub fn validate_object_ref_exact(&self, expected: &ObjectRef) -> Result<CreatedObject> {
        let id = expected.object_id.to_hex_literal();
        let object = self
            .get_object(&id)?
            .ok_or_else(|| anyhow::anyhow!("Input object {} does not exist", id))?;
        let actual = self
            .get_object_ref_exact(expected.object_id)?
            .ok_or_else(|| anyhow::anyhow!("Input object {} does not exist", id))?;
        ensure!(
            actual == *expected,
            "Stale or forged object reference for {}: expected version {} digest {}, current version {} digest {}",
            id,
            expected.version,
            expected.digest,
            actual.version,
            actual.digest
        );
        Ok(object)
    }

    pub fn validate_address_owned_object_ref(
        &self,
        expected: &ObjectRef,
        expected_owner: AccountAddress,
    ) -> Result<CreatedObject> {
        let object = self.validate_object_ref_exact(expected)?;
        let owner = self
            .get_object_owner(expected.object_id)?
            .ok_or_else(|| anyhow::anyhow!("Input object does not exist"))?;
        ensure!(
            owner == Owner::AddressOwner(expected_owner),
            "Object {} is not owned by {}",
            expected.object_id,
            expected_owner.to_hex_literal()
        );
        Ok(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::changeset::CreatedObject;
    use kanari_types::object::{IDRecord, UIDRecord};

    fn insert_test_object(
        state: &mut StateManager,
        id_address: AccountAddress,
        owner: AccountAddress,
        version: u64,
    ) {
        let object = CreatedObject {
            owner,
            uid: Some(UIDRecord::new(id_address)),
            id: Some(IDRecord::new(id_address)),
            type_: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
            data: vec![5; 40],
            version,
        };
        let mut changes = crate::changeset::ChangeSet::new();
        changes
            .created_objects
            .push((id_address.to_hex_literal(), object));
        state
            .apply_changeset_without_supply_validation(&changes)
            .unwrap();
    }

    #[test]
    fn missing_object_reference_is_rejected() {
        let state = StateManager::new_in_memory();
        let id = ObjectID::from_hex_literal("0x99").unwrap();
        let reference = ObjectRef::new(id, 1, kanari_types::object::ObjectDigest([1; 32]));
        assert!(state.validate_object_ref_exact(&reference).is_err());
    }

    #[test]
    fn exact_reference_round_trip_uses_full_object_contents() {
        let owner = AccountAddress::from_hex_literal("0x7").unwrap();
        let id_address = AccountAddress::from_hex_literal("0x77").unwrap();
        let id = ObjectID::new(id_address);
        let mut state = StateManager::new_in_memory();
        insert_test_object(&mut state, id_address, owner, 3);

        let reference = state.get_object_ref_exact(id).unwrap().unwrap();
        assert_eq!(reference.version, 3);
        assert!(state
            .validate_address_owned_object_ref(&reference, owner)
            .is_ok());
    }

    #[test]
    fn protocol_metadata_digest_survives_commit() {
        let owner = AccountAddress::from_hex_literal("0x8").unwrap();
        let id_address = AccountAddress::from_hex_literal("0x88").unwrap();
        let id = ObjectID::new(id_address);
        let mut state = StateManager::new_in_memory();
        insert_test_object(&mut state, id_address, owner, 4);
        let object = state.get_object(&id.to_hex_literal()).unwrap().unwrap();
        let previous_transaction = [9; 32];
        let protocol_owner = Owner::AddressOwner(owner);
        let digest = compute_object_digest(
            id,
            object.version,
            &protocol_owner,
            &object.type_,
            &object.data,
            Some(previous_transaction),
        )
        .unwrap();
        let metadata = ObjectMetadata {
            id,
            version: object.version,
            digest,
            owner: protocol_owner,
            previous_transaction: Some(previous_transaction),
        };
        state.save_object_protocol_metadata(&metadata).unwrap();
        state.commit().unwrap();

        assert_eq!(
            state.get_object_ref_exact(id).unwrap().unwrap(),
            metadata.object_ref()
        );
    }
}
