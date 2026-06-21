// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Helper functions for MoveRuntime resource parsing and object ID generation
use kanari_types::balance::BalanceModule;
use kanari_types::coin::CoinModule;

use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::{StructTag, TypeTag};

/// Size of a Move object UID in bytes (address)
const UID_SIZE: usize = 32;
/// Size of a u64 field in bytes
const U64_SIZE: usize = 8;

impl super::MoveRuntime {
    /// Preload potential object arguments into LoadedObjectsExt before execution
    /// This enables native_borrow_global and borrow_global_mut to resolve objects during VM execution
    pub(crate) fn preload_objects_for_execution(
        &self,
        session: &mut move_vm_runtime::session::Session<
            crate::storage::resolver::KanariMoveResolver,
        >,
        args: &[Vec<u8>],
    ) -> anyhow::Result<()> {
        use kanari_system_natives::object::LoadedObjectsExt;

        // Scan through arguments to find potential object IDs (32-byte addresses)
        for arg in args {
            if arg.len() == 32 {
                let Ok(object_addr) = AccountAddress::from_bytes(arg.as_slice()) else {
                    continue;
                };
                let object_id = object_addr.to_hex_literal();

                let stored_obj = self.object_storage.get_object(&object_id);

                // Try to load object from storage
                if let Some(stored_obj) = stored_obj {
                    // Insert into LoadedObjectsExt so native_borrow_global and borrow_global_mut can find it
                    let exts = session.get_native_extensions();
                    let loaded_ext = exts.get_mut::<LoadedObjectsExt>();
                    loaded_ext.insert(object_id.clone(), stored_obj.type_name, stored_obj.data);
                    log::debug!(
                        "[RUNTIME] Preloaded object {} into LoadedObjectsExt",
                        object_id
                    );
                }
            }
        }

        Ok(())
    }

    /// Check if struct tag represents a balance/coin resource
    pub(crate) fn is_balance_resource(&self, struct_tag: &StructTag) -> bool {
        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();

        (module_name == CoinModule::COIN_MODULE && struct_name == CoinModule::COIN_STRUCT)
            || (module_name == BalanceModule::BALANCE_MODULE
                && struct_name == BalanceModule::BALANCE_STRUCT)
    }

    /// Check if struct tag represents a treasury resource
    pub(crate) fn is_treasury_resource(&self, struct_tag: &StructTag) -> bool {
        struct_tag.name.as_str() == CoinModule::TREASURY_CAP_STRUCT
    }

    /// Extract balance value from bytes for resources that may include UID + Balance
    pub(crate) fn extract_balance_from_bytes(
        &self,
        bytes: &[u8],
        struct_tag: &StructTag,
    ) -> Option<u64> {
        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();

        // Balance<T> (no UID): just 8 bytes
        if module_name == BalanceModule::BALANCE_MODULE
            && struct_name == BalanceModule::BALANCE_STRUCT
            && bytes.len() == U64_SIZE
        {
            let balance_bytes: [u8; U64_SIZE] = bytes.try_into().ok()?;
            return Some(u64::from_le_bytes(balance_bytes));
        }

        // Coin<T> (with UID): [32-byte address][8-byte balance]
        if module_name == CoinModule::COIN_MODULE
            && struct_name == CoinModule::COIN_STRUCT
            && bytes.len() >= (UID_SIZE + U64_SIZE)
        {
            let balance_bytes: [u8; U64_SIZE] =
                bytes[UID_SIZE..(UID_SIZE + U64_SIZE)].try_into().ok()?;
            return Some(u64::from_le_bytes(balance_bytes));
        }

        None
    }

    /// Extract total supply from TreasuryCap bytes
    pub(crate) fn extract_treasury_total_from_bytes(&self, bytes: &[u8]) -> Option<u64> {
        // TreasuryCap: [32-byte address][8-byte total_supply]
        if bytes.len() >= (UID_SIZE + U64_SIZE) {
            let supply_bytes: [u8; U64_SIZE] =
                bytes[UID_SIZE..(UID_SIZE + U64_SIZE)].try_into().ok()?;
            Some(u64::from_le_bytes(supply_bytes))
        } else {
            None
        }
    }

    /// Extract token type string from struct tag's type parameters
    pub(crate) fn token_type_from_struct_tag(&self, struct_tag: &StructTag) -> Option<String> {
        if let Some(TypeTag::Struct(st)) = struct_tag.type_params.first() {
            // Normalize via Move's Display impl so all code paths use one canonical
            // token type key (e.g. `0x2::kanari::KANARI`).
            return Some(format!("{}", st));
        }
        None
    }

    /// Execute a public, bounded and read-only Move function.
    pub fn execute_safe_view_function(
        &self,
        package_addr: &str,
        module_name: &str,
        function_name: &str,
        type_args: &[String],
        args: &[Vec<u8>],
    ) -> anyhow::Result<serde_json::Value> {
        use kanari_system_natives::dynamic_field::DynamicFieldsExt;
        use kanari_system_natives::event::EventsExt;
        use kanari_system_natives::object::{
            BorrowedObjectsExt, DeletedObjectsExt, SavedObjectsExt,
        };
        use kanari_system_natives::transfer_natives::TransferredObjectsExt;
        use move_binary_format::file_format::{CompiledModule, Visibility};
        use move_core_types::identifier::{IdentStr, Identifier};
        use move_core_types::language_storage::ModuleId;
        use move_vm_types::loaded_data::runtime_types::Type as RuntimeType;

        const MAX_VIEW_ARGS: usize = 32;
        const MAX_VIEW_TYPE_ARGS: usize = 16;
        const MAX_VIEW_INPUT_BYTES: usize = 64 * 1024;
        const MAX_VIEW_RETURN_BYTES: usize = 256 * 1024;
        const MAX_VIEW_GAS: u64 = 250_000;

        if args.len() > MAX_VIEW_ARGS || type_args.len() > MAX_VIEW_TYPE_ARGS {
            anyhow::bail!("View argument count exceeds the configured limit");
        }
        let input_size = args.iter().try_fold(0usize, |total, arg| {
            total
                .checked_add(arg.len())
                .ok_or_else(|| anyhow::anyhow!("View input size overflow"))
        })?;
        if input_size > MAX_VIEW_INPUT_BYTES {
            anyhow::bail!("View input exceeds {} bytes", MAX_VIEW_INPUT_BYTES);
        }

        let addr_hex = package_addr
            .strip_prefix("0x")
            .or_else(|| package_addr.strip_prefix("0X"))
            .unwrap_or(package_addr);
        let addr = AccountAddress::from_hex_literal(&format!("0x{}", addr_hex))
            .map_err(|e| anyhow::anyhow!("Invalid package address: {}", e))?;
        let module_id = ModuleId::new(addr, Identifier::new(module_name)?);
        let module_bytes = self
            .state
            .get_module(&module_id)
            .ok_or_else(|| anyhow::anyhow!("View module {} is not published", module_id))?;
        let compiled = CompiledModule::deserialize_with_defaults(&module_bytes)?;
        let public_non_entry = compiled.function_defs().iter().any(|definition| {
            let handle = compiled.function_handle_at(definition.function);
            compiled.identifier_at(handle.name).as_str() == function_name
                && definition.visibility == Visibility::Public
                && !definition.is_entry
        });
        if !public_non_entry {
            anyhow::bail!("View function must be public and non-entry");
        }

        let vm_guard = self.read_vm();
        let mut session = self.create_session_with_storage_ext(&vm_guard);
        self.preload_objects_for_execution(&mut session, args)
            .map_err(|e| anyhow::anyhow!("Failed to preload objects: {}", e))?;

        let mut loaded_type_args = Vec::with_capacity(type_args.len());
        for type_arg in type_args {
            let tag = self.parse_type_tag_fast(type_arg)?;
            loaded_type_args.push(
                session
                    .load_type(&tag)
                    .map_err(|e| anyhow::anyhow!("Failed to load type {}: {:?}", type_arg, e))?,
            );
        }

        let function = IdentStr::new(function_name)
            .map_err(|e| anyhow::anyhow!("Invalid function name: {}", e))?;
        let loaded = session
            .load_function(&module_id, function, &loaded_type_args)
            .map_err(|e| anyhow::anyhow!("Failed to load view function: {:?}", e))?;
        if loaded.parameters.iter().any(|parameter| {
            matches!(
                parameter,
                RuntimeType::MutableReference(_) | RuntimeType::Signer
            )
        }) {
            anyhow::bail!("View functions cannot accept signer or mutable-reference parameters");
        }

        let mut gas_meter = crate::kanari_gas_meter::KanariGasMeter::new(MAX_VIEW_GAS);
        let execution = session.execute_function_bypass_visibility(
            &module_id,
            function,
            loaded_type_args,
            args.to_vec(),
            &mut gas_meter,
        );

        let native_writes = {
            let extensions = session.get_native_extensions();
            !extensions
                .get_mut::<TransferredObjectsExt>()
                .objects
                .is_empty()
                || !extensions.get_mut::<SavedObjectsExt>().objects.is_empty()
                || !extensions.get_mut::<DeletedObjectsExt>().objects.is_empty()
                || !extensions.get_mut::<DynamicFieldsExt>().ops.is_empty()
                || !extensions
                    .get_mut::<BorrowedObjectsExt>()
                    .objects
                    .is_empty()
                || !extensions.get_mut::<EventsExt>().events.is_empty()
        };

        let (session_result, _new_storage) = session.finish();
        let (move_changeset, events) =
            session_result.map_err(|e| anyhow::anyhow!("View session error: {:?}", e))?;
        let move_writes = !move_changeset.accounts().is_empty() || !events.is_empty();
        if native_writes || move_writes {
            anyhow::bail!("View function attempted to modify state or emit events");
        }

        let return_values = execution.map_err(|error| {
            anyhow::anyhow!(
                "View function execution failed: {} ({:?})",
                Self::explain_view_vm_error(&error),
                error
            )
        })?;
        let return_size =
            return_values
                .return_values
                .iter()
                .try_fold(0usize, |total, (bytes, _)| {
                    total
                        .checked_add(bytes.len())
                        .ok_or_else(|| anyhow::anyhow!("View return size overflow"))
                })?;
        if return_size > MAX_VIEW_RETURN_BYTES {
            anyhow::bail!("View return exceeds {} bytes", MAX_VIEW_RETURN_BYTES);
        }

        let results: Vec<serde_json::Value> = return_values
            .return_values
            .into_iter()
            .map(|(bytes, _)| Self::bytes_to_json_fast(&bytes))
            .collect();
        if results.len() == 1 {
            results
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("View function returned no values"))
        } else {
            Ok(serde_json::Value::Array(results))
        }
    }
}
