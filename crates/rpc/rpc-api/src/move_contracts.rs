//! Move contract RPC API for Kanari blockchain

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use jsonrpc_core::{Error, ErrorCode, Result};
use jsonrpc_derive::rpc;
use log::{info, error, debug};

use mona_types::address::Address;
use mona_vm::{MonaVM, VMConfig, GasParameters, VMStorage};
use mona_storage::{SmartContractStorage, SmartContractAddress, RocksDBStorage, ExecutionTrace, GasUsageRecord};
use mona_blockchain::block::{Transaction, TransactionType, ContractTransaction};

/// Move contract deployment request
#[derive(Debug, Deserialize)]
pub struct DeployContractRequest {
    pub source_code: String,
    pub dependencies: Vec<String>,
    pub deployer: String,
    pub gas_limit: u64,
    pub max_kari: u64,
    pub private_key: Option<String>,
}

/// Move contract function call request
#[derive(Debug, Deserialize)]
pub struct CallContractRequest {
    pub contract_address: String,
    pub function_name: String,
    pub arguments: Vec<String>, // JSON-encoded arguments
    pub caller: String,
    pub gas_limit: u64,
    pub max_kari: u64,
    pub private_key: Option<String>,
}

/// Contract deployment response
#[derive(Debug, Serialize)]
pub struct DeployContractResponse {
    pub transaction_hash: String,
    pub contract_address: String,
    pub gas_used: u64,
    pub kari_spent: u64,
    pub deployment_timestamp: u64,
    pub module_name: String,
    pub status: String,
}

/// Contract function call response
#[derive(Debug, Serialize)]
pub struct CallContractResponse {
    pub transaction_hash: String,
    pub success: bool,
    pub gas_used: u64,
    pub kari_spent: u64,
    pub return_values: Vec<String>,
    pub events: Vec<ContractEventResponse>,
    pub error_message: Option<String>,
}

/// Contract event response
#[derive(Debug, Serialize)]
pub struct ContractEventResponse {
    pub event_type: String,
    pub data: String, // hex-encoded
    pub contract_address: String,
}

/// Contract information response
#[derive(Debug, Serialize)]
pub struct ContractInfoResponse {
    pub address: String,
    pub name: String,
    pub version: String,
    pub deployer: String,
    pub deployed_at: u64,
    pub bytecode_size: usize,
    pub compiler_version: String,
    pub gas_used_deployment: u64,
    pub storage_size: u64,
    pub kari_spent_24h: u64,
}

/// Gas estimation response
#[derive(Debug, Serialize)]
pub struct GasEstimationResponse {
    pub estimated_gas: u64,
    pub estimated_kari_cost: u64,
    pub gas_price: u64, // Kari per gas unit
}

/// Execution history response
#[derive(Debug, Serialize)]
pub struct ExecutionHistoryResponse {
    pub executions: Vec<ExecutionTraceResponse>,
    pub total_count: usize,
}

/// Execution trace response
#[derive(Debug, Serialize)]
pub struct ExecutionTraceResponse {
    pub transaction_hash: String,
    pub function_name: String,
    pub caller: String,
    pub gas_used: u64,
    pub kari_spent: u64,
    pub execution_time_ms: u64,
    pub status: String,
    pub timestamp: u64,
    pub events_count: usize,
    pub error_message: Option<String>,
}

/// Move contract RPC trait
#[rpc(server)]
pub trait MoveContractRpc {
    /// Deploy a Move smart contract
    #[rpc(name = "move_deployContract")]
    fn deploy_contract(&self, request: DeployContractRequest) -> Result<DeployContractResponse>;

    /// Call a function on a deployed Move contract
    #[rpc(name = "move_callContract")]
    fn call_contract(&self, request: CallContractRequest) -> Result<CallContractResponse>;

    /// Get contract information
    #[rpc(name = "move_getContractInfo")]
    fn get_contract_info(&self, contract_address: String) -> Result<ContractInfoResponse>;

    /// Estimate gas for a contract operation
    #[rpc(name = "move_estimateGas")]
    fn estimate_gas(&self, request: CallContractRequest) -> Result<GasEstimationResponse>;

    /// Get contract execution history
    #[rpc(name = "move_getExecutionHistory")]
    fn get_execution_history(&self, contract_address: String, limit: Option<usize>) -> Result<ExecutionHistoryResponse>;

    /// List deployed contracts (paginated)
    #[rpc(name = "move_listContracts")]
    fn list_contracts(&self, page: Option<u32>, per_page: Option<u32>) -> Result<Vec<String>>;

    /// Get contract gas usage statistics
    #[rpc(name = "move_getGasUsage")]
    fn get_gas_usage(&self, contract_address: String, from_timestamp: Option<u64>, to_timestamp: Option<u64>) -> Result<Vec<GasUsageRecord>>;

    /// Get Move module information
    #[rpc(name = "move_getModuleInfo")]
    fn get_module_info(&self, contract_address: String, module_id: String) -> Result<serde_json::Value>;
}

/// Move contract RPC implementation
pub struct MoveContractRpcImpl {
    vm: Arc<MonaVM>,
    storage: Arc<SmartContractStorage>,
    gas_params: Arc<GasParameters>,
}

impl MoveContractRpcImpl {
    /// Create a new Move contract RPC implementation
    pub fn new(
        storage_path: std::path::PathBuf,
    ) -> Result<Self> {
        // Initialize storage
        let storage = Arc::new(
            RocksDBStorage::new(storage_path)
                .map_err(|e| Error::new(ErrorCode::InternalError))?
        );
        let smart_contract_storage = Arc::new(
            SmartContractStorage::new(storage, 50 * 1024 * 1024) // 50MB cache
        );

        // Initialize VM
        let gas_params = Arc::new(GasParameters::default());
        let vm_storage = Arc::new(VMStorage::new(smart_contract_storage.clone()));
        let vm_config = VMConfig {
            max_gas_per_txn: 50_000_000, // Higher limit for RPC
            max_memory_per_txn: 10 * 1024 * 1024, // 10MB
            max_stack_depth: 2000,
            enable_tracing: true,
        };
        
        let vm = Arc::new(
            MonaVM::new(vm_config, gas_params.clone(), vm_storage)
                .map_err(|e| Error::new(ErrorCode::InternalError))?
        );

        Ok(Self {
            vm,
            storage: smart_contract_storage,
            gas_params,
        })
    }
}

impl MoveContractRpc for MoveContractRpcImpl {
    fn deploy_contract(&self, request: DeployContractRequest) -> Result<DeployContractResponse> {
        info!("RPC: Deploying contract for {}", request.deployer);
        
        // Parse deployer address
        let deployer = Address::from_str(&request.deployer)
            .map_err(|_| Error::invalid_params("Invalid deployer address"))?;

        // Deploy contract using VM
        let deployment_info = self.vm.deploy_contract(
            request.source_code,
            request.dependencies,
            deployer,
            request.gas_limit,
        ).map_err(|e| {
            error!("Contract deployment failed: {:?}", e);
            Error::new(ErrorCode::InternalError)
        })?;

        // Generate transaction hash (simplified)
        let transaction_hash = format!("{:x}", blake3::hash(
            format!("deploy:{}:{}", deployment_info.contract_address.to_hex_literal(), deployment_info.timestamp).as_bytes()
        ));

        info!("Contract deployed successfully at {}", deployment_info.contract_address.to_hex_literal());

        Ok(DeployContractResponse {
            transaction_hash,
            contract_address: deployment_info.contract_address.to_hex_literal(),
            gas_used: deployment_info.gas_used,
            kari_spent: deployment_info.gas_used, // 1:1 ratio
            deployment_timestamp: deployment_info.timestamp,
            module_name: deployment_info.module_name,
            status: "success".to_string(),
        })
    }

    fn call_contract(&self, request: CallContractRequest) -> Result<CallContractResponse> {
        info!("RPC: Calling function {} on contract {}", request.function_name, request.contract_address);
        
        // Parse addresses
        let contract_address = Address::from_str(&request.contract_address)
            .map_err(|_| Error::invalid_params("Invalid contract address"))?;
        let caller = Address::from_str(&request.caller)
            .map_err(|_| Error::invalid_params("Invalid caller address"))?;

        // Parse arguments (simplified JSON parsing)
        let arguments: Vec<Vec<u8>> = request.arguments.iter()
            .map(|arg| arg.as_bytes().to_vec())
            .collect();

        // Execute function call
        let result = self.vm.call_function(
            contract_address,
            request.function_name.clone(),
            arguments,
            caller,
            request.gas_limit,
        ).map_err(|e| {
            error!("Function call failed: {:?}", e);
            Error::new(ErrorCode::InternalError)
        })?;

        // Generate transaction hash
        let transaction_hash = format!("{:x}", blake3::hash(
            format!("call:{}:{}:{}", contract_address.to_hex_literal(), request.function_name, chrono::Utc::now().timestamp()).as_bytes()
        ));

        // Convert return values to hex strings
        let return_values: Vec<String> = result.return_values.iter()
            .map(|val| hex::encode(val))
            .collect();

        // Convert events
        let events: Vec<ContractEventResponse> = result.events.iter()
            .map(|event| ContractEventResponse {
                event_type: event.event_type.clone(),
                data: hex::encode(&event.data),
                contract_address: contract_address.to_hex_literal(),
            })
            .collect();

        info!("Function call completed: success={}, gas_used={}", result.success, result.gas_used);

        Ok(CallContractResponse {
            transaction_hash,
            success: result.success,
            gas_used: result.gas_used,
            kari_spent: result.gas_used,
            return_values,
            events,
            error_message: result.error_message,
        })
    }

    fn get_contract_info(&self, contract_address: String) -> Result<ContractInfoResponse> {
        debug!("RPC: Getting contract info for {}", contract_address);
        
        let address = SmartContractAddress::from_hex(&contract_address)
            .map_err(|_| Error::invalid_params("Invalid contract address"))?;

        let metadata = self.storage.load_contract_metadata(&address)
            .map_err(|e| {
                error!("Failed to load contract metadata: {:?}", e);
                Error::new(ErrorCode::InternalError)
            })?
            .ok_or_else(|| Error::invalid_params("Contract not found"))?;

        // Get additional statistics
        let current_time = chrono::Utc::now().timestamp() as u64;
        let one_day_ago = current_time - 86400;
        let kari_spent_24h = self.storage.get_contract_kari_spent(&address, one_day_ago, current_time)
            .unwrap_or(0);
        let storage_size = self.storage.get_contract_storage_size(&address)
            .unwrap_or(0);

        Ok(ContractInfoResponse {
            address: contract_address,
            name: metadata.name,
            version: metadata.version,
            deployer: metadata.deployer.to_hex_literal(),
            deployed_at: metadata.deployed_at,
            bytecode_size: metadata.bytecode_size,
            compiler_version: metadata.compiler_version,
            gas_used_deployment: metadata.gas_used_deployment,
            storage_size,
            kari_spent_24h,
        })
    }

    fn estimate_gas(&self, request: CallContractRequest) -> Result<GasEstimationResponse> {
        debug!("RPC: Estimating gas for function {} on contract {}", request.function_name, request.contract_address);
        
        // Parse addresses
        let contract_address = Address::from_str(&request.contract_address)
            .map_err(|_| Error::invalid_params("Invalid contract address"))?;
        let caller = Address::from_str(&request.caller)
            .map_err(|_| Error::invalid_params("Invalid caller address"))?;

        // Parse arguments
        let arguments: Vec<Vec<u8>> = request.arguments.iter()
            .map(|arg| arg.as_bytes().to_vec())
            .collect();

        // Estimate gas
        let estimated_gas = self.vm.estimate_gas(
            contract_address,
            request.function_name,
            arguments,
            caller,
        ).map_err(|e| {
            error!("Gas estimation failed: {:?}", e);
            Error::new(ErrorCode::InternalError)
        })?;

        Ok(GasEstimationResponse {
            estimated_gas,
            estimated_kari_cost: estimated_gas, // 1:1 ratio
            gas_price: 1,
        })
    }

    fn get_execution_history(&self, contract_address: String, limit: Option<usize>) -> Result<ExecutionHistoryResponse> {
        debug!("RPC: Getting execution history for contract {}", contract_address);
        
        let address = SmartContractAddress::from_hex(&contract_address)
            .map_err(|_| Error::invalid_params("Invalid contract address"))?;

        let limit = limit.unwrap_or(50).min(1000); // Cap at 1000

        // This is a placeholder - in a real implementation, you'd query the storage
        // for execution traces
        Ok(ExecutionHistoryResponse {
            executions: Vec::new(),
            total_count: 0,
        })
    }

    fn list_contracts(&self, page: Option<u32>, per_page: Option<u32>) -> Result<Vec<String>> {
        debug!("RPC: Listing contracts");
        
        // This is a placeholder - in a real implementation, you'd maintain
        // an index of deployed contracts
        Ok(Vec::new())
    }

    fn get_gas_usage(&self, contract_address: String, from_timestamp: Option<u64>, to_timestamp: Option<u64>) -> Result<Vec<GasUsageRecord>> {
        debug!("RPC: Getting gas usage for contract {}", contract_address);
        
        let address = SmartContractAddress::from_hex(&contract_address)
            .map_err(|_| Error::invalid_params("Invalid contract address"))?;

        let current_time = chrono::Utc::now().timestamp() as u64;
        let from_time = from_timestamp.unwrap_or(current_time - 86400); // Default: last 24h
        let to_time = to_timestamp.unwrap_or(current_time);

        let gas_records = self.storage.load_gas_usage_records(&address, from_time, to_time)
            .map_err(|e| {
                error!("Failed to load gas usage records: {:?}", e);
                Error::new(ErrorCode::InternalError)
            })?;

        Ok(gas_records)
    }

    fn get_module_info(&self, contract_address: String, module_id: String) -> Result<serde_json::Value> {
        debug!("RPC: Getting module info for {} in contract {}", module_id, contract_address);
        
        let address = SmartContractAddress::from_hex(&contract_address)
            .map_err(|_| Error::invalid_params("Invalid contract address"))?;

        let module_info = self.storage.load_move_module(&address, &module_id)
            .map_err(|e| {
                error!("Failed to load module info: {:?}", e);
                Error::new(ErrorCode::InternalError)
            })?
            .ok_or_else(|| Error::invalid_params("Module not found"))?;

        // Convert to JSON
        Ok(serde_json::to_value(module_info)
            .map_err(|_| Error::new(ErrorCode::InternalError))?)
    }
}

/// Register Move contract RPC methods
pub fn register_move_contract_rpc(
    io: &mut jsonrpc_core::IoHandler,
    storage_path: std::path::PathBuf,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let rpc_impl = MoveContractRpcImpl::new(storage_path)?;
    io.extend_with(rpc_impl.to_delegate());
    info!("Move contract RPC methods registered");
    Ok(())
}
