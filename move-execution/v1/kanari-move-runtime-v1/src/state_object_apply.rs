// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Canonical object-only effects application.

use crate::state::StateManager;
use crate::state_object_apply_helpers::FEE_SINK_TOTAL;
use anyhow::{Result, ensure};
use kanari_types::object_effects::{ObjectTransactionEffectsV1, ObjectWriteKind};

impl StateManager {
    /// Apply effects without reading or writing account state.
    pub fn apply_object_effects(&mut self, effects: &ObjectTransactionEffectsV1) -> Result<()> {
        effects.validate()?;
        let fee = effects.gas_cost_summary.net_gas_usage()?;
        ensure!(fee == 0 || effects.gas_object.is_some(), "Missing gas object effect");

        let mut next = self.clone();
        for write in &effects.created {
            ensure!(write.kind == ObjectWriteKind::Created, "Invalid create effect");
            ensure!(write.previous_object_ref.is_none(), "Create has previous reference");
            ensure!(
                next.get_object_ref_exact(write.object_ref.object_id)?.is_none(),
                "Object already exists"
            );
            next.write_protocol_object(write)?;
        }
        for write in &effects.mutated {
            let previous = write
                .previous_object_ref
                .ok_or_else(|| anyhow::anyhow!("Mutation missing previous reference"))?;
            next.validate_object_ref_exact(&previous)?;
            ensure!(write.object_ref.object_id == previous.object_id, "Object ID changed");
            ensure!(write.object_ref.version == effects.lamport_version, "Invalid version");
            next.write_protocol_object(write)?;
        }
        for deleted in &effects.deleted {
            next.validate_object_ref_exact(&deleted.object_ref)?;
            next.remove_protocol_object(deleted.object_ref.object_id)?;
        }

        if fee > 0 {
            let total = next
                .load_internal::<u64>(FEE_SINK_TOTAL)?
                .unwrap_or_default()
                .checked_add(fee)
                .ok_or_else(|| anyhow::anyhow!("Fee counter overflow"))?;
            next.save_internal(FEE_SINK_TOTAL, &total)?;
        }
        next.commit()?;
        *self = next;
        Ok(())
    }

    pub fn object_fee_sink_total(&self) -> Result<u64> {
        Ok(self.load_internal(FEE_SINK_TOTAL)?.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kanari_types::object::{ObjectID, Owner};
    use kanari_types::object_effects::{GasCostSummary, ObjectWrite};

    #[test]
    fn accepts_immutable_object_without_account() {
        let mut state = StateManager::new_in_memory();
        let id = ObjectID::from_hex_literal("0xa01").unwrap();
        let write = ObjectWrite::new(
            id,
            1,
            None,
            Owner::Immutable,
            "0x2::package::Package".to_string(),
            vec![1],
            [1; 32],
            ObjectWriteKind::Created,
        )
        .unwrap();
        let mut effects = ObjectTransactionEffectsV1::new([1; 32], 1, GasCostSummary::default());
        effects.created.push(write);
        state.apply_object_effects(&effects).unwrap();
        assert_eq!(state.get_object_owner(id).unwrap(), Some(Owner::Immutable));
    }
}
