// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::state::default_owner_kind_for_type;
use crate::storage::resolver::KanariMoveResolver;
use anyhow::{Context, Result, ensure};
use kanari_crypto::hash_data_blake3;
use kanari_system_natives::dynamic_field::DynamicFieldsExt;
use kanari_system_natives::event::EventsExt;
use kanari_system_natives::object::{
    BorrowedObjectsExt, DeletedObjectsExt, LoadedObjectsExt, SavedObjectsExt,
};
use kanari_system_natives::transfer_natives::TransferredObjectsExt;
use kanari_types::clock::{Clock, ClockModule};

use kanari_types::error::KanariUnwrapExt;
use kanari_types::event::Event;
use log::debug;
use move_binary_format::file_format::CompiledModule;
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::{IdentStr, Identifier};
use move_core_types::language_storage::{ModuleId, StructTag, TypeTag};
use move_core_types::runtime_value::{MoveStruct, MoveTypeLayout, MoveValue};
use move_vm_runtime::move_vm::MoveVM;
use move_vm_runtime::native_extensions::NativeContextExtensions;
use move_vm_runtime::native_functions::NativeFunctionTable;
use move_vm_runtime::session::Session;
use sha3::{Digest, Sha3_256};
mod gas_ops;
mod helpers;
pub mod load_system_modules;
mod object_ops;
mod parsers;
use kanari_types::address::Address as KanariAddress;
use kanari_types::gas::GasOperation;
use kanari_types::object::UIDRecord;
use kanari_types::transaction::{ObjectInput, ObjectOwnerKind, ObjectRef};
use kanari_types::tx_context::TxContextModule;
mod move_runtime_extensions;
use crate::changeset::ChangeSet;
use crate::state::StateManager;
use crate::storage::move_vm_state::MoveVMState;
use crate::storage::object_storage::{ObjectStorage, ObjectStore, StoredObject};
use crate::storage::persistent_store::PersistentStore;
use move_binary_format::compatibility::Compatibility;
use move_binary_format::normalized;
use move_bytecode_verifier::verifier::verify_module_unmetered;
use move_vm_types::loaded_data::runtime_types::Type as RuntimeType;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

#[derive(Clone)]
pub struct MoveRuntime {
    pub(crate) vm: Arc<RwLock<MoveVM>>,
    pub(crate) all_natives: Arc<NativeFunctionTable>,
    pub(crate) resolver: KanariMoveResolver,
    pub(crate) state: MoveVMState,
    pub(crate) published_modules: Arc<RwLock<HashSet<ModuleId>>>,
    pub(crate) object_storage: Arc<dyn ObjectStore>,
    // Cache parsed type tags to avoid repeated string parsing.
    pub(crate) type_tag_cache: Arc<RwLock<HashMap<String, TypeTag>>>,
    // Serialize module publishes so compatibility checks and storage writes cannot race.
    pub(crate) module_publish_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModulePublishIntent {
    Publish,
    Upgrade,
    Bootstrap,
}

type LoadedMutableObject = (
    usize,
    String,
    AccountAddress,
    kanari_types::transaction::ObjectOwnerKind,
    String,
    u64,
);

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObjectParamBindingRequirement {
    param_index: usize,
    mutable: bool,
}

#[derive(Clone)]
pub struct EntryFunctionObjectContext {
    pub object_inputs: Vec<ObjectInput>,
    pub sender: Option<AccountAddress>,
    pub gas_info: Option<(u64, u64)>,
    pub timestamp: Option<u64>,
    pub tx_hash: Option<Vec<u8>>,
    pub persist_runtime_state: bool,
    pub state_overlay: Option<crate::StateOverlay>,
}

#[derive(Clone)]
struct ExecutionOptions {
    sender: Option<AccountAddress>,
    gas_info: Option<(u64, u64)>,
    timestamp: Option<u64>,
    tx_hash: Option<Vec<u8>>,
    object_inputs: Vec<ObjectInput>,
    persist_runtime_state: bool,
    bypass_entry_check: bool,
    state_overlay: Option<crate::StateOverlay>,
}

impl ExecutionOptions {
    fn new(
        sender: Option<AccountAddress>,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
    ) -> Self {
        Self {
            sender,
            gas_info,
            timestamp,
            tx_hash,
            object_inputs: Vec::new(),
            persist_runtime_state: true,
            bypass_entry_check: false,
            state_overlay: None,
        }
    }

    fn with_object_inputs(mut self, object_inputs: Vec<ObjectInput>) -> Self {
        self.object_inputs = object_inputs;
        self
    }

    fn with_persistence(mut self, persist_runtime_state: bool) -> Self {
        self.persist_runtime_state = persist_runtime_state;
        self
    }

    fn with_state_overlay(mut self, state_overlay: Option<crate::StateOverlay>) -> Self {
        self.state_overlay = state_overlay;
        self
    }

    fn bypass_entry_check(mut self) -> Self {
        self.bypass_entry_check = true;
        self
    }
}

impl MoveRuntime {
    fn is_tx_context_struct(struct_tag: &StructTag) -> bool {
        let sys_addr = KanariAddress::kanari_system_account_address();
        struct_tag.address == sys_addr
            && struct_tag.module.as_str() == TxContextModule::TX_CONTEXT_MODULE
            && struct_tag.name.as_str() == TxContextModule::TX_CONTEXT_STRUCT
    }

    fn object_param_mutability<F>(param_type: &RuntimeType, is_key_struct: F) -> Option<bool>
    where
        F: Fn(&RuntimeType) -> bool,
    {
        match param_type {
            // Generic object types such as Coin<T> can lose their key-ability
            // information while resolving a published package's dependency
            // graph. They are still object references when passed by ref.
            RuntimeType::Reference(inner)
                if is_key_struct(inner)
                    || matches!(&**inner, RuntimeType::StructInstantiation(_)) =>
            {
                Some(false)
            }
            RuntimeType::MutableReference(inner)
                if is_key_struct(inner)
                    || matches!(&**inner, RuntimeType::StructInstantiation(_)) =>
            {
                Some(true)
            }
            RuntimeType::Struct(_) | RuntimeType::StructInstantiation(_)
                if is_key_struct(param_type) =>
            {
                Some(true)
            }
            _ => None,
        }
    }

    fn is_runtime_struct_like(ty: &RuntimeType) -> bool {
        matches!(
            ty,
            RuntimeType::Struct(_) | RuntimeType::StructInstantiation(_)
        )
    }

    fn object_digest(data: &[u8]) -> String {
        format!("0x{}", hex::encode(hash_data_blake3(data)))
    }

    fn load_object_by_ref_checked(&self, object_ref: &ObjectRef) -> Result<StoredObject> {
        let stored_obj = self
            .object_storage
            .get_object(&object_ref.object_id)?
            .with_context(|| format!("Object input {} was not found", object_ref.object_id))?;
        if let Some(version) = object_ref.version {
            ensure!(
                stored_obj.version == version,
                "Object input {} version mismatch: expected {}, found {}",
                object_ref.object_id,
                version,
                stored_obj.version
            );
        }
        if let Some(expected_digest) = &object_ref.digest {
            let actual_digest = Self::object_digest(&stored_obj.data);
            ensure!(
                actual_digest == *expected_digest,
                "Object input {} digest mismatch: expected {}, found {}",
                object_ref.object_id,
                expected_digest,
                actual_digest
            );
        }
        Ok(stored_obj)
    }

    fn is_tx_context_param<F>(param_type: &RuntimeType, type_tag_for_param: F) -> bool
    where
        F: Fn(&RuntimeType) -> Option<TypeTag>,
    {
        matches!(
            type_tag_for_param(param_type),
            Some(TypeTag::Struct(struct_tag)) if Self::is_tx_context_struct(&struct_tag)
        )
    }

    #[cfg(test)]
    fn validate_declared_object_input_bindings(
        object_inputs: &[ObjectInput],
        requirements: &[ObjectParamBindingRequirement],
    ) -> Result<()> {
        Self::validate_object_input_bindings(object_inputs, requirements, false)
    }

    fn validate_object_input_bindings(
        object_inputs: &[ObjectInput],
        requirements: &[ObjectParamBindingRequirement],
        allow_extra_dependency_inputs: bool,
    ) -> Result<()> {
        if object_inputs.len() < requirements.len()
            || (!allow_extra_dependency_inputs && object_inputs.len() != requirements.len())
        {
            anyhow::bail!(
                "Declared object_inputs count mismatch: function expects {} object params, got {}",
                requirements.len(),
                object_inputs.len()
            );
        }

        for (input, requirement) in object_inputs.iter().zip(requirements.iter()) {
            ensure!(
                !requirement.mutable || input.mutable,
                "Object input {} mutability does not match function parameter {}",
                input.object_ref.object_id,
                requirement.param_index
            );
            ensure!(
                input.owner.is_some(),
                "Object input {} must declare owner semantics for function parameter {}",
                input.object_ref.object_id,
                requirement.param_index
            );
            if requirement.mutable {
                ensure!(
                    !matches!(input.owner, Some(ObjectOwnerKind::Immutable)),
                    "Immutable object input {} cannot bind to mutable function parameter {}",
                    input.object_ref.object_id,
                    requirement.param_index
                );
            }
        }

        Ok(())
    }

    fn read_vm(&self) -> RwLockReadGuard<'_, MoveVM> {
        self.vm
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_vm(&self) -> RwLockWriteGuard<'_, MoveVM> {
        self.vm
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn new_with_natives(natives: Vec<NativeFunctionTable>) -> Result<Self> {
        let state = if cfg!(miri) {
            MoveVMState::new_in_memory()?
        } else {
            MoveVMState::open_default()?
        };
        Self::new_internal(natives, state, None)
    }

    pub fn new_with_natives_in_memory(natives: Vec<NativeFunctionTable>) -> Result<Self> {
        let state = MoveVMState::new_in_memory()?;
        Self::new_internal(natives, state, None)
    }

    pub(crate) fn new_with_natives_and_store(
        natives: Vec<NativeFunctionTable>,
        store: Arc<PersistentStore>,
    ) -> Result<Self> {
        let state = MoveVMState::new(store.clone());
        Self::new_internal(natives, state, Some(store))
    }

    fn new_internal(
        natives: Vec<NativeFunctionTable>,
        state: MoveVMState,
        shared_store: Option<Arc<PersistentStore>>,
    ) -> Result<Self> {
        let all_natives: NativeFunctionTable = natives
            .into_iter()
            .flat_map(|table| table.into_iter())
            .collect();

        let vm = MoveVM::new(all_natives.clone()).require("VM init error")?;

        let object_storage: Arc<dyn ObjectStore> = match shared_store {
            Some(store) if !cfg!(miri) => match ObjectStorage::boxed_with_store(store) {
                Ok(store) => Arc::from(store),
                Err(e) => {
                    log::warn!("[RUNTIME] shared object store load failed: {}", e);
                    Arc::from(ObjectStorage::boxed_inmemory())
                }
            },
            _ if cfg!(miri) => Arc::from(ObjectStorage::boxed_inmemory()),
            _ => match ObjectStorage::boxed_with_store(state.store()) {
                Ok(store) => Arc::from(store),
                Err(e) => {
                    log::warn!("[RUNTIME] shared object store load failed: {}", e);
                    Arc::from(ObjectStorage::boxed_inmemory())
                }
            },
        };
        let resolver = KanariMoveResolver::without_trace(state.clone(), object_storage.clone());

        let published_modules: HashSet<ModuleId> = state
            .get_all_module_ids()
            .context("Failed to load persistent Move module index")?
            .into_iter()
            .collect();

        Ok(MoveRuntime {
            vm: Arc::new(RwLock::new(vm)),
            all_natives: Arc::new(all_natives),
            resolver,
            state,
            published_modules: Arc::new(RwLock::new(published_modules)),
            object_storage,
            type_tag_cache: Arc::new(RwLock::new(HashMap::new())),
            module_publish_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn new_with_kanari_natives() -> Result<Self> {
        let natives = Self::get_kanari_natives_list();
        let runtime = Self::new_with_natives(natives)?;
        if !cfg!(miri) {
            runtime.load_system_modules()?;
        }
        Ok(runtime)
    }

    pub fn new_with_kanari_natives_in_memory() -> Result<Self> {
        let natives = Self::get_kanari_natives_list();
        let runtime = Self::new_with_natives_in_memory(natives)?;
        runtime.load_system_modules()?;
        Ok(runtime)
    }

    pub fn new_with_kanari_natives_and_store(store: Arc<PersistentStore>) -> Result<Self> {
        let natives = Self::get_kanari_natives_list();
        let runtime = Self::new_with_natives_and_store(natives, store)?;
        runtime.load_system_modules()?;
        Ok(runtime)
    }

    fn get_kanari_natives_list() -> Vec<NativeFunctionTable> {
        let sys_addr = KanariAddress::kanari_system_account_address();
        vec![
            move_stdlib_natives::all_natives(
                KanariAddress::std_account_address(),
                move_stdlib_natives::GasParameters::zeros(),
            ),
            kanari_system_natives::all_natives(
                sys_addr,
                kanari_system_natives::GasParameters::zeros(),
            ),
        ]
    }

    pub fn spawn_worker(&self) -> Result<Self> {
        let vm = MoveVM::new(self.all_natives.as_ref().clone()).require("Worker VM init error")?;

        Ok(MoveRuntime {
            vm: Arc::new(RwLock::new(vm)),
            all_natives: self.all_natives.clone(),
            resolver: self.resolver.clone(),
            state: self.state.clone(),
            published_modules: self.published_modules.clone(),
            object_storage: self.object_storage.clone(),
            type_tag_cache: self.type_tag_cache.clone(),
            module_publish_lock: self.module_publish_lock.clone(),
        })
    }

    pub fn spawn_isolated_worker(&self) -> Result<Self> {
        let vm = MoveVM::new(self.all_natives.as_ref().clone())
            .require("Isolated worker VM init error")?;

        let object_storage: Arc<dyn ObjectStore> =
            match ObjectStorage::boxed_with_store(self.state.store()) {
                Ok(store) => Arc::from(store),
                Err(e) => {
                    log::warn!("[RUNTIME] isolated object store load failed: {}", e);
                    Arc::from(ObjectStorage::boxed_inmemory())
                }
            };

        let resolver =
            KanariMoveResolver::without_trace(self.state.clone(), object_storage.clone());

        let published_modules = self
            .published_modules
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone();

        let runtime = MoveRuntime {
            vm: Arc::new(RwLock::new(vm)),
            all_natives: self.all_natives.clone(),
            resolver,
            state: self.state.clone(),
            published_modules: Arc::new(RwLock::new(published_modules)),
            object_storage,
            type_tag_cache: Arc::new(RwLock::new(HashMap::new())),
            module_publish_lock: Arc::new(Mutex::new(())),
        };

        runtime.preload_system_modules_into_vm()?;
        Ok(runtime)
    }

    /// Rebuild the VM instance so cached module state is refreshed.
    pub fn reload_vm_cache(&self) -> Result<()> {
        let new_vm =
            MoveVM::new(self.all_natives.as_ref().clone()).require("Failed to reload MoveVM")?;

        // Replace the VM instance to clear internal caches.
        *self.write_vm() = new_vm;

        // Preload published modules so follow-up executions can resolve dependencies immediately.
        self.preload_system_modules_into_vm()?;

        log::debug!("[RUNTIME] MoveVM cache cleared and reloaded");
        Ok(())
    }

    /// Refresh module identity and VM caches after canonical state has committed.
    pub fn refresh_committed_modules(&self) -> Result<()> {
        let module_ids = self.state.get_all_module_ids()?;
        *self
            .published_modules
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = module_ids.into_iter().collect();
        self.reload_vm_cache()
    }

    pub fn clear_object_cache(&self) -> Result<()> {
        self.object_storage
            .clear()
            .require("Failed to clear runtime object cache")
    }

    /// Preload system modules into the VM cache to ensure dependencies are available
    fn preload_system_modules_into_vm(&self) -> Result<()> {
        let vm_guard = self.read_vm();
        let session = self.create_session_with_storage_ext(&vm_guard);

        // Read the published module set from the runtime index.
        let module_ids: Vec<ModuleId> = self
            .published_modules
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned()
            .collect();

        // Deserialize published modules so the fresh VM repopulates its caches.
        for module_id in module_ids {
            if let Some(module_bytes) = self.state.try_get_module(&module_id)?
                && CompiledModule::deserialize_with_defaults(&module_bytes).is_ok()
            {
                log::debug!("[RUNTIME] Preloaded module into cache: {}", module_id);
            }
        }

        drop(session);
        drop(vm_guard);

        Ok(())
    }

    pub(crate) fn apply_move_changeset(
        &self,
        move_cs: move_core_types::effects::ChangeSet,
    ) -> Result<()> {
        let store = self.state.store();
        let transaction_guard = store.transaction_guard();
        let mut updates = Vec::<(Vec<u8>, Vec<u8>)>::new();
        let mut deletes = Vec::<Vec<u8>>::new();
        let mut module_index = store
            .load::<Vec<String>>(b"module_index")?
            .unwrap_or_default()
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        let mut published_module_updates = Vec::new();
        let mut published_module_deletes = Vec::new();

        for (addr, account_changes) in move_cs.accounts() {
            for (module_name, op) in account_changes.modules() {
                let module_id = ModuleId::new(*addr, Identifier::new(module_name.as_str())?);
                let key = format!(
                    "module:{}:{}",
                    module_id.address().to_hex_literal(),
                    module_id.name()
                );
                match op {
                    move_core_types::effects::Op::New(bytes)
                    | move_core_types::effects::Op::Modify(bytes) => {
                        updates.push((key.as_bytes().to_vec(), bcs::to_bytes(bytes)?));
                        module_index.insert(key);
                        published_module_updates.push(module_id);
                    }
                    move_core_types::effects::Op::Delete => {
                        deletes.push(key.as_bytes().to_vec());
                        module_index.remove(&key);
                        published_module_deletes.push(module_id);
                    }
                }
            }
            for (struct_tag, op) in account_changes.resources() {
                let key = format!("resource:{}:{}", addr.to_hex_literal(), struct_tag);
                match op {
                    move_core_types::effects::Op::New(bytes)
                    | move_core_types::effects::Op::Modify(bytes) => {
                        updates.push((key.as_bytes().to_vec(), bcs::to_bytes(bytes)?));

                        if struct_tag.module.as_str() == "coin"
                            && struct_tag.name.as_str() == "Coin"
                        {
                            let object_key = format!("object:{}", addr.to_hex_literal());
                            if let Some(mut object) =
                                store.load::<StoredObject>(object_key.as_bytes())?
                            {
                                object.data = bytes.to_vec();
                                updates.push((
                                    object_key.as_bytes().to_vec(),
                                    bcs::to_bytes(&object)?,
                                ));
                            }
                        }
                    }
                    move_core_types::effects::Op::Delete => {
                        deletes.push(key.into_bytes());
                    }
                }
            }
        }

        updates.push((
            b"module_index".to_vec(),
            bcs::to_bytes(&module_index.into_iter().collect::<Vec<_>>())?,
        ));
        store.apply_raw_changes(&updates, &deletes)?;

        let mut modules = self
            .published_modules
            .write()
            .unwrap_or_else(|p| p.into_inner());
        modules.extend(published_module_updates);
        for module_id in published_module_deletes {
            modules.remove(&module_id);
        }
        drop(transaction_guard);
        Ok(())
    }

    fn load_system_modules(&self) -> Result<()> {
        self.load_move_stdlib()?;
        self.load_kanari_system()?;
        Ok(())
    }

    pub fn upgrade_module(
        &self,
        module_bytes: Vec<u8>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
    ) -> Result<ChangeSet> {
        self.upgrade_module_with_context_and_persistence(
            module_bytes,
            sender,
            gas_info,
            timestamp,
            None,
            true,
        )
    }

    pub fn publish_module(
        &self,
        module_bytes: Vec<u8>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
    ) -> Result<ChangeSet> {
        self.publish_module_with_context_and_persistence(
            module_bytes,
            sender,
            gas_info,
            timestamp,
            None,
            true,
        )
    }

    pub fn bootstrap_module_with_context_and_persistence(
        &self,
        module_bytes: Vec<u8>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        self.publish_module_with_intent_and_persistence(
            module_bytes,
            sender,
            gas_info,
            timestamp,
            tx_hash,
            persist_runtime_state,
            ModulePublishIntent::Bootstrap,
        )
    }

    pub fn upgrade_module_with_context_and_persistence(
        &self,
        module_bytes: Vec<u8>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        self.publish_module_with_intent_and_persistence(
            module_bytes,
            sender,
            gas_info,
            timestamp,
            tx_hash,
            persist_runtime_state,
            ModulePublishIntent::Upgrade,
        )
    }

    pub fn publish_module_with_context_and_persistence(
        &self,
        module_bytes: Vec<u8>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        self.publish_module_with_intent_and_persistence(
            module_bytes,
            sender,
            gas_info,
            timestamp,
            tx_hash,
            persist_runtime_state,
            ModulePublishIntent::Publish,
        )
    }

    fn publish_module_with_intent_and_persistence(
        &self,
        module_bytes: Vec<u8>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
        persist_runtime_state: bool,
        intent: ModulePublishIntent,
    ) -> Result<ChangeSet> {
        // The caller provides these context fields for API compatibility. The
        // bytecode publish path has no TxContext and must not fabricate them.
        let _ = (timestamp, tx_hash);
        let publish_guard = self
            .module_publish_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let compiled = CompiledModule::deserialize_with_defaults(&module_bytes)?;
        let module_id = compiled.self_id();
        self.verify_module_publish_safety(sender, &module_id, &compiled, &module_bytes, intent)?;

        let (move_changeset, events, vm_gas_used) = {
            // 🟢 Separate Lock into a variable first to prevent it from being dropped immediately
            let vm_guard = self.read_vm();
            let mut session = self.create_session_with_storage_ext(&vm_guard);

            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);

            session
                .publish_module(module_bytes.clone(), sender, &mut metered_gas)
                .require("Move VM operation failed")?;

            let result = session
                .finish()
                .0
                .require("Failed to finish publish session")?;
            (result.0, result.1, metered_gas.gas_used())
        };

        let mut cs = ChangeSet::new();
        cs.publish_module(sender, module_id.name().to_string());
        self.parse_move_changeset(&move_changeset, &mut cs);
        self.parse_move_events(&events, &mut cs);

        if let Some((gas_limit, gas_price)) = gas_info {
            let gas_op = GasOperation::PublishModule {
                module_size: module_bytes.len(),
            };
            let (written, deleted) = self.calculate_storage_impact(&move_changeset, &cs, None)?;
            self.apply_gas_info(&mut cs, gas_limit, gas_price, gas_op, written, deleted)?;
        }
        cs.set_gas_used(cs.gas_used.max(vm_gas_used));

        if persist_runtime_state {
            self.apply_move_changeset(move_changeset)?;
            self.reload_vm_cache()?;
        }

        drop(publish_guard);
        Ok(cs)
    }

    pub fn publish_package_with_context_and_persistence(
        &self,
        modules: Vec<(String, Vec<u8>)>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        self.publish_package_with_intent_and_persistence(
            modules,
            sender,
            gas_info,
            timestamp,
            tx_hash,
            persist_runtime_state,
            ModulePublishIntent::Publish,
        )
    }

    pub fn upgrade_package_with_context_and_persistence(
        &self,
        modules: Vec<(String, Vec<u8>)>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        self.publish_package_with_intent_and_persistence(
            modules,
            sender,
            gas_info,
            timestamp,
            tx_hash,
            persist_runtime_state,
            ModulePublishIntent::Upgrade,
        )
    }

    fn publish_package_with_intent_and_persistence(
        &self,
        modules: Vec<(String, Vec<u8>)>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
        persist_runtime_state: bool,
        intent: ModulePublishIntent,
    ) -> Result<ChangeSet> {
        // Package publishing shares the same context rule as single modules.
        let _ = (timestamp, tx_hash);
        let publish_guard = self
            .module_publish_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());

        let mut compiled_modules = Vec::with_capacity(modules.len());
        let mut total_module_size = 0usize;
        for (module_name, module_bytes) in &modules {
            let compiled = CompiledModule::deserialize_with_defaults(module_bytes)?;
            let module_id = compiled.self_id();
            if module_id.name().as_str() != module_name {
                anyhow::bail!(
                    "Module publish rejected: declared module name {} does not match bytecode self id {}",
                    module_name,
                    module_id.name()
                );
            }
            self.verify_module_publish_safety(sender, &module_id, &compiled, module_bytes, intent)?;
            total_module_size = total_module_size.saturating_add(module_bytes.len());
            compiled_modules.push((module_id, module_bytes.clone()));
        }

        let (move_changeset, events, vm_gas_used) = {
            let vm_guard = self.read_vm();
            let mut session = self.create_session_with_storage_ext(&vm_guard);
            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);

            for (_, module_bytes) in &compiled_modules {
                session
                    .publish_module(module_bytes.clone(), sender, &mut metered_gas)
                    .require("Move VM package publish failed")?;
            }

            let result = session
                .finish()
                .0
                .require("Failed to finish publish package session")?;
            (result.0, result.1, metered_gas.gas_used())
        };

        let mut cs = ChangeSet::new();
        for (module_id, _) in &compiled_modules {
            cs.publish_module(sender, module_id.name().to_string());
        }
        self.parse_move_changeset(&move_changeset, &mut cs);
        self.parse_move_events(&events, &mut cs);

        if let Some((gas_limit, gas_price)) = gas_info {
            let gas_op = GasOperation::PublishModule {
                module_size: total_module_size,
            };
            let (written, deleted) = self.calculate_storage_impact(&move_changeset, &cs, None)?;
            self.apply_gas_info(&mut cs, gas_limit, gas_price, gas_op, written, deleted)?;
        }
        cs.set_gas_used(cs.gas_used.max(vm_gas_used));

        if persist_runtime_state {
            self.apply_move_changeset(move_changeset)?;
            self.reload_vm_cache()?;
        }

        drop(publish_guard);
        Ok(cs)
    }

    fn verify_module_publish_safety(
        &self,
        sender: AccountAddress,
        module_id: &ModuleId,
        compiled: &CompiledModule,
        module_bytes: &[u8],
        intent: ModulePublishIntent,
    ) -> Result<()> {
        if module_id.address() != &sender {
            anyhow::bail!(
                "Module publish rejected: sender {} cannot publish module {}. \
                 Module address must match the transaction sender.",
                sender,
                module_id
            );
        }

        let old_bytes = self.verify_module_publish_intent(module_id, intent)?;
        verify_module_unmetered(compiled).require("Module bytecode verification failed")?;
        self.verify_module(compiled)?;
        if let Some(old_bytes) = old_bytes {
            self.verify_module_upgrade_compatibility(
                module_id,
                compiled,
                module_bytes,
                &old_bytes,
            )?;
        }
        Ok(())
    }

    fn verify_module_publish_intent(
        &self,
        module_id: &ModuleId,
        intent: ModulePublishIntent,
    ) -> Result<Option<Vec<u8>>> {
        let old_bytes = self.state.try_get_module(module_id)?;
        match (intent, old_bytes) {
            (ModulePublishIntent::Bootstrap, old_bytes) => Ok(old_bytes),
            (ModulePublishIntent::Publish, Some(_)) => {
                anyhow::bail!(
                    "Module publish rejected: module {} already exists. Use upgrade for compatibility-checked replacement.",
                    module_id
                );
            }
            (ModulePublishIntent::Publish, None) => Ok(None),
            (ModulePublishIntent::Upgrade, None) => {
                anyhow::bail!(
                    "Module upgrade rejected: module {} does not exist. Publish the package before upgrading it.",
                    module_id
                );
            }
            (ModulePublishIntent::Upgrade, Some(old_bytes)) => Ok(Some(old_bytes)),
        }
    }

    fn verify_module_upgrade_compatibility(
        &self,
        module_id: &ModuleId,
        compiled: &CompiledModule,
        module_bytes: &[u8],
        old_bytes: &[u8],
    ) -> Result<()> {
        if old_bytes == module_bytes {
            return Ok(());
        }

        let old_compiled = CompiledModule::deserialize_with_defaults(old_bytes)?;
        let old_norm = normalized::Module::new(&old_compiled);
        let new_norm = normalized::Module::new(compiled);

        Compatibility::full_check()
            .check(&old_norm, &new_norm)
            .map_err(|e| {
                anyhow::anyhow!(
                    "Incompatible module upgrade for {} rejected: {:?}. \
                     Existing resources/objects must remain layout-compatible; publish a compatible module or run an explicit migration path first.",
                    module_id,
                    e
                )
            })
    }
    pub(crate) fn execute_init_function_with_context(
        &self,
        module_addr: AccountAddress,
        module_name: &str,
        args: Vec<Vec<u8>>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
    ) -> Result<ChangeSet> {
        self.execute_init_function_internal(module_addr, module_name, args, timestamp, tx_hash)
            .require("Failed to execute init()")
    }
    fn execute_init_function_internal(
        &self,
        module_addr: AccountAddress,
        module_name: &str,
        args: Vec<Vec<u8>>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
    ) -> Result<ChangeSet> {
        let module_id = ModuleId::new(module_addr, Identifier::new(module_name)?);
        self.execute_entry_function_internal(
            &module_id,
            "init",
            vec![],
            args,
            ExecutionOptions::new(Some(module_addr), None, timestamp, tx_hash).bypass_entry_check(),
        )
    }

    fn preprocess_entry_args(args: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        args.into_iter()
            .map(|arg| {
                // Only perform preprocessing for string-like inputs that are meant to be converted
                // Skip preprocessing if the arg is already a properly serialized BCS value

                // Check if this looks like a potential address string (hex string)
                if let Ok(s) = std::str::from_utf8(&arg) {
                    let s_trim = s.trim();
                    if s_trim.starts_with("0x") || s_trim.chars().all(|c| c.is_ascii_hexdigit()) {
                        // This looks like a hex string that should be converted to bytes
                        let clean_hex = s_trim.strip_prefix("0x").unwrap_or(s_trim);
                        if let Ok(bytes) = hex::decode(clean_hex)
                            && bytes.len() == AccountAddress::LENGTH
                        {
                            // This is a valid address hex string
                            return bytes;
                        }
                    } else if s_trim.parse::<u64>().is_ok() {
                        // This looks like a number string
                        if let Ok(n) = s_trim.parse::<u64>() {
                            return bcs::to_bytes(&n).unwrap_or(arg);
                        }
                    }
                }

                // For all other cases, return the original arg without modification
                // This prevents corrupting already-serialized BCS arguments
                arg
            })
            .collect()
    }

    fn build_tx_context_bytes(
        &self,
        sender: Option<AccountAddress>,
        timestamp: Option<u64>,
        tx_hash: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let sender_addr = sender.unwrap_or(AccountAddress::ZERO);
        let epoch_timestamp_ms = timestamp.unwrap_or(0);
        let tx_hash = if let Some(raw_tx_hash) = tx_hash {
            if raw_tx_hash.len() == 32 {
                raw_tx_hash.to_vec()
            } else {
                hash_data_blake3(raw_tx_hash).to_vec()
            }
        } else {
            let mut input = Vec::new();
            input.extend_from_slice(b"kanari-txctx-v1");
            input.extend_from_slice(sender_addr.as_ref());
            input.extend_from_slice(&epoch_timestamp_ms.to_le_bytes());
            hash_data_blake3(&input).to_vec()
        };
        let move_value = MoveValue::Struct(MoveStruct(vec![
            MoveValue::Address(sender_addr),
            MoveValue::vector_u8(tx_hash),
            MoveValue::U64(0),
            MoveValue::U64(epoch_timestamp_ms),
            MoveValue::U64(0),
        ]));
        move_value
            .simple_serialize()
            .require("Failed to serialize TxContext MoveValue")
    }

    fn synthesize_otw_bytes_from_layout(layout: &MoveTypeLayout) -> Option<Vec<u8>> {
        let move_value = match layout {
            MoveTypeLayout::Struct(struct_layout) if struct_layout.0.is_empty() => {
                MoveValue::Struct(MoveStruct(vec![]))
            }
            MoveTypeLayout::Struct(struct_layout)
                if struct_layout.0.len() == 1
                    && matches!(struct_layout.0.first(), Some(MoveTypeLayout::Bool)) =>
            {
                MoveValue::Struct(MoveStruct(vec![MoveValue::Bool(true)]))
            }
            _ => return None,
        };

        move_value.simple_serialize()
    }

    /// Convert an object-id string into a `UIDRecord` when possible.
    fn uid_from_object_id(object_id: &str) -> Option<kanari_types::object::UIDRecord> {
        AccountAddress::from_hex_literal(object_id)
            .ok()
            .map(kanari_types::object::UIDRecord::new)
    }

    /// Convert an object-id string into an `IDRecord` when possible.
    fn id_from_object_id(object_id: &str) -> Option<kanari_types::object::IDRecord> {
        AccountAddress::from_hex_literal(object_id)
            .ok()
            .map(kanari_types::object::IDRecord::new)
    }

    fn maybe_add_token_balance(
        &self,
        cs: &mut ChangeSet,
        owner: AccountAddress,
        type_name: &str,
        data: &[u8],
        object_id: &str,
        source: &str,
    ) {
        if let Ok(struct_tag) = type_name.parse::<move_core_types::language_storage::StructTag>()
            && self.is_balance_resource(&struct_tag)
            && let Some(amount) = self.extract_balance_from_bytes(data, &struct_tag)
            && let Some(token_type) = self.token_type_from_struct_tag(&struct_tag)
        {
            cs.add_token_balance_set(owner, token_type.clone(), amount);
            debug!(
                "[RUNTIME] Extracted balance from {} object {}: {} = {}",
                source, object_id, token_type, amount
            );
        }
    }

    fn resolve_saved_owner_metadata(
        &self,
        loaded_mutable_objects: &[LoadedMutableObject],
        object_id: &str,
    ) -> anyhow::Result<(
        AccountAddress,
        kanari_types::transaction::ObjectOwnerKind,
        u64,
    )> {
        if let Some((_, _, owner, owner_kind, _, version)) = loaded_mutable_objects
            .iter()
            .find(|(_, id, _, _, _, _)| id == object_id)
        {
            return Ok((*owner, owner_kind.clone(), *version + 1));
        }
        if let Some(stored) = self.object_storage.get_object(object_id)? {
            return Ok((stored.owner, stored.owner_kind, stored.version + 1));
        }
        Ok((
            AccountAddress::ZERO,
            kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                AccountAddress::ZERO.to_hex_literal(),
            ),
            1,
        ))
    }

    fn build_created_object(
        owner: AccountAddress,
        owner_kind: kanari_types::transaction::ObjectOwnerKind,
        object_id: &str,
        type_name: &str,
        data: Vec<u8>,
        version: u64,
    ) -> crate::changeset::CreatedObject {
        crate::changeset::CreatedObject {
            owner,
            owner_kind,
            uid: Self::uid_from_object_id(object_id),
            id: Self::id_from_object_id(object_id),
            type_: type_name.to_string(),
            data,
            version,
        }
    }

    fn upsert_created_object(
        &self,
        cs: &mut ChangeSet,
        owner: AccountAddress,
        owner_kind: kanari_types::transaction::ObjectOwnerKind,
        object_id: &str,
        type_name: &str,
        data: Vec<u8>,
        version: u64,
        source: &str,
    ) {
        self.maybe_add_token_balance(cs, owner, type_name, &data, object_id, source);
        let updated_obj =
            Self::build_created_object(owner, owner_kind, object_id, type_name, data, version);
        cs.created_objects.retain(|(k, _)| k != object_id);
        cs.created_objects
            .push((object_id.to_string(), updated_obj));
    }

    pub fn persist_created_objects(&self, cs: &ChangeSet) -> Result<()> {
        for (id, created) in &cs.created_objects {
            let stored = StoredObject {
                id: id.clone(),
                owner: created.owner,
                owner_kind: created.owner_kind.clone(),
                type_name: created.type_.clone(),
                data: created.data.clone(),
                version: created.version,
            };
            self.object_storage
                .store_object(stored)
                .map_err(anyhow::Error::new)?;
        }
        Ok(())
    }

    pub fn persist_deleted_objects(&self, cs: &ChangeSet) -> Result<()> {
        for obj_id in &cs.deleted_objects {
            self.object_storage
                .delete_object(obj_id)
                .map_err(anyhow::Error::new)?;
        }
        Ok(())
    }

    pub fn preload_object_snapshot(
        &self,
        object_id: &str,
        owner: AccountAddress,
        type_name: &str,
        data: Vec<u8>,
        version: u64,
    ) -> Result<()> {
        let stored = StoredObject {
            id: object_id.to_string(),
            owner,
            owner_kind: default_owner_kind_for_type(type_name, owner),
            type_name: type_name.to_string(),
            data,
            version,
        };
        self.object_storage
            .store_object(stored)
            .require("Object storage operation failed")
    }

    pub fn ensure_system_clock(&self, state: &mut StateManager) -> Result<AccountAddress> {
        if let Some(id) = state.get_system_clock_object_id()? {
            return Ok(id);
        }

        if !std::env::var("KANARI_CLOCK_CREATE_VM")
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
        {
            return self.ensure_system_clock_native(state);
        }

        let module_id = ClockModule::get_module_id()?;
        let func_name = ClockModule::function_names().create;

        let system_sender = AccountAddress::ZERO;
        let genesis_tx_hash = hash_data_blake3(b"KANARI::GENESIS::CLOCK").to_vec();

        let cs = self.execute_system_function_with_tx_hash_and_persistence(
            &module_id,
            func_name,
            vec![],
            vec![],
            Some(system_sender),
            None,
            Some(0),
            Some(genesis_tx_hash),
            false,
        )?;

        state.apply_changeset_without_supply_validation(&cs)?;
        self.persist_created_objects(&cs)?;
        self.persist_deleted_objects(&cs)?;

        let (object_id, _) = cs
            .created_objects
            .iter()
            .find(|(_, created)| created.type_.contains("::clock::Clock"))
            .require("Clock object was not created by clock::create")?;

        let addr = AccountAddress::from_hex_literal(object_id)?;
        state.set_system_clock_object_id(addr)?;
        Ok(addr)
    }

    fn ensure_system_clock_native(&self, state: &mut StateManager) -> Result<AccountAddress> {
        let genesis_tx_hash = hash_data_blake3(b"KANARI::GENESIS::CLOCK");
        let mut hasher = Sha3_256::new();
        hasher.update(genesis_tx_hash);
        hasher.update(0u64.to_le_bytes());
        let digest = hasher.finalize();
        let addr = AccountAddress::from_bytes(&digest[..AccountAddress::LENGTH])
            .context("Failed to derive native system clock object id")?;
        let clock = Clock {
            id: UIDRecord::new(addr),
            timestamp_ms: 0,
        };
        let clock_type = {
            let module_id = ClockModule::get_module_id()?;
            format!(
                "{}::{}::{}",
                module_id.address().to_hex_literal(),
                module_id.name(),
                ClockModule::CLOCK_STRUCT
            )
        };
        let mut changeset = ChangeSet::new();
        changeset.created_objects.push((
            addr.to_hex_literal(),
            crate::changeset::CreatedObject {
                owner: AccountAddress::ZERO,
                owner_kind: kanari_types::transaction::ObjectOwnerKind::Shared,
                uid: Some(clock.id.clone()),
                id: None,
                type_: clock_type,
                data: bcs::to_bytes(&clock).context("Failed to encode native system clock")?,
                version: 1,
            },
        ));
        state.apply_changeset_without_supply_validation(&changeset)?;
        self.persist_created_objects(&changeset)?;
        self.persist_deleted_objects(&changeset)?;
        state.set_system_clock_object_id(addr)?;
        Ok(addr)
    }

    pub fn build_native_clock_consensus_commit_prologue(
        &self,
        state: &StateManager,
        clock_id: AccountAddress,
        timestamp_ms: u64,
    ) -> Result<ChangeSet> {
        let object_id = clock_id.to_hex_literal();
        let existing = state
            .get_object(&object_id)?
            .with_context(|| format!("System clock object {object_id} not found"))?;
        let current_clock: Clock = bcs::from_bytes(&existing.data)
            .context("Failed to decode system clock object for native prologue")?;
        ensure!(
            timestamp_ms >= current_clock.timestamp_ms,
            "Clock timestamp is not monotonic: new={} current={}",
            timestamp_ms,
            current_clock.timestamp_ms
        );

        let clock = Clock {
            id: UIDRecord::new(clock_id),
            timestamp_ms,
        };
        let data = bcs::to_bytes(&clock).context("Failed to encode native clock prologue")?;
        let version = if data == existing.data {
            existing.version
        } else {
            existing.version.saturating_add(1)
        };

        let mut changeset = ChangeSet::new();
        changeset.created_objects.push((
            object_id,
            crate::changeset::CreatedObject {
                owner: existing.owner,
                owner_kind: existing.owner_kind,
                uid: Some(clock.id),
                id: existing.id,
                type_: existing.type_,
                data,
                version,
            },
        ));
        Ok(changeset)
    }

    pub fn execute_entry_function(
        &self,
        module_id: &ModuleId,
        function_name: &str,
        type_args: Vec<TypeTag>,
        args: Vec<Vec<u8>>,
        sender: Option<AccountAddress>,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
    ) -> Result<ChangeSet> {
        self.execute_entry_function_internal(
            module_id,
            function_name,
            type_args,
            args,
            ExecutionOptions::new(sender, gas_info, timestamp, None),
        )
    }

    pub fn execute_entry_function_with_object_context_and_persistence(
        &self,
        module_id: &ModuleId,
        function_name: &str,
        type_args: Vec<TypeTag>,
        args: Vec<Vec<u8>>,
        context: EntryFunctionObjectContext,
    ) -> Result<ChangeSet> {
        self.execute_entry_function_internal(
            module_id,
            function_name,
            type_args,
            args,
            ExecutionOptions::new(
                context.sender,
                context.gas_info,
                context.timestamp,
                context.tx_hash,
            )
            .with_object_inputs(context.object_inputs)
            .with_persistence(context.persist_runtime_state)
            .with_state_overlay(context.state_overlay),
        )
    }

    fn execute_system_function_with_tx_hash_and_persistence(
        &self,
        module_id: &ModuleId,
        function_name: &str,
        type_args: Vec<TypeTag>,
        args: Vec<Vec<u8>>,
        sender: Option<AccountAddress>,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        self.execute_entry_function_internal(
            module_id,
            function_name,
            type_args,
            args,
            ExecutionOptions::new(sender, gas_info, timestamp, tx_hash)
                .with_persistence(persist_runtime_state)
                .bypass_entry_check(),
        )
    }

    fn execute_entry_function_internal(
        &self,
        module_id: &ModuleId,
        function_name: &str,
        type_args: Vec<TypeTag>,
        args: Vec<Vec<u8>>,
        options: ExecutionOptions,
    ) -> Result<ChangeSet> {
        let ExecutionOptions {
            sender,
            gas_info,
            timestamp,
            tx_hash,
            object_inputs,
            persist_runtime_state,
            bypass_entry_check,
            state_overlay,
        } = options;

        let vm_guard = self.read_vm();
        let (resolver, resolver_trace) = self
            .resolver
            .tracing_clone_with_overlay(state_overlay.clone());
        let mut session =
            self.create_session_with_resolver(&vm_guard, resolver, state_overlay.clone());
        let mut deterministic_reads = std::collections::BTreeSet::from([format!(
            "module:{}:{}",
            module_id.address().to_hex_literal(),
            module_id.name()
        )
        .into_bytes()]);

        // Preload object arguments so native object borrows can resolve them from extensions.
        self.preload_objects_for_execution(
            &mut session,
            &object_inputs,
            sender,
            state_overlay.as_deref(),
        )?;

        let explicit_object_ids: HashSet<String> = object_inputs
            .iter()
            .map(|input| input.object_ref.object_id.clone())
            .collect();
        let mut explicit_object_bindings = object_inputs.iter();

        let mut ty_args_loaded = vec![];
        for tag in type_args.iter() {
            ty_args_loaded.push(
                session
                    .load_type(tag)
                    .require("Object storage operation failed")?,
            );
        }

        let ident = IdentStr::new(function_name).require("Invalid function name")?;

        let mut final_args = Self::preprocess_entry_args(args);
        deterministic_reads.extend(
            final_args
                .iter()
                .filter(|arg| arg.len() == AccountAddress::LENGTH)
                .map(|arg| format!("object:0x{}", hex::encode(arg)).into_bytes()),
        );
        self.preload_object_ids_from_args(
            &mut session,
            &final_args,
            sender,
            state_overlay.as_deref(),
        )?;
        let tx_context_bytes =
            self.build_tx_context_bytes(sender, timestamp, tx_hash.as_deref())?;

        let mut loaded_mutable_objects: Vec<LoadedMutableObject> = Vec::new();

        if let Ok(func) = session.load_function(module_id, ident, &ty_args_loaded) {
            let type_tag_for_param = |param_type: &RuntimeType| {
                session
                    .get_type_tag(param_type)
                    .ok()
                    .or_else(|| match param_type {
                        RuntimeType::Reference(inner) | RuntimeType::MutableReference(inner) => {
                            session.get_type_tag(inner).ok()
                        }
                        _ => None,
                    })
            };
            let is_key_struct_param = |param_type: &RuntimeType| {
                Self::is_runtime_struct_like(param_type)
                    && session
                        .get_type_abilities(param_type)
                        .map(|abilities| abilities.has_key())
                        .unwrap_or(false)
            };

            let binding_requirements = func
                .parameters
                .iter()
                .enumerate()
                .filter_map(|(i, param_type)| {
                    let mutable = Self::object_param_mutability(param_type, is_key_struct_param)?;
                    if Self::is_tx_context_param(param_type, type_tag_for_param) {
                        return None;
                    }
                    Some(ObjectParamBindingRequirement {
                        param_index: i,
                        mutable,
                    })
                })
                .collect::<Vec<_>>();

            // System functions (bypass_entry_check) bind object references via the
            // plain 32-byte id argument path below rather than via declared
            // object_inputs, so the declared-binding count check does not apply.
            if !bypass_entry_check {
                Self::validate_object_input_bindings(&object_inputs, &binding_requirements, true)?;
            }

            for (i, param_type) in func.parameters.iter().enumerate() {
                if i >= final_args.len() {
                    let needs_declared_object_input = !bypass_entry_check
                        && Self::object_param_mutability(param_type, is_key_struct_param).is_some()
                        && !Self::is_tx_context_param(param_type, type_tag_for_param);
                    if needs_declared_object_input {
                        final_args.resize_with(i + 1, Vec::new);
                    } else {
                        break;
                    }
                }

                let mut bound_from_explicit_input = false;

                if final_args[i].is_empty()
                    && let Some(TypeTag::Struct(struct_tag)) = type_tag_for_param(param_type)
                    && struct_tag.address == *module_id.address()
                    && struct_tag.module.as_str() == module_id.name().as_str()
                    && struct_tag.name.as_str() == module_id.name().as_str().to_ascii_uppercase()
                    && let Ok(layout) = session.type_to_type_layout(param_type)
                    && let Some(otw_bytes) = Self::synthesize_otw_bytes_from_layout(&layout)
                {
                    final_args[i] = otw_bytes;
                }

                if Self::object_param_mutability(param_type, is_key_struct_param).is_some()
                    && let Some(TypeTag::Struct(_)) = type_tag_for_param(param_type)
                    && let Some(explicit_input) = explicit_object_bindings.next()
                {
                    let explicit_addr =
                        AccountAddress::from_hex_literal(&explicit_input.object_ref.object_id)
                            .with_context(|| {
                                format!(
                                    "Invalid explicit object input {} for parameter {}",
                                    explicit_input.object_ref.object_id, i
                                )
                            })?;
                    final_args[i] = explicit_addr.to_vec();
                    bound_from_explicit_input = true;
                }

                let is_potential_id = final_args[i].len() == 32;

                if is_potential_id
                    && Self::object_param_mutability(param_type, is_key_struct_param).is_some()
                    && !Self::is_tx_context_param(param_type, type_tag_for_param)
                {
                    let Ok(object_addr) = AccountAddress::from_bytes(final_args[i].as_slice())
                    else {
                        continue;
                    };
                    let object_id = object_addr.to_hex_literal();
                    if !explicit_object_ids.is_empty() && !explicit_object_ids.contains(&object_id)
                    {
                        continue;
                    }

                    if let Some(stored_obj) =
                        self.get_object_for_execution(&object_id, state_overlay.as_deref())?
                    {
                        if !bound_from_explicit_input && let Some(s_addr) = sender {
                            let sys_addr = KanariAddress::kanari_system_account_address();
                            let std_addr = KanariAddress::std_account_address();
                            if stored_obj.owner != s_addr
                                && stored_obj.owner != AccountAddress::ZERO
                                && stored_obj.owner != sys_addr
                                && stored_obj.owner != std_addr
                            {
                                return Err(anyhow::anyhow!("Ownership verification failed"));
                            }
                        }

                        final_args[i] = stored_obj.data.clone();

                        if matches!(
                            param_type,
                            RuntimeType::MutableReference(_)
                                | RuntimeType::Struct(_)
                                | RuntimeType::StructInstantiation(_)
                        ) {
                            loaded_mutable_objects.push((
                                i,
                                stored_obj.id.clone(),
                                stored_obj.owner,
                                stored_obj.owner_kind.clone(),
                                stored_obj.type_name.clone(),
                                stored_obj.version,
                            ));
                        }
                    }
                }
            }
            if func.parameters.len() == final_args.len() + 1
                && let Some(last_param_type) = func.parameters.last()
                && let Some(TypeTag::Struct(struct_tag)) = type_tag_for_param(last_param_type)
                && Self::is_tx_context_struct(&struct_tag)
            {
                final_args.push(tx_context_bytes);
            }
        }

        // Extensions are already added by create_session_with_storage_ext() - no need to add again

        let (execution_result, vm_gas_used) = if bypass_entry_check {
            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);
            let result = session.execute_function_bypass_visibility(
                module_id,
                ident,
                ty_args_loaded,
                final_args,
                &mut metered_gas,
            );
            (result, metered_gas.gas_used())
        } else {
            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);
            let result = session.execute_entry_function(
                module_id,
                ident,
                ty_args_loaded,
                final_args,
                &mut metered_gas,
            );
            (result, metered_gas.gas_used())
        };

        let mut cs = ChangeSet::new();

        match execution_result {
            Ok(return_values) => {
                // Extract data from native extensions before finishing the session
                let (
                    transferred,
                    captured_events,
                    saved_objects,
                    deleted_objects,
                    dynamic_fields_ops,
                    borrowed_objects,
                ) = {
                    let exts_after = session.get_native_extensions();
                    (
                        exts_after.get_mut::<TransferredObjectsExt>().take_all(),
                        exts_after.get_mut::<EventsExt>().take_all(),
                        exts_after.get_mut::<SavedObjectsExt>().take_all(),
                        exts_after.get_mut::<DeletedObjectsExt>().take_all(),
                        exts_after.get_mut::<DynamicFieldsExt>().take_all(),
                        exts_after.get_mut::<BorrowedObjectsExt>().take_all(),
                    )
                };

                let (res, _) = session.finish();
                let (move_changeset, events) = res.require("exec error")?;

                for (addr, account_changes) in move_changeset.accounts() {
                    for op in account_changes.resources().values() {
                        if let move_core_types::effects::Op::Delete = op {
                            cs.add_deleted_object(addr.to_hex_literal());
                        }
                    }
                }

                self.parse_move_changeset(&move_changeset, &mut cs);
                self.parse_move_events(&events, &mut cs);

                let mut processed_ids = std::collections::HashSet::new();

                for (idx, data, _) in return_values.mutable_reference_outputs {
                    if let Some((_, id, owner, owner_kind, type_name, version)) =
                        loaded_mutable_objects
                            .iter()
                            .find(|(i, _, _, _, _, _)| *i == idx as usize)
                    {
                        self.upsert_created_object(
                            &mut cs,
                            *owner,
                            owner_kind.clone(),
                            id,
                            type_name,
                            data.clone(),
                            version + 1,
                            "writeback",
                        );
                        processed_ids.insert(id.clone());
                    }
                }

                for saved in saved_objects {
                    if processed_ids.contains(&saved.object_id) {
                        continue;
                    }

                    let (owner, owner_kind, version) = self
                        .resolve_saved_owner_metadata(&loaded_mutable_objects, &saved.object_id)?;

                    self.upsert_created_object(
                        &mut cs,
                        owner,
                        owner_kind,
                        &saved.object_id,
                        &saved.object_type,
                        saved.data.clone(),
                        version,
                        "saved",
                    );
                    processed_ids.insert(saved.object_id);
                }

                // Record objects that were updated through `borrow_global_mut`.
                for borrowed in borrowed_objects {
                    if processed_ids.contains(&borrowed.object_id) {
                        continue;
                    }

                    let (owner, owner_kind, version) = self.resolve_saved_owner_metadata(
                        &loaded_mutable_objects,
                        &borrowed.object_id,
                    )?;

                    self.upsert_created_object(
                        &mut cs,
                        owner,
                        owner_kind,
                        &borrowed.object_id,
                        &borrowed.object_type,
                        borrowed.data.clone(),
                        version,
                        "borrowed_mut",
                    );
                    processed_ids.insert(borrowed.object_id);
                }

                self.add_transferred_objects_to_changeset(
                    &mut cs,
                    transferred,
                    persist_runtime_state,
                    state_overlay.as_ref(),
                )?;

                for ev in captured_events.into_iter() {
                    cs.add_event(Event {
                        key: ev.key,
                        sequence_number: ev.sequence_number,
                        type_tag: ev.type_tag,
                        event_data: ev.event_data,
                    });
                }

                for deleted_obj in deleted_objects {
                    cs.add_deleted_object(deleted_obj.object_id);
                }

                for op in dynamic_fields_ops {
                    match op {
                        kanari_system_natives::dynamic_field::DynamicFieldOp::Add {
                            object_id,
                            name_bytes,
                            value_bytes,
                        } => {
                            cs.added_dynamic_fields
                                .push((object_id, name_bytes, value_bytes));
                        }
                        kanari_system_natives::dynamic_field::DynamicFieldOp::Remove {
                            object_id,
                            name_bytes,
                        } => {
                            cs.removed_dynamic_fields.push((object_id, name_bytes));
                        }
                    }
                }

                if let Some((gas_limit, gas_price)) = gas_info {
                    let complexity = 1;
                    let gas_op = GasOperation::ExecuteFunction { complexity };
                    let (written, deleted) = self.calculate_storage_impact(
                        &move_changeset,
                        &cs,
                        state_overlay.as_ref(),
                    )?;
                    self.apply_gas_info(&mut cs, gas_limit, gas_price, gas_op, written, deleted)?;
                }
                cs.set_gas_used(cs.gas_used.max(vm_gas_used));

                if persist_runtime_state {
                    self.apply_move_changeset(move_changeset)?;
                }

                if persist_runtime_state {
                    self.persist_created_objects(&cs)?;
                    self.persist_deleted_objects(&cs)?;
                }

                let reads = match resolver_trace.lock() {
                    Ok(reads) => reads.clone(),
                    Err(poisoned) => poisoned.into_inner().clone(),
                };
                cs.record_resolver_reads(reads);
                cs.record_resolver_reads(deterministic_reads);
                Ok(cs)
            }
            Err(e) => {
                if let Some((gas_limit, gas_price)) = gas_info {
                    let penalty_complexity = 5;
                    let _ = self.apply_gas_info(
                        &mut cs,
                        gas_limit,
                        gas_price,
                        GasOperation::ExecuteFunction {
                            complexity: penalty_complexity,
                        },
                        0,
                        0,
                    );
                    if persist_runtime_state {
                        self.persist_created_objects(&cs)?;
                    }
                }
                Err(anyhow::anyhow!("exec error: {:?}", e))
            }
        }
    }

    /// Create a session with the native extensions required by the runtime.
    fn create_session_with_storage_ext<'r>(
        &'r self,
        vm_guard: &'r std::sync::RwLockReadGuard<'r, MoveVM>,
    ) -> Session<'r, 'r, KanariMoveResolver> {
        self.create_session_with_resolver(vm_guard, self.resolver.clone(), None)
    }

    fn create_session_with_resolver<'r>(
        &'r self,
        vm_guard: &'r std::sync::RwLockReadGuard<'r, MoveVM>,
        resolver: KanariMoveResolver,
        state_overlay: Option<crate::StateOverlay>,
    ) -> Session<'r, 'r, KanariMoveResolver> {
        // Register the core extensions used by object, event, and dynamic-field natives.
        let mut extensions = NativeContextExtensions::default();
        extensions.add(DynamicFieldsExt::default());
        extensions.add(self.dynamic_field_storage_ext(state_overlay));
        extensions.add(EventsExt::default());
        extensions.add(SavedObjectsExt::default());
        extensions.add(DeletedObjectsExt::default());
        extensions.add(TransferredObjectsExt::default());

        // Track objects loaded and mutated through object native functions.
        extensions.add(LoadedObjectsExt::default());
        extensions.add(BorrowedObjectsExt::default());

        vm_guard.new_session_with_extensions(resolver, extensions)
    }

    /// Execute a read-only function without persisting any state changes.
    pub fn execute_view_function(
        &self,
        package_addr: &str,
        module_name: &str,
        function_name: &str,
        type_args: &[String],
        args: &[Vec<u8>],
        object_inputs: &[ObjectInput],
    ) -> Result<serde_json::Value> {
        use move_core_types::account_address::AccountAddress;

        // Normalize the package address before constructing the module id.
        let addr_hex = if package_addr.starts_with("0x") || package_addr.starts_with("0X") {
            &package_addr[2..]
        } else {
            package_addr
        };
        let addr = AccountAddress::from_hex_literal(&format!("0x{}", addr_hex))
            .require("Invalid package address")?;

        let module_id = ModuleId::new(addr, Identifier::new(module_name)?);

        // Reuse the same session setup as entry-function execution.
        let vm_guard = self.read_vm();
        let mut session = self.create_session_with_storage_ext(&vm_guard);

        // Preload only object refs explicitly declared by the request.
        self.preload_objects_for_execution(&mut session, object_inputs, None, None)
            .require("Failed to preload objects")?;

        // Parse and load type arguments before invocation.
        let mut ty_args_loaded = Vec::with_capacity(type_args.len());
        for type_arg in type_args {
            // Parse type tag from string (e.g., "0x1::aptos_coin::AptosCoin") - uses cache
            let type_tag = self.parse_type_tag_fast(type_arg)?;
            ty_args_loaded.push(
                session
                    .load_type(&type_tag)
                    .require("Failed to load type argument")?,
            );
        }

        let ident = IdentStr::new(function_name).require("Invalid function name")?;
        let mut final_args = args.to_vec();

        if let Ok(func) = session.load_function(&module_id, ident, &ty_args_loaded) {
            let type_tag_for_param = |param_type: &RuntimeType| {
                session
                    .get_type_tag(param_type)
                    .ok()
                    .or_else(|| match param_type {
                        RuntimeType::Reference(inner) | RuntimeType::MutableReference(inner) => {
                            session.get_type_tag(inner).ok()
                        }
                        _ => None,
                    })
            };
            let is_key_struct_param = |param_type: &RuntimeType| {
                Self::is_runtime_struct_like(param_type)
                    && session
                        .get_type_abilities(param_type)
                        .map(|abilities| abilities.has_key())
                        .unwrap_or(false)
            };

            let binding_requirements = func
                .parameters
                .iter()
                .enumerate()
                .filter_map(|(i, param_type)| {
                    let mutable = Self::object_param_mutability(param_type, is_key_struct_param)?;
                    if Self::is_tx_context_param(param_type, type_tag_for_param) {
                        return None;
                    }
                    Some(ObjectParamBindingRequirement {
                        param_index: i,
                        mutable,
                    })
                })
                .collect::<Vec<_>>();
            Self::validate_object_input_bindings(object_inputs, &binding_requirements, true)?;

            let mut explicit_object_bindings = object_inputs.iter();
            for (i, param_type) in func.parameters.iter().enumerate() {
                if Self::object_param_mutability(param_type, is_key_struct_param).is_some()
                    && !Self::is_tx_context_param(param_type, type_tag_for_param)
                    && let Some(explicit_input) = explicit_object_bindings.next()
                {
                    if final_args.len() <= i {
                        final_args.resize_with(i + 1, Vec::new);
                    }
                    let stored_obj = self.load_object_by_ref_checked(&explicit_input.object_ref)?;
                    final_args[i] = stored_obj.data;
                }
            }
        }

        const MAX_VIEW_GAS_UNITS: u64 = 1_000_000;
        let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(MAX_VIEW_GAS_UNITS);
        let execution_result = session.execute_function_bypass_visibility(
            &module_id,
            ident,
            ty_args_loaded,
            final_args,
            &mut metered_gas,
        );

        // Finish the session without persisting any writes.
        let (res, _) = session.finish();
        let (_, _) = res.require("Session error")?;

        match execution_result {
            Ok(return_values) => {
                // Convert return values into JSON without layout-heavy decoding.
                let results: Vec<serde_json::Value> = return_values
                    .return_values
                    .into_iter()
                    .map(|(bytes, _)| Self::bytes_to_json_fast(&bytes))
                    .collect();

                if results.len() == 1 {
                    results
                        .into_iter()
                        .next()
                        .require("View function returned no values")
                } else {
                    Ok(serde_json::Value::Array(results))
                }
            }
            Err(e) => Err(anyhow::anyhow!(
                "View function execution failed: {} ({:?})",
                Self::explain_view_vm_error(&e),
                e
            )),
        }
    }

    fn explain_view_vm_error(e: &move_binary_format::errors::VMError) -> &'static str {
        match e.sub_status() {
            Some(kanari_system_natives::object::E_OBJECT_NOT_FOUND) => {
                "object not found; the object id may be wrong, deleted, or not indexed yet"
            }
            Some(kanari_system_natives::object::E_OBJECT_LAYOUT_UNAVAILABLE) => {
                "object type layout is unavailable for this view call"
            }
            Some(kanari_system_natives::object::E_OBJECT_TYPE_MISMATCH) => {
                "object type mismatch; this commonly happens after calling a newer package version with an object created by an older package address/type"
            }
            Some(kanari_system_natives::object::E_OBJECT_DESERIALIZE_FAILED) => {
                "object data could not be deserialized with the current struct layout; this commonly happens after an incompatible contract upgrade"
            }
            Some(1100) => {
                "object data could not be deserialized or loaded; check object id, type args, and contract version"
            }
            _ => "VM aborted during view function execution",
        }
    }

    /// Parse type tags with a small in-memory cache for repeated lookups.
    fn parse_type_tag_fast(&self, type_str: &str) -> Result<TypeTag> {
        use move_core_types::language_storage::TypeTag;

        // Check the cache before parsing common type strings again.
        if let Some(cached) = self.get_cached_type_tag(type_str) {
            return Ok(cached);
        }

        // Handle primitive types directly before falling back to struct parsing.
        let result = match type_str {
            "u8" => TypeTag::U8,
            "u16" => TypeTag::U16,
            "u32" => TypeTag::U32,
            "u64" => TypeTag::U64,
            "u128" => TypeTag::U128,
            "u256" => TypeTag::U256,
            "bool" => TypeTag::Bool,
            "address" => TypeTag::Address,
            "signer" => TypeTag::Signer,
            _ => {
                // Fall back to simple `address::module::name` struct parsing.
                if type_str.contains("::") {
                    let parts: Vec<&str> = type_str.split("::").collect();
                    if parts.len() >= 3 {
                        TypeTag::Struct(Box::new(StructTag {
                            address: AccountAddress::from_hex_literal(parts[0])
                                .require("Invalid address in type")?,
                            module: Identifier::new(parts[1]).require("Invalid module in type")?,
                            name: Identifier::new(parts[2]).require("Invalid name in type")?,
                            type_params: vec![],
                        }))
                    } else {
                        return Err(anyhow::anyhow!("Unsupported type: {}", type_str));
                    }
                } else {
                    return Err(anyhow::anyhow!("Unsupported type: {}", type_str));
                }
            }
        };

        // Cache the parsed result for later calls.
        self.cache_type_tag(type_str.to_string(), result.clone());
        Ok(result)
    }

    /// Read a cached type tag if present.
    fn get_cached_type_tag(&self, type_str: &str) -> Option<TypeTag> {
        if let Ok(cache) = self.type_tag_cache.read() {
            return cache.get(type_str).cloned();
        }
        None
    }

    /// Store a parsed type tag in the cache.
    fn cache_type_tag(&self, type_str: String, type_tag: TypeTag) {
        if let Ok(mut cache) = self.type_tag_cache.write() {
            cache.insert(type_str, type_tag);
        }
    }

    /// Convert return bytes into JSON with a lightweight best-effort strategy.
    fn bytes_to_json_fast(bytes: &[u8]) -> serde_json::Value {
        // Try a few simple decodings before falling back to hex.
        if bytes.len() <= 8 {
            // Likely a primitive type (u8, u16, u32, u64, bool)
            if bytes.len() == 1 {
                return serde_json::Value::Number(serde_json::Number::from(bytes[0]));
            }
            if bytes.len() == 8
                && let Ok(num) = bcs::from_bytes::<u64>(bytes)
            {
                return serde_json::Value::Number(serde_json::Number::from(num));
            }
        }

        // For addresses (32 bytes)
        if bytes.len() == 32
            && let Ok(addr) = AccountAddress::from_bytes(bytes)
        {
            return serde_json::Value::String(addr.to_hex_literal());
        }

        serde_json::Value::Array(bytes.iter().copied().map(serde_json::Value::from).collect())
    }
}

#[cfg(test)]
mod binding_tests {
    use super::*;
    use move_vm_types::loaded_data::runtime_types::CachedStructIndex;
    use proptest::prelude::*;

    #[test]
    fn generic_struct_references_are_object_input_candidates() {
        let generic_coin_ref =
            RuntimeType::MutableReference(Box::new(RuntimeType::StructInstantiation(Box::new((
                CachedStructIndex(0),
                vec![RuntimeType::TyParam(0)],
            )))));

        assert_eq!(
            MoveRuntime::object_param_mutability(&generic_coin_ref, |_| true),
            Some(true)
        );
    }

    #[test]
    fn key_struct_values_are_object_input_candidates() {
        let coin_value = RuntimeType::StructInstantiation(Box::new((
            CachedStructIndex(0),
            vec![RuntimeType::TyParam(0)],
        )));

        assert_eq!(
            MoveRuntime::object_param_mutability(&coin_value, |_| true),
            Some(true)
        );
    }

    #[test]
    fn non_struct_references_are_not_object_input_candidates() {
        let vector_ref =
            RuntimeType::Reference(Box::new(RuntimeType::Vector(Box::new(RuntimeType::U8))));

        assert_eq!(
            MoveRuntime::object_param_mutability(&vector_ref, |_| false),
            None
        );
    }

    #[test]
    fn declared_object_inputs_must_match_reference_param_count() {
        let err = MoveRuntime::validate_declared_object_input_bindings(
            &[ObjectInput {
                object_ref: kanari_types::transaction::ObjectRef::new(
                    "0x1",
                    Some(1),
                    Some("d".to_string()),
                ),
                owner: Some(kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                    "0x1".to_string(),
                )),
                mutable: true,
            }],
            &[],
        )
        .expect_err("count mismatch should fail");

        assert!(err.to_string().contains("count mismatch"));
    }

    #[test]
    fn declared_object_inputs_must_match_reference_param_mutability() {
        let err = MoveRuntime::validate_declared_object_input_bindings(
            &[ObjectInput {
                object_ref: kanari_types::transaction::ObjectRef::new(
                    "0x1",
                    Some(1),
                    Some("d".to_string()),
                ),
                owner: Some(kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                    "0x1".to_string(),
                )),
                mutable: false,
            }],
            &[ObjectParamBindingRequirement {
                param_index: 0,
                mutable: true,
            }],
        )
        .expect_err("mutability mismatch should fail");

        assert!(err.to_string().contains("mutability"));
    }

    #[test]
    fn mutable_object_input_can_bind_immutable_reference_param() {
        MoveRuntime::validate_declared_object_input_bindings(
            &[ObjectInput {
                object_ref: kanari_types::transaction::ObjectRef::new(
                    "0x1",
                    Some(1),
                    Some("d".to_string()),
                ),
                owner: Some(kanari_types::transaction::ObjectOwnerKind::AddressOwner(
                    "0x1".to_string(),
                )),
                mutable: true,
            }],
            &[ObjectParamBindingRequirement {
                param_index: 0,
                mutable: false,
            }],
        )
        .expect("mutable input should satisfy immutable reference binding");
    }

    #[test]
    fn immutable_object_cannot_bind_mutable_reference_param() {
        let err = MoveRuntime::validate_declared_object_input_bindings(
            &[ObjectInput {
                object_ref: kanari_types::transaction::ObjectRef::new(
                    "0x1",
                    Some(1),
                    Some("d".to_string()),
                ),
                owner: Some(kanari_types::transaction::ObjectOwnerKind::Immutable),
                mutable: true,
            }],
            &[ObjectParamBindingRequirement {
                param_index: 0,
                mutable: true,
            }],
        )
        .expect_err("immutable mutable-ref binding should fail");

        assert!(err.to_string().contains("Immutable object input"));
    }

    #[test]
    fn generic_immutable_reference_is_an_object_input_candidate() {
        let generic_pool_ref = RuntimeType::Reference(Box::new(RuntimeType::StructInstantiation(
            Box::new((CachedStructIndex(1), vec![RuntimeType::TyParam(0)])),
        )));

        assert_eq!(
            MoveRuntime::object_param_mutability(&generic_pool_ref, |_| false),
            Some(false)
        );
    }

    #[test]
    fn multiple_object_inputs_validate_in_parameter_order() {
        let inputs = (0..2)
            .map(|index| ObjectInput {
                object_ref: kanari_types::transaction::ObjectRef::new(
                    format!("0x{}", index + 1),
                    Some(1),
                    Some(format!("digest-{index}")),
                ),
                owner: Some(kanari_types::transaction::ObjectOwnerKind::Shared),
                mutable: true,
            })
            .collect::<Vec<_>>();
        let requirements = vec![
            ObjectParamBindingRequirement {
                param_index: 0,
                mutable: true,
            },
            ObjectParamBindingRequirement {
                param_index: 1,
                mutable: false,
            },
        ];

        MoveRuntime::validate_declared_object_input_bindings(&inputs, &requirements)
            .expect("multiple object inputs should validate in order");
    }

    #[test]
    fn object_input_requires_owner_semantics() {
        let err = MoveRuntime::validate_declared_object_input_bindings(
            &[ObjectInput {
                object_ref: kanari_types::transaction::ObjectRef::new(
                    "0x1",
                    Some(1),
                    Some("d".to_string()),
                ),
                owner: None,
                mutable: true,
            }],
            &[ObjectParamBindingRequirement {
                param_index: 0,
                mutable: true,
            }],
        )
        .expect_err("object input without owner semantics should fail");

        assert!(err.to_string().contains("must declare owner semantics"));
    }

    fn policy_object_input(
        index: usize,
        mutable: bool,
        owner: Option<ObjectOwnerKind>,
    ) -> ObjectInput {
        ObjectInput {
            object_ref: kanari_types::transaction::ObjectRef::new(
                format!("0x{:x}", index + 1),
                Some(index as u64 + 1),
                Some(format!("digest-{index}")),
            ),
            owner,
            mutable,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1024))]
        #[test]
        fn raw_address_mutability_policy_never_allows_cross_owner_owned_objects(
            requested_mutable in any::<bool>(),
            is_coin in any::<bool>(),
        ) {
            let owner = AccountAddress::from_hex_literal("0x1111").unwrap();
            let sender = AccountAddress::from_hex_literal("0x2222").unwrap();
            let object_type = if is_coin {
                format!("0x2::coin::Coin<{}>", kanari_types::gas_coin::GAS_COIN)
            } else {
                "0x48::escrow_like::Marker".to_string()
            };

            prop_assert!(!MoveRuntime::can_mutably_borrow_preloaded_object(
                requested_mutable,
                &object_type,
                &ObjectOwnerKind::AddressOwner(owner.to_hex_literal()),
                owner,
                Some(sender),
                false,
            ));
        }

        #[test]
        fn explicit_cross_owner_policy_allows_only_mutable_non_coin_owned_objects(
            requested_mutable in any::<bool>(),
            is_coin in any::<bool>(),
        ) {
            let owner = AccountAddress::from_hex_literal("0x1111").unwrap();
            let sender = AccountAddress::from_hex_literal("0x2222").unwrap();
            let object_type = if is_coin {
                format!("0x2::coin::Coin<{}>", kanari_types::gas_coin::GAS_COIN)
            } else {
                "0x48::escrow_like::Marker".to_string()
            };

            let allowed = MoveRuntime::can_mutably_borrow_preloaded_object(
                requested_mutable,
                &object_type,
                &ObjectOwnerKind::AddressOwner(owner.to_hex_literal()),
                owner,
                Some(sender),
                true,
            );

            prop_assert_eq!(allowed, requested_mutable && !is_coin);
        }

        #[test]
        fn owner_and_shared_policy_respects_mutable_and_immutable_flags(
            requested_mutable in any::<bool>(),
            use_shared in any::<bool>(),
            use_immutable in any::<bool>(),
        ) {
            let owner = AccountAddress::from_hex_literal("0x1111").unwrap();
            let owner_kind = if use_immutable {
                ObjectOwnerKind::Immutable
            } else if use_shared {
                ObjectOwnerKind::Shared
            } else {
                ObjectOwnerKind::AddressOwner(owner.to_hex_literal())
            };

            let allowed = MoveRuntime::can_mutably_borrow_preloaded_object(
                requested_mutable,
                "0x48::escrow_like::Marker",
                &owner_kind,
                owner,
                Some(owner),
                false,
            );

            prop_assert_eq!(allowed, requested_mutable && !use_immutable);
        }

        #[test]
        fn object_input_binding_policy_matrix_is_strict(
            requirement_mutability in prop::collection::vec(any::<bool>(), 0..8),
            input_mutability in prop::collection::vec(any::<bool>(), 0..10),
            owner_selector in prop::collection::vec(0u8..4, 0..10),
            allow_extra_dependency_inputs in any::<bool>(),
        ) {
            let requirements = requirement_mutability
                .iter()
                .enumerate()
                .map(|(param_index, mutable)| ObjectParamBindingRequirement {
                    param_index,
                    mutable: *mutable,
                })
                .collect::<Vec<_>>();
            let inputs = input_mutability
                .iter()
                .enumerate()
                .map(|(index, mutable)| {
                    let owner = match owner_selector.get(index).copied().unwrap_or(0) {
                        0 => Some(ObjectOwnerKind::AddressOwner(format!("0x{:x}", index + 0x100))),
                        1 => Some(ObjectOwnerKind::Shared),
                        2 => Some(ObjectOwnerKind::Immutable),
                        _ => None,
                    };
                    policy_object_input(index, *mutable, owner)
                })
                .collect::<Vec<_>>();

            let result = MoveRuntime::validate_object_input_bindings(
                &inputs,
                &requirements,
                allow_extra_dependency_inputs,
            );

            let count_ok = inputs.len() >= requirements.len()
                && (allow_extra_dependency_inputs || inputs.len() == requirements.len());
            let per_param_ok = inputs
                .iter()
                .zip(requirements.iter())
                .all(|(input, requirement)| {
                    (!requirement.mutable || input.mutable)
                        && input.owner.is_some()
                        && (!requirement.mutable
                            || !matches!(input.owner, Some(ObjectOwnerKind::Immutable)))
                });

            prop_assert_eq!(result.is_ok(), count_ok && per_param_ok);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/move_runtime_tests.rs"]
mod tests;
