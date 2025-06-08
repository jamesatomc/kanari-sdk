//! Move language adapter for Kanari blockchain

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use log::{debug, info, error};

use move_core_types::{
    account_address::AccountAddress,
    language_storage::ModuleId,
};
use move_binary_format::{
    CompiledModule,
    file_format::{FunctionDefinition, Visibility},
};
use move_compiler::{Compiler, shared::NumericalAddress};
use move_symbol_pool::Symbol;
use sha3::Digest;

use crate::types::{VMResult, VMError};
use crate::{ExecutionContext, ExecutionResult};
use mona_types::address::Address;

/// Move function visibility levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MoveFunctionVisibility {
    Public,
    Script,
    Friend,
    Private,
}

/// Move function information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveFunction {
    pub name: String,
    pub visibility: MoveFunctionVisibility,
    pub is_entry: bool,
    pub generic_type_params: Vec<String>,
    pub parameters: Vec<String>,
    pub return_types: Vec<String>,
}

/// Move module information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveModuleInfo {
    pub module_id: String,
    pub address: String,
    pub name: String,
    pub functions: Vec<MoveFunction>,
    pub structs: Vec<String>,
    pub dependencies: Vec<String>,
    pub bytecode_hash: String,
}

/// Move adapter for compiling and executing Move code
pub struct MoveAdapter {
    /// Compiled modules cache
    modules: HashMap<ModuleId, CompiledModule>,
    /// Module information cache
    module_info: HashMap<ModuleId, MoveModuleInfo>,
    /// Address mappings for named addresses
    address_mappings: HashMap<String, AccountAddress>,
}

impl MoveAdapter {
    /// Create a new Move adapter
    pub fn new() -> Self {
        let mut address_mappings = HashMap::new();
        
        // Add standard address mappings
        address_mappings.insert("std".to_string(), AccountAddress::from_hex_literal("0x1").unwrap());
        address_mappings.insert("kanari_framework".to_string(), AccountAddress::from_hex_literal("0x2").unwrap());
        address_mappings.insert("kanari_system".to_string(), AccountAddress::from_hex_literal("0x3").unwrap());

        Self {
            modules: HashMap::new(),
            module_info: HashMap::new(),
            address_mappings,
        }
    }

    /// Compile Move source code
    pub fn compile_module(&mut self, source_path: &str, dependencies: Vec<&str>) -> VMResult<CompiledModule> {
        info!("Compiling Move module from: {}", source_path);

        // Set up compiler addresses
        let mut addresses = std::collections::BTreeMap::new();
        for (name, addr) in &self.address_mappings {
            addresses.insert(
                Symbol::from(name.as_str()),
                NumericalAddress::new(addr.into_bytes(), move_compiler::shared::NumberFormat::Hex)
            );
        }

        // Compile the source
        let compiler = Compiler::from_files(
            None, // vfs_root
            vec![source_path.to_string()],
            dependencies.iter().map(|s| s.to_string()).collect(),
            addresses,
        );

        match compiler.build_and_report() {
            Ok((_, compiled_units)) => {
                if compiled_units.is_empty() {
                    return Err(VMError::RuntimeError {
                        message: "No modules compiled".to_string(),
                    });
                }

                // Take the first compiled module
                let unit = &compiled_units[0];
                let module = unit.named_module.clone();
                let compiled_module = module.module;

                // Cache the module
                let module_id = compiled_module.self_id();
                self.modules.insert(module_id.clone(), compiled_module.clone());

                // Extract and cache module information
                let module_info = self.extract_module_info(&compiled_module)?;
                self.module_info.insert(module_id, module_info);

                info!("Successfully compiled module: {}", compiled_module.name());
                Ok(compiled_module)
            },
            Err(errors) => {
                error!("Move compilation failed: {:?}", errors);
                Err(VMError::RuntimeError {
                    message: format!("Compilation failed: {:?}", errors),
                })
            }
        }
    }

    /// Extract module information from compiled module
    fn extract_module_info(&self, module: &CompiledModule) -> VMResult<MoveModuleInfo> {
        let module_id = module.self_id();
        let mut functions = Vec::new();
        let mut structs = Vec::new();        // Extract function information
        for (_i, func_def) in module.function_defs().iter().enumerate() {
            let func_handle = module.function_handle_at(func_def.function);
            let func_name = module.identifier_at(func_handle.name).to_string();
              let visibility = match func_def.visibility {
                Visibility::Public => MoveFunctionVisibility::Public,
                Visibility::Friend => MoveFunctionVisibility::Friend,
                Visibility::Private => MoveFunctionVisibility::Private,
            };

            let is_entry = func_def.is_entry;

            functions.push(MoveFunction {
                name: func_name,
                visibility,
                is_entry,
                generic_type_params: Vec::new(), // TODO: Extract actual type parameters
                parameters: Vec::new(), // TODO: Extract actual parameters
                return_types: Vec::new(), // TODO: Extract actual return types
            });
        }

        // Extract struct information
        for struct_def in module.struct_defs() {
            let struct_handle = module.struct_handle_at(struct_def.struct_handle);
            let struct_name = module.identifier_at(struct_handle.name).to_string();
            structs.push(struct_name);
        }

        // Extract dependencies
        let mut dependencies = Vec::new();
        for module_handle in module.module_handles() {
            let addr = module.address_identifier_at(module_handle.address);
            let name = module.identifier_at(module_handle.name);
            let dep_id = format!("{}::{}", addr, name);
            dependencies.push(dep_id);
        }        // Calculate bytecode hash
        let mut bytecode = Vec::new();
        module.serialize(&mut bytecode).map_err(|e| VMError::RuntimeError {
            message: format!("Module serialization failed: {:?}", e),
        })?;
        let bytecode_hash = hex::encode(sha3::Sha3_256::digest(&bytecode));

        Ok(MoveModuleInfo {
            module_id: module_id.to_string(),
            address: module_id.address().to_hex_literal(),
            name: module_id.name().to_string(),
            functions,
            structs,
            dependencies,
            bytecode_hash,
        })
    }

    /// Deploy a compiled module
    pub fn deploy_module(
        &mut self,
        module: CompiledModule,
        deployer: Address,
        context: &mut ExecutionContext,
    ) -> VMResult<ExecutionResult> {
        let module_id = module.self_id();
        info!("Deploying module: {} by {}", module_id, deployer.to_hex_literal());        // Verify module integrity
        if let Err(e) = move_bytecode_verifier::verify_module_unmetered(&module) {
            return Err(VMError::RuntimeError {
                message: format!("Module verification failed: {:?}", e),
            });
        }

        // Calculate deployment gas
        let mut bytecode = Vec::new();
        module.serialize(&mut bytecode).map_err(|e| VMError::RuntimeError {
            message: format!("Module serialization failed: {:?}", e),
        })?;
        let deployment_gas = 10000 + (bytecode.len() as u64) * 100;

        if !context.has_gas(deployment_gas) {
            return Err(VMError::GasLimitExceeded {
                used: context.gas_used + deployment_gas,
                limit: context.gas_limit,
            });
        }

        context.consume_gas(deployment_gas).map_err(|e| VMError::RuntimeError {
            message: e,
        })?;

        // Store the module
        self.modules.insert(module_id.clone(), module.clone());

        // Extract and store module information
        let module_info = self.extract_module_info(&module)?;
        self.module_info.insert(module_id, module_info);

        Ok(ExecutionResult::success(
            Vec::new(),
            deployment_gas,
            Vec::new(),
        ))
    }

    /// Execute a Move function
    pub fn execute_function(
        &self,
        module_id: &ModuleId,
        function_name: &str,
        args: Vec<Vec<u8>>,
        context: &mut ExecutionContext,
    ) -> VMResult<ExecutionResult> {
        info!("Executing function {}::{}", module_id, function_name);

        // Check if module exists
        let module = self.modules.get(module_id)
            .ok_or_else(|| VMError::ContractNotFound {
                address: module_id.to_string(),
            })?;

        // Find the function
        let _function = self.find_function(module, function_name)?;

        // Calculate execution gas
        let execution_gas = 1000 + (args.len() as u64) * 100;

        if !context.has_gas(execution_gas) {
            return Err(VMError::GasLimitExceeded {
                used: context.gas_used + execution_gas,
                limit: context.gas_limit,
            });
        }

        context.consume_gas(execution_gas).map_err(|e| VMError::RuntimeError {
            message: e,
        })?;

        // Simplified execution - in a real implementation, this would use the Move VM
        debug!("Executing function with {} arguments", args.len());

        // For now, return a successful result
        Ok(ExecutionResult::success(
            vec![1], // Mock return value
            execution_gas,
            Vec::new(),
        ))
    }    /// Find a function in a module
    fn find_function<'a>(&self, module: &'a CompiledModule, function_name: &str) -> VMResult<&'a FunctionDefinition> {
        for func_def in module.function_defs() {
            let func_handle = module.function_handle_at(func_def.function);
            let name = module.identifier_at(func_handle.name);
            if name.as_str() == function_name {
                return Ok(func_def);
            }
        }

        Err(VMError::FunctionNotFound {
            module: module.self_id().to_string(),
            function: function_name.to_string(),
        })
    }

    /// Get module information
    pub fn get_module_info(&self, module_id: &ModuleId) -> Option<&MoveModuleInfo> {
        self.module_info.get(module_id)
    }

    /// Get all loaded modules
    pub fn get_loaded_modules(&self) -> Vec<ModuleId> {
        self.modules.keys().cloned().collect()
    }

    /// Check if a module is loaded
    pub fn is_module_loaded(&self, module_id: &ModuleId) -> bool {
        self.modules.contains_key(module_id)
    }

    /// Add address mapping
    pub fn add_address_mapping(&mut self, name: String, address: AccountAddress) {
        self.address_mappings.insert(name, address);
    }

    /// Get address mapping
    pub fn get_address_mapping(&self, name: &str) -> Option<AccountAddress> {
        self.address_mappings.get(name).copied()
    }
}

impl Default for MoveAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_move_adapter_creation() {
        let adapter = MoveAdapter::new();
        
        // Check default address mappings
        assert!(adapter.get_address_mapping("std").is_some());
        assert!(adapter.get_address_mapping("kanari_framework").is_some());
        assert!(adapter.get_address_mapping("kanari_system").is_some());
    }

    #[test]
    fn test_address_mapping() {
        let mut adapter = MoveAdapter::new();
        let test_addr = AccountAddress::from_hex_literal("0x42").unwrap();
        
        adapter.add_address_mapping("test".to_string(), test_addr);
        assert_eq!(adapter.get_address_mapping("test"), Some(test_addr));
    }
}
