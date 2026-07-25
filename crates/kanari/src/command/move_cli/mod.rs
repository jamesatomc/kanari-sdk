// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

pub mod build;
pub mod call;
pub mod docgen;
pub mod new;
pub mod publish;
pub mod test;
pub mod verify;

use kanari_system_natives::dynamic_field::DynamicFieldsExt;
use kanari_system_natives::event::EventsExt;
use kanari_system_natives::object::{DeletedObjectsExt, SavedObjectsExt};
use kanari_system_natives::transfer_natives::TransferredObjectsExt;
use kanari_types::address::Address as KanariAddress;
use kanari_types::error::KanariUnwrapExt;
use move_core_types::{account_address::AccountAddress, identifier::Identifier};
use move_package::source_package::layout::SourcePackageLayout;
use move_stdlib_natives::{GasParameters, NurseryGasParameters, all_natives, nursery_natives};
use move_unit_test::extensions::set_extension_hook;
use move_vm_runtime::native_functions::NativeFunction;
use std::path::PathBuf;

use anyhow::Context;
use clap::Subcommand;

type NativeFunctionRecord = (AccountAddress, Identifier, Identifier, NativeFunction);

/// Top-level `move` subcommands supported by the kanari CLI.
#[derive(Subcommand)]
pub enum MoveCommand {
    /// Build the current Move package
    Build(build::Build),
    /// Create a new Move package
    New(new::New),
    /// Run Move unit tests
    Test(test::Test),
    /// Generate Move docs
    Docgen(docgen::Docgen),
    /// Publish Move modules on chain
    Publish(publish::Publish),
    /// Upgrade already-published Move modules on chain
    Upgrade(publish::Publish),
    /// Verify Move module bytecode locally (RPC)
    Verify(verify::Verify),
    /// Call Move function on blockchain
    Call(call::Call),
}

impl MoveCommand {
    /// Execute the selected Move subcommand. This provides a thin dispatch
    /// layer that constructs a default `BuildConfig` where required.
    pub fn execute(self) -> anyhow::Result<()> {
        match self {
            MoveCommand::Build(b) => {
                let config = b.build_config.clone();
                b.execute(None, config)
            }
            MoveCommand::New(n) => n.execute_with_defaults(None),
            MoveCommand::Test(t) => {
                let config = t.build_config.clone();
                // Construct standard library natives so native functions used by Move
                // packages (e.g., stdlib and unit_test helpers) are available to the VM
                let std_addr = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)
                    .invariant("stdlib address");
                let std_natives = all_natives(std_addr, GasParameters::zeros())
                    .into_iter()
                    .chain(nursery_natives(
                        false,
                        std_addr,
                        NurseryGasParameters::zeros(),
                    ));

                // Construct kanari crypto/system natives (registered under package address 0x2)
                let system_addr =
                    AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS)
                        .invariant("system address");
                let system_natives = kanari_system_natives::all_natives(
                    system_addr,
                    kanari_system_natives::GasParameters::zeros(),
                )
                .into_iter();
                // Register native-context extensions for the unit-test runner so
                // extensions like event capture and transfer tracking are available.
                set_extension_hook(Box::new(|exts| {
                    exts.add(EventsExt::default());
                    exts.add(TransferredObjectsExt::default());
                    exts.add(DeletedObjectsExt::default());
                    exts.add(SavedObjectsExt::default());
                    exts.add(DynamicFieldsExt::default());
                    exts.add(
                        kanari_system_natives::dynamic_field::DynamicFieldStorageExt::default(),
                    );
                    exts.add(kanari_system_natives::object::LoadedObjectsExt::default());
                    exts.add(kanari_system_natives::object::BorrowedObjectsExt::default());
                }));

                // Merge all natives and pass into test runner
                let natives = std_natives.chain(system_natives).collect();
                t.execute(None, config, natives, None)
            }
            MoveCommand::Docgen(d) => {
                let config = d.build_config.clone();
                d.execute(None, config)
            }
            MoveCommand::Publish(p) => {
                let config = p.build_config.clone();
                p.execute_as(None, config, publish::PublishOperation::Publish)
            }
            MoveCommand::Upgrade(p) => {
                let config = p.build_config.clone();
                p.execute_as(None, config, publish::PublishOperation::Upgrade)
            }
            MoveCommand::Verify(v) => v.execute(),
            MoveCommand::Call(c) => c.execute(),
        }
    }
}

/// Reroot the current working directory to the root of the Move package
/// containing the given path (or the current directory if None).
pub fn reroot_path(path: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    // Always root ourselves to the package root, and then compile relative to that.
    let rooted_path = SourcePackageLayout::try_find_root(&path.canonicalize()?)?;
    std::env::set_current_dir(rooted_path)
        .context("Failed to set current directory to Move package root")?;

    Ok(PathBuf::from("."))
}
