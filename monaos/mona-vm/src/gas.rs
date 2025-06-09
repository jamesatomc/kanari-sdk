//! Gas metering and cost calculation for Move VM

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Gas parameters for different operation types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasParameters {
    pub base: BaseCosts,
    pub move_ops: MoveCosts,
    pub storage: StorageCosts,
    pub custom: GasCosts,
}

impl Default for GasParameters {
    fn default() -> Self {
        Self {
            base: BaseCosts::default(),
            move_ops: MoveCosts::default(),
            storage: StorageCosts::default(),
            custom: GasCosts::default(),
        }
    }
}

/// Base operation costs (in gas units)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseCosts {
    /// Cost to create a new account
    pub account_creation: u64,
    /// Cost to verify a signature
    pub signature_verification: u64,
    /// Cost per byte of transaction data
    pub per_byte: u64,
    /// Base cost for any transaction
    pub base_transaction: u64,
    /// Base cost for function calls
    pub function_call_base: u64,
    /// Cost to emit an event
    pub emit_event: u64,
    /// Cost per byte for serialization
    pub serialization_per_byte: u64,
    /// Cost per byte for event data
    pub event_per_byte: u64,
}

impl Default for BaseCosts {
    fn default() -> Self {
        Self {
            account_creation: 1000,
            signature_verification: 500,
            per_byte: 1,
            base_transaction: 100,
            function_call_base: 50,
            emit_event: 200,
            serialization_per_byte: 2,
            event_per_byte: 3,
        }
    }
}

/// Move language operation costs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveCosts {
    /// Function call overhead
    pub function_call: u64,
    /// Module loading cost
    pub module_load: u64,
    /// Type instantiation cost
    pub type_instantiation: u64,
    /// Vector operations
    pub vector_ops: VectorCosts,
    /// Arithmetic operations
    pub arithmetic: ArithmeticCosts,
    /// Comparison operations
    pub comparison: u64,
    /// Boolean operations
    pub boolean: u64,
    /// Control flow operations
    pub control_flow: ControlFlowCosts,
}

impl Default for MoveCosts {
    fn default() -> Self {
        Self {
            function_call: 100,
            module_load: 500,
            type_instantiation: 50,
            vector_ops: VectorCosts::default(),
            arithmetic: ArithmeticCosts::default(),
            comparison: 10,
            boolean: 5,
            control_flow: ControlFlowCosts::default(),
        }
    }
}

/// Vector operation costs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorCosts {
    pub create: u64,
    pub push_back: u64,
    pub pop_back: u64,
    pub length: u64,
    pub borrow: u64,
    pub borrow_mut: u64,
    pub destroy_empty: u64,
    pub swap: u64,
}

impl Default for VectorCosts {
    fn default() -> Self {
        Self {
            create: 50,
            push_back: 20,
            pop_back: 20,
            length: 5,
            borrow: 10,
            borrow_mut: 15,
            destroy_empty: 10,
            swap: 15,
        }
    }
}

/// Arithmetic operation costs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArithmeticCosts {
    pub add: u64,
    pub sub: u64,
    pub mul: u64,
    pub div: u64,
    pub mod_op: u64,
    pub shl: u64,
    pub shr: u64,
    pub and: u64,
    pub or: u64,
    pub xor: u64,
    pub not: u64,
}

impl Default for ArithmeticCosts {
    fn default() -> Self {
        Self {
            add: 5,
            sub: 5,
            mul: 10,
            div: 20,
            mod_op: 20,
            shl: 5,
            shr: 5,
            and: 5,
            or: 5,
            xor: 5,
            not: 5,
        }
    }
}

/// Control flow operation costs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlFlowCosts {
    pub branch: u64,
    pub loop_op: u64,
    pub abort: u64,
    pub ret: u64,
}

impl Default for ControlFlowCosts {
    fn default() -> Self {
        Self {
            branch: 10,
            loop_op: 15,
            abort: 50,
            ret: 5,
        }
    }
}

/// Storage operation costs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageCosts {
    /// Cost per byte for storage reads
    pub read_per_byte: u64,
    /// Cost per byte for storage writes
    pub write_per_byte: u64,
    /// Base cost for storage operations
    pub base_read: u64,
    pub base_write: u64,
    /// Cost for deleting storage items
    pub delete: u64,
}

impl Default for StorageCosts {
    fn default() -> Self {
        Self {
            read_per_byte: 5,
            write_per_byte: 10,
            base_read: 50,
            base_write: 100,
            delete: 75,
        }
    }
}

/// Custom gas costs for specific operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasCosts {
    /// Contract deployment cost
    pub contract_deployment: u64,
    /// Event emission cost
    pub emit_event: u64,
    /// Native function call costs
    pub native_functions: HashMap<String, u64>,
}

impl Default for GasCosts {
    fn default() -> Self {
        let mut native_functions = HashMap::new();
        native_functions.insert("kari_transfer".to_string(), 1000);
        native_functions.insert("kari_balance".to_string(), 500);
        native_functions.insert("kari_mint".to_string(), 2000);
        native_functions.insert("kari_burn".to_string(), 1500);
        native_functions.insert("hash_sha256".to_string(), 300);
        native_functions.insert("hash_sha3_256".to_string(), 400);
        native_functions.insert("verify_signature".to_string(), 800);

        Self {
            contract_deployment: 10000,
            emit_event: 200,
            native_functions,
        }
    }
}

/// Gas meter for tracking gas consumption
pub struct GasMeter {
    pub gas_limit: u64,
    pub gas_used: u64,
    pub gas_price: u64,
    pub parameters: GasParameters,
}

impl GasMeter {
    /// Create a new gas meter
    pub fn new(gas_limit: u64, gas_price: u64, parameters: GasParameters) -> Self {
        Self {
            gas_limit,
            gas_used: 0,
            gas_price,
            parameters,
        }
    }

    /// Check if we have enough gas
    pub fn has_gas(&self, amount: u64) -> bool {
        self.gas_used + amount <= self.gas_limit
    }

    /// Consume gas
    pub fn consume_gas(&mut self, amount: u64) -> Result<(), String> {
        if self.gas_used + amount > self.gas_limit {
            return Err(format!(
                "Out of gas: trying to use {}, already used {}, limit {}",
                amount, self.gas_used, self.gas_limit
            ));
        }
        self.gas_used += amount;
        Ok(())
    }

    /// Get remaining gas
    pub fn remaining_gas(&self) -> u64 {
        self.gas_limit.saturating_sub(self.gas_used)
    }

    /// Calculate cost for bytecode size
    pub fn calculate_bytecode_cost(&self, bytecode_size: usize) -> u64 {
        self.parameters.base.per_byte * (bytecode_size as u64)
    }

    /// Calculate cost for storage operation
    pub fn calculate_storage_cost(&self, operation: StorageOperation, size: usize) -> u64 {
        match operation {
            StorageOperation::Read => {
                self.parameters.storage.base_read + 
                self.parameters.storage.read_per_byte * (size as u64)
            },
            StorageOperation::Write => {
                self.parameters.storage.base_write + 
                self.parameters.storage.write_per_byte * (size as u64)
            },
            StorageOperation::Delete => {
                self.parameters.storage.delete
            },
        }
    }

    /// Calculate cost for Move operation
    pub fn calculate_move_operation_cost(&self, operation: MoveOperation) -> u64 {
        match operation {
            MoveOperation::FunctionCall => self.parameters.move_ops.function_call,
            MoveOperation::ModuleLoad => self.parameters.move_ops.module_load,
            MoveOperation::TypeInstantiation => self.parameters.move_ops.type_instantiation,
            MoveOperation::VectorCreate => self.parameters.move_ops.vector_ops.create,
            MoveOperation::VectorPushBack => self.parameters.move_ops.vector_ops.push_back,
            MoveOperation::VectorPopBack => self.parameters.move_ops.vector_ops.pop_back,
            MoveOperation::ArithmeticAdd => self.parameters.move_ops.arithmetic.add,
            MoveOperation::ArithmeticMul => self.parameters.move_ops.arithmetic.mul,
            MoveOperation::ArithmeticDiv => self.parameters.move_ops.arithmetic.div,
            MoveOperation::Comparison => self.parameters.move_ops.comparison,
            MoveOperation::Branch => self.parameters.move_ops.control_flow.branch,
            MoveOperation::Loop => self.parameters.move_ops.control_flow.loop_op,
        }
    }

    /// Calculate total transaction cost (gas * gas_price)
    pub fn calculate_total_cost(&self) -> u64 {
        self.gas_used * self.gas_price
    }
}

/// Storage operation types
#[derive(Debug, Clone, Copy)]
pub enum StorageOperation {
    Read,
    Write,
    Delete,
}

/// Move operation types for gas calculation
#[derive(Debug, Clone, Copy)]
pub enum MoveOperation {
    FunctionCall,
    ModuleLoad,
    TypeInstantiation,
    VectorCreate,
    VectorPushBack,
    VectorPopBack,
    ArithmeticAdd,
    ArithmeticMul,
    ArithmeticDiv,
    Comparison,
    Branch,
    Loop,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_meter() {
        let params = GasParameters::default();
        let mut meter = GasMeter::new(1000, 50, params);

        assert_eq!(meter.remaining_gas(), 1000);
        assert!(meter.has_gas(500));

        meter.consume_gas(300).unwrap();
        assert_eq!(meter.gas_used, 300);
        assert_eq!(meter.remaining_gas(), 700);

        // Test gas exhaustion
        let result = meter.consume_gas(800);
        assert!(result.is_err());
    }

    #[test]
    fn test_gas_calculations() {
        let params = GasParameters::default();
        let meter = GasMeter::new(1000, 50, params);

        let bytecode_cost = meter.calculate_bytecode_cost(100);
        assert_eq!(bytecode_cost, 100); // 1 gas per byte

        let storage_cost = meter.calculate_storage_cost(StorageOperation::Write, 50);
        assert_eq!(storage_cost, 100 + 50 * 10); // base_write + per_byte * size

        let move_cost = meter.calculate_move_operation_cost(MoveOperation::FunctionCall);
        assert_eq!(move_cost, 100);
    }

    #[test]
    fn test_total_cost_calculation() {
        let params = GasParameters::default();
        let mut meter = GasMeter::new(1000, 50, params);

        meter.consume_gas(200).unwrap();
        let total_cost = meter.calculate_total_cost();
        assert_eq!(total_cost, 200 * 50); // gas_used * gas_price
    }
}
