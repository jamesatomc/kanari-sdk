// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::engine::BlockchainEngine;
use anyhow::{Context, Result, ensure};
use kanari_types::object_effects::ObjectTransactionEffectsV1;
use kanari_types::object_transaction::ObjectTransactionKind;
use kanari_types::signed_object_transaction::SignedObjectTransaction;

impl BlockchainEngine {
    pub fn object_command_gas_units(
        &self,
        transaction: &SignedObjectTransaction,
    ) -> Result<u64> {
        let units = match &transaction.data.kind {
            ObjectTransactionKind::Pay { coins, .. } => 100u64
                .checked_add((coins.len() as u64).saturating_mul(10))
                .ok_or_else(|| anyhow::anyhow!("Pay gas overflow"))?,
            ObjectTransactionKind::TransferObjects { objects, .. } => 50u64
                .checked_add((objects.len() as u64).saturating_mul(5))
                .ok_or_else(|| anyhow::anyhow!("Transfer gas overflow"))?,
            ObjectTransactionKind::MoveCall(_) => {
                anyhow::bail!("MoveCall object execution is not enabled yet")
            }
            ObjectTransactionKind::Publish { .. } => {
                anyhow::bail!("Publish object execution is not enabled yet")
            }
        };
        ensure!(
            units <= transaction.data.gas_data.budget,
            "Required gas units {} exceed budget {}",
            units,
            transaction.data.gas_data.budget
        );
        Ok(units)
    }

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
    ) -> Result<ObjectTransactionEffectsV1> {
        let transaction = self
            .pending_object_transaction(digest)?
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

    /// Canonical direct-object execution entry point used by both RPC and P2P.
    pub fn execute_protocol_transaction(
        &self,
        transaction: SignedObjectTransaction,
    ) -> Result<(Vec<u8>, ObjectTransactionEffectsV1)> {
        let digest = transaction.digest()?;
        ensure!(
            !self.is_object_transaction_executed(&digest)?,
            "Object transaction already executed"
        );
        let gas_units = self.object_command_gas_units(&transaction)?;
        if self.pending_object_transaction(&digest)?.is_none() {
            self.submit_protocol_transaction(transaction)?;
        }
        let effects = self.execute_submitted_object_command(&digest, gas_units)?;
        Ok((digest, effects))
    }

    /// Finalize valid durable admissions after restart and release any entry
    /// that can no longer execute against the current object state.
    pub fn recover_pending_object_transactions(&self) -> Result<(usize, usize)> {
        let (_, mut removed) = self.repair_object_transaction_pool()?;
        let pending = self.pending_object_transactions()?;
        let mut executed = 0usize;
        for transaction in pending {
            let digest = match transaction.digest() {
                Ok(digest) => digest,
                Err(_) => continue,
            };
            let result = self
                .object_command_gas_units(&transaction)
                .and_then(|gas| self.execute_submitted_object_command(&digest, gas).map(|_| ()));
            match result {
                Ok(()) => executed = executed.saturating_add(1),
                Err(error) => {
                    log::warn!(
                        "Releasing unrecoverable object transaction 0x{}: {}",
                        hex::encode(&digest),
                        error
                    );
                    self.release_object_transaction(&digest)?;
                    removed = removed.saturating_add(1);
                }
            }
        }
        Ok((executed, removed))
    }

    pub fn execute_object_command_now(
        &self,
        transaction: SignedObjectTransaction,
        _gas_used: u64,
    ) -> Result<ObjectTransactionEffectsV1> {
        self.execute_protocol_transaction(transaction)
            .map(|(_, effects)| effects)
    }
}
