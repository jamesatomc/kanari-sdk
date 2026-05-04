// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::storage::resolver::KanariMoveResolver;
use anyhow::Result;
use kanari_crypto::hash_data_blake3;
use kanari_system_natives::dynamic_field::DynamicFieldsExt;
use kanari_system_natives::event::EventsExt;
use kanari_system_natives::object::{DeletedObjectsExt, SavedObjectsExt};
use kanari_system_natives::transfer_natives::TransferredObjectsExt;
use kanari_types::clock::ClockModule;
use kanari_types::event::Event;
use log::debug;
use move_binary_format::file_format::CompiledModule;
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::{IdentStr, Identifier};
use move_core_types::language_storage::{ModuleId, StructTag, TypeTag};
use move_vm_runtime::move_vm::MoveVM;
use move_vm_runtime::native_extensions::NativeContextExtensions;
use move_vm_runtime::native_functions::NativeFunctionTable;
use move_vm_runtime::session::Session;
use move_vm_types::gas::UnmeteredGasMeter;
mod gas_ops;
mod helpers;
mod load_system_modules;
mod object_ops;
mod parsers;
use kanari_types::address::Address as KanariAddress;
use kanari_types::gas_v2::GasOperation;
use kanari_types::tx_context::TxContextModule;
pub mod move_runtime_extensions;
use crate::changeset::ChangeSet;
use crate::state::StateManager;
use crate::storage::move_vm_state::MoveVMState;
use crate::storage::object_storage::{ObjectStorage, ObjectStore, StoredObject};
use kanari_types::tx_context::TxContextRecord;
use move_vm_types::loaded_data::runtime_types::Type as RuntimeType;

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct MoveRuntime {
    pub(crate) vm: Arc<RwLock<MoveVM>>,
    pub(crate) all_natives: Arc<NativeFunctionTable>,
    pub(crate) resolver: KanariMoveResolver,
    pub(crate) state: MoveVMState,
    pub(crate) published_modules: Arc<RwLock<HashSet<ModuleId>>>,
    pub(crate) object_storage: Arc<dyn ObjectStore>,
}

#[derive(serde::Serialize)]
struct AutoMergeReceiptData {
    owner: AccountAddress,
    merged_count: u64,
    total_amount: u64,
}

type LoadedMutableObject = (usize, String, AccountAddress, String, u64);

#[derive(Clone)]
struct ExecutionOptions {
    sender: Option<AccountAddress>,
    gas_info: Option<(u64, u64)>,
    timestamp: Option<u64>,
    tx_hash: Option<Vec<u8>>,
    persist_runtime_state: bool,
    bypass_entry_check: bool,
}

impl MoveRuntime {
    pub fn new() -> Result<Self> {
        Self::new_with_natives(vec![])
    }

    pub fn new_with_natives(natives: Vec<NativeFunctionTable>) -> Result<Self> {
        let state = if cfg!(miri) {
            MoveVMState::new_in_memory()?
        } else {
            MoveVMState::open_default()?
        };
        Self::new_internal(natives, state)
    }

    pub fn new_with_natives_in_memory(natives: Vec<NativeFunctionTable>) -> Result<Self> {
        let state = MoveVMState::new_in_memory()?;
        Self::new_internal(natives, state)
    }

    fn new_internal(natives: Vec<NativeFunctionTable>, state: MoveVMState) -> Result<Self> {
        let all_natives: NativeFunctionTable = natives
            .into_iter()
            .flat_map(|table| table.into_iter())
            .collect();

        let vm = MoveVM::new(all_natives.clone())
            .map_err(|e| anyhow::anyhow!(format!("VM init error: {:?}", e)))?;

        let object_storage: Arc<dyn ObjectStore> = if cfg!(miri) {
            Arc::from(ObjectStorage::boxed_inmemory())
        } else {
            match ObjectStorage::boxed_with_persistence() {
                Ok(store) => Arc::from(store),
                Err(e) => {
                    log::warn!("[RUNTIME] DB load failed. Fallback to in-memory: {}", e);
                    Arc::from(ObjectStorage::boxed_inmemory())
                }
            }
        };

        let resolver = KanariMoveResolver {
            state: state.clone(),
            _object_storage: object_storage.clone(),
        };

        let published_modules: HashSet<ModuleId> = state
            .get_all_module_ids()
            .unwrap_or_default()
            .into_iter()
            .collect();

        Ok(MoveRuntime {
            vm: Arc::new(RwLock::new(vm)),
            all_natives: Arc::new(all_natives),
            resolver,
            state,
            published_modules: Arc::new(RwLock::new(published_modules)),
            object_storage,
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
        let vm = MoveVM::new(self.all_natives.as_ref().clone())
            .map_err(|e| anyhow::anyhow!("Worker VM init error: {:?}", e))?;

        Ok(MoveRuntime {
            vm: Arc::new(RwLock::new(vm)),
            all_natives: self.all_natives.clone(),
            resolver: self.resolver.clone(),
            state: self.state.clone(),
            published_modules: self.published_modules.clone(),
            object_storage: self.object_storage.clone(),
        })
    }

    // 🟢 Create new function for Hot-Reload
    pub fn reload_vm_cache(&self) -> Result<()> {
        let new_vm = MoveVM::new(self.all_natives.as_ref().clone())
            .map_err(|e| anyhow::anyhow!("Failed to reload MoveVM: {:?}", e))?;

        // Overwrite old MoveVM with newly created one, clearing all caches
        *self.vm.write().unwrap() = new_vm;

        // Preload essential system modules into the new VM's cache to ensure dependencies are available
        // This is critical for module upgrades where dependencies must be resolvable
        self.preload_system_modules_into_vm()?;

        log::info!("[RUNTIME] MoveVM cache cleared and reloaded (Hot-Reload successful)");
        Ok(())
    }

    /// Preload system modules into the VM cache to ensure dependencies are available
    fn preload_system_modules_into_vm(&self) -> Result<()> {
        let vm_guard = self.vm.read().unwrap();
        let session = self.create_session_with_storage_ext(&vm_guard);

        // Get all published module IDs from our index
        let module_ids: Vec<ModuleId> = self
            .published_modules
            .read()
            .unwrap()
            .iter()
            .cloned()
            .collect();

        // Load each module into the VM cache by accessing it through the session
        for module_id in module_ids {
            if let Some(module_bytes) = self.state.get_module(&module_id) {
                // Deserialize to trigger caching in the VM
                if CompiledModule::deserialize_with_defaults(&module_bytes).is_ok() {
                    log::debug!("[RUNTIME] Preloaded module into cache: {}", module_id);
                }
            }
        }

        // Finish session without making changes (just for caching)
        drop(session);
        drop(vm_guard);

        Ok(())
    }

    pub(crate) fn apply_move_changeset(
        &self,
        move_cs: move_core_types::effects::ChangeSet,
    ) -> Result<()> {
        for (addr, account_changes) in move_cs.accounts() {
            for (module_name, op) in account_changes.modules() {
                if let move_core_types::effects::Op::New(bytes)
                | move_core_types::effects::Op::Modify(bytes) = op
                {
                    let module_id = ModuleId::new(*addr, Identifier::new(module_name.as_str())?);
                    self.state.save_module(&module_id, bytes)?;
                    let mut modules = self
                        .published_modules
                        .write()
                        .unwrap_or_else(|p| p.into_inner());
                    modules.insert(module_id);
                }
            }
            for (struct_tag, op) in account_changes.resources() {
                match op {
                    move_core_types::effects::Op::New(bytes)
                    | move_core_types::effects::Op::Modify(bytes) => {
                        self.state.save_resource(addr, struct_tag, bytes)?;
                    }
                    move_core_types::effects::Op::Delete => {
                        self.state.delete_resource(addr, struct_tag)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn load_system_modules(&self) -> Result<()> {
        self.load_move_stdlib()?;
        self.load_kanari_system()?;
        Ok(())
    }

    pub fn publish_module(
        &self,
        module_bytes: Vec<u8>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        _timestamp: Option<u64>,
    ) -> Result<ChangeSet> {
        self.publish_module_with_persistence(module_bytes, sender, gas_info, true)
    }

    pub fn publish_module_with_persistence(
        &self,
        module_bytes: Vec<u8>,
        sender: AccountAddress,
        gas_info: Option<(u64, u64)>,
        persist_runtime_state: bool,
    ) -> Result<ChangeSet> {
        let compiled = CompiledModule::deserialize_with_defaults(&module_bytes)?;
        let module_id = compiled.self_id();

        let (move_changeset, events) = {
            // 🟢 Separate Lock into a variable first to prevent it from being dropped immediately
            let vm_guard = self.vm.read().unwrap();
            let mut session = self.create_session_with_storage_ext(&vm_guard);

            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);

            session
                .publish_module(module_bytes.clone(), sender, &mut metered_gas)
                .map_err(|e| anyhow::anyhow!("{:?}", e))?;

            session.finish().0.map_err(|e| anyhow::anyhow!("{:?}", e))?
        };

        if persist_runtime_state {
            self.apply_move_changeset(move_changeset.clone())?;

            // 🟢 Perform Hot-Reload to clear Cache immediately after Publish is done!
            self.reload_vm_cache()?;
        }

        let mut cs = ChangeSet::new();
        cs.publish_module(sender, module_id.name().to_string());
        self.parse_move_changeset(&move_changeset, &mut cs);
        self.parse_move_events(&events, &mut cs);

        if let Some((gas_limit, gas_price)) = gas_info {
            let gas_op = GasOperation::PublishModule {
                module_size: module_bytes.len(),
            };
            let (written, deleted) = self.calculate_storage_impact(&move_changeset, &cs);
            self.apply_gas_info(
                &mut cs,
                Some(sender),
                gas_limit,
                gas_price,
                gas_op,
                written,
                deleted,
            )?;
        }

        Ok(cs)
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
        module_id: &ModuleId,
        function_name: &str,
        type_args: &[TypeTag],
        args: &[Vec<u8>],
        sender: Option<AccountAddress>,
        timestamp: Option<u64>,
        tx_hash: Option<&[u8]>,
    ) -> Result<Vec<u8>> {
        let sender_addr = sender.unwrap_or(AccountAddress::ZERO);
        let epoch_timestamp_ms = timestamp.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
        });
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
            input.extend_from_slice(module_id.address().as_ref());
            input.extend_from_slice(module_id.name().as_str().as_bytes());
            input.extend_from_slice(function_name.as_bytes());
            input.extend_from_slice(&bcs::to_bytes(type_args)?);
            for arg in args {
                input.extend_from_slice(&(arg.len() as u64).to_le_bytes());
                input.extend_from_slice(arg);
            }
            hash_data_blake3(&input).to_vec()
        };
        bcs::to_bytes(&TxContextRecord::from_address(
            sender_addr,
            tx_hash,
            0,
            epoch_timestamp_ms,
            0,
        ))
        .map_err(Into::into)
    }

    fn uid_from_object_id(object_id: &str) -> Option<kanari_types::object::UIDRecord> {
        AccountAddress::from_hex_literal(object_id)
            .ok()
            .map(kanari_types::object::UIDRecord::new)
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

    fn resolve_saved_owner_and_version(
        &self,
        loaded_mutable_objects: &[LoadedMutableObject],
        object_id: &str,
    ) -> (AccountAddress, u64) {
        if let Some((_, _, owner, _, version)) = loaded_mutable_objects
            .iter()
            .find(|(_, id, _, _, _)| id == object_id)
        {
            return (*owner, *version + 1);
        }
        if let Some(stored) = self.object_storage.get_object(object_id) {
            return (stored.owner, stored.version + 1);
        }
        (AccountAddress::ZERO, 1)
    }

    pub fn persist_created_objects(&self, cs: &ChangeSet) {
        for (id, created) in &cs.created_objects {
            let stored = StoredObject {
                id: id.clone(),
                owner: created.owner,
                type_name: created.type_.clone(),
                data: created.data.clone(),
                version: created.version,
            };
            let _ = self.object_storage.store_object(stored);
        }
    }

    pub fn persist_deleted_objects(&self, cs: &ChangeSet) {
        for obj_id in &cs.deleted_objects {
            let _ = self.object_storage.delete_object(obj_id);
        }
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
            type_name: type_name.to_string(),
            data,
            version,
        };
        self.object_storage
            .store_object(stored)
            .map_err(|e| anyhow::anyhow!("{:?}", e))
    }

    pub fn ensure_system_clock(&self, state: &mut StateManager) -> Result<AccountAddress> {
        if let Some(id) = state.get_system_clock_object_id()? {
            return Ok(id);
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

        state.apply_changeset(&cs)?;
        self.persist_created_objects(&cs);
        self.persist_deleted_objects(&cs);

        let (object_id, _) = cs
            .created_objects
            .iter()
            .find(|(_, created)| created.type_.contains("::clock::Clock"))
            .ok_or_else(|| anyhow::anyhow!("Clock object was not created by clock::create"))?;

        let addr = AccountAddress::from_hex_literal(object_id)?;
        state.set_system_clock_object_id(addr)?;
        Ok(addr)
    }

    pub fn execute_clock_consensus_commit_prologue(
        &self,
        clock_id: AccountAddress,
        timestamp_ms: u64,
    ) -> Result<ChangeSet> {
        let module_id = ClockModule::get_module_id()?;
        let func_name = ClockModule::function_names().consensus_commit_prologue;

        let args = vec![bcs::to_bytes(&clock_id)?, bcs::to_bytes(&timestamp_ms)?];

        let system_sender = AccountAddress::ZERO;
        let tx_hash = Some(vec![0u8; 32]);

        self.execute_system_function_with_tx_hash_and_persistence(
            &module_id,
            func_name,
            vec![],
            args,
            Some(system_sender),
            None,
            Some(timestamp_ms),
            tx_hash,
            false,
        )
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
            ExecutionOptions {
                sender,
                gas_info,
                timestamp,
                tx_hash: None,
                persist_runtime_state: true,
                bypass_entry_check: false,
            },
        )
    }

    pub fn execute_entry_function_with_tx_hash(
        &self,
        module_id: &ModuleId,
        function_name: &str,
        type_args: Vec<TypeTag>,
        args: Vec<Vec<u8>>,
        sender: Option<AccountAddress>,
        gas_info: Option<(u64, u64)>,
        timestamp: Option<u64>,
        tx_hash: Option<Vec<u8>>,
    ) -> Result<ChangeSet> {
        self.execute_entry_function_internal(
            module_id,
            function_name,
            type_args,
            args,
            ExecutionOptions {
                sender,
                gas_info,
                timestamp,
                tx_hash,
                persist_runtime_state: true,
                bypass_entry_check: false,
            },
        )
    }

    pub fn execute_entry_function_with_tx_hash_and_persistence(
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
            ExecutionOptions {
                sender,
                gas_info,
                timestamp,
                tx_hash,
                persist_runtime_state,
                bypass_entry_check: false,
            },
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
            ExecutionOptions {
                sender,
                gas_info,
                timestamp,
                tx_hash,
                persist_runtime_state,
                bypass_entry_check: true,
            },
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
            persist_runtime_state,
            bypass_entry_check,
        } = options;

        let vm_guard = self.vm.read().unwrap();
        let mut session = self.create_session_with_storage_ext(&vm_guard);

        let mut auto_merged_coin_ids = Vec::new();
        let mut merged_coin_types = std::collections::HashSet::new();
        let mut total_merge_reads: u64 = 0;
        let mut synthetic_events = Vec::new();

        let mut ty_args_loaded = vec![];
        for tag in type_args.iter() {
            ty_args_loaded.push(
                session
                    .load_type(tag)
                    .map_err(|e| anyhow::anyhow!("{:?}", e))?,
            );
        }

        let ident = IdentStr::new(function_name).map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let mut final_args = Self::preprocess_entry_args(args);
        let tx_context_bytes = self.build_tx_context_bytes(
            module_id,
            function_name,
            &type_args,
            &final_args,
            sender,
            timestamp,
            tx_hash.as_deref(),
        )?;

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

            for (i, param_type) in func.parameters.iter().enumerate() {
                if i >= final_args.len() {
                    break;
                }

                let is_potential_id = final_args[i].len() == 32;

                if is_potential_id
                    && let Some(TypeTag::Struct(struct_tag)) = type_tag_for_param(param_type)
                {
                    let object_id = format!("0x{}", hex::encode(final_args[i].as_slice()));

                    if let Some(mut stored_obj) = self.object_storage.get_object(&object_id) {
                        if let Some(s_addr) = sender {
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

                        let is_coin = struct_tag.module.as_str() == "coin"
                            && struct_tag.name.as_str() == "Coin";
                        if is_coin
                            && let Some(s_addr) = sender
                            && let Some(coin_t) = struct_tag.type_params.first()
                        {
                            let merge_key = format!("{:?}_{}", s_addr, coin_t);
                            if !merged_coin_types.contains(&merge_key) {
                                merged_coin_types.insert(merge_key);
                                let all_coins: Vec<_> = self
                                    .object_storage
                                    .get_coins_by_type_and_owner(s_addr, coin_t)
                                    .into_iter()
                                    .take(200)
                                    .collect();
                                total_merge_reads += all_coins.len() as u64;

                                if all_coins.len() > 1 {
                                    let mut total_balance: u64 = 0;
                                    let mut successfully_merged_ids = Vec::new();
                                    for c in &all_coins {
                                        if c.data.len() == 40 {
                                            let mut bal_bytes = [0u8; 8];
                                            bal_bytes.copy_from_slice(&c.data[32..40]);
                                            if let Some(new_total) = total_balance
                                                .checked_add(u64::from_le_bytes(bal_bytes))
                                            {
                                                total_balance = new_total;
                                                if c.id != stored_obj.id {
                                                    successfully_merged_ids.push(c.id.clone());
                                                }
                                            }
                                        }
                                    }
                                    if stored_obj.data.len() == 40 {
                                        stored_obj.data[32..40]
                                            .copy_from_slice(&total_balance.to_le_bytes());
                                    }
                                    for id in &successfully_merged_ids {
                                        auto_merged_coin_ids.push(id.clone());
                                    }

                                    if let Ok(event_bytes) = bcs::to_bytes(&AutoMergeReceiptData {
                                        owner: s_addr,
                                        merged_count: successfully_merged_ids.len() as u64,
                                        total_amount: total_balance,
                                    }) {
                                        let event_type = TypeTag::Struct(Box::new(StructTag {
                                            address: KanariAddress::kanari_system_account_address(),
                                            module: Identifier::new("system_events").unwrap(),
                                            name: Identifier::new("AutoMergeReceipt").unwrap(),
                                            type_params: vec![TypeTag::Struct(struct_tag.clone())],
                                        }));
                                        synthetic_events.push((event_type, event_bytes));
                                    }
                                }
                            }
                        }

                        final_args[i] = stored_obj.data.clone();

                        if let RuntimeType::MutableReference(_) = param_type {
                            loaded_mutable_objects.push((
                                i,
                                stored_obj.id.clone(),
                                stored_obj.owner,
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
            {
                let sys_addr = KanariAddress::kanari_system_account_address();
                if struct_tag.address == sys_addr
                    && struct_tag.module.as_str() == TxContextModule::TX_CONTEXT_MODULE
                    && struct_tag.name.as_str() == TxContextModule::TX_CONTEXT_STRUCT
                {
                    final_args.push(tx_context_bytes);
                }
            }
        }

        // Extensions are already added by create_session_with_storage_ext() - no need to add again

        let execution_result = if bypass_entry_check {
            let mut unmetered_gas = UnmeteredGasMeter;
            session.execute_function_bypass_visibility(
                module_id,
                ident,
                ty_args_loaded,
                final_args,
                &mut unmetered_gas,
            )
        } else {
            let provided_gas_limit = gas_info.map(|(limit, _)| limit).unwrap_or(1_000_000);
            let mut metered_gas = crate::kanari_gas_meter::KanariGasMeter::new(provided_gas_limit);
            session.execute_entry_function(
                module_id,
                ident,
                ty_args_loaded,
                final_args,
                &mut metered_gas,
            )
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
                ) = {
                    let exts_after = session.get_native_extensions();
                    (
                        exts_after.get_mut::<TransferredObjectsExt>().take_all(),
                        exts_after.get_mut::<EventsExt>().take_all(),
                        exts_after.get_mut::<SavedObjectsExt>().take_all(),
                        exts_after.get_mut::<DeletedObjectsExt>().take_all(),
                        exts_after.get_mut::<DynamicFieldsExt>().take_all(),
                    )
                };

                let (res, _new_storage) = session.finish();
                let (move_changeset, events) =
                    res.map_err(|e| anyhow::anyhow!("exec error: {:?}", e))?;

                for (addr, account_changes) in move_changeset.accounts() {
                    for op in account_changes.resources().values() {
                        if let move_core_types::effects::Op::Delete = op {
                            cs.add_deleted_object(addr.to_hex_literal());
                        }
                    }
                }

                if persist_runtime_state {
                    self.apply_move_changeset(move_changeset.clone())?;
                }
                self.parse_move_changeset(&move_changeset, &mut cs);
                self.parse_move_events(&events, &mut cs);

                let mut processed_ids = std::collections::HashSet::new();

                for (idx, data, _layout) in return_values.mutable_reference_outputs {
                    if let Some((_, id, owner, type_name, version)) = loaded_mutable_objects
                        .iter()
                        .find(|(i, _, _, _, _)| *i == idx as usize)
                    {
                        let updated_obj = crate::changeset::CreatedObject {
                            owner: *owner,
                            uid: Self::uid_from_object_id(id),
                            type_: type_name.clone(),
                            data: data.clone(),
                            version: version + 1,
                        };

                        self.maybe_add_token_balance(
                            &mut cs,
                            *owner,
                            type_name,
                            &data,
                            id,
                            "writeback",
                        );

                        auto_merged_coin_ids.retain(|merged_id| merged_id != id);

                        cs.created_objects.retain(|(k, _)| k != id);
                        cs.created_objects.push((id.clone(), updated_obj));
                        processed_ids.insert(id.clone());
                    }
                }

                for saved in saved_objects {
                    if processed_ids.contains(&saved.object_id) {
                        continue;
                    }

                    let (owner, version) = self
                        .resolve_saved_owner_and_version(&loaded_mutable_objects, &saved.object_id);

                    let updated_obj = crate::changeset::CreatedObject {
                        owner,
                        uid: Self::uid_from_object_id(&saved.object_id),
                        type_: saved.object_type.clone(),
                        data: saved.data.clone(),
                        version,
                    };

                    self.maybe_add_token_balance(
                        &mut cs,
                        owner,
                        &saved.object_type,
                        &saved.data,
                        &saved.object_id,
                        "saved",
                    );

                    cs.created_objects.retain(|(k, _)| k != &saved.object_id);
                    cs.created_objects
                        .push((saved.object_id.clone(), updated_obj));
                    processed_ids.insert(saved.object_id);
                }

                self.add_transferred_objects_to_changeset(&mut cs, transferred);

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
                for merged_id in auto_merged_coin_ids {
                    cs.add_deleted_object(merged_id);
                }
                for (evt_type, evt_data) in synthetic_events {
                    cs.add_event(Event {
                        key: Default::default(),
                        sequence_number: 0,
                        type_tag: evt_type.to_string(),
                        event_data: evt_data,
                    });
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
                    let complexity = 1 + (total_merge_reads as u32 / 10);
                    let gas_op = GasOperation::ExecuteFunction { complexity };
                    let (written, deleted) = self.calculate_storage_impact(&move_changeset, &cs);
                    self.apply_gas_info(
                        &mut cs, sender, gas_limit, gas_price, gas_op, written, deleted,
                    )?;
                }

                if persist_runtime_state {
                    self.persist_created_objects(&cs);
                    self.persist_deleted_objects(&cs);
                }

                Ok(cs)
            }
            Err(e) => {
                if let Some((gas_limit, gas_price)) = gas_info {
                    let penalty_complexity = 5 + (total_merge_reads as u32);
                    let _ = self.apply_gas_info(
                        &mut cs,
                        sender,
                        gas_limit,
                        gas_price,
                        GasOperation::ExecuteFunction {
                            complexity: penalty_complexity,
                        },
                        0,
                        0,
                    );
                    if persist_runtime_state {
                        self.persist_created_objects(&cs);
                    }
                }
                Err(anyhow::anyhow!("exec error: {:?}", e))
            }
        }
    }

    /// Create a new session with all required extensions injected
    fn create_session_with_storage_ext<'r>(
        &'r self,
        vm_guard: &'r std::sync::RwLockReadGuard<'r, MoveVM>,
    ) -> Session<'r, 'r, KanariMoveResolver> {
        // Create extensions container and add all required extensions
        let mut extensions = NativeContextExtensions::default();
        extensions.add(DynamicFieldsExt::default());
        extensions.add(EventsExt::default());
        extensions.add(SavedObjectsExt::default());
        extensions.add(DeletedObjectsExt::default());
        extensions.add(TransferredObjectsExt::default());

        // Create session with extensions
        vm_guard.new_session_with_extensions(self.resolver.clone(), extensions)
    }

    pub fn execute_block_prologue(
        &self,
        state: &mut StateManager,
        timestamp_ms: u64,
    ) -> Result<()> {
        let module_id = ClockModule::get_module_id()?;
        let func_name = ClockModule::function_names().consensus_commit_prologue;

        let mut args: Vec<Vec<u8>> = vec![];
        let clock_id = self.ensure_system_clock(state)?;
        args.push(bcs::to_bytes(&clock_id)?);
        args.push(bcs::to_bytes(&timestamp_ms)?);

        let system_sender = AccountAddress::ZERO;

        let result = self.execute_entry_function(
            &module_id,
            func_name,
            vec![],
            args,
            Some(system_sender),
            None,
            Some(timestamp_ms),
        );

        match result {
            Ok(change_set) => {
                // Apply the changeset to persist state updates (e.g., clock timestamp change)
                state.apply_changeset(&change_set)?;
                Ok(())
            }
            Err(e) => {
                log::error!("[Block Prologue] Failed to update clock: {:?}", e);
                Err(anyhow::anyhow!("Clock update failed: {}", e))
            }
        }
    }
}
