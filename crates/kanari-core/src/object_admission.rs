// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Admission checks for object-centric transactions.
//!
//! These checks are intentionally separate from the legacy sender/sequence
//! mempool so the new path can be wired into RPC/DAG without mutating legacy
//! transaction semantics during migration.

use crate::{BlockchainEngine, ObjectMempool};
use anyhow::{Context, Result, ensure};
use kanari_types::object::{ObjectRef, Owner, compute_object_digest};
use kanari_types::object_transaction::ObjectArg;
use kanari_types::signed_object_transaction::SignedObjectTransaction;

impl BlockchainEngine {
    /// Validate signatures, exact object references and ownership before a
    /// transaction is allowed into the object-centric mempool.
    pub fn validate_object_transaction_for_admission(
        &self,
        transaction: &SignedObjectTransaction,
    ) -> Result<()> {
        transaction.verify()?;

        let state = self.state.read().unwrap_or_else(|poisoned| {
            log::error!("State lock poisoned while validating object transaction; recovering...");
            poisoned.into_inner()
        });

        for object_arg in transaction.data.input_objects() {
            match object_arg {
                ObjectArg::ImmOrOwnedObject(reference) => {
                    let stored = Self::load_exact_object_ref(&state, reference)
                        .with_context(|| format!("Failed to resolve input object {}", reference.object_id))?;
                    ensure!(
                        stored.owner == transaction.data.sender,
                        "Sender {} does not own input object {}",
                        transaction.data.sender.to_hex_literal(),
                        reference.object_id
                    );
                }
                ObjectArg::Receiving(reference) => {
                    Self::load_exact_object_ref(&state, reference).with_context(|| {
                        format!("Failed to resolve receiving object {}", reference.object_id)
                    })?;
                }
                ObjectArg::SharedObject {
                    id,
                    initial_shared_version,
                    mutable: _,
                } => {
                    let stored = state
                        .get_object(&id.to_hex_literal())?
                        .ok_or_else(|| anyhow::anyhow!("Shared input object {} does not exist", id))?;
                    ensure!(
                        stored.version >= *initial_shared_version,
                        "Shared object {} has version {} lower than initial shared version {}",
                        id,
                        stored.version,
                        initial_shared_version
                    );
                }
            }
        }

        for gas_ref in &transaction.data.gas_data.payment {
            let stored = Self::load_exact_object_ref(&state, gas_ref)
                .with_context(|| format!("Failed to resolve gas object {}", gas_ref.object_id))?;
            ensure!(
                stored.owner == transaction.data.gas_data.owner,
                "Gas owner {} does not own gas object {}",
                transaction.data.gas_data.owner.to_hex_literal(),
                gas_ref.object_id
            );
            ensure!(
                stored.type_.contains("::coin::Coin<"),
                "Gas object {} is not a coin object",
                gas_ref.object_id
            );
        }

        Ok(())
    }

    /// Validate then reserve mutable object inputs in the provided object pool.
    ///
    /// The caller owns the pool so RPC, DAG production and tests can decide how
    /// to scope pending transactions without tying this path to the legacy
    /// account-centric mempool field.
    pub fn admit_object_transaction(
        &self,
        pool: &mut ObjectMempool,
        transaction: SignedObjectTransaction,
    ) -> Result<Vec<u8>> {
        self.validate_object_transaction_for_admission(&transaction)?;
        pool.admit_verified(transaction)
    }

    fn load_exact_object_ref(
        state: &kanari_move_runtime_v1::state::StateManager,
        expected: &ObjectRef,
    ) -> Result<kanari_move_runtime_v1::changeset::CreatedObject> {
        let stored = state
            .get_object(&expected.object_id.to_hex_literal())?
            .ok_or_else(|| anyhow::anyhow!("Input object {} does not exist", expected.object_id))?;

        let owner = Owner::AddressOwner(stored.owner);
        let digest = compute_object_digest(
            expected.object_id,
            stored.version,
            &owner,
            &stored.type_,
            &stored.data,
            None,
        )?;
        let actual = ObjectRef::new(expected.object_id, stored.version, digest);

        ensure!(
            actual == *expected,
            "Object reference mismatch for {}: expected version {} digest {}, found version {} digest {}",
            expected.object_id,
            expected.version,
            expected.digest,
            actual.version,
            actual.digest
        );

        Ok(stored)
    }
}
