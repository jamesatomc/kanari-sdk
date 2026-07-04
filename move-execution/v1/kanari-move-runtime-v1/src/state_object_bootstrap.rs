// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Object-centric state bootstrap without account genesis execution.

use crate::state::StateManager;
use crate::storage::persistent_store::PersistentStore;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::sync::Arc;

impl StateManager {
    /// Open canonical object state directly. Framework packages and genesis
    /// objects are installed through object effects, not account resources.
    pub fn try_new_object_store(store: Arc<PersistentStore>) -> Result<Self> {
        let total_supply = store
            .load::<u64>(b"total_supply")
            .context("Failed to load object total supply")?
            .unwrap_or_default();
        let global_token_supplies = store
            .load::<BTreeMap<String, u64>>(b"global_token_supplies")
            .context("Failed to load object token supplies")?
            .unwrap_or_default();
        let smt = store
            .get_db()
            .map(|db| Arc::new(smt::SparseMerkleTree::new(db)));

        Ok(Self {
            store,
            overlay: BTreeMap::new(),
            total_supply,
            global_token_supplies,
            smt,
            events: Vec::new(),
        })
    }

    pub fn new_object_store_in_memory() -> Result<Self> {
        let store = Arc::new(
            PersistentStore::open_in_memory().context("Failed to create object state store")?,
        );
        Self::try_new_object_store(store)
    }
}
