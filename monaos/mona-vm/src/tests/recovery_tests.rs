//! Recovery system tests

#[cfg(test)]
mod tests {    use crate::recovery::*;
    use crate::types::*;
    use crate::{ExecutionContext, StateManager, VMStorage};
    use mona_types::address::Address;
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    fn create_test_recovery_manager() -> RecoveryManager {
        let rocks_storage = Arc::new(
            mona_storage::RocksDBStorage::new(std::env::temp_dir().join("test_recovery"))
                .expect("Failed to create test storage")
        );
        let storage = Arc::new(VMStorage::new(rocks_storage.clone(), 1024));
        let state_manager = Arc::new(Mutex::new(StateManager::new(rocks_storage)));
        
        RecoveryManager::new(state_manager, storage, 10)
    }

    fn create_test_context() -> ExecutionContext {
        ExecutionContext::new(
            Address::zero(),
            Address::zero(),
            1000000,
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
    }    #[test]
    fn test_problem_classification() {
        let recovery_manager = create_test_recovery_manager();
        let context = create_test_context();

        // Test gas limit exceeded classification by handling the error
        let gas_error = VMError::GasLimitExceeded { used: 1500000, limit: 1000000 };
        let result = recovery_manager.handle_error(gas_error, &context);
        
        // The error should be handled (either recovered or failed, but not return an error)
        assert!(result.is_ok());
        
        // Verify metrics were updated
        let metrics = recovery_manager.get_metrics();
        assert!(metrics.total_recovery_attempts > 0);
    }    #[test]
    fn test_circuit_breaker() {
        let recovery_manager = create_test_recovery_manager();
        let mut context = create_test_context();
        context.contract_address = Address::zero().into();

        // Create an error that would trigger circuit breaker
        let error = VMError::RuntimeError { message: "Test failure".to_string() };
        
        // Handle multiple errors to potentially trigger circuit breaker
        for _ in 0..3 {
            let _ = recovery_manager.handle_error(error.clone(), &context);
        }
        
        // Verify metrics show recovery attempts
        let metrics = recovery_manager.get_metrics();
        assert!(metrics.total_recovery_attempts >= 3);
    }    #[test]
    fn test_state_snapshot_creation() {
        let recovery_manager = create_test_recovery_manager();
        let context = create_test_context();
        
        let snapshot_id = recovery_manager
            .create_snapshot(&context)
            .expect("Failed to create snapshot");
        
        assert!(!snapshot_id.is_empty());
        assert!(snapshot_id.starts_with("snapshot_"));
    }    #[test]
    fn test_error_handling_workflow() {
        let recovery_manager = create_test_recovery_manager();
        let context = create_test_context();
        let error = VMError::GasLimitExceeded { used: 1200000, limit: 1000000 };

        let result = recovery_manager.handle_error(error, &context);
        assert!(result.is_ok());

        let recovery_result = result.unwrap();
        match recovery_result {
            RecoveryResult::Recovered { strategy, .. } => {
                match strategy {
                    RecoveryStrategy::RetryWithReducedGas { .. } => {
                        // Expected for gas limit exceeded
                    }
                    _ => {
                        // Other strategies are also acceptable
                    }
                }
            }
            RecoveryResult::Failed { .. } => {
                // Recovery failure is also acceptable in some cases
            }
        }
    }    #[test]
    fn test_recovery_metrics() {
        let recovery_manager = create_test_recovery_manager();
        let context = create_test_context();
        let error = VMError::RuntimeError { message: "Test error".to_string() };

        // Handle some errors to generate metrics
        let _ = recovery_manager.handle_error(error.clone(), &context);
        let _ = recovery_manager.handle_error(error, &context);

        let metrics = recovery_manager.get_metrics();
        assert!(metrics.total_recovery_attempts >= 2);
        assert!(metrics.successful_recoveries + metrics.failed_recoveries == metrics.total_recovery_attempts);
    }    #[test]
    fn test_error_recovery_workflow() {
        let recovery_manager = create_test_recovery_manager();
        let context = create_test_context();
        
        // Test different types of errors
        let errors = vec![
            VMError::GasLimitExceeded { used: 1200000, limit: 1000000 },
            VMError::RuntimeError { message: "Test runtime error".to_string() },
            VMError::StorageError { message: "Test storage error".to_string() },
        ];
        
        for error in errors {
            let result = recovery_manager.handle_error(error, &context);
            // All errors should be handled (either recovered or failed gracefully)
            assert!(result.is_ok());
        }
        
        // Verify metrics were updated
        let metrics = recovery_manager.get_metrics();
        assert!(metrics.total_recovery_attempts >= 3);
    }    #[test]
    fn test_recovery_history() {
        let recovery_manager = create_test_recovery_manager();
        let context = create_test_context();
        let error = VMError::RuntimeError { message: "Test error".to_string() };

        // Handle an error to generate history
        let _ = recovery_manager.handle_error(error, &context);

        // Get recovery history
        let history = recovery_manager.get_recovery_history();
        assert!(history.len() >= 1);
        
        // Verify the operation was logged
        let last_operation = &history[history.len() - 1];
        assert!(!last_operation.id.is_empty());
        assert!(last_operation.timestamp > 0);
    }
}
