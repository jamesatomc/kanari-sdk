// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use move_binary_format::file_format::Visibility;

const MAX_VIEW_ARGS: usize = 32;
const MAX_VIEW_TYPE_ARGS: usize = 16;
const MAX_VIEW_INPUT_BYTES: usize = 64 * 1024;
const MAX_VIEW_RETURN_BYTES: usize = 256 * 1024;
const MAX_VIEW_GAS: u64 = 250_000;

impl MoveRuntime {
    /// Execute a public, non-entry Move function in a bounded read-only session.
    pub fn execute_safe_view_function(
        &self,
        package_addr: &str,
        module_name: &str,
        function_name: &str,
        type_args: &[String],
        args: &[Vec<u8>],
    ) -> Result<serde_json::Value> {
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
            matches!(parameter, RuntimeType::MutableReference(_) | RuntimeType::Signer)
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
            !extensions.get_mut::<TransferredObjectsExt>().objects.is_empty()
                || !extensions.get_mut::<SavedObjectsExt>().objects.is_empty()
                || !extensions.get_mut::<DeletedObjectsExt>().objects.is_empty()
                || !extensions.get_mut::<DynamicFieldsExt>().ops.is_empty()
                || !extensions.get_mut::<BorrowedObjectsExt>().objects.is_empty()
                || !extensions.get_mut::<EventsExt>().events.is_empty()
        };

        let (session_result, _new_storage) = session.finish();
        let (move_changeset, events) = session_result
            .map_err(|e| anyhow::anyhow!("View session error: {:?}", e))?;
        let move_writes = move_changeset.accounts().next().is_some() || !events.is_empty();
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
        let return_size = return_values.return_values.iter().try_fold(
            0usize,
            |total, (bytes, _)| {
                total
                    .checked_add(bytes.len())
                    .ok_or_else(|| anyhow::anyhow!("View return size overflow"))
            },
        )?;
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
