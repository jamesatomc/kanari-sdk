// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Canonical object-only effects application.

use crate::state::StateManager;
use crate::state_object_apply_helpers::FEE_SINK_TOTAL;
use anyhow::{Result, ensure};
use kanari_types::object_effects::{
    ObjectTombstone, ObjectTransactionEffectsV1, ObjectWriteKind,
};

impl StateManager {
    /// Validate and stage effects in the overlay without committing them.
    ///
    /// Callers may add auxiliary updates, such as mempool finalization and a
    /// replay marker, before one `commit()` persists the complete transition.
    pub fn stage_object_effects(&mut self, effects: &ObjectTransactionEffectsV1) -> Result<()> {
        effects.validate()?;
        let fee = effects.gas_cost_summary.net_gas_usage()?;
        ensure!(
            fee == 0 || effects.gas_object.is_some(),
            "Missing gas object effect"
        );

        let mut next = self.clone();
        for write in &effects.created {
            ensure!(write.kind == ObjectWriteKind::Created, "Invalid create effect");
            ensure!(
                write.previous_object_ref.is_none(),
                "Create has previous reference"
            );
            ensure!(
                !next.object_id_has_history(write.object_ref.object_id)?,
                "Object ID {} has already been used",
                write.object_ref.object_id
            );
            next.write_protocol_object(write)?;
        }
        for write in &effects.mutated {
            let previous = write
                .previous_object_ref
                .ok_or_else(|| anyhow::anyhow!("Mutation missing previous reference"))?;
            next.validate_object_ref_exact(&previous)?;
            ensure!(
                write.object_ref.object_id == previous.object_id,
                "Object ID changed"
            );
            ensure!(
                write.object_ref.version == effects.lamport_version,
                "Invalid version"
            );
            next.write_protocol_object(write)?;
        }
        for deleted in &effects.deleted {
            next.validate_object_ref_exact(&deleted.object_ref)?;
            next.remove_protocol_object(deleted.object_ref.object_id)?;
            next.save_object_tombstone(&ObjectTombstone {
                object_ref: deleted.object_ref,
                deletion_transaction: effects.transaction_digest,
                kind: deleted.kind,
            })?;
        }

        if fee > 0 {
            let total = next
                .load_internal::<u64>(FEE_SINK_TOTAL)?
                .unwrap_or_default()
                .checked_add(fee)
                .ok_or_else(|| anyhow::anyhow!("Fee counter overflow"))?;
            next.save_internal(FEE_SINK_TOTAL, &total)?;
        }
        *self = next;
        Ok(())
    }

    /// Apply effects without reading or writing account state.
    pub fn apply_object_effects(&mut self, effects: &ObjectTransactionEffectsV1) -> Result<()> {
        self.stage_object_effects(effects)?;
        self.commit()
    }

    pub fn object_fee_sink_total(&self) -> Result<u64> {
        Ok(self.load_internal(FEE_SINK_TOTAL)?.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanari_types::object::{ObjectID, Owner};
    use kanari_types::object_effects::{
        GasCostSummary, ObjectDelete, ObjectDeleteKind, ObjectWrite,
    };

    fn immutable_create(
        id: ObjectID,
        transaction_digest: [u8; 32],
    ) -> ObjectTransactionEffectsV1 {
        let write = ObjectWrite::new(
            id,
            1,
            None,
            Owner::Immutable,
            "0x2::package::Package".to_string(),
            vec![1],
            transaction_digest,
            ObjectWriteKind::Created,
        )
        .unwrap();
        let mut effects = ObjectTransactionEffectsV1::new(
            transaction_digest,
            1,
            GasCostSummary::default(),
        );
        effects.created.push(write);
        effects
    }

    #[test]
    fn stages_without_persisting_until_commit() {
        let mut state = StateManager::new_in_memory();
        let id = ObjectID::from_hex_literal("0xa00").unwrap();
        state.stage_object_effects(&immutable_create(id, [7; 32])).unwrap();
        assert!(state.get_object_ref_exact(id).unwrap().is_some());
        assert!(!state.overlay.is_empty());
        state.commit().unwrap();
        assert!(state.overlay.is_empty());
    }

    #[test]
    fn accepts_immutable_object_without_account() {
        let mut state = StateManager::new_in_memory();
        let id = ObjectID::from_hex_literal("0xa01").unwrap();
        state.apply_object_effects(&immutable_create(id, [1; 32])).unwrap();
        assert_eq!(state.get_object_owner(id).unwrap(), Some(Owner::Immutable));
    }

    #[test]
    fn deletion_tombstone_prevents_object_id_reuse() {
        let mut state = StateManager::new_in_memory();
        let id = ObjectID::from_hex_literal("0xa02").unwrap();
        state.apply_object_effects(&immutable_create(id, [2; 32])).unwrap();
        let reference = state.get_object_ref_exact(id).unwrap().unwrap();

        let mut deletion = ObjectTransactionEffectsV1::new(
            [3; 32],
            reference.version + 1,
            GasCostSummary::default(),
        );
        deletion.deleted.push(ObjectDelete {
            object_ref: reference,
            kind: ObjectDeleteKind::Deleted,
        });
        state.apply_object_effects(&deletion).unwrap();

        assert!(state.get_object_tombstone(id).unwrap().is_some());
        assert!(state.apply_object_effects(&immutable_create(id, [4; 32])).is_err());
    }
}
