pub mod vm_state;
pub mod vm_transaction;
pub mod vm_module;
pub mod build;
pub mod publish;
pub mod blockchain_integration;
pub mod storage;
pub mod utils;

// Re-export main types and functions
pub use vm_state::{VMState, VM_STATE};
pub use vm_transaction::{VMTransaction, execute_vm_transaction, convert_to_vm_transaction};
pub use vm_module::{VMModule, load_module_from_mvsm};
pub use build::Build;
pub use publish::Publish;
pub use storage::save_vm_state;
pub use utils::{reroot_path, generate_object_id, generate_random_hex};
