// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::storage::move_vm_state::MoveVMState;
use crate::storage::object_storage::ObjectStore;
use anyhow::Result;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::{ModuleId, StructTag};
use move_core_types::resolver::{LinkageResolver, ModuleResolver, ResourceResolver};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

/// A resolver that fetches modules and resources directly from persistent storage.
/// This enables "Stateless" execution (Sui-like) without pre-loading everything into memory.
#[derive(Clone)]
pub struct KanariMoveResolver {
    pub(crate) state: MoveVMState,
    /// Keeps the backing object store alive for the lifetime of this resolver.
    pub(crate) object_storage: Arc<dyn ObjectStore>,
    read_trace: Option<Arc<Mutex<BTreeSet<Vec<u8>>>>>,
}

impl KanariMoveResolver {
    pub(crate) fn without_trace(state: MoveVMState, object_storage: Arc<dyn ObjectStore>) -> Self {
        Self {
            state,
            object_storage,
            read_trace: None,
        }
    }

    pub(crate) fn tracing_clone_with_overlay(
        &self,
        overlay: Option<crate::StateOverlay>,
    ) -> (Self, Arc<Mutex<BTreeSet<Vec<u8>>>>) {
        let trace = Arc::new(Mutex::new(BTreeSet::new()));
        (
            Self {
                state: self.state.with_overlay(overlay),
                object_storage: self.object_storage.clone(),
                read_trace: Some(trace.clone()),
            },
            trace,
        )
    }

    fn record_read(&self, key: Vec<u8>) {
        if let Some(trace) = &self.read_trace {
            match trace.lock() {
                Ok(mut reads) => {
                    reads.insert(key);
                }
                Err(poisoned) => {
                    poisoned.into_inner().insert(key);
                }
            }
        }
    }
}

impl LinkageResolver for KanariMoveResolver {
    type Error = anyhow::Error;
}

impl ModuleResolver for KanariMoveResolver {
    type Error = anyhow::Error;

    fn get_module(&self, id: &ModuleId) -> Result<Option<Vec<u8>>, Self::Error> {
        self.record_read(
            format!("module:{}:{}", id.address().to_hex_literal(), id.name()).into_bytes(),
        );
        // Fetch module directly from RocksDB via MoveVMState
        self.state.try_get_module(id)
    }
}

impl ResourceResolver for KanariMoveResolver {
    type Error = anyhow::Error;

    fn get_resource(
        &self,
        address: &AccountAddress,
        tag: &StructTag,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        self.record_read(format!("resource:{}:{}", address.to_hex_literal(), tag).into_bytes());
        if let Some(data) = self.state.try_get_resource(address, tag)? {
            return Ok(Some(data));
        }

        self.record_read(format!("object:{}", address.to_hex_literal()).into_bytes());
        if let Some(data) = self.state.try_get_object(address)? {
            log::debug!("[RESOLVER] Loaded Sui-style object at {}", address);
            return Ok(Some(data));
        }

        Ok(None)
    }
}
