// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Atomic application of canonical object effects.

use crate::changeset::{ChangeSet, CreatedObject};
use crate::state::StateManager;
use anyhow::{Result, ensure};
use kanari_types::object::{IDRecord, ObjectMetadata, Owner, UIDRecord, compute_object_digest};
use kanari_types::object_effects::{
    ObjectTransactionEffectsV1, ObjectWrite, ObjectWriteKind,
};

fn address_owner(write: &ObjectWrite) -> Result<move_core_types::account_address::AccountAddress> {
    match write.owner {
        Owner::AddressOwner(owner) => Ok(owner),
        Owner::ObjectOwner(_) => anyhow::bail!(
            "Object-owned writes require the StoredObject V2 migration"
        ),
        Owner::Shared { .. } => anyhow::bail!(
            "Shared-object writes require consensus ownership persistence"
        ),
        Owner::Immutable => anyhow::bail!(
            "Immutable writes require the StoredObject V2 migration"
        ),
    }
}

fn created_object(write: &ObjectWrite) -> Result<CreatedObject> {
    let id_address = write.object_ref.object_id.address();
    Ok(CreatedObject {
        owner: address_owner(write)?,
        uid: Some(UIDRecord::new(id_address)),
        id: Some(IDRecord::new(id_address)),
        type_: write.type_name.clone(),
        data: write.contents.clone(),
        version: write.object_ref.version,
    })
}

fn validate_write_digest(write: &ObjectWrite) -> Result<()> {
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
        "Object effect digest does not match canonical write contents"
    );
    Ok(())
}

impl StateManager {
    /// Apply one effects certificate atomically.
    ///
    /// Non-zero gas settlement is deliberately rejected until a canonical fee
    /// recipient object is included in effects. This prevents silently reducing
    /// visible native supply while migrating away from account gas credits.
    pub fn apply_object_effects_v1(
        &mut self,
        effects: &ObjectTransactionEffectsV1,
    ) -> Result<()> {
        effects.validate()?;
        ensure!(
            effects.gas_cost_summary.net_gas_usage()? == 0,
            "Non-zero object gas settlement requires a fee recipient object"
        );

        let mut candidate = self.clone();
        let mut changeset = ChangeSet::new();

        for write in &effects.created {
            validate_write_digest(write)?;
            ensure!(
                write.kind == ObjectWriteKind::Created,
                "Created effects contain a non-created write"
            );
            ensure!(
                candidate
                    .get_object(&write.object_ref.object_id.to_hex_literal())?
                    .is_none(),
                "Created object {} already exists",
                write.object_ref.object_id
            );
            changeset.created_objects.push((
                write.object_ref.object_id.to_hex_literal(),
                created_object(write)?,
            ));
        }

        for write in &effects.mutated {
            validate_write_digest(write)?;
            let previous = write.previous_object_ref.ok_or_else(|| {
                anyhow::anyhow!("Mutated object is missing its previous object reference")
            })?;
            candidate.validate_object_ref_exact(&previous)?;
            changeset.created_objects.push((
                write.object_ref.object_id.to_hex_literal(),
                created_object(write)?,
            ));
        }

        for deleted in &effects.deleted {
            candidate.validate_object_ref_exact(&deleted.object_ref)?;
            changeset
                .deleted_objects
                .push(deleted.object_ref.object_id.to_hex_literal());
        }

        candidate.apply_changeset(&changeset)?;

        for write in effects.created.iter().chain(effects.mutated.iter()) {
            candidate.save_object_protocol_metadata(&ObjectMetadata {
                id: write.object_ref.object_id,
                version: write.object_ref.version,
                digest: write.object_ref.digest,
                owner: write.owner.clone(),
                previous_transaction: Some(effects.transaction_digest),
            })?;
        }

        candidate.commit()?;
        *self = candidate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanari_types::object::{ObjectID, Owner};
    use kanari_types::object_effects::{
        GasCostSummary, ObjectTransactionEffectsV1, ObjectWrite, ObjectWriteKind,
    };

    #[test]
    fn applies_created_object_and_metadata_atomically() {
        let mut state = StateManager::new_in_memory();
        let id = ObjectID::from_hex_literal("0x901").unwrap();
        let owner = move_core_types::account_address::AccountAddress::from_hex_literal("0x77")
            .unwrap();
        let tx_digest = [3; 32];
        let write = ObjectWrite::new(
            id,
            1,
            None,
            Owner::AddressOwner(owner),
            "0x2::example::Record".to_string(),
            vec![1, 2, 3],
            tx_digest,
            ObjectWriteKind::Created,
        )
        .unwrap();
        let mut effects = ObjectTransactionEffectsV1::new(
            tx_digest,
            1,
            GasCostSummary::default(),
        );
        effects.created.push(write.clone());

        state.apply_object_effects_v1(&effects).unwrap();
        assert_eq!(
            state.get_object_ref_exact(id).unwrap().unwrap(),
            write.object_ref
        );
    }

    #[test]
    fn rejects_non_zero_gas_without_fee_recipient() {
        let mut state = StateManager::new_in_memory();
        let effects = ObjectTransactionEffectsV1::new(
            [4; 32],
            1,
            GasCostSummary {
                computation_cost: 1,
                ..GasCostSummary::default()
            },
        );
        assert!(state.apply_object_effects_v1(&effects).is_err());
    }
}
