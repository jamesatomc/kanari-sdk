// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use crate::vm_module::VMModule;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

// VM Transaction State Manager - Make it public so it can be accessed by the RPC API
lazy_static! {
    pub static ref VM_STATE: Arc<RwLock<VMState>> = {
        // Initialize VM_STATE with empty state (no DB loading)
        let state = VMState::new();
        Arc::new(RwLock::new(state))
    };
}

// Structure to track VM State
pub struct VMState {
    pub modules: HashMap<String, VMModule>,
    pub last_execution: u64,
    pub execution_count: u64,
    pub last_signature: Option<String>,
    pub last_signer: Option<String>,
}

impl VMState {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            last_execution: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            execution_count: 0,
            last_signature: None,
            last_signer: None,
        }
    }

    pub fn register_module(&mut self, module: VMModule) {
        self.modules.insert(module.module_id.clone(), module);
    }
}

// Helper function to find a module with different address formats
pub fn find_module_with_variations(state: &VMState, module_id: &str) -> Result<VMModule, String> {
    // First try exact match
    if let Some(module) = state.modules.get(module_id) {
        return Ok(module.clone());
    }

    // Try variations
    let variations = generate_module_id_variations(module_id);
    for variant in &variations {
        if let Some(module) = state.modules.get(variant) {
            return Ok(module.clone());
        }
    }

    // Try fuzzy match (lowercase contains)
    let module_id_lower = module_id.to_lowercase();
    for (id, module) in state.modules.iter() {
        if id.to_lowercase().contains(&module_id_lower)
            || module_id_lower.contains(&id.to_lowercase())
        {
            return Ok(module.clone());
        }
    }

    Err(format!("Module not found: {}", module_id))
}

// Enhanced generate_module_id_variations function
fn generate_module_id_variations(module_id: &str) -> Vec<String> {
    let mut variations = Vec::new();

    if let Some((addr_part, name_part)) = module_id.split_once("::") {
        if addr_part.starts_with("0x") {
            variations.push(format!("{}::{}", &addr_part[2..], name_part));
        } else {
            variations.push(format!("0x{}::{}", addr_part, name_part));
        }

        variations.push(format!("{}::{}", addr_part.to_lowercase(), name_part));
        variations.push(format!("{}::{}", addr_part.to_uppercase(), name_part));

        if let Some(addr_without_prefix) = addr_part.strip_prefix("0x") {
            if let Some(non_zero_pos) = addr_without_prefix.find(|c| c != '0') {
                if non_zero_pos > 0 {
                    let trimmed = &addr_without_prefix[non_zero_pos..];
                    variations.push(format!("0x{}::{}", trimmed, name_part));
                }
            }

            let padded = format!("{:0>64}", addr_without_prefix);
            variations.push(format!("0x{}::{}", padded, name_part));
        }
    }

    variations
}
