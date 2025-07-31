//! VM Execution Engine
//!
//! This module provides the core execution engine for smart contracts,
//! including bytecode interpretation, stack management, and opcode execution.

use crate::vm::{Contract, GAS_COSTS, GasMeter, VMContext, VMError, VMLog, VMResult, VMStorage};
use log::{debug, error, info, warn};
use mona_crypto::hash_data_blake3;
use mona_types::address::Address;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// VM opcode definitions
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Opcode {
    // Arithmetic operations
    ADD = 0x01,
    MUL = 0x02,
    SUB = 0x03,
    DIV = 0x04,
    MOD = 0x06,
    ADDMOD = 0x08,
    MULMOD = 0x09,
    EXP = 0x0a,
    SIGNEXTEND = 0x0b,

    // Comparison operations
    LT = 0x10,
    GT = 0x11,
    SLT = 0x12,
    SGT = 0x13,
    EQ = 0x14,
    ISZERO = 0x15,
    AND = 0x16,
    OR = 0x17,
    XOR = 0x18,
    NOT = 0x19,
    BYTE = 0x1a,
    SHL = 0x1b,
    SHR = 0x1c,
    SAR = 0x1d,

    // Cryptographic operations
    KECCAK256 = 0x20,
    BLAKE3 = 0x21,

    // Environmental information
    ADDRESS = 0x30,
    BALANCE = 0x31,
    ORIGIN = 0x32,
    CALLER = 0x33,
    CALLVALUE = 0x34,
    CALLDATALOAD = 0x35,
    CALLDATASIZE = 0x36,
    CALLDATACOPY = 0x37,
    CODESIZE = 0x38,
    CODECOPY = 0x39,
    GASPRICE = 0x3a,
    EXTCODESIZE = 0x3b,
    EXTCODECOPY = 0x3c,
    RETURNDATASIZE = 0x3d,
    RETURNDATACOPY = 0x3e,

    // Block information
    BLOCKHASH = 0x40,
    COINBASE = 0x41,
    TIMESTAMP = 0x42,
    NUMBER = 0x43,
    DIFFICULTY = 0x44,
    GASLIMIT = 0x45,
    CHAINID = 0x46,

    // Stack operations
    POP = 0x50,
    MLOAD = 0x51,
    MSTORE = 0x52,
    MSTORE8 = 0x53,
    SLOAD = 0x54,
    SSTORE = 0x55,
    JUMP = 0x56,
    JUMPI = 0x57,
    PC = 0x58,
    MSIZE = 0x59,
    GAS = 0x5a,
    JUMPDEST = 0x5b,

    // Push operations (PUSH1 to PUSH32)
    PUSH1 = 0x60,
    PUSH2 = 0x61,
    PUSH3 = 0x62,
    PUSH4 = 0x63,
    PUSH5 = 0x64,
    PUSH32 = 0x7f,

    // Duplicate operations (DUP1 to DUP16)
    DUP1 = 0x80,
    DUP2 = 0x81,
    DUP16 = 0x8f,

    // Swap operations (SWAP1 to SWAP16)
    SWAP1 = 0x90,
    SWAP2 = 0x91,
    SWAP16 = 0x9f,

    // Logging operations
    LOG0 = 0xa0,
    LOG1 = 0xa1,
    LOG2 = 0xa2,
    LOG3 = 0xa3,
    LOG4 = 0xa4,

    // System operations
    CREATE = 0xf0,
    CALL = 0xf1,
    CALLCODE = 0xf2,
    RETURN = 0xf3,
    DELEGATECALL = 0xf4,
    CREATE2 = 0xf5,
    STATICCALL = 0xfa,
    REVERT = 0xfd,
    INVALID = 0xfe,
    SELFDESTRUCT = 0xff,

    // Stop operation
    STOP = 0x00,
}

impl From<u8> for Opcode {
    fn from(byte: u8) -> Self {
        match byte {
            0x00 => Opcode::STOP,
            0x01 => Opcode::ADD,
            0x02 => Opcode::MUL,
            0x03 => Opcode::SUB,
            0x04 => Opcode::DIV,
            0x06 => Opcode::MOD,
            0x08 => Opcode::ADDMOD,
            0x09 => Opcode::MULMOD,
            0x0a => Opcode::EXP,
            0x0b => Opcode::SIGNEXTEND,
            0x10 => Opcode::LT,
            0x11 => Opcode::GT,
            0x12 => Opcode::SLT,
            0x13 => Opcode::SGT,
            0x14 => Opcode::EQ,
            0x15 => Opcode::ISZERO,
            0x16 => Opcode::AND,
            0x17 => Opcode::OR,
            0x18 => Opcode::XOR,
            0x19 => Opcode::NOT,
            0x1a => Opcode::BYTE,
            0x1b => Opcode::SHL,
            0x1c => Opcode::SHR,
            0x1d => Opcode::SAR,
            0x20 => Opcode::KECCAK256,
            0x21 => Opcode::BLAKE3,
            0x30 => Opcode::ADDRESS,
            0x31 => Opcode::BALANCE,
            0x32 => Opcode::ORIGIN,
            0x33 => Opcode::CALLER,
            0x34 => Opcode::CALLVALUE,
            0x35 => Opcode::CALLDATALOAD,
            0x36 => Opcode::CALLDATASIZE,
            0x37 => Opcode::CALLDATACOPY,
            0x38 => Opcode::CODESIZE,
            0x39 => Opcode::CODECOPY,
            0x3a => Opcode::GASPRICE,
            0x3b => Opcode::EXTCODESIZE,
            0x3c => Opcode::EXTCODECOPY,
            0x3d => Opcode::RETURNDATASIZE,
            0x3e => Opcode::RETURNDATACOPY,
            0x40 => Opcode::BLOCKHASH,
            0x41 => Opcode::COINBASE,
            0x42 => Opcode::TIMESTAMP,
            0x43 => Opcode::NUMBER,
            0x44 => Opcode::DIFFICULTY,
            0x45 => Opcode::GASLIMIT,
            0x46 => Opcode::CHAINID,
            0x50 => Opcode::POP,
            0x51 => Opcode::MLOAD,
            0x52 => Opcode::MSTORE,
            0x53 => Opcode::MSTORE8,
            0x54 => Opcode::SLOAD,
            0x55 => Opcode::SSTORE,
            0x56 => Opcode::JUMP,
            0x57 => Opcode::JUMPI,
            0x58 => Opcode::PC,
            0x59 => Opcode::MSIZE,
            0x5a => Opcode::GAS,
            0x5b => Opcode::JUMPDEST,
            0x60..=0x7f => {
                // PUSH1 to PUSH32
                let push_size = byte - 0x60 + 1;
                match push_size {
                    1 => Opcode::PUSH1,
                    2 => Opcode::PUSH2,
                    3 => Opcode::PUSH3,
                    4 => Opcode::PUSH4,
                    5 => Opcode::PUSH5,
                    32 => Opcode::PUSH32,
                    _ => Opcode::PUSH1, // Default to PUSH1 for intermediate values
                }
            }
            0x80..=0x8f => {
                // DUP1 to DUP16
                match byte - 0x80 + 1 {
                    1 => Opcode::DUP1,
                    2 => Opcode::DUP2,
                    16 => Opcode::DUP16,
                    _ => Opcode::DUP1, // Default to DUP1
                }
            }
            0x90..=0x9f => {
                // SWAP1 to SWAP16
                match byte - 0x90 + 1 {
                    1 => Opcode::SWAP1,
                    2 => Opcode::SWAP2,
                    16 => Opcode::SWAP16,
                    _ => Opcode::SWAP1, // Default to SWAP1
                }
            }
            0xa0 => Opcode::LOG0,
            0xa1 => Opcode::LOG1,
            0xa2 => Opcode::LOG2,
            0xa3 => Opcode::LOG3,
            0xa4 => Opcode::LOG4,
            0xf0 => Opcode::CREATE,
            0xf1 => Opcode::CALL,
            0xf2 => Opcode::CALLCODE,
            0xf3 => Opcode::RETURN,
            0xf4 => Opcode::DELEGATECALL,
            0xf5 => Opcode::CREATE2,
            0xfa => Opcode::STATICCALL,
            0xfd => Opcode::REVERT,
            0xfe => Opcode::INVALID,
            0xff => Opcode::SELFDESTRUCT,
            _ => Opcode::INVALID,
        }
    }
}

/// VM execution stack
#[derive(Debug, Clone)]
pub struct VMStack {
    stack: Vec<[u8; 32]>, // 256-bit words
    limit: usize,
}

impl VMStack {
    pub fn new(limit: usize) -> Self {
        Self {
            stack: Vec::with_capacity(limit),
            limit,
        }
    }

    pub fn push(&mut self, value: [u8; 32]) -> Result<(), VMError> {
        if self.stack.len() >= self.limit {
            return Err(VMError::StackOverflow);
        }
        self.stack.push(value);
        Ok(())
    }

    pub fn pop(&mut self) -> Result<[u8; 32], VMError> {
        self.stack.pop().ok_or(VMError::StackUnderflow)
    }

    pub fn peek(&self, index: usize) -> Result<[u8; 32], VMError> {
        let stack_index = self.stack.len().saturating_sub(index + 1);
        self.stack
            .get(stack_index)
            .copied()
            .ok_or(VMError::StackUnderflow)
    }

    pub fn swap(&mut self, index: usize) -> Result<(), VMError> {
        let len = self.stack.len();
        if index >= len {
            return Err(VMError::StackUnderflow);
        }
        let swap_index = len - index - 1;
        self.stack.swap(len - 1, swap_index);
        Ok(())
    }

    pub fn duplicate(&mut self, index: usize) -> Result<(), VMError> {
        let value = self.peek(index)?;
        self.push(value)
    }

    pub fn size(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

/// VM memory management
#[derive(Debug, Clone)]
pub struct VMMemory {
    data: Vec<u8>,
    limit: usize,
}

impl VMMemory {
    pub fn new(limit: usize) -> Self {
        Self {
            data: Vec::new(),
            limit,
        }
    }

    pub fn expand(&mut self, offset: usize, size: usize) -> Result<(), VMError> {
        let required_size = offset.saturating_add(size);
        if required_size > self.limit {
            return Err(VMError::InvalidMemoryAccess);
        }

        if required_size > self.data.len() {
            self.data.resize(required_size, 0);
        }
        Ok(())
    }

    pub fn load(&mut self, offset: usize) -> Result<[u8; 32], VMError> {
        self.expand(offset, 32)?;
        let mut result = [0u8; 32];
        let end = (offset + 32).min(self.data.len());
        let copy_len = end.saturating_sub(offset);
        result[..copy_len].copy_from_slice(&self.data[offset..end]);
        Ok(result)
    }

    pub fn store(&mut self, offset: usize, value: [u8; 32]) -> Result<(), VMError> {
        self.expand(offset, 32)?;
        self.data[offset..offset + 32].copy_from_slice(&value);
        Ok(())
    }

    pub fn store8(&mut self, offset: usize, value: u8) -> Result<(), VMError> {
        self.expand(offset, 1)?;
        self.data[offset] = value;
        Ok(())
    }

    pub fn copy_from(&mut self, dest_offset: usize, src: &[u8]) -> Result<(), VMError> {
        self.expand(dest_offset, src.len())?;
        let end = dest_offset + src.len();
        self.data[dest_offset..end].copy_from_slice(src);
        Ok(())
    }

    pub fn get_slice(&self, offset: usize, size: usize) -> &[u8] {
        let start = offset.min(self.data.len());
        let end = (offset + size).min(self.data.len());
        &self.data[start..end]
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }
}

/// Contract execution context
pub struct ContractExecutor {
    storage: Arc<RwLock<VMStorage>>,
    gas_meter: Arc<Mutex<GasMeter>>,
    stack_limit: usize,
    memory_limit: usize,
}

impl ContractExecutor {
    pub fn new(
        storage: Arc<RwLock<VMStorage>>,
        gas_meter: Arc<Mutex<GasMeter>>,
        stack_limit: usize,
        memory_limit: usize,
    ) -> Self {
        Self {
            storage,
            gas_meter,
            stack_limit,
            memory_limit,
        }
    }

    /// Execute contract constructor
    pub fn execute_constructor(
        &mut self,
        contract: &Contract,
        context: &VMContext,
        args: Vec<u8>,
    ) -> Result<VMResult, VMError> {
        debug!("Executing constructor for contract {}", contract.address);

        let mut execution_context = ExecutionContext::new(
            contract.bytecode.clone(),
            args,
            context.clone(),
            self.stack_limit,
            self.memory_limit,
        );

        self.execute_bytecode(&mut execution_context)
    }

    /// Execute contract function
    pub fn execute_function(
        &mut self,
        contract: &Contract,
        context: &VMContext,
        function_selector: [u8; 4],
        args: Vec<u8>,
    ) -> Result<VMResult, VMError> {
        debug!(
            "Executing function {:?} on contract {}",
            hex::encode(function_selector),
            contract.address
        );

        // Use runtime bytecode if available, otherwise use deployment bytecode
        let bytecode = contract
            .runtime_bytecode
            .as_ref()
            .unwrap_or(&contract.bytecode)
            .clone();

        // Prepare call data (function selector + arguments)
        let mut call_data = Vec::with_capacity(4 + args.len());
        call_data.extend_from_slice(&function_selector);
        call_data.extend_from_slice(&args);

        let mut execution_context = ExecutionContext::new(
            bytecode,
            call_data,
            context.clone(),
            self.stack_limit,
            self.memory_limit,
        );

        self.execute_bytecode(&mut execution_context)
    }

    /// Execute bytecode
    fn execute_bytecode(&mut self, context: &mut ExecutionContext) -> Result<VMResult, VMError> {
        let mut result = VMResult {
            success: true,
            return_data: Vec::new(),
            gas_used: 0,
            logs: Vec::new(),
            error: None,
            state_changes: HashMap::new(),
        };

        let mut jump_destinations = self.find_jump_destinations(&context.bytecode);

        loop {
            // Check gas limit
            {
                let gas_meter = self.gas_meter.lock().unwrap();
                if gas_meter.out_of_gas() {
                    result.success = false;
                    result.error = Some("Out of gas".to_string());
                    break;
                }
                result.gas_used = gas_meter.gas_used();
            }

            // Check if we've reached the end of bytecode
            if context.pc >= context.bytecode.len() {
                break;
            }

            // Get current opcode
            let opcode_byte = context.bytecode[context.pc];
            let opcode = Opcode::from(opcode_byte);
            context.pc += 1;

            debug!("Executing opcode: {:?} at PC: {}", opcode, context.pc - 1);

            // Execute the opcode
            match self.execute_opcode(opcode, context, &mut result, &mut jump_destinations) {
                Ok(ExecutionResult::Continue) => continue,
                Ok(ExecutionResult::Stop) => break,
                Ok(ExecutionResult::Return(data)) => {
                    result.return_data = data;
                    break;
                }
                Ok(ExecutionResult::Revert(data)) => {
                    result.success = false;
                    result.return_data = data;
                    result.error = Some("Execution reverted".to_string());
                    break;
                }
                Err(e) => {
                    result.success = false;
                    result.error = Some(format!("VM Error: {}", e));
                    break;
                }
            }
        }

        // Final gas usage
        {
            let gas_meter = self.gas_meter.lock().unwrap();
            result.gas_used = gas_meter.gas_used();
        }

        debug!(
            "Execution completed. Success: {}, Gas used: {}",
            result.success, result.gas_used
        );

        Ok(result)
    }

    /// Execute a single opcode
    fn execute_opcode(
        &mut self,
        opcode: Opcode,
        context: &mut ExecutionContext,
        result: &mut VMResult,
        jump_destinations: &mut HashMap<usize, bool>,
    ) -> Result<ExecutionResult, VMError> {
        // Consume base gas for the operation
        self.consume_gas(GAS_COSTS.base)?;

        match opcode {
            Opcode::STOP => Ok(ExecutionResult::Stop),

            // Arithmetic operations
            Opcode::ADD => {
                let a = self.u256_from_stack(&mut context.stack)?;
                let b = self.u256_from_stack(&mut context.stack)?;
                let result_val = a.wrapping_add(b);
                self.u256_to_stack(&mut context.stack, result_val)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::SUB => {
                let a = self.u256_from_stack(&mut context.stack)?;
                let b = self.u256_from_stack(&mut context.stack)?;
                let result_val = a.wrapping_sub(b);
                self.u256_to_stack(&mut context.stack, result_val)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::MUL => {
                let a = self.u256_from_stack(&mut context.stack)?;
                let b = self.u256_from_stack(&mut context.stack)?;
                let result_val = a.wrapping_mul(b);
                self.u256_to_stack(&mut context.stack, result_val)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::DIV => {
                let a = self.u256_from_stack(&mut context.stack)?;
                let b = self.u256_from_stack(&mut context.stack)?;
                let result_val = if b == 0 { 0 } else { a / b };
                self.u256_to_stack(&mut context.stack, result_val)?;
                Ok(ExecutionResult::Continue)
            }

            // Comparison operations
            Opcode::LT => {
                let a = self.u256_from_stack(&mut context.stack)?;
                let b = self.u256_from_stack(&mut context.stack)?;
                let result_val = if a < b { 1 } else { 0 };
                self.u256_to_stack(&mut context.stack, result_val)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::GT => {
                let a = self.u256_from_stack(&mut context.stack)?;
                let b = self.u256_from_stack(&mut context.stack)?;
                let result_val = if a > b { 1 } else { 0 };
                self.u256_to_stack(&mut context.stack, result_val)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::EQ => {
                let a = self.u256_from_stack(&mut context.stack)?;
                let b = self.u256_from_stack(&mut context.stack)?;
                let result_val = if a == b { 1 } else { 0 };
                self.u256_to_stack(&mut context.stack, result_val)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::ISZERO => {
                let a = self.u256_from_stack(&mut context.stack)?;
                let result_val = if a == 0 { 1 } else { 0 };
                self.u256_to_stack(&mut context.stack, result_val)?;
                Ok(ExecutionResult::Continue)
            }

            // Stack operations
            Opcode::POP => {
                context.stack.pop()?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::PUSH1 => {
                let value = self.read_push_data(&context.bytecode, &mut context.pc, 1)?;
                context.stack.push(value)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::PUSH4 => {
                let value = self.read_push_data(&context.bytecode, &mut context.pc, 4)?;
                context.stack.push(value)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::PUSH32 => {
                let value = self.read_push_data(&context.bytecode, &mut context.pc, 32)?;
                context.stack.push(value)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::DUP1 => {
                context.stack.duplicate(0)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::SWAP1 => {
                context.stack.swap(1)?;
                Ok(ExecutionResult::Continue)
            }

            // Memory operations
            Opcode::MLOAD => {
                let offset = self.u256_from_stack(&mut context.stack)? as usize;
                let value = context.memory.load(offset)?;
                context.stack.push(value)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::MSTORE => {
                let offset = self.u256_from_stack(&mut context.stack)? as usize;
                let value = context.stack.pop()?;
                context.memory.store(offset, value)?;
                Ok(ExecutionResult::Continue)
            }

            // Storage operations
            Opcode::SLOAD => {
                self.consume_gas(GAS_COSTS.sload)?;
                let key_bytes = context.stack.pop()?;
                let key = key_bytes.to_vec();

                let value = {
                    let storage = self.storage.read().unwrap();
                    storage
                        .get_storage(&context.vm_context.contract_address, &key)
                        .unwrap_or_else(|| vec![0u8; 32])
                };

                let mut value_array = [0u8; 32];
                value_array[..value.len().min(32)].copy_from_slice(&value[..value.len().min(32)]);
                context.stack.push(value_array)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::SSTORE => {
                self.consume_gas(GAS_COSTS.sstore)?;
                let key_bytes = context.stack.pop()?;
                let value_bytes = context.stack.pop()?;
                let key = key_bytes.to_vec();
                let value = value_bytes.to_vec();

                {
                    let mut storage = self.storage.write().unwrap();
                    storage.set_storage(
                        context.vm_context.contract_address.clone(),
                        key.clone(),
                        value.clone(),
                    );
                }

                // Record state change
                result.state_changes.insert(hex::encode(&key), value);
                Ok(ExecutionResult::Continue)
            }

            // Jump operations
            Opcode::JUMP => {
                let dest = self.u256_from_stack(&mut context.stack)? as usize;
                if !jump_destinations.get(&dest).unwrap_or(&false) {
                    return Err(VMError::InvalidJump);
                }
                context.pc = dest;
                Ok(ExecutionResult::Continue)
            }

            Opcode::JUMPI => {
                let dest = self.u256_from_stack(&mut context.stack)? as usize;
                let condition = self.u256_from_stack(&mut context.stack)?;
                if condition != 0 {
                    if !jump_destinations.get(&dest).unwrap_or(&false) {
                        return Err(VMError::InvalidJump);
                    }
                    context.pc = dest;
                }
                Ok(ExecutionResult::Continue)
            }

            Opcode::JUMPDEST => {
                // Valid jump destination, no operation
                Ok(ExecutionResult::Continue)
            }

            // Environmental operations
            Opcode::ADDRESS => {
                let address_bytes = context
                    .vm_context
                    .contract_address
                    .to_string()
                    .as_bytes()
                    .to_vec();
                let mut value = [0u8; 32];
                let copy_len = address_bytes.len().min(32);
                value[32 - copy_len..].copy_from_slice(&address_bytes[..copy_len]);
                context.stack.push(value)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::CALLER => {
                let caller_bytes = context.vm_context.caller.to_string().as_bytes().to_vec();
                let mut value = [0u8; 32];
                let copy_len = caller_bytes.len().min(32);
                value[32 - copy_len..].copy_from_slice(&caller_bytes[..copy_len]);
                context.stack.push(value)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::CALLVALUE => {
                self.u256_to_stack(&mut context.stack, context.vm_context.value as u128)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::CALLDATASIZE => {
                self.u256_to_stack(&mut context.stack, context.call_data.len() as u128)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::CALLDATALOAD => {
                let offset = self.u256_from_stack(&mut context.stack)? as usize;
                let mut value = [0u8; 32];
                let data_len = context.call_data.len();
                if offset < data_len {
                    let copy_len = (data_len - offset).min(32);
                    value[..copy_len]
                        .copy_from_slice(&context.call_data[offset..offset + copy_len]);
                }
                context.stack.push(value)?;
                Ok(ExecutionResult::Continue)
            }

            // Hashing operations
            Opcode::BLAKE3 => {
                let offset = self.u256_from_stack(&mut context.stack)? as usize;
                let size = self.u256_from_stack(&mut context.stack)? as usize;
                let data = context.memory.get_slice(offset, size);
                let hash = hash_data_blake3(data);
                let mut result_bytes = [0u8; 32];
                result_bytes.copy_from_slice(&hash);
                context.stack.push(result_bytes)?;
                Ok(ExecutionResult::Continue)
            }

            // Return operations
            Opcode::RETURN => {
                let offset = self.u256_from_stack(&mut context.stack)? as usize;
                let size = self.u256_from_stack(&mut context.stack)? as usize;
                let return_data = context.memory.get_slice(offset, size).to_vec();
                Ok(ExecutionResult::Return(return_data))
            }

            Opcode::REVERT => {
                let offset = self.u256_from_stack(&mut context.stack)? as usize;
                let size = self.u256_from_stack(&mut context.stack)? as usize;
                let revert_data = context.memory.get_slice(offset, size).to_vec();
                Ok(ExecutionResult::Revert(revert_data))
            }

            // Block information
            Opcode::TIMESTAMP => {
                self.u256_to_stack(
                    &mut context.stack,
                    context.vm_context.block_timestamp as u128,
                )?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::NUMBER => {
                self.u256_to_stack(&mut context.stack, context.vm_context.block_number as u128)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::CHAINID => {
                // Convert chain ID string to numeric value
                let chain_id_numeric = context.vm_context.chain_id.len() as u128;
                self.u256_to_stack(&mut context.stack, chain_id_numeric)?;
                Ok(ExecutionResult::Continue)
            }

            // Gas operations
            Opcode::GAS => {
                let remaining_gas = {
                    let gas_meter = self.gas_meter.lock().unwrap();
                    gas_meter.gas_remaining()
                };
                self.u256_to_stack(&mut context.stack, remaining_gas as u128)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::GASPRICE => {
                self.u256_to_stack(&mut context.stack, context.vm_context.gas_price as u128)?;
                Ok(ExecutionResult::Continue)
            }

            // Program counter
            Opcode::PC => {
                self.u256_to_stack(&mut context.stack, (context.pc - 1) as u128)?;
                Ok(ExecutionResult::Continue)
            }

            // Memory size
            Opcode::MSIZE => {
                self.u256_to_stack(&mut context.stack, context.memory.size() as u128)?;
                Ok(ExecutionResult::Continue)
            }

            // Logging operations
            Opcode::LOG0 => {
                self.execute_log(0, context, result)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::LOG1 => {
                self.execute_log(1, context, result)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::LOG2 => {
                self.execute_log(2, context, result)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::LOG3 => {
                self.execute_log(3, context, result)?;
                Ok(ExecutionResult::Continue)
            }

            Opcode::LOG4 => {
                self.execute_log(4, context, result)?;
                Ok(ExecutionResult::Continue)
            }

            _ => {
                warn!("Unimplemented opcode: {:?}", opcode);
                Err(VMError::InvalidInstruction(opcode as u8))
            }
        }
    }

    /// Helper method to consume gas
    fn consume_gas(&self, amount: u64) -> Result<(), VMError> {
        let mut gas_meter = self.gas_meter.lock().unwrap();
        gas_meter.consume_gas(amount)
    }

    /// Convert stack value to u128 (simplified 256-bit handling)
    fn u256_from_stack(&self, stack: &mut VMStack) -> Result<u128, VMError> {
        let bytes = stack.pop()?;
        // Convert first 16 bytes to u128 (simplified)
        let mut result_bytes = [0u8; 16];
        result_bytes.copy_from_slice(&bytes[16..32]);
        Ok(u128::from_be_bytes(result_bytes))
    }

    /// Convert u128 to stack value
    fn u256_to_stack(&self, stack: &mut VMStack, value: u128) -> Result<(), VMError> {
        let mut bytes = [0u8; 32];
        bytes[16..32].copy_from_slice(&value.to_be_bytes());
        stack.push(bytes)
    }

    /// Read push data from bytecode
    fn read_push_data(
        &self,
        bytecode: &[u8],
        pc: &mut usize,
        size: usize,
    ) -> Result<[u8; 32], VMError> {
        let mut result = [0u8; 32];
        let available = (bytecode.len() - *pc).min(size);

        if available > 0 {
            result[32 - available..].copy_from_slice(&bytecode[*pc..*pc + available]);
            *pc += size; // Always advance by the full size
        }

        Ok(result)
    }

    /// Find valid jump destinations in bytecode
    fn find_jump_destinations(&self, bytecode: &[u8]) -> HashMap<usize, bool> {
        let mut destinations = HashMap::new();
        let mut i = 0;

        while i < bytecode.len() {
            let opcode = Opcode::from(bytecode[i]);

            if opcode == Opcode::JUMPDEST {
                destinations.insert(i, true);
            }

            // Skip push data
            if bytecode[i] >= 0x60 && bytecode[i] <= 0x7f {
                let push_size = (bytecode[i] - 0x60 + 1) as usize;
                i += push_size + 1;
            } else {
                i += 1;
            }
        }

        destinations
    }

    /// Execute LOG operations
    fn execute_log(
        &self,
        topic_count: usize,
        context: &mut ExecutionContext,
        result: &mut VMResult,
    ) -> Result<(), VMError> {
        self.consume_gas(GAS_COSTS.log_base + GAS_COSTS.log_topic * topic_count as u64)?;

        let offset = self.u256_from_stack(&mut context.stack)? as usize;
        let size = self.u256_from_stack(&mut context.stack)? as usize;

        let mut topics = Vec::new();
        for _ in 0..topic_count {
            let topic_bytes = context.stack.pop()?;
            topics.push(hex::encode(topic_bytes));
        }

        let data = context.memory.get_slice(offset, size).to_vec();

        let log = VMLog {
            address: context.vm_context.contract_address.clone(),
            topics,
            data,
            block_number: context.vm_context.block_number,
            transaction_hash: format!("tx_{}", context.vm_context.block_timestamp),
        };

        result.logs.push(log);
        Ok(())
    }
}

/// Execution result types
#[derive(Debug)]
enum ExecutionResult {
    Continue,
    Stop,
    Return(Vec<u8>),
    Revert(Vec<u8>),
}

/// Execution context for a single contract call
struct ExecutionContext {
    /// Contract bytecode being executed
    bytecode: Vec<u8>,
    /// Call data (function selector + arguments)
    call_data: Vec<u8>,
    /// VM context
    vm_context: VMContext,
    /// Execution stack
    stack: VMStack,
    /// Contract memory
    memory: VMMemory,
    /// Program counter
    pc: usize,
}

impl ExecutionContext {
    fn new(
        bytecode: Vec<u8>,
        call_data: Vec<u8>,
        vm_context: VMContext,
        stack_limit: usize,
        memory_limit: usize,
    ) -> Self {
        Self {
            bytecode,
            call_data,
            vm_context,
            stack: VMStack::new(stack_limit),
            memory: VMMemory::new(memory_limit),
            pc: 0,
        }
    }
}
