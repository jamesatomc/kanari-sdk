//! Move smart contract simulation for panorama with enhanced Kari gas support

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use log::{debug, info, warn, error};
use tokio::sync::mpsc;
use serde_json;

use mona_types::address::Address;
use mona_vm::{MonaVM, VMConfig, GasParameters, VMStorage};
use mona_storage::{SmartContractStorage, SmartContractAddress, RocksDBStorage, ExecutionTrace, GasUsageRecord};
use common::get_kari_dir;
use crate::utils::{calculate_gas_fee, format_gas_fee_display};

/// Move contract simulation environment
pub struct MoveContractSimulator {
    /// VM instance for contract execution
    vm: Arc<MonaVM>,
    /// Storage for contracts and state
    storage: Arc<SmartContractStorage>,
    /// Gas parameters
    gas_params: Arc<GasParameters>,
    /// Deployed contracts registry
    deployed_contracts: Arc<RwLock<HashMap<Address, SmartContractAddress>>>,
    /// Execution history
    execution_history: Arc<RwLock<Vec<ExecutionTrace>>>,
}

impl MoveContractSimulator {
    /// Create a new Move contract simulator
    pub fn new(
        storage: Arc<SmartContractStorage>,
        gas_params: Arc<GasParameters>,
    ) -> Result<Self, String> {        let config = VMConfig {
            max_gas_per_transaction: 10_000_000, // 10M gas limit per transaction
            max_execution_time_ms: 30_000,       // 30 seconds
            max_memory_usage: 1024 * 1024,       // 1MB memory limit
            max_storage_operations: 10_000,      // Storage operations limit
            enable_tracing: true,
            module_cache_size: 1024,             // 1K modules cache
        };        // Create VMStorage with proper constructor (storage + cache_size_limit)
        // We need to create a RocksDBStorage instance first
        let kari_dir = common::get_kari_dir();
        let db_path = kari_dir.join("vm_simulation_db");
        let rocks_storage = Arc::new(
            mona_storage::RocksDBStorage::new(db_path)
                .map_err(|e| format!("Failed to create RocksDB storage: {}", e))?
        );
        
        let vm_storage = Arc::new(VMStorage::new(
            rocks_storage,
            10 * 1024 * 1024, // 10MB cache size limit
        ));
        let vm = Arc::new(
            MonaVM::new(config, gas_params.clone(), vm_storage)
                .map_err(|e| format!("Failed to create VM: {:?}", e))?
        );

        Ok(Self {
            vm,
            storage,
            gas_params,
            deployed_contracts: Arc::new(RwLock::new(HashMap::new())),
            execution_history: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Deploy a Move smart contract
    pub async fn deploy_contract(
        &self,
        source_code: String,
        dependencies: Vec<String>,
        deployer: Address,
        initial_gas: u64,
        tx: &mpsc::Sender<String>,
    ) -> Result<SmartContractAddress, String> {
        let start_time = std::time::Instant::now();
        
        // Calculate gas fee for deployment
        let gas_fee = calculate_gas_fee(Some(100)); // High priority for deployment
        let total_gas = initial_gas + gas_fee;
        
        info!("Deploying Move contract for deployer: {}", deployer.to_hex_literal());
        
        // Deploy contract using VM
        let deployment_info = self.vm.deploy_contract(
            source_code.clone(),
            dependencies,
            deployer,
            total_gas,
        ).map_err(|e| format!("Deployment failed: {:?}", e))?;

        let contract_address = SmartContractAddress::new(deployment_info.contract_address.as_bytes().try_into()
            .map_err(|_| "Invalid contract address format")?);

        // Record gas usage
        let gas_record = GasUsageRecord {
            contract_address: contract_address.clone(),
            transaction_hash: deployment_info.bytecode_hash.clone(),
            operation_type: "deploy".to_string(),
            kari_amount: deployment_info.gas_used,
            gas_price: 1, // 1 Kari per gas unit
            timestamp: deployment_info.timestamp,
            block_height: 0, // Will be updated when included in block
        };

        // Store gas usage record
        self.storage.store_gas_usage(&gas_record)
            .map_err(|e| format!("Failed to store gas usage: {:?}", e))?;

        // Register deployed contract
        if let Ok(mut contracts) = self.deployed_contracts.write() {
            contracts.insert(deployment_info.contract_address, contract_address.clone());
        }

        // Send deployment success notification
        let notification = format!(
            "{{\"event\":\"contract_deployed\",\"deployment\":{{\"contract_address\":\"{}\",\"deployer\":\"{}\",\"module_name\":\"{}\",\"gas_used\":{},\"kari_spent\":{},\"deployment_time_ms\":{},\"bytecode_hash\":\"{}\",\"timestamp\":{}}}}}",
            contract_address.to_hex_literal(),
            deployer.to_hex_literal(),
            deployment_info.module_name,
            deployment_info.gas_used,
            deployment_info.gas_used, // 1:1 ratio for Kari to gas
            start_time.elapsed().as_millis(),
            deployment_info.bytecode_hash,
            deployment_info.timestamp
        );
        
        let _ = tx.try_send(notification);

        info!("Successfully deployed contract at {} with gas used: {}", 
              contract_address.to_hex_literal(), deployment_info.gas_used);

        Ok(contract_address)
    }

    /// Execute a function call on a deployed Move contract
    pub async fn call_contract_function(
        &self,
        contract_address: SmartContractAddress,
        function_name: String,
        arguments: Vec<Vec<u8>>,
        caller: Address,
        gas_limit: u64,
        tx: &mpsc::Sender<String>,
    ) -> Result<Vec<Vec<u8>>, String> {
        let start_time = std::time::Instant::now();
        let transaction_hash = format!("{:x}", blake3::hash(
            format!("{}:{}:{}", contract_address.to_hex_literal(), function_name, caller.to_hex_literal()).as_bytes()
        ));

        // Calculate gas fee
        let gas_fee = calculate_gas_fee(None);
        let total_gas = gas_limit + gas_fee;

        info!("Calling function {} on contract {} by caller {}", 
              function_name, contract_address.to_hex_literal(), caller.to_hex_literal());

        // Convert SmartContractAddress to Address for VM
        let vm_contract_address = Address::from_bytes(&contract_address.bytes)
            .map_err(|e| format!("Invalid contract address: {}", e))?;

        // Execute function call using VM
        let result = self.vm.call_function(
            vm_contract_address,
            function_name.clone(),
            arguments.clone(),
            caller,
            total_gas,
        ).map_err(|e| format!("Function call failed: {:?}", e))?;

        // Record gas usage
        let gas_record = GasUsageRecord {
            contract_address: contract_address.clone(),
            transaction_hash: transaction_hash.clone(),
            operation_type: "call".to_string(),
            kari_amount: result.gas_used,
            gas_price: 1,
            timestamp: chrono::Utc::now().timestamp() as u64,
            block_height: 0,
        };

        self.storage.store_gas_usage(&gas_record)
            .map_err(|e| format!("Failed to store gas usage: {:?}", e))?;

        // Create execution trace
        let trace = ExecutionTrace {
            transaction_hash: transaction_hash.clone(),
            contract_address: contract_address.clone(),
            function_name: function_name.clone(),
            arguments: arguments.clone(),
            return_values: result.return_values.clone(),
            gas_used: result.gas_used,
            kari_spent: result.gas_used, // 1:1 ratio
            execution_time_ms: start_time.elapsed().as_millis() as u64,
            status: if result.success { "success".to_string() } else { "failed".to_string() },
            error_message: result.error_message,
            events: result.events.into_iter().map(|e| mona_storage::SmartContractEvent {
                contract_address: contract_address.clone(),
                event_type: e.event_type,
                event_data: e.data,
                transaction_hash: transaction_hash.clone(),
                block_height: 0,
                timestamp: chrono::Utc::now().timestamp() as u64,
            }).collect(),
            storage_changes: HashMap::new(), // TODO: Implement storage change tracking
        };

        // Store execution trace
        self.storage.store_execution_trace(&trace)
            .map_err(|e| format!("Failed to store execution trace: {:?}", e))?;

        // Add to history
        if let Ok(mut history) = self.execution_history.write() {
            history.push(trace.clone());
            // Keep only last 1000 executions
            if history.len() > 1000 {
                history.remove(0);
            }
        }

        // Send execution notification
        let notification = format!(
            "{{\"event\":\"function_called\",\"execution\":{{\"transaction_hash\":\"{}\",\"contract_address\":\"{}\",\"function_name\":\"{}\",\"caller\":\"{}\",\"gas_used\":{},\"kari_spent\":{},\"execution_time_ms\":{},\"status\":\"{}\",\"return_values_count\":{},\"events_count\":{}}}}}",
            transaction_hash,
            contract_address.to_hex_literal(),
            function_name,
            caller.to_hex_literal(),
            result.gas_used,
            result.gas_used,
            start_time.elapsed().as_millis(),
            trace.status,
            result.return_values.len(),
            trace.events.len()
        );

        let _ = tx.try_send(notification);

        info!("Function call completed with gas used: {} and {} return values", 
              result.gas_used, result.return_values.len());

        Ok(result.return_values)
    }

    /// Get contract information
    pub fn get_contract_info(&self, contract_address: &SmartContractAddress) -> Result<String, String> {
        // Load contract metadata
        let metadata = self.storage.load_contract_metadata(contract_address)
            .map_err(|e| format!("Failed to load metadata: {:?}", e))?;

        match metadata {
            Some(meta) => {
                // Get gas usage statistics
                let current_time = chrono::Utc::now().timestamp() as u64;
                let one_day_ago = current_time - 86400; // 24 hours
                let total_kari_spent = self.storage.get_contract_kari_spent(
                    contract_address, one_day_ago, current_time
                ).unwrap_or(0);

                // Get storage size
                let storage_size = self.storage.get_contract_storage_size(contract_address)
                    .unwrap_or(0);

                let info = format!(
                    "{{\"contract_address\":\"{}\",\"name\":\"{}\",\"version\":\"{}\",\"deployer\":\"{}\",\"deployed_at\":{},\"bytecode_size\":{},\"storage_size\":{},\"kari_spent_24h\":{},\"compiler_version\":\"{}\"}}",
                    contract_address.to_hex_literal(),
                    meta.name,
                    meta.version,
                    meta.deployer.to_hex_literal(),
                    meta.deployed_at,
                    meta.bytecode_size,
                    storage_size,
                    total_kari_spent,
                    meta.compiler_version
                );

                Ok(info)
            },
            None => Err("Contract not found".to_string()),
        }
    }

    /// List all deployed contracts
    pub fn list_deployed_contracts(&self) -> Vec<(Address, SmartContractAddress)> {
        if let Ok(contracts) = self.deployed_contracts.read() {
            contracts.iter().map(|(k, v)| (*k, v.clone())).collect()
        } else {
            Vec::new()
        }
    }

    /// Get execution history for a contract
    pub fn get_execution_history(&self, contract_address: &SmartContractAddress, limit: usize) -> Vec<ExecutionTrace> {
        if let Ok(history) = self.execution_history.read() {
            history.iter()
                .filter(|trace| trace.contract_address == *contract_address)
                .rev()
                .take(limit)
                .cloned()
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Estimate gas cost for a function call (simulation mode)
    pub async fn estimate_gas(
        &self,
        contract_address: SmartContractAddress,
        function_name: String,
        arguments: Vec<Vec<u8>>,
        caller: Address,
    ) -> Result<u64, String> {
        info!("Estimating gas for function {} on contract {}", 
              function_name, contract_address.to_hex_literal());

        // Convert address format
        let vm_contract_address = Address::from_bytes(&contract_address.bytes)
            .map_err(|e| format!("Invalid contract address: {}", e))?;

        // Use VM's gas estimation
        let estimated_gas = self.vm.estimate_gas(
            vm_contract_address,
            function_name,
            arguments,
            caller,
        ).map_err(|e| format!("Gas estimation failed: {:?}", e))?;

        info!("Estimated gas: {}", estimated_gas);
        Ok(estimated_gas)
    }
}
