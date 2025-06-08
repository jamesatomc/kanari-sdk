//! Core VM implementation for Move smart contracts with Kari gas integration

use std::collections::HashMap;
use std::sync::{Arc, RwLock, Mutex};
use std::time::{Duration, Instant};
use log::{debug, info, warn, error};

// Additional imports for full functionality
use move_core_types::{
    account_address::AccountAddress,
    language_storage::ModuleId,
};
use sha3::Digest;

use crate::types::{
    ContractAddress, ExecutionStats, DeploymentInfo, FunctionCall,
    TransactionReceipt, ContractEvent, VMConfig, VMError, VMResult, VMEvent
};
use crate::{ExecutionContext, ExecutionResult, StateManager, ContractState, GasParameters, MoveAdapter, VMStorage};
use mona_types::address::Address;

/// Main Move VM implementation
pub struct MonaVM {
    /// VM configuration
    config: VMConfig,
    /// Gas parameters for pricing
    gas_params: Arc<GasParameters>,
    /// Move language adapter
    move_adapter: Arc<Mutex<MoveAdapter>>,
    /// State manager for contract state
    state_manager: Arc<Mutex<StateManager>>,
    /// Storage interface
    storage: Arc<VMStorage>,
    /// Event listeners
    event_listeners: Arc<Mutex<Vec<Box<dyn Fn(&VMEvent) + Send + Sync>>>>,
    /// Module cache for compiled Move modules
    module_cache: Arc<RwLock<HashMap<String, Vec<u8>>>>,
    /// Execution statistics
    stats: Arc<RwLock<ExecutionStats>>,
}

impl MonaVM {
    /// Create a new MonaVM instance
    pub fn new(
        config: VMConfig,
        gas_params: Arc<GasParameters>,
        storage: Arc<VMStorage>,
    ) -> VMResult<Self> {
        let move_adapter = Arc::new(Mutex::new(MoveAdapter::new()));
        let state_manager = Arc::new(Mutex::new(StateManager::new(storage.clone())));
        
        Ok(Self {
            config,
            gas_params,
            move_adapter,
            state_manager,
            storage,
            event_listeners: Arc::new(Mutex::new(Vec::new())),
            module_cache: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(ExecutionStats {
                gas_used: 0,
                execution_time_ms: 0,
                instructions_executed: 0,
                memory_used: 0,
                storage_reads: 0,
                storage_writes: 0,
            })),
        })
    }

    /// Deploy a Move smart contract
    pub fn deploy_contract(
        &self,
        source_code: String,
        dependencies: Vec<String>,
        deployer: Address,
        initial_gas: u64,
    ) -> VMResult<DeploymentInfo> {
        let start_time = Instant::now();
        
        // Create execution context
        let mut context = ExecutionContext::new(
            deployer,
            Address::zero(), // No contract address yet
            initial_gas,
            chrono::Utc::now().timestamp() as u64,
            0, // Current block height - should be injected
        );

        // Compile Move module
        let compiled_module = {
            let mut adapter = self.move_adapter.lock().unwrap();
            // For now, we'll create a temporary file for compilation
            let temp_path = format!("temp_module_{}.move", rand::random::<u32>());
            std::fs::write(&temp_path, &source_code).map_err(|e| VMError::InternalError {
                message: format!("Failed to write temporary file: {}", e),
            })?;
            
            let result = adapter.compile_module(&temp_path, dependencies.iter().map(|s| s.as_str()).collect());
            
            // Clean up temporary file
            let _ = std::fs::remove_file(&temp_path);
            
            result?
        };

        // Generate contract address from deployer and bytecode
        let mut hasher = sha3::Sha3_256::new();
        hasher.update(deployer.as_bytes());
        hasher.update(compiled_module.name().as_str());
        let contract_address = Address::from_bytes(hasher.finalize().as_slice()).map_err(|e| {
            VMError::InternalError {
                message: format!("Failed to generate contract address: {}", e),
            }
        })?;

        // Update context with contract address
        context.contract_address = contract_address;

        // Deploy the module
        let execution_result = {
            let mut adapter = self.move_adapter.lock().unwrap();
            adapter.deploy_module(compiled_module.clone(), deployer, &mut context)?
        };

        // Store contract bytecode
        let mut bytecode = Vec::new();
        compiled_module.serialize(&mut bytecode).map_err(|e| VMError::InternalError {
            message: format!("Failed to serialize module: {}", e),
        })?;
        
        self.storage.store_contract_bytecode(contract_address, &bytecode)?;

        // Create contract state
        {
            let mut state_manager = self.state_manager.lock().unwrap();
            state_manager.create_contract_state(contract_address)?;
        }

        // Store deployment info
        let deployment_info = DeploymentInfo {
            contract_address,
            deployer,
            bytecode_hash: hex::encode(sha3::Sha3_256::digest(&bytecode)),
            gas_used: execution_result.gas_used,
            timestamp: chrono::Utc::now().timestamp() as u64,
            module_name: compiled_module.name().to_string(),
        };

        // Emit deployment event
        self.emit_event(VMEvent::ContractDeployed {
            address: contract_address,
            deployer,
            gas_used: execution_result.gas_used,
            timestamp: deployment_info.timestamp,
        });

        // Update stats
        self.update_stats(execution_result.gas_used, start_time.elapsed().as_millis() as u64);

        info!("Contract deployed successfully at address: {}", contract_address.to_hex_literal());
        Ok(deployment_info)
    }

    /// Execute a function call on a deployed contract
    pub fn call_function(
        &self,
        function_call: FunctionCall,
    ) -> VMResult<TransactionReceipt> {
        let start_time = Instant::now();
        
        // Create execution context
        let mut context = ExecutionContext::new(
            function_call.caller,
            function_call.contract_address,
            function_call.gas_limit,
            chrono::Utc::now().timestamp() as u64,
            0, // Current block height
        );

        // Check if contract exists
        {
            let state_manager = self.state_manager.lock().unwrap();
            if !state_manager.contract_exists(function_call.contract_address)? {
                return Err(VMError::ContractNotFound {
                    address: function_call.contract_address.to_hex_literal(),
                });
            }
        }

        // Load contract module
        let module_id = move_core_types::language_storage::ModuleId::new(
            AccountAddress::from_bytes(function_call.contract_address.as_bytes())
                .map_err(|e| VMError::InternalError {
                    message: format!("Invalid contract address: {}", e),
                })?,
            move_core_types::identifier::Identifier::new(&function_call.module_name)
                .map_err(|e| VMError::InternalError {
                    message: format!("Invalid module name: {}", e),
                })?,
        );

        // Execute function
        let execution_result = {
            let adapter = self.move_adapter.lock().unwrap();
            adapter.execute_function(
                &module_id,
                &function_call.function_name,
                function_call.args,
                &mut context,
            )?
        };

        // Generate transaction hash
        let mut hasher = sha3::Sha3_256::new();
        hasher.update(function_call.caller.as_bytes());
        hasher.update(function_call.contract_address.as_bytes());
        hasher.update(&function_call.function_name);
        hasher.update(&context.timestamp.to_le_bytes());
        let transaction_hash = hex::encode(hasher.finalize());

        // Create transaction receipt
        let receipt = TransactionReceipt {
            transaction_hash: transaction_hash.clone(),
            contract_address: Some(function_call.contract_address),
            caller: function_call.caller,
            gas_used: execution_result.gas_used,
            gas_price: self.gas_params.base_costs.gas_price,
            success: true,
            return_data: execution_result.return_data,
            events: execution_result.events.into_iter().map(|e| ContractEvent {
                contract_address: function_call.contract_address,
                event_type: "function_call".to_string(),
                data: e,
                indexed_data: Vec::new(),
            }).collect(),
            error_message: None,
        };

        // Store execution result
        self.storage.store_execution_result(&transaction_hash, &execution_result)?;

        // Emit function call event
        self.emit_event(VMEvent::FunctionCalled {
            contract: function_call.contract_address,
            function: function_call.function_name,
            caller: function_call.caller,
            gas_used: execution_result.gas_used,
            success: true,
            timestamp: context.timestamp,
        });

        // Update stats
        self.update_stats(execution_result.gas_used, start_time.elapsed().as_millis() as u64);

        Ok(receipt)
    }

    /// Get contract information
    pub fn get_contract_info(&self, address: ContractAddress) -> VMResult<Option<ContractState>> {
        let state_manager = self.state_manager.lock().unwrap();
        state_manager.get_contract_state(address)
    }

    /// Get contract storage
    pub fn get_contract_storage(
        &self,
        address: ContractAddress,
        key: &[u8],
    ) -> VMResult<Option<Vec<u8>>> {
        self.storage.load_contract_storage(address, key)
    }

    /// Add event listener
    pub fn add_event_listener<F>(&self, listener: F)
    where
        F: Fn(&VMEvent) + Send + Sync + 'static,
    {
        let mut listeners = self.event_listeners.lock().unwrap();
        listeners.push(Box::new(listener));
    }

    /// Get VM statistics
    pub fn get_stats(&self) -> ExecutionStats {
        self.stats.read().unwrap().clone()
    }

    /// Get VM configuration
    pub fn get_config(&self) -> &VMConfig {
        &self.config
    }

    /// Get gas parameters
    pub fn get_gas_params(&self) -> &GasParameters {
        &self.gas_params
    }

    /// Helper function to emit events
    fn emit_event(&self, event: VMEvent) {
        let listeners = self.event_listeners.lock().unwrap();
        for listener in listeners.iter() {
            listener(&event);
        }
    }

    /// Helper function to update execution statistics
    fn update_stats(&self, gas_used: u64, execution_time_ms: u64) {
        let mut stats = self.stats.write().unwrap();
        stats.gas_used += gas_used;
        stats.execution_time_ms += execution_time_ms;
        stats.instructions_executed += 1; // Simplified
        stats.storage_reads += 1; // Simplified
    }
}

impl Default for MonaVM {
    fn default() -> Self {
        let config = VMConfig::default();
        let gas_params = Arc::new(GasParameters::default());
        let storage = Arc::new(VMStorage::new(
            Arc::new(mona_storage::RocksDBStorage::new(std::env::temp_dir().join("test_vm")).unwrap()),
            1024 * 1024, // 1MB cache
        ));
        
        Self::new(config, gas_params, storage).unwrap()
    }
}
