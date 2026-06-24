// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Genesis initialization for Kanari blockchain
//!
//! This module executes the actual framework bytecode to initialize genesis state.
//! It loads compiled .mv files from disk, publishes all modules in dependency order,
//! and calls kanari::init() to mint initial coin supply through Move VM.

use crate::move_runtime::{MoveRuntime, load_system_modules};
use crate::state::StateManager;
use anyhow::{Context, Result};
use kanari_crypto::hash_data_blake3;
use kanari_types::address::Address as KanariAddress;

use std::format;

/// Initialize genesis state by executing framework modules from bytecode files
pub fn init_genesis(state: &mut StateManager) -> Result<()> {
    log::info!("=== Executing framework modules for genesis ===");

    // Find the framework bytecode directory
    let bytecode_dir = load_system_modules::find_kanari_system_modules_dir();

    if !bytecode_dir.exists() {
        anyhow::bail!(
            "Framework bytecode directory not found at {}. Set KANARI_FRAMEWORK_PATH or run 'kanari move build' first.",
            bytecode_dir.display()
        );
    }

    log::info!("Loading framework modules from: {}", bytecode_dir.display());

    // Build an in-memory runtime and preload framework/system natives.
    let runtime = MoveRuntime::new_with_kanari_natives_in_memory()
        .context("MoveRuntime initialization failed")?;
    log::info!("✓ MoveRuntime initialized with system modules");

    let system_addr = KanariAddress::kanari_system_account_address();

    // Discover framework modules and publish them in dependency order.
    let sorted_modules = load_system_modules::load_system_modules_from_dir(&bytecode_dir)
        .context("Failed to load system modules")?;

    log::info!("Discovered {} framework modules", sorted_modules.len());
    log::info!(
        "Publishing {} framework modules in dependency order",
        sorted_modules.len()
    );

    // Publish each module and apply the resulting changeset immediately.
    for (idx, module) in sorted_modules.iter().enumerate() {
        let module_name = &module.file_name;
        log::info!(
            "[{}/{}] Publishing {}...",
            idx + 1,
            sorted_modules.len(),
            module_name
        );

        match runtime.publish_module(
            module.bytes.clone(),
            system_addr,
            None, // No gas info for genesis
            None, // No timestamp
        ) {
            Ok(changeset) => {
                log::info!(
                    "   ✓ Published successfully ({} objects created, {} events)",
                    changeset.created_objects.len(),
                    changeset.events.len()
                );

                // Log created objects for genesis diagnostics.
                if !changeset.created_objects.is_empty() {
                    for (obj_id, obj) in &changeset.created_objects {
                        log::info!(
                            "      Created object: {} (type: {}, owner: {})",
                            obj_id,
                            obj.type_,
                            obj.owner.to_hex_literal()
                        );
                    }
                }

                // Log emitted events for genesis diagnostics.
                if !changeset.events.is_empty() {
                    for event in &changeset.events {
                        log::info!(
                            "      Event: type={}, seq={}, data_len={}",
                            event.type_tag,
                            event.sequence_number,
                            event.event_data.len()
                        );
                    }
                }

                // Persist state and object side effects for this module publish.
                state.apply_changeset(&changeset)?;
                runtime.persist_created_objects(&changeset)?;
                runtime.persist_deleted_objects(&changeset)?;

                // Run `kanari::init()` after publishing the main kanari module.
                if *module_name == "kanari.mv" {
                    log::info!("   Executing kanari::init() via Move VM...");

                    use move_core_types::account_address::AccountAddress as MoveAccountAddress;

                    let move_system_addr = MoveAccountAddress::from_hex_literal(
                        system_addr.to_hex_literal().as_str(),
                    )?;

                    let witness_bytes = Vec::new();
                    let init_tx_hash = hash_data_blake3(b"KANARI::GENESIS::INIT::KANARI").to_vec();

                    match runtime.execute_init_function_with_context(
                        move_system_addr,
                        "kanari",
                        vec![witness_bytes],
                        Some(0),
                        Some(init_tx_hash),
                    ) {
                        Ok(init_changeset) => {
                            log::info!(
                                "   ✓ kanari::init() executed ({} objects created, {} events)",
                                init_changeset.created_objects.len(),
                                init_changeset.events.len()
                            );

                            // Log created objects for genesis diagnostics.
                            for (obj_id, obj) in &init_changeset.created_objects {
                                log::info!(
                                    "      Created object: {} (type: {}, owner: {})",
                                    obj_id,
                                    obj.type_,
                                    obj.owner.to_hex_literal()
                                );
                            }

                            // Log emitted events for genesis diagnostics.
                            if !init_changeset.events.is_empty() {
                                for event in &init_changeset.events {
                                    log::info!(
                                        "      Event: type={}, seq={}",
                                        event.type_tag,
                                        event.sequence_number
                                    );
                                }
                            }

                            state.apply_changeset(&init_changeset)?;
                            runtime.persist_created_objects(&init_changeset)?;
                            runtime.persist_deleted_objects(&init_changeset)?;
                        }
                        Err(e) => {
                            log::error!("   ❌ Failed to execute kanari::init(): {:?}", e);
                            return Err(e).context("kanari::init() execution failed");
                        }
                    }
                }
            }
            Err(e) => {
                log::error!("   ❌ Failed to publish {}: {:?}", module_name, e);
                return Err(e).context(format!("Failed to publish module: {}", module_name));
            }
        }
    }

    log::info!("✓ All framework modules published");
    log::info!("=== Genesis initialization complete ===");

    // Commit the accumulated genesis overlay to persistent storage.
    state.commit()?;

    Ok(())
}
