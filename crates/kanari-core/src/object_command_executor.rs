// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::engine::BlockchainEngine;
use anyhow::{Context, Result};
use kanari_types::object_transaction::ObjectTransactionKind;
use kanari_types::signed_object_transaction::SignedObjectTransaction;

impl BlockchainEngine {
    pub fn submit_protocol_transaction(
        &self,
        transaction: SignedObjectTransaction,
    ) -> Result<Vec<u8>> {
        transaction.verify()?;
        {
            let state = self.state_read();
            for reference in transaction.data.owned_input_refs() {
                state.validate_address_owned_object_ref(&reference, transaction.data.sender)?;
            }
            for reference in &transaction.data.gas_data.payment {
                state.validate_address_owned_object_ref(
                    reference,
                    transaction.data.gas_data.owner,
                )?;
            }
        }
        self.submit_object_transaction(transaction)
    }

    pub fn execute_submitted_object_command(
        &self,
        digest: &[u8],
        gas_used: u64,
    ) -> Result<kanari_types::object_effects::ObjectTransactionEffectsV1> {
        let transaction = self
            .pending_object_transactions()?
            .into_iter()
            .find(|transaction| transaction.digest().ok().as_deref() == Some(digest))
            .ok_or_else(|| anyhow::anyhow!("Pending object transaction was not found"))?;

        match &transaction.data.kind {
            ObjectTransactionKind::Pay { .. } | ObjectTransactionKind::TransferObjects { .. } => {}
            _ => anyhow::bail!("Submitted transaction is not a direct object command"),
        }

        let effects = match self.build_object_command_effects(&transaction, gas_used) {
            Ok(effects) => effects,
            Err(error) => {
                self.release_object_transaction(digest)?;
                return Err(error);
            }
        };
        if let Err(error) = self.state_write().apply_object_effects(&effects) {
            self.release_object_transaction(digest)?;
            return Err(error).context("Failed to apply object command effects");
        }
        self.finalize_object_transaction(digest)?
            .ok_or_else(|| anyhow::anyhow!("Object transaction disappeared before finalization"))?;
        Ok(effects)
    }

    pub fn execute_object_command_now(
        &self,
        transaction: SignedObjectTransaction,
        gas_used: u64,
    ) -> Result<kanari_types::object_effects::ObjectTransactionEffectsV1> {
        let digest = self.submit_protocol_transaction(transaction)?;
        self.execute_submitted_object_command(&digest, gas_used)
    }
}
