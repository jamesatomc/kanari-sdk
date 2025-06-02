// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

pub mod blockchain_integration;
pub mod build;
pub mod publish;
pub mod storage;
pub mod utils;
pub mod vm_module;
pub mod vm_state;
pub mod vm_transaction;

// Re-export main types and functions
pub use build::Build;
pub use publish::Publish;
pub use storage::save_vm_state;
pub use utils::{generate_object_id, generate_random_hex, reroot_path};
pub use vm_module::{VMModule, load_module_from_mvsm};
pub use vm_state::{VM_STATE, VMState};
pub use vm_transaction::{VMTransaction, convert_to_vm_transaction, execute_vm_transaction};
