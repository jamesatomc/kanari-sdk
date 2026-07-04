// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Object-reference validation exposed by `StateManager`.
//!
//! Legacy object rows do not persist digest/owner variants yet. During the
//! migration they are interpreted as address-owned objects and their digest is
//! derived deterministically from the complete stored representation.

use crate::changeset::CreatedObject;
use crate::state::StateManager;
use anyhow::{Result, ensure};
use kanari_types::object::{ObjectID, ObjectRef, Owner, compute_object_digest};
use move_core_types::account_address::AccountAddress;

impl StateManager {
    pub fn get_object_ref_exact(&self, object_id: ObjectID) -> Result<Option<ObjectRef>> {
        let id = object_id.to_hex_literal();
        let Some(object) = self.get_object(&id)? else {
            return Ok(None);
        };
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
        ensure!(
            object.owner == expected_owner,
            "Object {} is owned by {}, not {}",
            expected.object_id,
            object.owner.to_hex_literal(),
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
        let object = CreatedObject {
            owner,
            uid: Some(UIDRecord::new(id_address)),
            id: Some(IDRecord::new(id_address)),
            type_: "0x2::coin::Coin<0x2::kanari::KANARI>".to_string(),
            data: vec![5; 40],
            version: 3,
        };
        let mut changes = crate::changeset::ChangeSet::new();
        changes.created_objects.push((id.to_hex_literal(), object));
        state.apply_changeset_without_supply_validation(&changes).unwrap();

        let reference = state.get_object_ref_exact(id).unwrap().unwrap();
        assert_eq!(reference.version, 3);
        assert!(state
            .validate_address_owned_object_ref(&reference, owner)
            .is_ok());
    }
}
