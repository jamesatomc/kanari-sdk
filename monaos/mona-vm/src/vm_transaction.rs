use std::time::{SystemTime, UNIX_EPOCH, Duration};
use sha3::{Digest, Sha3_256};
use serde_json::{json, Value as JsonValue};
use mona_blockchain::block::Transaction;
use mona_blockchain::blockchain::BLOCKCHAIN_DATA;
use mona_types::gas::{calculate_gas_fee, format_gas_fee_display};

use crate::vm_state::{VM_STATE, find_module_with_variations};
use crate::vm_module::load_module_from_mvsm;

// VM Transaction Structure
#[derive(Debug)]
pub struct VMTransaction {
    pub tx_id: String,
    pub sender: String,
    pub module_id: String,
    pub function: String,
    pub args: Vec<Vec<u8>>,
    pub gas_budget: u64,
    pub timestamp: u64,
    pub signature: Option<Vec<u8>>,
    pub signer_address: Option<String>,
    pub mvsm_file: Option<String>,
}

impl VMTransaction {
    pub fn new(sender: String, module_id: String, function: String, args: Vec<Vec<u8>>, gas_budget: u64) -> Self {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let mut hasher = Sha3_256::new();
        hasher.update(format!("{}{}{}{}{}", sender, module_id, function, timestamp, gas_budget).as_bytes());
        let tx_id = format!("vm_tx_{}", hex::encode(&hasher.finalize()[..16]));
        
        Self { tx_id, sender, module_id, function, args, gas_budget, timestamp, signature: None, signer_address: None, mvsm_file: None }
    }
    
    pub fn with_signature(mut self, signature: Vec<u8>, signer_address: String) -> Self {
        self.signature = Some(signature);
        self.signer_address = Some(signer_address);
        self
    }
    
    pub fn with_mvsm_file(mut self, mvsm_file: String) -> Self {
        self.mvsm_file = Some(mvsm_file);
        self
    }
}

// Execute VM transaction function
pub fn execute_vm_transaction(tx: &VMTransaction) -> Result<JsonValue, String> {
    let start = std::time::Instant::now();
    
    // Get or load module
    let module = {
        let state = VM_STATE.read().map_err(|e| format!("VM state access failed: {}", e))?;
        match find_module_with_variations(&state, &tx.module_id) {
            Ok(module) => module,
            Err(_) => {
                drop(state);
                let loaded_module = load_module_from_mvsm(&tx.module_id, tx.mvsm_file.as_deref())
                    .map_err(|e| format!("{}\nTry --mvsm-file parameter", e))?;
                
                // Register module
                let mut state = VM_STATE.write().map_err(|e| format!("VM state write failed: {}", e))?;
                state.register_module(loaded_module.clone());
                
                // Also register with padded address
                let mut padded_module = loaded_module.clone();
                padded_module.module_id = format!("0x{:0>64}::{}", loaded_module.address.to_hex(), loaded_module.name);
                state.register_module(padded_module);
                
                loaded_module
            }
        }
    };
    
    // Validate function exists
    let function_lower = tx.function.to_lowercase();
    if !module.public_functions.iter().any(|f| f.to_lowercase() == function_lower) {
        return Err(format!("Function '{}' not found in module {}", tx.function, module.module_id));
    }
    
    // Simulate execution
    std::thread::sleep(Duration::from_millis(50));
    
    // Calculate gas
    let args_size: u64 = tx.args.iter().map(|arg| arg.len() as u64).sum();
    let gas_used = calculate_gas_fee(Some((args_size / 10).max(1)));
    
    let block_height = BLOCKCHAIN_DATA.iter().last().map(|b| b.index).unwrap_or(0);
    
    Ok(json!({
        "status": "success",
        "tx_id": tx.tx_id,
        "module": tx.module_id,
        "function": tx.function,
        "gas_used": gas_used,
        "gas_display": format_gas_fee_display(gas_used),
        "execution_time_ms": start.elapsed().as_millis(),
        "block_height": block_height,
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
    }))
}

// Convert blockchain transaction to VM transaction
pub fn convert_to_vm_transaction(transaction: &Transaction) -> Option<VMTransaction> {
    let data = transaction.data.as_ref().filter(|d| !d.is_empty())?;
    let vm_data = std::str::from_utf8(data).ok()?;
    
    if !vm_data.starts_with("VM:") { return None; }
    
    let parts: Vec<&str> = vm_data.split(':').collect();
    if parts.len() < 4 { return None; }
    
    Some(VMTransaction {
        tx_id: transaction.transaction_id.clone(),
        sender: transaction.sender.to_hex_literal(),
        module_id: parts[1].to_string(),
        function: parts[2].to_string(),
        args: Vec::new(),
        gas_budget: parts[3].parse().unwrap_or(1000000),
        timestamp: transaction.timestamp,
        signature: None,
        signer_address: None,
        mvsm_file: None,
    })
}
