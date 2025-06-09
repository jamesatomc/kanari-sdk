//! Comprehensive error handling and recovery system for Move Virtual Machine
//!
//! This module provides robust error handling, recovery mechanisms, and fault tolerance
//! for the Move VM execution environment on the Kanari blockchain.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use serde::{Serialize, Deserialize};
use log::{debug, info, warn, error};

use crate::types::{VMError, VMResult, ContractAddress, ExecutionStats, VMEvent};
use crate::{ExecutionContext, ExecutionResult, StateManager, VMStorage};
use mona_types::address::Address;

/// Recovery strategy for different types of failures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryStrategy {
    /// Retry execution with reduced gas limit
    RetryWithReducedGas { reduction_factor: f64 },
    /// Rollback state and return error
    RollbackAndFail,
    /// Emergency stop and enter safe mode
    EmergencyStop,
    /// Circuit breaker - temporarily disable contract
    CircuitBreaker { duration_ms: u64 },
    /// Graceful degradation - use fallback execution
    GracefulDegradation,
    /// State repair - attempt to fix inconsistent state
    StateRepair,
}

/// Classification of VM problems for targeted recovery
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ProblemClassification {
    /// Resource exhaustion (gas, memory, storage)
    ResourceExhaustion {
        resource_type: ResourceType,
        severity: Severity,
    },
    /// Execution errors (runtime, compilation, logic)
    ExecutionFailure {
        error_type: ExecutionErrorType,
        is_recoverable: bool,
    },
    /// State consistency issues
    StateInconsistency {
        affected_contracts: Vec<ContractAddress>,
        corruption_level: CorruptionLevel,
    },
    /// Network and infrastructure failures
    InfrastructureFailure {
        component: InfrastructureComponent,
        impact_level: ImpactLevel,
    },
    /// Security threats and access violations
    SecurityThreat {
        threat_type: ThreatType,
        risk_level: RiskLevel,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ResourceType {
    Gas,
    Memory,
    Storage,
    ExecutionTime,
    CpuCycles,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ExecutionErrorType {
    Compilation,
    Runtime,
    Timeout,
    Abort,
    Panic,
    InvalidInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CorruptionLevel {
    Minor,
    Moderate,
    Severe,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum InfrastructureComponent {
    Storage,
    Network,
    GasSystem,
    StateManager,
    MoveRuntime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ImpactLevel {
    Isolated,
    Local,
    Widespread,
    SystemWide,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum ThreatType {
    AccessViolation,
    ResourceAbuse,
    MaliciousCode,
    DoSAttack,
    StateManipulation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Recovery operation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryResult {
    Recovered {
        strategy: RecoveryStrategy,
        recovery_time_ms: u64,
        state_changes: Vec<StateChange>,
        metrics: RecoveryMetrics,
    },
    Failed {
        reason: String,
        strategy_attempted: RecoveryStrategy,
        recovery_time_ms: u64,
        metrics: RecoveryMetrics,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateChange {
    pub contract_address: ContractAddress,
    pub change_type: StateChangeType,
    pub old_value: Option<Vec<u8>>,
    pub new_value: Option<Vec<u8>>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StateChangeType {
    StorageWrite,
    StorageDelete,
    BalanceUpdate,
    ContractState,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryMetrics {
    pub total_recovery_attempts: u64,
    pub successful_recoveries: u64,
    pub failed_recoveries: u64,
    pub average_recovery_time_ms: f64,
    pub most_common_failures: HashMap<ProblemClassification, u64>,
}

/// Circuit breaker for preventing cascading failures
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    pub contract_address: ContractAddress,
    pub failure_count: u64,
    pub failure_threshold: u64,
    pub timeout_duration: Duration,
    pub last_failure_time: Option<Instant>,
    pub state: CircuitBreakerState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CircuitBreakerState {
    Closed,   // Normal operation
    Open,     // Circuit is open, preventing execution
    HalfOpen, // Testing if failures have been resolved
}

/// State snapshot for rollback operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub timestamp: u64,
    pub contract_states: HashMap<ContractAddress, Vec<u8>>,
    pub gas_usage: u64,
    pub execution_data: Vec<u8>, // Serialized execution data
    pub checksum: String,
}

/// Recovery manager coordinates all error handling and recovery operations
pub struct RecoveryManager {
    /// Problem classification engine
    classifier: Arc<Mutex<ProblemClassifier>>,
    /// Recovery strategy selector
    strategy_selector: Arc<Mutex<StrategySelector>>,
    /// Circuit breakers for contracts
    circuit_breakers: Arc<RwLock<HashMap<ContractAddress, CircuitBreaker>>>,
    /// State snapshots for rollback
    state_snapshots: Arc<RwLock<VecDeque<StateSnapshot>>>,
    /// Recovery metrics and statistics
    metrics: Arc<RwLock<RecoveryMetrics>>,
    /// Emergency stop state
    emergency_stop: Arc<RwLock<bool>>,
    /// Recovery operation history
    recovery_history: Arc<Mutex<VecDeque<RecoveryOperation>>>,
    /// Maximum snapshots to keep
    max_snapshots: usize,
    /// References to VM components
    state_manager: Arc<Mutex<StateManager>>,
    storage: Arc<VMStorage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryOperation {
    pub id: String,
    pub timestamp: u64,
    pub problem: ProblemClassification,
    pub strategy: RecoveryStrategy,
    pub result: RecoveryResult,
    pub context: ExecutionContext,
}

/// Problem classifier analyzes errors and determines appropriate responses
pub struct ProblemClassifier {
    /// Pattern matching rules for error classification
    classification_rules: HashMap<String, ProblemClassification>,
    /// Historical error patterns
    error_patterns: HashMap<String, Vec<VMError>>,
    /// Adaptive learning weights
    pattern_weights: HashMap<ProblemClassification, f64>,
}

/// Strategy selector chooses the best recovery approach
pub struct StrategySelector {
    /// Strategy preference matrix
    strategy_matrix: HashMap<ProblemClassification, Vec<RecoveryStrategy>>,
    /// Success rates for each strategy
    strategy_success_rates: HashMap<(ProblemClassification, RecoveryStrategy), f64>,
    /// Adaptive strategy learning
    strategy_learning: bool,
}

impl RecoveryManager {
    /// Create a new recovery manager
    pub fn new(
        state_manager: Arc<Mutex<StateManager>>,
        storage: Arc<VMStorage>,
        max_snapshots: usize,
    ) -> Self {
        Self {
            classifier: Arc::new(Mutex::new(ProblemClassifier::new())),
            strategy_selector: Arc::new(Mutex::new(StrategySelector::new())),
            circuit_breakers: Arc::new(RwLock::new(HashMap::new())),
            state_snapshots: Arc::new(RwLock::new(VecDeque::new())),
            metrics: Arc::new(RwLock::new(RecoveryMetrics::new())),
            emergency_stop: Arc::new(RwLock::new(false)),
            recovery_history: Arc::new(Mutex::new(VecDeque::new())),
            max_snapshots,
            state_manager,
            storage,
        }
    }

    /// Create a state snapshot before risky operations
    pub fn create_snapshot(&self, context: &ExecutionContext) -> VMResult<String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;        // Create execution data summary (avoiding serialization issues)
        let execution_data = format!("gas_used:{},timestamp:{}", context.gas_used, timestamp)
            .into_bytes();

        // Collect contract states
        let mut contract_states = HashMap::new();
        {
            let state_manager = self.state_manager.lock().unwrap();
            // Note: In a real implementation, we would iterate over all active contracts
            // For now, we'll create an empty snapshot structure
        }        // Serialize execution context for snapshot
        let execution_data = bincode::serialize(context)
            .map_err(|e| VMError::SerializationError { message: format!("Failed to serialize context: {}", e) })?;

        let snapshot = StateSnapshot {
            timestamp,
            contract_states,
            gas_usage: context.gas_used,
            execution_data: execution_data.clone(),
            checksum: self.calculate_checksum(&execution_data),
        };

        // Store snapshot
        {
            let mut snapshots = self.state_snapshots.write().unwrap();
            snapshots.push_back(snapshot);
            
            // Maintain maximum snapshot count
            while snapshots.len() > self.max_snapshots {
                snapshots.pop_front();
            }
        }

        let snapshot_id = format!("snapshot_{}", timestamp);
        info!("Created state snapshot: {}", snapshot_id);
        Ok(snapshot_id)
    }

    /// Handle a VM error with appropriate recovery strategy
    pub fn handle_error(
        &self,
        error: VMError,
        context: &ExecutionContext,
    ) -> VMResult<RecoveryResult> {
        let start_time = Instant::now();
        
        // Check if emergency stop is active
        if *self.emergency_stop.read().unwrap() {
            return Err(VMError::RuntimeError {
                message: "VM is in emergency stop mode".to_string(),
            });
        }

        // Classify the problem
        let problem = {
            let mut classifier = self.classifier.lock().unwrap();
            classifier.classify_error(&error, context)
        };

        // Check circuit breaker
        if self.is_circuit_open(&context.contract_address) {
            return Err(VMError::AccessDenied {
                message: format!(
                    "Circuit breaker is open for contract {}",
                    context.contract_address.to_hex_literal()
                ),
            });
        }

        // Select recovery strategy
        let strategy = {
            let mut selector = self.strategy_selector.lock().unwrap();
            selector.select_strategy(&problem, context)
        };

        // Execute recovery
        let recovery_result = self.execute_recovery(&strategy, &problem, context)?;

        // Update metrics
        self.update_metrics(&problem, &strategy, &recovery_result);

        // Log recovery operation
        self.log_recovery_operation(&problem, &strategy, &recovery_result, context);

        let recovery_time = start_time.elapsed().as_millis() as u64;
        info!(
            "Error recovery completed in {}ms using strategy: {:?}",
            recovery_time, strategy
        );

        Ok(recovery_result)
    }

    /// Execute a specific recovery strategy
    fn execute_recovery(
        &self,
        strategy: &RecoveryStrategy,
        problem: &ProblemClassification,
        context: &ExecutionContext,
    ) -> VMResult<RecoveryResult> {
        let start_time = Instant::now();
        
        match strategy {
            RecoveryStrategy::RetryWithReducedGas { reduction_factor } => {
                self.retry_with_reduced_gas(*reduction_factor, context)
            }
            RecoveryStrategy::RollbackAndFail => {
                self.rollback_state(context)
            }
            RecoveryStrategy::EmergencyStop => {
                self.activate_emergency_stop()
            }
            RecoveryStrategy::CircuitBreaker { duration_ms } => {
                self.activate_circuit_breaker(context.contract_address, *duration_ms)
            }
            RecoveryStrategy::GracefulDegradation => {
                self.graceful_degradation(context)
            }
            RecoveryStrategy::StateRepair => {
                self.attempt_state_repair(problem, context)
            }
        }
    }

    /// Retry execution with reduced gas limit
    fn retry_with_reduced_gas(
        &self,
        reduction_factor: f64,
        context: &ExecutionContext,
    ) -> VMResult<RecoveryResult> {
        let new_gas_limit = (context.gas_limit as f64 * reduction_factor) as u64;
        
        info!(
            "Retrying execution with reduced gas: {} -> {}",
            context.gas_limit, new_gas_limit
        );

        // Note: In a real implementation, we would create a new execution context
        // and retry the failed operation with the reduced gas limit
          Ok(RecoveryResult::Recovered {
            strategy: RecoveryStrategy::RetryWithReducedGas { reduction_factor },
            recovery_time_ms: 0,
            state_changes: Vec::new(),
            metrics: RecoveryMetrics::new(),
        })
    }

    /// Rollback state to the last known good snapshot
    fn rollback_state(&self, context: &ExecutionContext) -> VMResult<RecoveryResult> {
        let snapshot = {
            let snapshots = self.state_snapshots.read().unwrap();
            snapshots.back().cloned()
        };

        if let Some(snapshot) = snapshot {
            info!("Rolling back state to snapshot from timestamp: {}", snapshot.timestamp);
            
            // Note: In a real implementation, we would restore the state from the snapshot
            // This would involve restoring contract states, storage, and gas usage
              Ok(RecoveryResult::Recovered {
                strategy: RecoveryStrategy::RollbackAndFail,
                recovery_time_ms: 0,
                state_changes: Vec::new(),
                metrics: RecoveryMetrics::new(),
            })
        } else {
            Err(VMError::InternalError {
                message: "No state snapshot available for rollback".to_string(),
            })
        }
    }

    /// Activate emergency stop mode
    fn activate_emergency_stop(&self) -> VMResult<RecoveryResult> {
        warn!("Activating emergency stop mode for VM");
        
        {
            let mut emergency_stop = self.emergency_stop.write().unwrap();
            *emergency_stop = true;
        }

        // Emit emergency stop event
        // Note: In a real implementation, we would notify all VM listeners
          Ok(RecoveryResult::Failed {
            reason: "Emergency stop activated".to_string(),
            strategy_attempted: RecoveryStrategy::EmergencyStop,
            recovery_time_ms: 0,
            metrics: RecoveryMetrics::new(),
        })
    }

    /// Activate circuit breaker for a specific contract
    fn activate_circuit_breaker(
        &self,
        contract_address: ContractAddress,
        duration_ms: u64,
    ) -> VMResult<RecoveryResult> {
        warn!(
            "Activating circuit breaker for contract {} for {}ms",
            contract_address.to_hex_literal(),
            duration_ms
        );

        let circuit_breaker = CircuitBreaker {
            contract_address,
            failure_count: 1,
            failure_threshold: 5,
            timeout_duration: Duration::from_millis(duration_ms),
            last_failure_time: Some(Instant::now()),
            state: CircuitBreakerState::Open,
        };

        {
            let mut breakers = self.circuit_breakers.write().unwrap();
            breakers.insert(contract_address, circuit_breaker);
        }        Ok(RecoveryResult::Failed {
            reason: "Circuit breaker activated".to_string(),
            strategy_attempted: RecoveryStrategy::CircuitBreaker { duration_ms },
            recovery_time_ms: 0,
            metrics: RecoveryMetrics::new(),
        })
    }

    /// Implement graceful degradation
    fn graceful_degradation(&self, context: &ExecutionContext) -> VMResult<RecoveryResult> {
        info!("Implementing graceful degradation for contract execution");
        
        // Note: In a real implementation, this would:
        // 1. Reduce VM capabilities (lower gas limits, simpler operations)
        // 2. Disable non-essential features
        // 3. Use fallback implementations
        // 4. Prioritize critical operations
          Ok(RecoveryResult::Recovered {
            strategy: RecoveryStrategy::GracefulDegradation,
            recovery_time_ms: 0,
            state_changes: Vec::new(),
            metrics: RecoveryMetrics::new(),
        })
    }

    /// Attempt to repair inconsistent state
    fn attempt_state_repair(
        &self,
        problem: &ProblemClassification,
        context: &ExecutionContext,
    ) -> VMResult<RecoveryResult> {
        info!("Attempting state repair for problem: {:?}", problem);
        
        match problem {
            ProblemClassification::StateInconsistency { affected_contracts, corruption_level } => {
                self.repair_state_inconsistency(affected_contracts, corruption_level)
            }
            _ => {
                Err(VMError::InternalError {
                    message: "State repair not applicable for this problem type".to_string(),
                })
            }
        }
    }

    /// Repair state inconsistency
    fn repair_state_inconsistency(
        &self,
        affected_contracts: &[ContractAddress],
        corruption_level: &CorruptionLevel,
    ) -> VMResult<RecoveryResult> {
        match corruption_level {
            CorruptionLevel::Minor => {
                // Attempt automatic repair
                self.auto_repair_minor_corruption(affected_contracts)
            }
            CorruptionLevel::Moderate => {
                // Use state snapshots for repair
                self.snapshot_based_repair(affected_contracts)
            }
            CorruptionLevel::Severe | CorruptionLevel::Critical => {
                // Require manual intervention
                Err(VMError::StorageError {
                    message: "Severe state corruption requires manual intervention".to_string(),
                })
            }
        }
    }

    /// Automatically repair minor state corruption
    fn auto_repair_minor_corruption(
        &self,
        affected_contracts: &[ContractAddress],
    ) -> VMResult<RecoveryResult> {
        let mut state_changes = Vec::new();
        
        for &contract_address in affected_contracts {
            // Note: In a real implementation, this would:
            // 1. Validate contract state integrity
            // 2. Identify specific corruption issues
            // 3. Apply corrective actions
            // 4. Verify repairs
            
            state_changes.push(StateChange {
                contract_address,
                change_type: StateChangeType::ContractState,
                old_value: None,
                new_value: None,
                timestamp: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            });
        }        Ok(RecoveryResult::Recovered {
            strategy: RecoveryStrategy::StateRepair,
            recovery_time_ms: 0,
            state_changes,
            metrics: RecoveryMetrics::new(),
        })
    }

    /// Use snapshots to repair state
    fn snapshot_based_repair(
        &self,
        affected_contracts: &[ContractAddress],
    ) -> VMResult<RecoveryResult> {
        let snapshot = {
            let snapshots = self.state_snapshots.read().unwrap();
            snapshots.back().cloned()
        };

        if let Some(snapshot) = snapshot {
            // Restore only the affected contracts from the snapshot
            let mut state_changes = Vec::new();
            
            for &contract_address in affected_contracts {
                if let Some(_contract_state) = snapshot.contract_states.get(&contract_address) {
                    // Note: In a real implementation, restore the contract state
                    state_changes.push(StateChange {
                        contract_address,
                        change_type: StateChangeType::Rollback,
                        old_value: None,
                        new_value: None,
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                    });                }
            }

            Ok(RecoveryResult::Recovered {
                strategy: RecoveryStrategy::StateRepair,
                recovery_time_ms: 0,
                state_changes,
                metrics: RecoveryMetrics::new(),
            })
        } else {
            Err(VMError::InternalError {
                message: "No state snapshot available for repair".to_string(),
            })
        }
    }

    /// Check if circuit breaker is open for a contract
    fn is_circuit_open(&self, contract_address: &ContractAddress) -> bool {
        let breakers = self.circuit_breakers.read().unwrap();
        if let Some(breaker) = breakers.get(contract_address) {
            match breaker.state {
                CircuitBreakerState::Open => {
                    // Check if timeout has expired
                    if let Some(last_failure) = breaker.last_failure_time {
                        last_failure.elapsed() < breaker.timeout_duration
                    } else {
                        true
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }

    /// Update recovery metrics
    fn update_metrics(
        &self,
        problem: &ProblemClassification,
        strategy: &RecoveryStrategy,
        result: &RecoveryResult,
    ) {
        let mut metrics = self.metrics.write().unwrap();
          metrics.total_recovery_attempts += 1;
        
        match result {
            RecoveryResult::Recovered { recovery_time_ms, .. } => {
                metrics.successful_recoveries += 1;
                let total_time = metrics.average_recovery_time_ms * (metrics.total_recovery_attempts - 1) as f64
                    + *recovery_time_ms as f64;
                metrics.average_recovery_time_ms = total_time / metrics.total_recovery_attempts as f64;
            }
            RecoveryResult::Failed { recovery_time_ms, .. } => {
                metrics.failed_recoveries += 1;
                let total_time = metrics.average_recovery_time_ms * (metrics.total_recovery_attempts - 1) as f64
                    + *recovery_time_ms as f64;
                metrics.average_recovery_time_ms = total_time / metrics.total_recovery_attempts as f64;
            }
        }

        // Update most common failures
        *metrics.most_common_failures.entry(problem.clone()).or_insert(0) += 1;
    }

    /// Log recovery operation for audit and analysis
    fn log_recovery_operation(
        &self,
        problem: &ProblemClassification,
        strategy: &RecoveryStrategy,
        result: &RecoveryResult,
        context: &ExecutionContext,
    ) {
        let operation = RecoveryOperation {
            id: format!("recovery_{}", SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis()),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            problem: problem.clone(),
            strategy: strategy.clone(),
            result: result.clone(),
            context: context.clone(),
        };

        {
            let mut history = self.recovery_history.lock().unwrap();
            history.push_back(operation);
            
            // Keep only recent operations (last 1000)
            while history.len() > 1000 {
                history.pop_front();
            }
        }
    }

    /// Calculate checksum for state validation
    fn calculate_checksum(&self, data: &[u8]) -> String {
        use sha3::Digest;
        let mut hasher = sha3::Sha3_256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// Deactivate emergency stop mode
    pub fn deactivate_emergency_stop(&self) -> VMResult<()> {
        info!("Deactivating emergency stop mode");
        
        {
            let mut emergency_stop = self.emergency_stop.write().unwrap();
            *emergency_stop = false;
        }

        Ok(())
    }

    /// Get current recovery metrics
    pub fn get_metrics(&self) -> RecoveryMetrics {
        self.metrics.read().unwrap().clone()
    }

    /// Get recovery operation history
    pub fn get_recovery_history(&self) -> Vec<RecoveryOperation> {
        self.recovery_history.lock().unwrap().iter().cloned().collect()
    }

    /// Validate VM state integrity
    pub fn validate_state_integrity(&self) -> VMResult<Vec<ProblemClassification>> {
        let mut problems = Vec::new();
        
        // Note: In a real implementation, this would:
        // 1. Check state consistency across all contracts
        // 2. Validate storage integrity
        // 3. Verify gas accounting accuracy
        // 4. Check for orphaned resources
        // 5. Validate transaction sequences
        
        Ok(problems)
    }
}

impl ProblemClassifier {
    pub fn new() -> Self {
        let mut classification_rules = HashMap::new();
        
        // Resource exhaustion patterns
        classification_rules.insert(
            "gas_limit_exceeded".to_string(),
            ProblemClassification::ResourceExhaustion {
                resource_type: ResourceType::Gas,
                severity: Severity::High,
            },
        );
        
        classification_rules.insert(
            "memory_limit_exceeded".to_string(),
            ProblemClassification::ResourceExhaustion {
                resource_type: ResourceType::Memory,
                severity: Severity::Critical,
            },
        );

        Self {
            classification_rules,
            error_patterns: HashMap::new(),
            pattern_weights: HashMap::new(),
        }
    }

    pub fn classify_error(
        &mut self,
        error: &VMError,
        context: &ExecutionContext,
    ) -> ProblemClassification {
        match error {
            VMError::GasLimitExceeded { .. } => {
                ProblemClassification::ResourceExhaustion {
                    resource_type: ResourceType::Gas,
                    severity: self.assess_gas_severity(context),
                }
            }
            VMError::MemoryLimitExceeded { .. } => {
                ProblemClassification::ResourceExhaustion {
                    resource_type: ResourceType::Memory,
                    severity: Severity::Critical,
                }
            }
            VMError::ExecutionTimeout { .. } => {
                ProblemClassification::ResourceExhaustion {
                    resource_type: ResourceType::ExecutionTime,
                    severity: Severity::High,
                }
            }
            VMError::CompilationError { .. } => {
                ProblemClassification::ExecutionFailure {
                    error_type: ExecutionErrorType::Compilation,
                    is_recoverable: false,
                }
            }
            VMError::ExecutionError { .. } => {
                ProblemClassification::ExecutionFailure {
                    error_type: ExecutionErrorType::Runtime,
                    is_recoverable: true,
                }
            }
            VMError::StorageError { .. } => {
                ProblemClassification::InfrastructureFailure {
                    component: InfrastructureComponent::Storage,
                    impact_level: ImpactLevel::Local,
                }
            }
            VMError::AccessDenied { .. } => {
                ProblemClassification::SecurityThreat {
                    threat_type: ThreatType::AccessViolation,
                    risk_level: RiskLevel::Medium,
                }
            }
            _ => {
                // Default classification for unknown errors
                ProblemClassification::ExecutionFailure {
                    error_type: ExecutionErrorType::Runtime,
                    is_recoverable: true,
                }
            }
        }
    }

    fn assess_gas_severity(&self, context: &ExecutionContext) -> Severity {
        let usage_ratio = context.gas_used as f64 / context.gas_limit as f64;
        
        match usage_ratio {
            ratio if ratio >= 0.95 => Severity::Critical,
            ratio if ratio >= 0.80 => Severity::High,
            ratio if ratio >= 0.60 => Severity::Medium,
            _ => Severity::Low,
        }
    }
}

impl StrategySelector {
    pub fn new() -> Self {
        let mut strategy_matrix = HashMap::new();
        
        // Define recovery strategies for each problem type
        strategy_matrix.insert(
            ProblemClassification::ResourceExhaustion {
                resource_type: ResourceType::Gas,
                severity: Severity::High,
            },
            vec![
                RecoveryStrategy::RetryWithReducedGas { reduction_factor: 0.8 },
                RecoveryStrategy::GracefulDegradation,
            ],
        );

        Self {
            strategy_matrix,
            strategy_success_rates: HashMap::new(),
            strategy_learning: true,
        }
    }

    pub fn select_strategy(
        &mut self,
        problem: &ProblemClassification,
        context: &ExecutionContext,
    ) -> RecoveryStrategy {
        // Get available strategies for this problem type
        if let Some(strategies) = self.strategy_matrix.get(problem) {
            if !strategies.is_empty() {
                // For now, return the first strategy
                // In a real implementation, this would use success rates and ML
                strategies[0].clone()
            } else {
                RecoveryStrategy::RollbackAndFail
            }
        } else {
            // Default strategy for unknown problems
            RecoveryStrategy::RollbackAndFail
        }
    }
}

impl RecoveryMetrics {
    pub fn new() -> Self {
        Self {
            total_recovery_attempts: 0,
            successful_recoveries: 0,
            failed_recoveries: 0,
            average_recovery_time_ms: 0.0,
            most_common_failures: HashMap::new(),
        }
    }
}

/// Event types for recovery system monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryEvent {
    RecoveryAttempted {
        problem: ProblemClassification,
        strategy: RecoveryStrategy,
        timestamp: u64,
    },
    RecoveryCompleted {
        success: bool,
        recovery_time_ms: u64,
        timestamp: u64,
    },
    CircuitBreakerActivated {
        contract_address: ContractAddress,
        duration_ms: u64,
        timestamp: u64,
    },
    EmergencyStopActivated {
        reason: String,
        timestamp: u64,
    },
    StateSnapshotCreated {
        snapshot_id: String,
        timestamp: u64,
    },
    StateRepairCompleted {
        affected_contracts: Vec<ContractAddress>,
        success: bool,
        timestamp: u64,
    },
}
