//! Gas metering system for the Virtual Machine
//!
//! This module provides gas cost definitions, gas metering functionality,
//! and gas limit enforcement for smart contract execution.

use crate::vm::VMError;
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Gas cost constants for different operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasCosts {
    // Base costs
    pub base: u64,
    pub very_low: u64,
    pub low: u64,
    pub mid: u64,
    pub high: u64,

    // Arithmetic operations
    pub add: u64,
    pub mul: u64,
    pub sub: u64,
    pub div: u64,
    pub mod_op: u64,
    pub addmod: u64,
    pub mulmod: u64,
    pub exp: u64,
    pub signextend: u64,

    // Comparison and bitwise operations
    pub lt: u64,
    pub gt: u64,
    pub slt: u64,
    pub sgt: u64,
    pub eq: u64,
    pub iszero: u64,
    pub and: u64,
    pub or: u64,
    pub xor: u64,
    pub not: u64,
    pub byte: u64,
    pub shl: u64,
    pub shr: u64,
    pub sar: u64,

    // Cryptographic operations
    pub keccak256: u64,
    pub blake3: u64,

    // Environmental information
    pub address: u64,
    pub balance: u64,
    pub origin: u64,
    pub caller: u64,
    pub callvalue: u64,
    pub calldataload: u64,
    pub calldatasize: u64,
    pub calldatacopy: u64,
    pub codesize: u64,
    pub codecopy: u64,
    pub gasprice: u64,
    pub extcodesize: u64,
    pub extcodecopy: u64,
    pub returndatasize: u64,
    pub returndatacopy: u64,

    // Block information
    pub blockhash: u64,
    pub coinbase: u64,
    pub timestamp: u64,
    pub number: u64,
    pub difficulty: u64,
    pub gaslimit: u64,
    pub chainid: u64,

    // Stack operations
    pub pop: u64,
    pub mload: u64,
    pub mstore: u64,
    pub mstore8: u64,
    pub sload: u64,
    pub sstore: u64,
    pub jump: u64,
    pub jumpi: u64,
    pub pc: u64,
    pub msize: u64,
    pub gas: u64,
    pub jumpdest: u64,

    // Push operations
    pub push0: u64,
    pub push1: u64,
    pub push32: u64,

    // Duplicate operations
    pub dup1: u64,
    pub dup16: u64,

    // Swap operations
    pub swap1: u64,
    pub swap16: u64,

    // Logging operations
    pub log_base: u64,
    pub log_topic: u64,
    pub log_data: u64,

    // System operations
    pub create: u64,
    pub call: u64,
    pub callcode: u64,
    pub return_op: u64,
    pub delegatecall: u64,
    pub create2: u64,
    pub staticcall: u64,
    pub revert: u64,
    pub selfdestruct: u64,

    // Memory expansion costs
    pub memory_word: u64,
    pub memory_quad_coeff: u64,

    // Contract operations
    pub contract_creation: u64,
    pub contract_call: u64,

    // Storage operations (additional costs)
    pub sstore_set: u64,
    pub sstore_reset: u64,
    pub sstore_clear_refund: u64,

    // Call operations (additional costs)
    pub call_value_transfer: u64,
    pub call_new_account: u64,
    pub call_stipend: u64,
}

impl Default for GasCosts {
    fn default() -> Self {
        Self {
            // Base costs
            base: 2,
            very_low: 3,
            low: 5,
            mid: 8,
            high: 10,

            // Arithmetic operations
            add: 3,
            mul: 5,
            sub: 3,
            div: 5,
            mod_op: 5,
            addmod: 8,
            mulmod: 8,
            exp: 10,
            signextend: 5,

            // Comparison and bitwise operations
            lt: 3,
            gt: 3,
            slt: 3,
            sgt: 3,
            eq: 3,
            iszero: 3,
            and: 3,
            or: 3,
            xor: 3,
            not: 3,
            byte: 3,
            shl: 3,
            shr: 3,
            sar: 3,

            // Cryptographic operations
            keccak256: 30,
            blake3: 25, // Blake3 is faster than Keccak256

            // Environmental information
            address: 2,
            balance: 400, // High cost for external balance lookups
            origin: 2,
            caller: 2,
            callvalue: 2,
            calldataload: 3,
            calldatasize: 2,
            calldatacopy: 3,
            codesize: 2,
            codecopy: 3,
            gasprice: 2,
            extcodesize: 700, // High cost for external code size
            extcodecopy: 700,
            returndatasize: 2,
            returndatacopy: 3,

            // Block information
            blockhash: 20,
            coinbase: 2,
            timestamp: 2,
            number: 2,
            difficulty: 2,
            gaslimit: 2,
            chainid: 2,

            // Stack operations
            pop: 2,
            mload: 3,
            mstore: 3,
            mstore8: 3,
            sload: 800,    // High cost for storage reads
            sstore: 20000, // Very high cost for storage writes
            jump: 8,
            jumpi: 10,
            pc: 2,
            msize: 2,
            gas: 2,
            jumpdest: 1,

            // Push operations
            push0: 2,
            push1: 3,
            push32: 3,

            // Duplicate operations
            dup1: 3,
            dup16: 3,

            // Swap operations
            swap1: 3,
            swap16: 3,

            // Logging operations
            log_base: 375,
            log_topic: 375,
            log_data: 8,

            // System operations
            create: 32000,
            call: 700,
            callcode: 700,
            return_op: 0,
            delegatecall: 700,
            create2: 32000,
            staticcall: 700,
            revert: 0,
            selfdestruct: 5000,

            // Memory expansion costs
            memory_word: 3,
            memory_quad_coeff: 512,

            // Contract operations
            contract_creation: 53000,
            contract_call: 25000,

            // Storage operations (additional costs)
            sstore_set: 20000,
            sstore_reset: 5000,
            sstore_clear_refund: 15000,

            // Call operations (additional costs)
            call_value_transfer: 9000,
            call_new_account: 25000,
            call_stipend: 2300,
        }
    }
}

/// Global gas costs instance
pub static GAS_COSTS: GasCosts = GasCosts {
    // Use default values but allow customization
    base: 2,
    very_low: 3,
    low: 5,
    mid: 8,
    high: 10,
    add: 3,
    mul: 5,
    sub: 3,
    div: 5,
    mod_op: 5,
    addmod: 8,
    mulmod: 8,
    exp: 10,
    signextend: 5,
    lt: 3,
    gt: 3,
    slt: 3,
    sgt: 3,
    eq: 3,
    iszero: 3,
    and: 3,
    or: 3,
    xor: 3,
    not: 3,
    byte: 3,
    shl: 3,
    shr: 3,
    sar: 3,
    keccak256: 30,
    blake3: 25,
    address: 2,
    balance: 400,
    origin: 2,
    caller: 2,
    callvalue: 2,
    calldataload: 3,
    calldatasize: 2,
    calldatacopy: 3,
    codesize: 2,
    codecopy: 3,
    gasprice: 2,
    extcodesize: 700,
    extcodecopy: 700,
    returndatasize: 2,
    returndatacopy: 3,
    blockhash: 20,
    coinbase: 2,
    timestamp: 2,
    number: 2,
    difficulty: 2,
    gaslimit: 2,
    chainid: 2,
    pop: 2,
    mload: 3,
    mstore: 3,
    mstore8: 3,
    sload: 800,
    sstore: 20000,
    jump: 8,
    jumpi: 10,
    pc: 2,
    msize: 2,
    gas: 2,
    jumpdest: 1,
    push0: 2,
    push1: 3,
    push32: 3,
    dup1: 3,
    dup16: 3,
    swap1: 3,
    swap16: 3,
    log_base: 375,
    log_topic: 375,
    log_data: 8,
    create: 32000,
    call: 700,
    callcode: 700,
    return_op: 0,
    delegatecall: 700,
    create2: 32000,
    staticcall: 700,
    revert: 0,
    selfdestruct: 5000,
    memory_word: 3,
    memory_quad_coeff: 512,
    contract_creation: 53000,
    contract_call: 25000,
    sstore_set: 20000,
    sstore_reset: 5000,
    sstore_clear_refund: 15000,
    call_value_transfer: 9000,
    call_new_account: 25000,
    call_stipend: 2300,
};

/// Gas metering and tracking
#[derive(Debug, Clone)]
pub struct GasMeter {
    /// Gas limit for the current execution
    gas_limit: u64,
    /// Gas used so far
    gas_used: u64,
    /// Gas refunds accumulated
    gas_refunds: u64,
    /// Memory cost tracking
    memory_cost: u64,
    /// Last memory size for cost calculation
    last_memory_size: usize,
}

impl GasMeter {
    /// Create a new gas meter with the specified limit
    pub fn new() -> Self {
        Self {
            gas_limit: 0,
            gas_used: 0,
            gas_refunds: 0,
            memory_cost: 0,
            last_memory_size: 0,
        }
    }

    /// Reset the gas meter with a new limit
    pub fn reset(&mut self, gas_limit: u64) {
        self.gas_limit = gas_limit;
        self.gas_used = 0;
        self.gas_refunds = 0;
        self.memory_cost = 0;
        self.last_memory_size = 0;
        debug!("Gas meter reset with limit: {}", gas_limit);
    }

    /// Consume gas for an operation
    pub fn consume_gas(&mut self, amount: u64) -> Result<(), VMError> {
        if self.gas_used + amount > self.gas_limit {
            warn!(
                "Out of gas: used {} + {} > limit {}",
                self.gas_used, amount, self.gas_limit
            );
            return Err(VMError::OutOfGas);
        }

        self.gas_used += amount;
        debug!("Consumed {} gas, total used: {}", amount, self.gas_used);
        Ok(())
    }

    /// Add gas refund
    pub fn refund_gas(&mut self, amount: u64) {
        self.gas_refunds += amount;
        debug!(
            "Gas refund: {}, total refunds: {}",
            amount, self.gas_refunds
        );
    }

    /// Calculate and consume memory expansion cost
    pub fn consume_memory_gas(&mut self, memory_size: usize) -> Result<(), VMError> {
        if memory_size <= self.last_memory_size {
            return Ok(()); // No expansion needed
        }

        let new_cost = self.calculate_memory_cost(memory_size);
        let additional_cost = new_cost.saturating_sub(self.memory_cost);

        if additional_cost > 0 {
            self.consume_gas(additional_cost)?;
            self.memory_cost = new_cost;
            self.last_memory_size = memory_size;
            debug!(
                "Memory expansion: {} bytes, cost: {}",
                memory_size, additional_cost
            );
        }

        Ok(())
    }

    /// Calculate memory cost for a given size
    fn calculate_memory_cost(&self, size: usize) -> u64 {
        let size_words = (size + 31) / 32; // Round up to word boundary
        let linear_cost = size_words as u64 * GAS_COSTS.memory_word;
        let quadratic_cost = (size_words * size_words) as u64 / GAS_COSTS.memory_quad_coeff;
        linear_cost + quadratic_cost
    }

    /// Check if out of gas
    pub fn out_of_gas(&self) -> bool {
        self.gas_used >= self.gas_limit
    }

    /// Get remaining gas
    pub fn gas_remaining(&self) -> u64 {
        self.gas_limit.saturating_sub(self.gas_used)
    }

    /// Get gas used
    pub fn gas_used(&self) -> u64 {
        self.gas_used
    }

    /// Get gas limit
    pub fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    /// Get gas refunds
    pub fn gas_refunds(&self) -> u64 {
        self.gas_refunds
    }

    /// Calculate final gas cost (used gas minus refunds, capped at half of used gas)
    pub fn finalize_gas(&self) -> u64 {
        let max_refund = self.gas_used / 2; // Maximum refund is half of gas used
        let actual_refund = self.gas_refunds.min(max_refund);
        self.gas_used.saturating_sub(actual_refund)
    }

    /// Estimate gas for bytecode deployment
    pub fn estimate_deployment_gas(&self, bytecode: &[u8]) -> u64 {
        let base_cost = GAS_COSTS.contract_creation;
        let code_cost = bytecode.len() as u64 * 200; // 200 gas per byte of code
        base_cost + code_cost
    }

    /// Estimate gas for function call
    pub fn estimate_call_gas(&self, data_size: usize, has_value: bool) -> u64 {
        let base_cost = GAS_COSTS.contract_call;
        let data_cost = data_size as u64 * 4; // 4 gas per byte of call data
        let value_cost = if has_value {
            GAS_COSTS.call_value_transfer
        } else {
            0
        };
        base_cost + data_cost + value_cost
    }

    /// Create a gas meter for a subcall with limited gas
    pub fn subcall(&self, gas_limit: u64) -> Self {
        let available_gas = self.gas_remaining();
        let actual_limit = gas_limit.min(available_gas);

        Self {
            gas_limit: actual_limit,
            gas_used: 0,
            gas_refunds: 0,
            memory_cost: 0,
            last_memory_size: 0,
        }
    }

    /// Merge results from a subcall back into this meter
    pub fn merge_subcall(&mut self, subcall_meter: &GasMeter) -> Result<(), VMError> {
        self.consume_gas(subcall_meter.gas_used)?;
        self.gas_refunds += subcall_meter.gas_refunds;
        Ok(())
    }
}

/// Gas estimation utilities
pub struct GasEstimator {
    base_costs: GasCosts,
}

impl GasEstimator {
    pub fn new() -> Self {
        Self {
            base_costs: GasCosts::default(),
        }
    }

    /// Estimate gas for simple operations
    pub fn estimate_simple_operation(&self, operation: &str) -> u64 {
        match operation {
            "transfer" => 21000,
            "approve" => 45000,
            "transferFrom" => 60000,
            "mint" => 50000,
            "burn" => 30000,
            _ => 50000, // Default estimate
        }
    }

    /// Estimate gas based on bytecode analysis
    pub fn estimate_from_bytecode(&self, bytecode: &[u8]) -> u64 {
        let mut estimated_gas = 0;
        let mut i = 0;

        while i < bytecode.len() {
            let opcode = bytecode[i];

            // Add base gas cost for each operation
            estimated_gas += match opcode {
                0x00 => self.base_costs.base,            // STOP
                0x01 => self.base_costs.add,             // ADD
                0x02 => self.base_costs.mul,             // MUL
                0x03 => self.base_costs.sub,             // SUB
                0x04 => self.base_costs.div,             // DIV
                0x06 => self.base_costs.mod_op,          // MOD
                0x10..=0x1d => self.base_costs.very_low, // Comparison ops
                0x20 => self.base_costs.keccak256,       // KECCAK256
                0x21 => self.base_costs.blake3,          // BLAKE3
                0x30..=0x3e => self.base_costs.low,      // Environmental ops
                0x40..=0x46 => self.base_costs.base,     // Block info ops
                0x50 => self.base_costs.pop,             // POP
                0x51 => self.base_costs.mload,           // MLOAD
                0x52 => self.base_costs.mstore,          // MSTORE
                0x54 => self.base_costs.sload,           // SLOAD
                0x55 => self.base_costs.sstore,          // SSTORE
                0x56 => self.base_costs.jump,            // JUMP
                0x57 => self.base_costs.jumpi,           // JUMPI
                0x60..=0x7f => self.base_costs.push1,    // PUSH ops
                0x80..=0x8f => self.base_costs.dup1,     // DUP ops
                0x90..=0x9f => self.base_costs.swap1,    // SWAP ops
                0xa0..=0xa4 => self.base_costs.log_base, // LOG ops
                0xf0 => self.base_costs.create,          // CREATE
                0xf1 => self.base_costs.call,            // CALL
                0xf3 => self.base_costs.return_op,       // RETURN
                0xf4 => self.base_costs.delegatecall,    // DELEGATECALL
                0xff => self.base_costs.selfdestruct,    // SELFDESTRUCT
                _ => self.base_costs.base,               // Default
            };

            // Skip push data
            if opcode >= 0x60 && opcode <= 0x7f {
                let push_size = (opcode - 0x60 + 1) as usize;
                i += push_size + 1;
            } else {
                i += 1;
            }
        }

        // Add some buffer for complexity
        estimated_gas + (estimated_gas / 10)
    }

    /// Estimate gas for contract deployment
    pub fn estimate_deployment(&self, bytecode: &[u8], constructor_args: &[u8]) -> u64 {
        let base_deployment = self.base_costs.contract_creation;
        let code_cost = bytecode.len() as u64 * 200;
        let constructor_cost = self.estimate_from_bytecode(bytecode);
        let args_cost = constructor_args.len() as u64 * 4;

        base_deployment + code_cost + constructor_cost + args_cost
    }

    /// Estimate gas for function call with dynamic analysis
    pub fn estimate_function_call(
        &self,
        function_signature: &str,
        args_size: usize,
        has_value: bool,
        is_view: bool,
    ) -> u64 {
        let base_call = if is_view {
            self.base_costs.staticcall
        } else {
            self.base_costs.call
        };

        let data_cost = args_size as u64 * 4;
        let value_cost = if has_value {
            self.base_costs.call_value_transfer
        } else {
            0
        };

        // Function-specific estimates
        let function_cost = match function_signature {
            sig if sig.starts_with("transfer(") => 21000,
            sig if sig.starts_with("approve(") => 45000,
            sig if sig.starts_with("transferFrom(") => 60000,
            sig if sig.contains("mint") => 50000,
            sig if sig.contains("burn") => 30000,
            sig if sig.contains("swap") => 80000,
            sig if sig.contains("create") => 100000,
            _ => 50000, // Default estimate
        };

        base_call + data_cost + value_cost + function_cost
    }
}

/// Gas price oracle for dynamic gas pricing
#[derive(Debug, Clone)]
pub struct GasPriceOracle {
    base_gas_price: u64,
    congestion_multiplier: f64,
    recent_prices: Vec<u64>,
    max_history: usize,
}

impl GasPriceOracle {
    pub fn new(base_gas_price: u64) -> Self {
        Self {
            base_gas_price,
            congestion_multiplier: 1.0,
            recent_prices: Vec::new(),
            max_history: 100,
        }
    }

    /// Update gas price based on network congestion
    pub fn update_congestion(&mut self, pending_transactions: usize, block_utilization: f64) {
        // Simple congestion-based pricing
        let congestion_factor = if pending_transactions > 1000 {
            1.5 + (pending_transactions as f64 / 10000.0)
        } else {
            1.0 + (pending_transactions as f64 / 2000.0)
        };

        let utilization_factor = 1.0 + block_utilization;
        self.congestion_multiplier = congestion_factor * utilization_factor;

        // Cap the multiplier
        self.congestion_multiplier = self.congestion_multiplier.min(5.0).max(0.5);
    }

    /// Get current recommended gas price
    pub fn get_gas_price(&self, priority: GasPriority) -> u64 {
        let base_price = (self.base_gas_price as f64 * self.congestion_multiplier) as u64;

        match priority {
            GasPriority::Low => (base_price as f64 * 0.8) as u64,
            GasPriority::Standard => base_price,
            GasPriority::High => (base_price as f64 * 1.2) as u64,
            GasPriority::Urgent => (base_price as f64 * 1.5) as u64,
        }
    }

    /// Record a gas price from a recent transaction
    pub fn record_price(&mut self, price: u64) {
        self.recent_prices.push(price);
        if self.recent_prices.len() > self.max_history {
            self.recent_prices.remove(0);
        }
    }

    /// Get average recent gas price
    pub fn average_recent_price(&self) -> u64 {
        if self.recent_prices.is_empty() {
            self.base_gas_price
        } else {
            self.recent_prices.iter().sum::<u64>() / self.recent_prices.len() as u64
        }
    }
}

/// Gas priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GasPriority {
    Low,
    Standard,
    High,
    Urgent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_meter_basic_operations() {
        let mut meter = GasMeter::new();
        meter.reset(1000);

        assert_eq!(meter.gas_limit(), 1000);
        assert_eq!(meter.gas_used(), 0);

        // Consume some gas
        assert!(meter.consume_gas(100).is_ok());
        assert_eq!(meter.gas_used(), 100);
        assert_eq!(meter.gas_remaining(), 900);

        // Try to consume more than available
        assert!(meter.consume_gas(1000).is_err());
    }

    #[test]
    fn test_memory_cost_calculation() {
        let mut meter = GasMeter::new();
        meter.reset(100000);

        // Test memory expansion
        assert!(meter.consume_memory_gas(32).is_ok());
        let first_cost = meter.gas_used();

        assert!(meter.consume_memory_gas(64).is_ok());
        let second_cost = meter.gas_used();

        assert!(second_cost > first_cost);
    }

    #[test]
    fn test_gas_refunds() {
        let mut meter = GasMeter::new();
        meter.reset(1000);

        meter.consume_gas(500).unwrap();
        meter.refund_gas(100);

        assert_eq!(meter.gas_refunds(), 100);
        assert_eq!(meter.finalize_gas(), 400); // 500 - 100
    }

    #[test]
    fn test_gas_estimator() {
        let estimator = GasEstimator::new();

        let transfer_estimate = estimator.estimate_simple_operation("transfer");
        assert_eq!(transfer_estimate, 21000);

        // Test bytecode estimation
        let simple_bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01]; // PUSH1 1, PUSH1 2, ADD
        let estimate = estimator.estimate_from_bytecode(&simple_bytecode);
        assert!(estimate > 0);
    }

    #[test]
    fn test_gas_price_oracle() {
        let mut oracle = GasPriceOracle::new(1000);

        let standard_price = oracle.get_gas_price(GasPriority::Standard);
        assert_eq!(standard_price, 1000);

        let high_price = oracle.get_gas_price(GasPriority::High);
        assert!(high_price > standard_price);

        // Test congestion impact
        oracle.update_congestion(2000, 0.8);
        let congested_price = oracle.get_gas_price(GasPriority::Standard);
        assert!(congested_price > 1000);
    }
}
