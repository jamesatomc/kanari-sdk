//! Example usage of the Move VM error handling and recovery system

use mona_vm::{
    MonaVM, VMConfig, GasParameters, FunctionCall
};
use mona_types::address::Address;
use std::sync::Arc;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    println!("Starting Move VM Error Handling Demo...");

    // Create VM instance with error handling
    let vm = create_vm_with_error_handling()?;
    
    println!("=== Move VM Error Handling and Recovery Demo ===\n");

    // Demo 1: Basic error recovery
    demo_basic_error_recovery(&vm)?;

    // Demo 2: System health monitoring
    demo_system_health_monitoring(&vm)?;

    // Demo 3: Performance monitoring and alerts
    demo_performance_monitoring(&vm)?;

    // Demo 4: State snapshots and recovery
    demo_snapshot_recovery(&vm)?;

    println!("=== Demo completed successfully! ===");
    Ok(())
}

fn create_vm_with_error_handling() -> Result<MonaVM, Box<dyn std::error::Error>> {
    println!("Creating Move VM with enhanced error handling...");
    
    let config = VMConfig::default();
    let gas_params = Arc::new(GasParameters::default());
    let rocks_storage = Arc::new(
        mona_storage::RocksDBStorage::new(PathBuf::from("./demo_vm_data"))?
    );
    
    let vm = MonaVM::new(config, gas_params, rocks_storage)?;
    println!("✓ VM created with recovery and diagnostic systems enabled\n");
    
    Ok(vm)
}

fn demo_basic_error_recovery(vm: &MonaVM) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Demo 1: Basic Error Recovery ---");
    
    // Create a function call that will likely fail (contract doesn't exist)
    let function_call = FunctionCall {
        contract_address: Address::zero(),
        module_name: "nonexistent_module".to_string(),
        function_name: "test_function".to_string(),
        args: vec![],
        caller: Address::zero(),
        gas_limit: 100, // Very low gas limit
    };

    println!("Executing function call that will trigger error recovery...");
    
    match vm.call_function_with_recovery(function_call) {
        Ok(receipt) => {
            println!("✓ Function call succeeded after recovery!");
            println!("  Gas used: {}", receipt.gas_used);
        }
        Err(error) => {
            println!("✗ Function call failed: {}", error);
            println!("  Error was properly handled by recovery system");
        }
    }

    // Check recovery metrics
    let health = vm.get_system_health()?;
    println!("Recovery metrics: {}", 
        serde_json::to_string_pretty(&health["recovery_metrics"])?);
    
    println!();
    Ok(())
}

#[allow(unused_variables)]
fn demo_circuit_breaker(vm: &MonaVM) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Demo 2: Circuit Breaker Functionality ---");
    
    // Note: Circuit breaker functionality would be implemented here
    // For now, just demonstrate basic error handling
    println!("Circuit breaker functionality - placeholder for future implementation");
    
    println!();
    Ok(())
}

fn demo_system_health_monitoring(vm: &MonaVM) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Demo 3: System Health Monitoring ---");
    
    // Generate some activity to create metrics
    println!("Generating VM activity for health monitoring...");
    
    for i in 1..=5 {
        let function_call = FunctionCall {
            contract_address: Address::zero(),
            module_name: format!("module_{}", i),
            function_name: "benchmark_function".to_string(),
            args: vec![],
            caller: Address::zero(),
            gas_limit: 500000 + (i * 100000), // Varying gas limits
        };
        
        let _ = vm.call_function(function_call);
    }

    // Get comprehensive health status
    let health = vm.get_system_health()?;
    println!("System Health Report:");
    println!("{}", serde_json::to_string_pretty(&health)?);
    
    println!();
    Ok(())
}

fn demo_performance_monitoring(vm: &MonaVM) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Demo 4: Performance Monitoring and Alerts ---");
    
    let diagnostic_manager = vm.get_diagnostic_manager();
    
    // Simulate various performance scenarios
    println!("Simulating performance scenarios...");
    
    let scenarios = [
        ("Fast execution", 100000, 50),
        ("Medium execution", 500000, 150),
        ("Slow execution", 800000, 300),
        ("Very slow execution", 950000, 500), // Should trigger alert
    ];

    for (description, gas, time) in scenarios {
        println!("  Simulating: {}", description);
        diagnostic_manager.update_performance_metrics(gas, time)?;
    }

    // Check for generated alerts
    let alerts = vm.get_active_alerts()?;
    println!("Generated {} alerts:", alerts.len());
    
    for alert in alerts {
        println!("  🚨 {}: {} ({})", 
            alert.category, 
            alert.message,
            format!("{:?}", alert.severity)
        );
    }    // Get performance metrics
    #[allow(unused_variables)]
    let metrics = diagnostic_manager.get_performance_metrics()?;
    println!("Performance Metrics collected successfully");
    
    println!();
    Ok(())
}

fn demo_snapshot_recovery(vm: &MonaVM) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Demo 5: State Snapshots and Recovery ---");
    
    // Create initial snapshot
    println!("Creating initial state snapshot...");
    let snapshot_id = vm.create_recovery_snapshot(
        "Demo: Initial state before risky operations".to_string()
    )?;
    println!("✓ Snapshot created with ID: {}", snapshot_id);

    // Simulate some state-changing operations
    println!("Performing state-changing operations...");
    for i in 1..=3 {
        let function_call = FunctionCall {
            contract_address: Address::zero(),
            module_name: "state_changer".to_string(),
            function_name: format!("modify_state_{}", i),
            args: vec![],
            caller: Address::zero(),
            gas_limit: 1000000,
        };
        
        let _ = vm.call_function(function_call);
        println!("  Operation {} completed", i);
    }

    // Simulate a critical error requiring rollback
    println!("Simulating critical error requiring state rollback...");
    
    // Restore from snapshot
    println!("Restoring from snapshot...");
    vm.restore_from_snapshot(&snapshot_id)?;
    println!("✓ State successfully restored from snapshot");
    
    println!();
    Ok(())
}

/// Helper function to demonstrate error classification
fn demo_error_classification() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Error Classification Examples ---");
    
    use mona_vm::{VMError, ExecutionContext};
    use std::time::SystemTime;
    
    let context = ExecutionContext::new(
        Address::zero(),
        Address::zero(),
        1000000,
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    );

    // Create diagnostic manager for error analysis
    let diagnostic_config = mona_vm::diagnostics::DiagnosticConfig {
        max_reports: 1000,
        metrics_retention_hours: 24,
        alert_thresholds: mona_vm::diagnostics::AlertThresholds {
            gas_usage_threshold: 0.8,
            memory_usage_threshold: 0.9,
            error_rate_threshold: 0.1,
            response_time_threshold: 1000,
            storage_latency_threshold: 500,
        },
        enable_detailed_tracing: true,
        enable_performance_profiling: true,
    };
    
    let diagnostic_manager = mona_vm::DiagnosticManager::new(diagnostic_config);

    // Demonstrate different error types and their analysis
    let errors = vec![
        VMError::GasLimitExceeded { used: 1200000, limit: 1000000 },
        VMError::MemoryLimitExceeded { used: 1024 * 1024 * 10, limit: 1024 * 1024 * 8 },
        VMError::ExecutionTimeout { duration_ms: 5000 },
        VMError::RuntimeError { message: "Division by zero".to_string() },
    ];    for error in errors {
        println!("Analyzing error: {:?}", error);
        let report = diagnostic_manager.generate_diagnostic_report(&error, &context, None)?;
        
        // Extract error type from the error variant
        let error_type = match &report.error {
            VMError::GasLimitExceeded { .. } => "GasLimitExceeded",
            VMError::MemoryLimitExceeded { .. } => "MemoryLimitExceeded",
            VMError::ExecutionTimeout { .. } => "ExecutionTimeout",
            VMError::RuntimeError { .. } => "RuntimeError",
            VMError::CompilationError { .. } => "CompilationError",
            VMError::ExecutionError { .. } => "ExecutionError",
            VMError::StorageError { .. } => "StorageError",
            VMError::AccessDenied { .. } => "AccessDenied",
            VMError::InternalError { .. } => "InternalError",
            VMError::InvalidArguments { .. } => "InvalidArguments",
            VMError::ContractNotFound { .. } => "ContractNotFound",
            VMError::InvalidBytecode { .. } => "InvalidBytecode",
            VMError::ContractError { .. } => "ContractError",
            VMError::FunctionNotFound { .. } => "FunctionNotFound",
            VMError::StorageOperationLimitExceeded => "StorageOperationLimitExceeded",
            VMError::InsufficientFunds { .. } => "InsufficientFunds",
            VMError::SerializationError { .. } => "SerializationError",
        };
        
        println!("  Classification: {}", error_type);
        println!("  Severity: {:?}", report.severity);
        println!("  Recommendations: {}", report.recommendations.len());
        
        for rec in &report.recommendations {
            println!("    - {}: {}", rec.category, rec.description);
        }
        println!();
    }
    
    Ok(())
}
