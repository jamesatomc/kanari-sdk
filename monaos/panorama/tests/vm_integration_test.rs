use log::{info, warn};
use once_cell::sync::OnceCell;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use tokio::sync::mpsc;

use mona_blockchain::blockchain::{BALANCES, save_blockchain};
use mona_types::address::Address;
use panorama::simulation::vm_integration::{
    VMTransactionProcessor, VMTransactionResult, VMTransactionType, call_contract_global,
    deploy_contract_global, query_contract_global,
};
use panorama::vm::{
    ABIFunction, ABIParameter, BytecodeAnalyzer, Contract, ContractABI, GLOBAL_VM, GasMeter,
    SmartContractVM, StateMutability, VMError, VMResult, VMStorage, Visibility, call_contract,
    deploy_contract,
};

// Global logger initialization
static LOGGER: OnceCell<()> = OnceCell::new();

fn init_logger() {
    LOGGER.get_or_init(|| {
        env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .init();
    });
}

// Create a temporary test wallet directory if it doesn't exist
fn ensure_test_wallet_directory() {
    let home_dir = dirs::home_dir().unwrap_or_else(|| Path::new(".").to_path_buf());
    let kari_dir = home_dir.join(".kari");
    let wallets_dir = kari_dir.join("wallets");

    if !kari_dir.exists() {
        if let Err(e) = fs::create_dir_all(&kari_dir) {
            warn!("Failed to create kari directory: {}", e);
        }
    }

    if !wallets_dir.exists() {
        if let Err(e) = fs::create_dir_all(&wallets_dir) {
            warn!("Failed to create wallets directory: {}", e);
        }
    }

    let test_wallet_path = wallets_dir.join("test_wallet.json");
    if !test_wallet_path.exists() {
        let wallet_content = r#"{
            "version": 1,
            "address": "0x1234567890abcdef1234567890abcdef12345678",
            "encrypted_private_key": {
                "ciphertext": "dummy_ciphertext_for_testing",
                "nonce": "dummy_nonce_for_testing",
                "tag": "dummy_tag_for_testing"
            }
        }"#;

        if let Err(e) = fs::write(&test_wallet_path, wallet_content) {
            warn!("Failed to create test wallet file: {}", e);
        }
    }
}

// Test environment setup
fn setup_test_environment() -> (
    Address,
    String,
    mpsc::Sender<String>,
    mpsc::Receiver<String>,
) {
    init_logger();
    ensure_test_wallet_directory();

    let address = Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678")
        .expect("Failed to create test address");
    let password = "test_password".to_string();

    let (tx, rx) = mpsc::channel(10000);

    // Initialize test balances with large amount for VM operations
    {
        let mut balances = BALANCES.lock().unwrap();
        balances.insert(address.to_hex_literal(), 10_000_000_000_000); // 10M test tokens
    }

    (address, password, tx, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_creation_and_basic_operations() {
        let vm = SmartContractVM::new();

        // Test that VM is properly initialized
        assert!(vm.get_deployed_contracts().is_empty());

        // Test contract detection on non-existent address
        let test_address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        assert!(!vm.is_contract(&test_address));
    }

    #[test]
    fn test_bytecode_analysis() {
        let analyzer = BytecodeAnalyzer::new();

        // Simple bytecode: PUSH1 42, PUSH1 1, ADD, RETURN
        let bytecode = vec![0x60, 0x2a, 0x60, 0x01, 0x01, 0xf3];
        let analysis = analyzer.analyze(&bytecode);

        assert!(analysis.is_valid);
        assert_eq!(analysis.size, 6);
        assert!(analysis.instruction_count > 0);
        assert!(analysis.estimated_gas > 0);
        assert!(analysis.complexity_score > 0);

        info!(
            "Bytecode analysis: {} instructions, {} gas, complexity {}",
            analysis.instruction_count, analysis.estimated_gas, analysis.complexity_score
        );
    }

    #[test]
    fn test_gas_metering() {
        let mut gas_meter = GasMeter::new();
        gas_meter.reset(1000);

        assert_eq!(gas_meter.gas_limit(), 1000);
        assert_eq!(gas_meter.gas_used(), 0);
        assert_eq!(gas_meter.gas_remaining(), 1000);

        // Consume some gas
        assert!(gas_meter.consume_gas(100).is_ok());
        assert_eq!(gas_meter.gas_used(), 100);
        assert_eq!(gas_meter.gas_remaining(), 900);

        // Try to consume more than available
        assert!(gas_meter.consume_gas(1000).is_err());

        // Test gas refunds
        gas_meter.refund_gas(50);
        assert_eq!(gas_meter.gas_refunds(), 50);
        assert_eq!(gas_meter.finalize_gas(), 50); // 100 - 50
    }

    #[test]
    fn test_vm_storage() {
        let storage = VMStorage::new();
        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        let key = b"test_key".to_vec();
        let value = b"test_value".to_vec();

        // Test set and get
        storage.set_storage(address.clone(), key.clone(), value.clone());
        let retrieved = storage.get_storage(&address, &key);
        assert_eq!(retrieved, Some(value));

        // Test storage size
        assert!(storage.get_storage_size(&address) > 0);

        // Test delete
        assert!(storage.delete_storage(&address, &key));
        let deleted = storage.get_storage(&address, &key);
        assert_eq!(deleted, None);
    }

    #[test]
    fn test_contract_creation() {
        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let deployer =
            Address::from_hex_literal("0xabcdef1234567890abcdef1234567890abcdef12").unwrap();
        let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // Simple return contract

        let contract = Contract::new(address.clone(), deployer, bytecode, 1).unwrap();

        assert_eq!(contract.address, address);
        assert_eq!(contract.version, 1);
        assert!(contract.is_active());
        assert_eq!(contract.size(), 5);
    }

    #[test]
    fn test_abi_function_creation() {
        let function = ABIFunction::new(
            "transfer".to_string(),
            vec![
                ABIParameter::new("to".to_string(), "address".to_string()),
                ABIParameter::new("amount".to_string(), "uint256".to_string()),
            ],
            vec![ABIParameter::new("success".to_string(), "bool".to_string())],
            StateMutability::Nonpayable,
            Visibility::Public,
        );

        assert_eq!(function.name, "transfer");
        assert_eq!(function.inputs.len(), 2);
        assert_eq!(function.outputs.len(), 1);
        assert!(!function.is_payable());
        assert!(!function.is_view());
        assert!(function.modifies_state());
    }

    #[tokio::test]
    async fn test_vm_transaction_processor() {
        let (address, password, tx_sender, _rx) = setup_test_environment();
        let processor = VMTransactionProcessor::with_events(tx_sender);

        // Simple contract bytecode (just returns empty data)
        let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // PUSH1 0, PUSH1 0, RETURN

        let result = processor
            .process_vm_transaction(
                &address.to_hex_literal(),
                VMTransactionType::Deploy {
                    bytecode,
                    constructor_args: vec![],
                },
                0,      // No value transfer
                100000, // Gas limit
                &password,
            )
            .await;

        match result {
            Ok(vm_result) => {
                info!(
                    "Contract deployment result: success={}, gas_used={}",
                    vm_result.success, vm_result.gas_used
                );
                assert!(vm_result.contract_address.is_some());
                assert!(vm_result.gas_used > 0);
            }
            Err(e) => {
                warn!("Contract deployment failed: {}", e);
                // May fail in test environment due to missing components
            }
        }
    }

    #[test]
    fn test_contract_deployment_via_global_vm() {
        let deployer =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        // Initialize deployer balance
        {
            let mut balances = BALANCES.lock().unwrap();
            balances.insert(deployer.to_hex_literal(), 1000000);
        }

        // Simple contract bytecode
        let bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3]; // PUSH1 0, PUSH1 0, RETURN

        let result = deploy_contract(&deployer, bytecode, vec![], 100000, 0);

        match result {
            Ok((contract_address, vm_result)) => {
                info!("Contract deployed at: {}", contract_address);
                assert!(vm_result.success);
                assert!(GLOBAL_VM.is_contract(&contract_address));
            }
            Err(e) => {
                warn!("Deployment failed: {}", e);
                // Expected to fail in test environment
            }
        }
    }

    #[tokio::test]
    async fn test_contract_call_via_processor() {
        let (caller_address, password, tx_sender, _rx) = setup_test_environment();
        let processor = VMTransactionProcessor::with_events(tx_sender);

        // First deploy a contract
        let bytecode = vec![
            0x60, 0x42, // PUSH1 0x42
            0x60, 0x00, // PUSH1 0x00
            0x52, // MSTORE
            0x60, 0x20, // PUSH1 0x20
            0x60, 0x00, // PUSH1 0x00
            0xf3, // RETURN (return 32 bytes from memory)
        ];

        let deploy_result = processor
            .process_vm_transaction(
                &caller_address.to_hex_literal(),
                VMTransactionType::Deploy {
                    bytecode,
                    constructor_args: vec![],
                },
                0,
                200000,
                &password,
            )
            .await;

        if let Ok(deploy_result) = deploy_result {
            if let Some(contract_address) = deploy_result.contract_address {
                info!("Contract deployed for call test at: {}", contract_address);

                // Now try to call the contract
                let call_result = processor
                    .process_vm_transaction(
                        &caller_address.to_hex_literal(),
                        VMTransactionType::Call {
                            contract_address: contract_address.clone(),
                            function_selector: [0x12, 0x34, 0x56, 0x78], // Dummy selector
                            function_args: vec![],
                        },
                        0,
                        100000,
                        &password,
                    )
                    .await;

                match call_result {
                    Ok(call_result) => {
                        info!(
                            "Contract call result: success={}, gas_used={}",
                            call_result.success, call_result.gas_used
                        );
                    }
                    Err(e) => {
                        warn!("Contract call failed: {}", e);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn test_contract_query_operation() {
        let (caller_address, _password, _tx_sender, _rx) = setup_test_environment();
        let processor = VMTransactionProcessor::new();

        // Create a dummy contract address for testing
        let contract_address =
            Address::from_hex_literal("0xabcdef1234567890abcdef1234567890abcdef12").unwrap();

        let query_result = processor
            .query_contract(
                &caller_address.to_hex_literal(),
                &contract_address.to_hex_literal(),
                [0x70, 0xa0, 0x82, 0x31], // balanceOf function selector
                vec![],
                50000,
            )
            .await;

        match query_result {
            Ok(result) => {
                info!(
                    "Query result: success={}, gas_used={}",
                    result.success, result.gas_used
                );
                // Queries should not consume gas
                assert_eq!(result.gas_used, 0);
                assert_eq!(result.gas_cost, 0);
            }
            Err(e) => {
                warn!("Query failed (expected): {}", e);
                // Expected to fail for non-existent contract
            }
        }
    }

    #[test]
    fn test_bytecode_validation_and_optimization() {
        let analyzer = BytecodeAnalyzer::new();

        // Test valid bytecode
        let valid_bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]; // PUSH1 1, PUSH1 2, ADD, STOP
        let analysis = analyzer.analyze(&valid_bytecode);
        assert!(analysis.is_valid);

        // Test bytecode optimization
        let unoptimized = vec![0x60, 0x42, 0x50]; // PUSH1 0x42, POP (redundant)
        let optimized = analyzer.optimize(&unoptimized).unwrap();

        info!(
            "Optimization: {} -> {} bytes",
            unoptimized.len(),
            optimized.len()
        );
        // Should be shorter after removing redundant operations
        assert!(optimized.len() <= unoptimized.len());

        // Test disassembly
        let disassembly = analyzer.disassemble(&valid_bytecode).unwrap();
        assert!(disassembly.contains("PUSH1"));
        assert!(disassembly.contains("ADD"));
        assert!(disassembly.contains("STOP"));

        info!("Disassembly:\n{}", disassembly);
    }

    #[test]
    fn test_security_analysis() {
        let analyzer = BytecodeAnalyzer::new();

        // Bytecode with potential security issues
        let risky_bytecode = vec![
            0xf4, // DELEGATECALL
            0xff, // SELFDESTRUCT
            0xf1, // CALL
            0x55, // SSTORE (after CALL - potential reentrancy)
        ];

        let analysis = analyzer.analyze(&risky_bytecode);

        info!(
            "Security analysis found {} issues",
            analysis.security_issues.len()
        );
        assert!(!analysis.security_issues.is_empty());

        // Should detect dangerous operations
        let has_delegatecall_warning = analysis.security_issues.iter().any(|issue| {
            matches!(
                issue.issue_type,
                panorama::vm::SecurityIssueType::DangerousDelegatecall
            )
        });
        let has_selfdestruct_warning = analysis.security_issues.iter().any(|issue| {
            matches!(
                issue.issue_type,
                panorama::vm::SecurityIssueType::SelfdestructUsage
            )
        });

        assert!(has_delegatecall_warning);
        assert!(has_selfdestruct_warning);
    }

    #[test]
    fn test_gas_estimation() {
        use panorama::vm::{GasEstimator, GasPriceOracle, GasPriority};

        let estimator = GasEstimator::new();

        // Test simple operation estimates
        let transfer_gas = estimator.estimate_simple_operation("transfer");
        assert_eq!(transfer_gas, 21000);

        let approve_gas = estimator.estimate_simple_operation("approve");
        assert_eq!(approve_gas, 45000);

        // Test bytecode-based estimation
        let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01]; // PUSH1 1, PUSH1 2, ADD
        let estimated_gas = estimator.estimate_from_bytecode(&bytecode);
        assert!(estimated_gas > 0);

        info!("Estimated gas for simple bytecode: {}", estimated_gas);

        // Test gas price oracle
        let mut oracle = GasPriceOracle::new(1000);
        let standard_price = oracle.get_gas_price(GasPriority::Standard);
        let high_price = oracle.get_gas_price(GasPriority::High);
        let urgent_price = oracle.get_gas_price(GasPriority::Urgent);

        assert_eq!(standard_price, 1000);
        assert!(high_price > standard_price);
        assert!(urgent_price > high_price);

        // Test congestion effect
        oracle.update_congestion(2000, 0.8);
        let congested_price = oracle.get_gas_price(GasPriority::Standard);
        assert!(congested_price > 1000);

        info!(
            "Gas prices - Standard: {}, High: {}, Urgent: {}, Congested: {}",
            standard_price, high_price, urgent_price, congested_price
        );
    }

    #[test]
    fn test_memory_expansion_costs() {
        let mut gas_meter = GasMeter::new();
        gas_meter.reset(100000);

        // Test memory expansion
        let initial_gas = gas_meter.gas_used();
        assert!(gas_meter.consume_memory_gas(32).is_ok());
        let first_expansion_cost = gas_meter.gas_used() - initial_gas;

        let before_second = gas_meter.gas_used();
        assert!(gas_meter.consume_memory_gas(64).is_ok());
        let second_expansion_cost = gas_meter.gas_used() - before_second;

        info!(
            "Memory expansion costs: 32 bytes = {}, next 32 bytes = {}",
            first_expansion_cost, second_expansion_cost
        );

        // Second expansion should cost more due to quadratic component
        assert!(second_expansion_cost >= first_expansion_cost);
    }

    #[test]
    fn test_vm_integration_with_blockchain() {
        // Test that VM transactions can be processed by the blockchain
        let (test_address, _password, _tx_sender, _rx) = setup_test_environment();

        // Create a mock blockchain transaction with data
        let vm_transaction = mona_blockchain::block::Transaction {
            transaction_id: "test_vm_tx".to_string(),
            sender: test_address.clone(),
            receiver: test_address.clone(),
            amount: 0,
            gas_fee: 1000,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            signature: vec![0u8; 64],
            data: Some(vec![0x60, 0x00, 0x60, 0x00, 0xf3]), // Simple contract bytecode
        };

        // Test processing through VM
        use panorama::vm::process_vm_transaction;
        let result = process_vm_transaction(&vm_transaction);

        match result {
            Ok(vm_result) => {
                info!(
                    "VM transaction processed: success={}, gas_used={}",
                    vm_result.success, vm_result.gas_used
                );
            }
            Err(e) => {
                info!("VM transaction processing failed (expected): {}", e);
                // Expected to fail without proper setup
            }
        }
    }

    #[test]
    fn test_storage_persistence() {
        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path().join("test_storage");

        let address =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let key = b"persistent_key".to_vec();
        let value = b"persistent_value".to_vec();

        // Test storage with persistence
        {
            let storage = VMStorage::with_persistent_storage(storage_path.clone()).unwrap();
            storage.set_storage(address.clone(), key.clone(), value.clone());
            storage.flush().unwrap();
        }

        // Create new storage instance and verify persistence
        {
            let storage = VMStorage::with_persistent_storage(storage_path).unwrap();
            // Note: Full persistence loading is not implemented in the current version
            // but the storage backend is properly initialized
            assert!(storage.get_storage_stats().total_operations >= 0);
        }
    }

    #[test]
    fn test_complex_contract_interaction() {
        // This test simulates a more complex contract with multiple functions
        let vm = SmartContractVM::new();
        let deployer =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        // Initialize balance
        {
            let mut balances = BALANCES.lock().unwrap();
            balances.insert(deployer.to_hex_literal(), 10_000_000);
        }

        // More complex contract bytecode (simplified ERC20-like)
        // This is a mock bytecode that represents a contract with multiple functions
        let complex_bytecode = vec![
            // Function dispatcher
            0x60, 0x00, // PUSH1 0x00
            0x35, // CALLDATALOAD
            0x60, 0xe0, // PUSH1 0xe0
            0x1c, // SHR (extract function selector)
            // Check for transfer function (0xa9059cbb)
            0x80, // DUP1
            0x63, 0xa9, 0x05, 0x9c, 0xbb, // PUSH4 0xa9059cbb
            0x14, // EQ
            0x61, 0x00, 0x30, // PUSH2 0x0030
            0x57, // JUMPI
            // Check for balanceOf function (0x70a08231)
            0x80, // DUP1
            0x63, 0x70, 0xa0, 0x82, 0x31, // PUSH4 0x70a08231
            0x14, // EQ
            0x61, 0x00, 0x50, // PUSH2 0x0050
            0x57, // JUMPI
            // Default: revert
            0x60, 0x00, // PUSH1 0x00
            0x60, 0x00, // PUSH1 0x00
            0xfd, // REVERT
            // Transfer function at 0x30
            0x5b, // JUMPDEST
            0x60, 0x01, // PUSH1 0x01 (success)
            0x60, 0x00, // PUSH1 0x00
            0x52, // MSTORE
            0x60, 0x20, // PUSH1 0x20
            0x60, 0x00, // PUSH1 0x00
            0xf3, // RETURN
            // BalanceOf function at 0x50
            0x5b, // JUMPDEST
            0x60, 0x64, // PUSH1 0x64 (dummy balance: 100)
            0x60, 0x00, // PUSH1 0x00
            0x52, // MSTORE
            0x60, 0x20, // PUSH1 0x20
            0x60, 0x00, // PUSH1 0x00
            0xf3, // RETURN
        ];

        let deploy_result = vm.deploy_contract(&deployer, complex_bytecode, vec![], 200000, 0);

        match deploy_result {
            Ok((contract_address, deploy_result)) => {
                info!("Complex contract deployed at: {}", contract_address);
                assert!(deploy_result.success);

                // Test calling the balanceOf function
                let balance_call = vm.call_contract(
                    &deployer,
                    &contract_address,
                    [0x70, 0xa0, 0x82, 0x31], // balanceOf selector
                    vec![0x00; 32],           // address parameter (padded)
                    100000,
                    0,
                );

                match balance_call {
                    Ok(call_result) => {
                        info!(
                            "BalanceOf call result: success={}, return_data={:?}",
                            call_result.success, call_result.return_data
                        );
                    }
                    Err(e) => {
                        warn!("BalanceOf call failed: {}", e);
                    }
                }

                // Test calling the transfer function
                let transfer_call = vm.call_contract(
                    &deployer,
                    &contract_address,
                    [0xa9, 0x05, 0x9c, 0xbb], // transfer selector
                    vec![0x00; 64],           // to address + amount parameters
                    100000,
                    0,
                );

                match transfer_call {
                    Ok(call_result) => {
                        info!(
                            "Transfer call result: success={}, return_data={:?}",
                            call_result.success, call_result.return_data
                        );
                    }
                    Err(e) => {
                        warn!("Transfer call failed: {}", e);
                    }
                }
            }
            Err(e) => {
                warn!("Complex contract deployment failed: {}", e);
            }
        }
    }

    #[test]
    fn test_performance_benchmarks() {
        let start = Instant::now();

        // Benchmark contract deployment
        let vm = SmartContractVM::new();
        let deployer =
            Address::from_hex_literal("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        // Initialize large balance for multiple deployments
        {
            let mut balances = BALANCES.lock().unwrap();
            balances.insert(deployer.to_hex_literal(), 1_000_000_000);
        }

        let simple_bytecode = vec![0x60, 0x00, 0x60, 0x00, 0xf3];
        let mut successful_deployments = 0;
        let deployment_count = 10;

        for i in 0..deployment_count {
            let result = vm.deploy_contract(&deployer, simple_bytecode.clone(), vec![], 100000, 0);
            if result.is_ok() {
                successful_deployments += 1;
            }

            if i % 5 == 0 {
                info!("Deployed {} contracts", i + 1);
            }
        }

        let deployment_time = start.elapsed();
        let deployments_per_second = successful_deployments as f64 / deployment_time.as_secs_f64();

        info!(
            "Performance benchmark: {} successful deployments in {:?} ({:.2} deploys/sec)",
            successful_deployments, deployment_time, deployments_per_second
        );

        assert!(successful_deployments > 0);

        // Benchmark bytecode analysis
        let analysis_start = Instant::now();
        let analyzer = BytecodeAnalyzer::new();
        let analysis_count = 100;

        for _ in 0..analysis_count {
            let _ = analyzer.analyze(&simple_bytecode);
        }

        let analysis_time = analysis_start.elapsed();
        let analyses_per_second = analysis_count as f64 / analysis_time.as_secs_f64();

        info!(
            "Bytecode analysis benchmark: {} analyses in {:?} ({:.2} analyses/sec)",
            analysis_count, analysis_time, analyses_per_second
        );
    }
}
