use mona_storage::{BlockchainStorage, RocksDBStorage, StorageError};
use common::get_kari_dir;
use crate::vm_state::VM_STATE;

// Add a standalone VM state save function
pub fn save_vm_state() -> Result<(), StorageError> {
    let kari_dir = get_kari_dir();
    let db_path = kari_dir.join("storage").join("mvsm_db");
    let storage = RocksDBStorage::new(db_path)?;
    
    match VM_STATE.try_read() {
        Ok(vm_state) => {
            let modules_count = vm_state.modules.len();
            
            // Save comprehensive VM state metadata
            let vm_metadata = serde_json::json!({
                "modules_count": modules_count,
                "last_execution": vm_state.last_execution,
                "execution_count": vm_state.execution_count,
                "last_signature": vm_state.last_signature.clone(),
                "last_signer": vm_state.last_signer.clone(),
                "modules": vm_state.modules.keys().collect::<Vec<_>>(),
                "system_info": {
                    "version": "2.0",
                    "save_timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                }
            });
            
            let metadata_bytes = vm_metadata.to_string().into_bytes();
            storage.save_data(b"vm_state_metadata", &metadata_bytes)?;
            
            // Save individual module summaries for quick lookup
            for (module_id, module) in vm_state.modules.iter() {
                let module_summary = serde_json::json!({
                    "module_id": module_id,
                    "address": format!("0x{}", module.address.to_hex()),
                    "name": module.name,
                    "deploy_block_height": module.deploy_block_height,
                    "bytecode_size": module.bytecode.len(),
                    "function_count": module.public_functions.len(),
                });
                
                let summary_key = format!("module_summary_{}", module_id);
                storage.save_data(summary_key.as_bytes(), &module_summary.to_string().into_bytes())?;
            }
            
            log::info!("Saved VM state with {} modules", modules_count);
        }
        Err(e) => {
            log::warn!("Could not access VM state for save: {}", e);
            
            // Save fallback metadata
            let fallback_metadata = serde_json::json!({
                "status": "fallback_save",
                "timestamp": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                "error": format!("{}", e),
            });
            
            storage.save_data(b"vm_state_metadata", &fallback_metadata.to_string().into_bytes())?;
        }
    }
    
    storage.flush()?;
    log::debug!("VM state saved successfully to secure storage");
    
    Ok(())
}
