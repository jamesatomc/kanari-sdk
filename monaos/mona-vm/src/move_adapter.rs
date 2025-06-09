//! Move language adapter for Kanari blockchain with mona-types integration

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use log::{debug, info, error};

use move_core_types::{
    account_address::AccountAddress,
    language_storage::ModuleId,
    runtime_value::MoveValue,
    resolver::{LinkageResolver, ModuleResolver, ResourceResolver},
};
use move_binary_format::{
    CompiledModule,
    file_format::{FunctionDefinition, Visibility},
};
use move_compiler::{Compiler, shared::NumericalAddress};
use move_symbol_pool::Symbol;
use move_vm_runtime::{move_vm::MoveVM, session::Session};
use sha3::Digest;

use crate::types::{VMResult, VMError};
use crate::{ExecutionContext, ExecutionResult};
use mona_types::{
    address::Address,
    gas_coin::KariBalance,
    balance::Balance,
    event::{EventData, EventEmitter, MemoryEventEmitter, TransferEvent},
};

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

/// Enhanced Move adapter for compiling and executing Move code with mona-types
pub struct MoveAdapter {
    /// Move VM runtime instance
    move_vm: MoveVM,
    /// Compiled modules cache
    modules: HashMap<ModuleId, CompiledModule>,
    /// Module information cache
    module_info: HashMap<ModuleId, MoveModuleInfo>,    /// Address mappings for named addresses
    address_mappings: HashMap<String, AccountAddress>,
    /// Event emitter for Move events
    event_emitter: MemoryEventEmitter,
    /// KARI balance tracker
    kari_balances: HashMap<Address, KariBalance>,
}

impl MoveAdapter {
    /// Create a new Move adapter
    pub fn new() -> Self {
        let mut address_mappings = HashMap::new();
        
        // Add standard address mappings
        address_mappings.insert("std".to_string(), AccountAddress::from_hex_literal("0x1").unwrap());
        address_mappings.insert("kanari_framework".to_string(), AccountAddress::from_hex_literal("0x2").unwrap());
        address_mappings.insert("kanari_system".to_string(), AccountAddress::from_hex_literal("0x3").unwrap());        Self {
            move_vm: MoveVM::new(vec![]).expect("Failed to create Move VM"),
            modules: HashMap::new(),
            module_info: HashMap::new(),
            address_mappings,
            event_emitter: MemoryEventEmitter::new(),
            kari_balances: HashMap::new(),
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
    }    /// Get address mapping
    pub fn get_address_mapping(&self, name: &str) -> Option<AccountAddress> {
        self.address_mappings.get(name).copied()
    }

    // Enhanced functions for mona-types integration    /// Execute a Move function with enhanced integration
    pub fn execute_function_with_context(
        &mut self,
        sender: Address,
        module_id: &ModuleId,
        function_name: &str,
        _type_args: Vec<String>,
        args: Vec<MoveValue>,
        _gas_budget: u64,
    ) -> VMResult<ExecutionResult> {        debug!("Executing Move function: {}::{}", module_id, function_name);

        // Check gas budget
        if _gas_budget == 0 {
            return Err(VMError::GasLimitExceeded { used: 0, limit: 0 });
        }

        // Create Move session
        // Note: This is a simplified version - real implementation would need proper resolver
        let _session = self.move_vm.new_session(&DummyResolver);

        // Convert mona Address to AccountAddress
        let _sender_addr = convert_address_to_account_address(sender);

        // Simplified execution - in a real implementation, this would use the Move VM
        debug!("Executing function with {} arguments", args.len());

        // For now, return a successful result
        Ok(ExecutionResult {
            success: true,
            return_value: vec![],
            gas_used: 1000, // Placeholder
            events: vec![],
            error: None,
        })
    }

    /// Transfer KARI tokens using Move VM
    pub fn transfer_kari(
        &mut self,
        from: Address,
        to: Address,
        amount: u64,
        gas_budget: u64,
    ) -> VMResult<ExecutionResult> {
        debug!("Transferring {} KARI from {} to {}", amount, from, to);

        // Check if sender has sufficient balance
        let from_balance = self.kari_balances.get(&from).cloned().unwrap_or_default();
        if from_balance.value() < amount {
            return Err(VMError::InsufficientFunds {
                required: amount,
                available: from_balance.value(),
            });
        }        // Calculate gas fee (simplified calculation)
        let gas_fee = 100; // Base gas fee
        let total_cost = amount + gas_fee;

        if from_balance.value() < total_cost {
            return Err(VMError::InsufficientFunds {
                required: total_cost,
                available: from_balance.value(),
            });
        }

        // Update balances
        let new_from_balance = Balance::with_value(from_balance.value() - total_cost);
        let to_balance = self.kari_balances.get(&to).cloned().unwrap_or_default();
        let new_to_balance = Balance::with_value(to_balance.value() + amount);

        self.kari_balances.insert(from, new_from_balance);
        self.kari_balances.insert(to, new_to_balance);        // Emit transfer event
        let transfer_event = TransferEvent {
            from,
            to,
            amount,
            coin_type: "KARI".to_string(),
        };
        self.event_emitter.emit(transfer_event);Ok(ExecutionResult {
            success: true,
            return_value: vec![],
            gas_used: gas_fee,
            events: Vec::new(), // Empty events for now
            error: None,
        })
    }

    /// Get KARI balance for an address
    pub fn get_kari_balance(&self, address: &Address) -> u64 {
        self.kari_balances.get(address)
            .map(|b| b.value())
            .unwrap_or(0)
    }

    /// Set KARI balance for an address (for testing/genesis)
    pub fn set_kari_balance(&mut self, address: Address, balance: u64) {
        self.kari_balances.insert(address, Balance::with_value(balance));
    }   
    
     /// Get emitted events
    pub fn get_events(&self) -> Vec<EventData> {
        self.event_emitter.get_events()
    }


    /// Clear events (usually called after processing)
    pub fn clear_events(&mut self) {
        // MemoryEventEmitter doesn't have a clear method, so we recreate it
        self.event_emitter = MemoryEventEmitter::new();
    }    // Private helper functions - simplified implementations
    fn _execute_script_function(
        &self,
        _session: Session<&DummyResolver>,
        _sender: AccountAddress,
        _module_id: &ModuleId,
        _function_name: &str,
        _type_args: Vec<String>,
        _args: Vec<MoveValue>,
        _gas_budget: u64,
    ) -> VMResult<ExecutionResult> {
        // Simplified implementation
        Ok(ExecutionResult {
            success: true,
            return_value: vec![],
            gas_used: 1000, // Placeholder
            events: vec![],
            error: None,
        })
    }
}

impl Default for MoveAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Dummy resolver for Move VM (simplified)
struct DummyResolver;

impl LinkageResolver for DummyResolver {
    type Error = VMError;
}

impl ModuleResolver for DummyResolver {
    type Error = VMError;

    fn get_module(&self, _module_id: &ModuleId) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(None)
    }
}

impl ResourceResolver for DummyResolver {
    type Error = VMError;

    fn get_resource(
        &self,
        _address: &AccountAddress,
        _typ: &move_core_types::language_storage::StructTag,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(None)
    }
}

/// Convert mona Address to Move AccountAddress
fn convert_address_to_account_address(addr: Address) -> AccountAddress {
    AccountAddress::new(*addr.to_bytes())
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
