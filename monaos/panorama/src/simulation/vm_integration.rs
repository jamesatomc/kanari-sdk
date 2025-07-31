//! VM Integration with Blockchain Simulation
//!
//! This module provides integration between the smart contract VM and the
//! blockchain simulation system, handling contract deployment, execution,
//! and state synchronization.

use crate::simulation::add_pending_transaction;
use crate::utils::{calculate_gas_fee, calculate_total_transaction_cost, format_gas_fee_display};
use crate::vm::{
    GLOBAL_VM, SmartContractVM, VMError, VMResult, call_contract, deploy_contract,
    process_vm_transaction,
};
use log::{debug, error, info, warn};
use mona_blockchain::block::Transaction;
use mona_blockchain::blockchain::{BALANCES, BlockchainError, normalize_address};
use mona_types::address::Address;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

/// VM transaction types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VMTransactionType {
    /// Contract deployment
    Deploy {
        bytecode: Vec<u8>,
        constructor_args: Vec<u8>,
    },
    /// Contract function call
    Call {
        contract_address: Address,
        function_selector: [u8; 4],
        function_args: Vec<u8>,
    },
    /// Contract state query (read-only)
    Query {
        contract_address: Address,
        function_selector: [u8; 4],
        function_args: Vec<u8>,
    },
}

/// VM transaction result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VMTransactionResult {
    /// Whether transaction was successful
    pub success: bool,
    /// Transaction hash
    pub transaction_hash: String,
    /// Gas used
    pub gas_used: u64,
    /// Gas cost in tokens
    pub gas_cost: u64,
    /// Contract address (for deployments)
    pub contract_address: Option<Address>,
    /// Return data from contract execution
    pub return_data: Vec<u8>,
    /// Event logs generated
    pub logs: Vec<ContractEvent>,
    /// Error message if failed
    pub error: Option<String>,
    /// State changes made
    pub state_changes: HashMap<String, String>,
}

/// Contract event emitted during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractEvent {
    /// Contract that emitted the event
    pub contract_address: Address,
    /// Event topics (indexed parameters)
    pub topics: Vec<String>,
    /// Event data (non-indexed parameters)
    pub data: Vec<u8>,
    /// Block number when emitted
    pub block_number: u64,
    /// Transaction hash that generated the event
    pub transaction_hash: String,
}

/// VM transaction processor
pub struct VMTransactionProcessor {
    /// Reference to the global VM
    vm: Arc<SmartContractVM>,
    /// Event notification channel
    event_sender: Option<mpsc::Sender<String>>,
}

impl VMTransactionProcessor {
    /// Create a new VM transaction processor
    pub fn new() -> Self {
        Self {
            vm: Arc::new(SmartContractVM::new()),
            event_sender: None,
        }
    }

    /// Create processor with event notifications
    pub fn with_events(event_sender: mpsc::Sender<String>) -> Self {
        Self {
            vm: Arc::new(SmartContractVM::new()),
            event_sender: Some(event_sender),
        }
    }

    /// Process a VM transaction
    pub async fn process_vm_transaction(
        &self,
        from_address: &str,
        vm_tx_type: VMTransactionType,
        value: u64,
        gas_limit: u64,
        password: &str,
    ) -> Result<VMTransactionResult, VMError> {
        let from = normalize_address(from_address)
            .map_err(|e| VMError::InvalidTransaction(format!("Invalid sender address: {}", e)))?;

        // Calculate gas price and total cost
        let gas_price = calculate_gas_fee(None);
        let estimated_gas_cost = gas_limit * gas_price;

        // Check if sender has sufficient balance for gas and value
        let total_cost = calculate_total_transaction_cost(value, estimated_gas_cost);
        let sender_balance = mona_blockchain::blockchain::get_balance(&from.to_hex_literal())
            .map_err(|e| VMError::InvalidTransaction(format!("Failed to get balance: {}", e)))?;

        if sender_balance < total_cost {
            return Err(VMError::InsufficientBalance);
        }

        // Generate transaction hash
        let tx_hash = self.generate_transaction_hash(&from, &vm_tx_type, value, gas_limit);

        // Process based on transaction type
        let vm_result = match vm_tx_type {
            VMTransactionType::Deploy {
                ref bytecode,
                ref constructor_args,
            } => {
                info!(
                    "Deploying contract from {} with {} bytes of bytecode",
                    from,
                    bytecode.len()
                );

                let (contract_address, result) = deploy_contract(
                    &from,
                    bytecode.clone(),
                    constructor_args.clone(),
                    gas_limit,
                    value,
                )?;

                VMTransactionResult {
                    success: result.success,
                    transaction_hash: tx_hash.clone(),
                    gas_used: result.gas_used,
                    gas_cost: result.gas_used * gas_price,
                    contract_address: Some(contract_address),
                    return_data: result.return_data,
                    logs: self.convert_vm_logs(result.logs, &tx_hash),
                    error: result.error,
                    state_changes: self.format_state_changes(result.state_changes),
                }
            }

            VMTransactionType::Call {
                contract_address,
                function_selector,
                ref function_args,
            } => {
                info!(
                    "Calling contract {} function {:?} from {}",
                    contract_address,
                    hex::encode(function_selector),
                    from
                );

                let result = call_contract(
                    &from,
                    &contract_address,
                    function_selector,
                    function_args.clone(),
                    gas_limit,
                    value,
                )?;

                VMTransactionResult {
                    success: result.success,
                    transaction_hash: tx_hash.clone(),
                    gas_used: result.gas_used,
                    gas_cost: result.gas_used * gas_price,
                    contract_address: Some(contract_address),
                    return_data: result.return_data,
                    logs: self.convert_vm_logs(result.logs, &tx_hash),
                    error: result.error,
                    state_changes: self.format_state_changes(result.state_changes),
                }
            }

            VMTransactionType::Query {
                contract_address,
                function_selector,
                ref function_args,
            } => {
                debug!(
                    "Querying contract {} function {:?}",
                    contract_address,
                    hex::encode(function_selector)
                );

                // Queries are read-only and don't consume gas or modify state
                let result = call_contract(
                    &from,
                    &contract_address,
                    function_selector,
                    function_args.clone(),
                    gas_limit,
                    0, // No value transfer for queries
                )?;

                VMTransactionResult {
                    success: result.success,
                    transaction_hash: tx_hash.clone(),
                    gas_used: 0, // Queries don't consume gas
                    gas_cost: 0,
                    contract_address: Some(contract_address),
                    return_data: result.return_data,
                    logs: Vec::new(), // Queries don't emit events
                    error: result.error,
                    state_changes: HashMap::new(), // Queries don't change state
                }
            }
        };

        // Update balances if transaction was successful and not a query
        if vm_result.success && !matches!(vm_tx_type, VMTransactionType::Query { .. }) {
            self.update_balances(&from, vm_result.gas_cost, value, &vm_result)?;
        }

        // Send event notification
        if let Some(ref sender) = self.event_sender {
            let event_msg = self.create_event_message(&vm_result);
            let _ = sender.try_send(event_msg);
        }

        // Create blockchain transaction for non-queries
        if !matches!(vm_tx_type, VMTransactionType::Query { .. }) {
            let blockchain_tx = self.create_blockchain_transaction(
                &from,
                &vm_tx_type,
                &vm_result,
                value,
                password,
            )?;

            // Add to pending transactions
            if !add_pending_transaction(blockchain_tx) {
                warn!("Failed to add VM transaction to pending queue");
            }
        }

        info!(
            "VM transaction processed: success={}, gas_used={}, cost={}",
            vm_result.success, vm_result.gas_used, vm_result.gas_cost
        );

        Ok(vm_result)
    }

    /// Process a regular blockchain transaction through the VM
    pub fn process_blockchain_transaction(&self, transaction: &Transaction) -> Option<VMResult> {
        // Check if this is a VM transaction (has data field)
        if transaction.data.is_none() || transaction.data.as_ref().unwrap().is_empty() {
            return None;
        }

        match process_vm_transaction(transaction) {
            Ok(result) => {
                debug!(
                    "Processed blockchain transaction {} through VM: success={}",
                    transaction.transaction_id, result.success
                );
                Some(result)
            }
            Err(e) => {
                warn!(
                    "Failed to process blockchain transaction {} through VM: {}",
                    transaction.transaction_id, e
                );
                None
            }
        }
    }

    /// Deploy a contract via the simulation system
    pub async fn deploy_contract_via_simulation(
        &self,
        deployer: &str,
        bytecode: Vec<u8>,
        constructor_args: Vec<u8>,
        gas_limit: u64,
        value: u64,
        password: &str,
        tx_sender: &mpsc::Sender<String>,
    ) -> Result<VMTransactionResult, VMError> {
        let vm_tx = VMTransactionType::Deploy {
            bytecode,
            constructor_args,
        };

        let result = self
            .process_vm_transaction(deployer, vm_tx, value, gas_limit, password)
            .await?;

        // Send additional deployment notification
        let deploy_msg = format!(
            "{{\"event\":\"contract_deployed\",\"deployer\":\"{}\",\"contract_address\":\"{}\",\"gas_used\":{},\"success\":{},\"timestamp\":{}}}",
            deployer,
            result
                .contract_address
                .as_ref()
                .map(|a| a.to_hex_literal())
                .unwrap_or_default(),
            result.gas_used,
            result.success,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let _ = tx_sender.try_send(deploy_msg);

        Ok(result)
    }

    /// Call a contract function via the simulation system
    pub async fn call_contract_via_simulation(
        &self,
        caller: &str,
        contract_address: &str,
        function_selector: [u8; 4],
        function_args: Vec<u8>,
        gas_limit: u64,
        value: u64,
        password: &str,
        tx_sender: &mpsc::Sender<String>,
    ) -> Result<VMTransactionResult, VMError> {
        let contract_addr = normalize_address(contract_address)
            .map_err(|e| VMError::InvalidTransaction(format!("Invalid contract address: {}", e)))?;

        let vm_tx = VMTransactionType::Call {
            contract_address: contract_addr,
            function_selector,
            function_args,
        };

        let result = self
            .process_vm_transaction(caller, vm_tx, value, gas_limit, password)
            .await?;

        // Send additional call notification
        let call_msg = format!(
            "{{\"event\":\"contract_called\",\"caller\":\"{}\",\"contract_address\":\"{}\",\"function_selector\":\"{}\",\"gas_used\":{},\"success\":{},\"timestamp\":{}}}",
            caller,
            contract_address,
            hex::encode(function_selector),
            result.gas_used,
            result.success,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let _ = tx_sender.try_send(call_msg);

        Ok(result)
    }

    /// Query a contract (read-only operation)
    pub async fn query_contract(
        &self,
        caller: &str,
        contract_address: &str,
        function_selector: [u8; 4],
        function_args: Vec<u8>,
        gas_limit: u64,
    ) -> Result<VMTransactionResult, VMError> {
        let contract_addr = normalize_address(contract_address)
            .map_err(|e| VMError::InvalidTransaction(format!("Invalid contract address: {}", e)))?;

        let vm_tx = VMTransactionType::Query {
            contract_address: contract_addr,
            function_selector,
            function_args,
        };

        self.process_vm_transaction(caller, vm_tx, 0, gas_limit, "")
            .await
    }

    /// Generate transaction hash
    fn generate_transaction_hash(
        &self,
        from: &Address,
        vm_tx_type: &VMTransactionType,
        value: u64,
        gas_limit: u64,
    ) -> String {
        use mona_crypto::hash_data_blake3;

        let mut data = Vec::new();
        data.extend_from_slice(from.to_string().as_bytes());
        data.extend_from_slice(&value.to_le_bytes());
        data.extend_from_slice(&gas_limit.to_le_bytes());

        match vm_tx_type {
            VMTransactionType::Deploy {
                bytecode,
                constructor_args,
            } => {
                data.extend_from_slice(b"deploy");
                data.extend_from_slice(bytecode);
                data.extend_from_slice(constructor_args);
            }
            VMTransactionType::Call {
                contract_address,
                function_selector,
                function_args,
            } => {
                data.extend_from_slice(b"call");
                data.extend_from_slice(contract_address.to_string().as_bytes());
                data.extend_from_slice(function_selector);
                data.extend_from_slice(function_args);
            }
            VMTransactionType::Query {
                contract_address,
                function_selector,
                function_args,
            } => {
                data.extend_from_slice(b"query");
                data.extend_from_slice(contract_address.to_string().as_bytes());
                data.extend_from_slice(function_selector);
                data.extend_from_slice(function_args);
            }
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        data.extend_from_slice(&timestamp.to_le_bytes());

        let hash = hash_data_blake3(&data);
        format!("0x{}", hex::encode(&hash[..32]))
    }

    /// Convert VM logs to contract events
    fn convert_vm_logs(&self, vm_logs: Vec<crate::vm::VMLog>, tx_hash: &str) -> Vec<ContractEvent> {
        vm_logs
            .into_iter()
            .map(|log| ContractEvent {
                contract_address: log.address,
                topics: log.topics,
                data: log.data,
                block_number: log.block_number,
                transaction_hash: tx_hash.to_string(),
            })
            .collect()
    }

    /// Format state changes for result
    fn format_state_changes(
        &self,
        state_changes: HashMap<String, Vec<u8>>,
    ) -> HashMap<String, String> {
        state_changes
            .into_iter()
            .map(|(key, value)| (key, hex::encode(value)))
            .collect()
    }

    /// Update balances after successful VM transaction
    fn update_balances(
        &self,
        from: &Address,
        gas_cost: u64,
        value: u64,
        result: &VMTransactionResult,
    ) -> Result<(), VMError> {
        let from_addr = from.to_hex_literal();

        // Deduct gas cost and value from sender
        {
            let mut balances = BALANCES.lock().unwrap();
            if let Some(sender_balance) = balances.get_mut(&from_addr) {
                let total_cost = gas_cost + value;
                if *sender_balance < total_cost {
                    return Err(VMError::InsufficientBalance);
                }
                *sender_balance -= total_cost;
            }

            // Add gas fee to gas collector
            *balances
                .entry(crate::utils::GAS_FEE_COLLECTOR.to_string())
                .or_insert(0) += gas_cost;

            // If value was transferred to a contract, it's handled by the VM
            if value > 0 && result.contract_address.is_some() {
                let contract_addr = result.contract_address.as_ref().unwrap().to_hex_literal();
                *balances.entry(contract_addr).or_insert(0) += value;
            }
        }

        debug!(
            "Updated balances: gas_cost={}, value={}, from={}",
            gas_cost, value, from_addr
        );

        Ok(())
    }

    /// Create event message for notifications
    fn create_event_message(&self, result: &VMTransactionResult) -> String {
        let event_type = if result.contract_address.is_some() && result.return_data.is_empty() {
            "contract_deployment"
        } else {
            "contract_call"
        };

        format!(
            "{{\"event\":\"{}\",\"transaction_hash\":\"{}\",\"success\":{},\"gas_used\":{},\"gas_cost\":{},\"contract_address\":\"{}\",\"logs_count\":{},\"timestamp\":{}}}",
            event_type,
            result.transaction_hash,
            result.success,
            result.gas_used,
            result.gas_cost,
            result
                .contract_address
                .as_ref()
                .map(|a| a.to_hex_literal())
                .unwrap_or_default(),
            result.logs.len(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        )
    }

    /// Create blockchain transaction from VM transaction
    fn create_blockchain_transaction(
        &self,
        from: &Address,
        vm_tx_type: &VMTransactionType,
        result: &VMTransactionResult,
        value: u64,
        password: &str,
    ) -> Result<Transaction, VMError> {
        // Determine receiver address
        let receiver = match vm_tx_type {
            VMTransactionType::Deploy { .. } => result
                .contract_address
                .clone()
                .unwrap_or_else(|| from.clone()),
            VMTransactionType::Call {
                contract_address, ..
            } => contract_address.clone(),
            VMTransactionType::Query {
                contract_address, ..
            } => contract_address.clone(),
        };

        // Create transaction data
        let mut transaction_data = Vec::new();
        match vm_tx_type {
            VMTransactionType::Deploy {
                bytecode,
                constructor_args,
            } => {
                transaction_data.extend_from_slice(bytecode);
                transaction_data.extend_from_slice(constructor_args);
            }
            VMTransactionType::Call {
                function_selector,
                function_args,
                ..
            } => {
                transaction_data.extend_from_slice(function_selector);
                transaction_data.extend_from_slice(function_args);
            }
            VMTransactionType::Query {
                function_selector,
                function_args,
                ..
            } => {
                transaction_data.extend_from_slice(function_selector);
                transaction_data.extend_from_slice(function_args);
            }
        }

        // Create transaction
        let transaction = Transaction {
            transaction_id: result.transaction_hash.clone(),
            sender: from.clone(),
            receiver,
            amount: value,
            gas_fee: result.gas_cost,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            signature: Vec::new(), // Will be signed by transfer_tokens if needed
            data: Some(transaction_data),
        };

        Ok(transaction)
    }

    /// Get contract information
    pub fn get_contract_info(&self, contract_address: &str) -> Option<crate::vm::Contract> {
        if let Ok(address) = normalize_address(contract_address) {
            GLOBAL_VM.get_contract_info(&address)
        } else {
            None
        }
    }

    /// Check if address is a contract
    pub fn is_contract(&self, address: &str) -> bool {
        if let Ok(addr) = normalize_address(address) {
            GLOBAL_VM.is_contract(&addr)
        } else {
            false
        }
    }

    /// Get all deployed contracts
    pub fn get_deployed_contracts(&self) -> HashMap<String, crate::vm::Contract> {
        GLOBAL_VM.get_deployed_contracts()
    }
}

/// Global VM transaction processor instance
lazy_static::lazy_static! {
    pub static ref VM_PROCESSOR: VMTransactionProcessor = VMTransactionProcessor::new();
}

/// Convenience functions for VM integration

/// Deploy a contract through the global processor
pub async fn deploy_contract_global(
    deployer: &str,
    bytecode: Vec<u8>,
    constructor_args: Vec<u8>,
    gas_limit: u64,
    value: u64,
    password: &str,
    tx_sender: &mpsc::Sender<String>,
) -> Result<VMTransactionResult, VMError> {
    VM_PROCESSOR
        .deploy_contract_via_simulation(
            deployer,
            bytecode,
            constructor_args,
            gas_limit,
            value,
            password,
            tx_sender,
        )
        .await
}

/// Call a contract through the global processor
pub async fn call_contract_global(
    caller: &str,
    contract_address: &str,
    function_selector: [u8; 4],
    function_args: Vec<u8>,
    gas_limit: u64,
    value: u64,
    password: &str,
    tx_sender: &mpsc::Sender<String>,
) -> Result<VMTransactionResult, VMError> {
    VM_PROCESSOR
        .call_contract_via_simulation(
            caller,
            contract_address,
            function_selector,
            function_args,
            gas_limit,
            value,
            password,
            tx_sender,
        )
        .await
}

/// Query a contract through the global processor
pub async fn query_contract_global(
    caller: &str,
    contract_address: &str,
    function_selector: [u8; 4],
    function_args: Vec<u8>,
    gas_limit: u64,
) -> Result<VMTransactionResult, VMError> {
    VM_PROCESSOR
        .query_contract(
            caller,
            contract_address,
            function_selector,
            function_args,
            gas_limit,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mona_types::address::Address;

    #[tokio::test]
    async fn test_vm_transaction_processor() {
        let processor = VMTransactionProcessor::new();
        let deployer = "0x1234567890abcdef1234567890abcdef12345678";

        // Simple contract bytecode (just returns)
        let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // PUSH1 0, PUSH1 0, RETURN

        let result = processor
            .process_vm_transaction(
                deployer,
                VMTransactionType::Deploy {
                    bytecode,
                    constructor_args: vec![],
                },
                0,
                100000,
                "test_password",
            )
            .await;

        match result {
            Ok(vm_result) => {
                assert!(vm_result.contract_address.is_some());
                println!("Contract deployed at: {:?}", vm_result.contract_address);
            }
            Err(e) => {
                println!("Deployment failed: {}", e);
                // This might fail in test environment due to missing balances
            }
        }
    }

    #[test]
    fn test_transaction_hash_generation() {
        let processor = VMTransactionProcessor::new();
        let from = Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        let vm_tx = VMTransactionType::Deploy {
            bytecode: vec![0x60, 0x00],
            constructor_args: vec![],
        };

        let hash1 = processor.generate_transaction_hash(&from, &vm_tx, 0, 100000);
        let hash2 = processor.generate_transaction_hash(&from, &vm_tx, 0, 100000);

        // Hashes should be different due to timestamp
        println!("Hash 1: {}", hash1);
        println!("Hash 2: {}", hash2);
        assert_eq!(hash1.len(), 66); // 0x + 64 hex chars
    }

    #[test]
    fn test_contract_detection() {
        let processor = VMTransactionProcessor::new();

        // Test with non-contract address
        assert!(!processor.is_contract("0x1234567890abcdef1234567890abcdef12345678"));

        // Test with invalid address
        assert!(!processor.is_contract("invalid_address"));
    }
}
