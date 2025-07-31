//! Bytecode handling and analysis for the Virtual Machine
//!
//! This module provides utilities for parsing, validating, optimizing,
//! and analyzing smart contract bytecode.

use crate::vm::{Opcode, VMError};
use log::{debug, info, warn};
use mona_crypto::hash_data_blake3;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Bytecode analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BytecodeAnalysis {
    /// Total bytecode size
    pub size: usize,
    /// Number of instructions
    pub instruction_count: usize,
    /// Opcodes used and their frequencies
    pub opcode_frequencies: HashMap<u8, usize>,
    /// Jump destinations
    pub jump_destinations: HashSet<usize>,
    /// Potential security issues
    pub security_issues: Vec<SecurityIssue>,
    /// Gas estimation
    pub estimated_gas: u64,
    /// Complexity score (0-100)
    pub complexity_score: u8,
    /// Whether bytecode is valid
    pub is_valid: bool,
    /// Error messages if invalid
    pub errors: Vec<String>,
}

/// Security issue types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityIssue {
    /// Issue type
    pub issue_type: SecurityIssueType,
    /// Location in bytecode
    pub location: usize,
    /// Severity level
    pub severity: Severity,
    /// Description of the issue
    pub description: String,
}

/// Security issue types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SecurityIssueType {
    /// Reentrancy vulnerability
    Reentrancy,
    /// Integer overflow/underflow
    IntegerOverflow,
    /// Unchecked external call
    UncheckedCall,
    /// Gas limit issues
    GasLimit,
    /// Uninitialized storage
    UninitializedStorage,
    /// Dangerous delegatecall
    DangerousDelegatecall,
    /// Selfdestruct usage
    SelfdestructUsage,
    /// Random number vulnerability
    WeakRandomness,
    /// Access control issues
    AccessControl,
    /// DoS vulnerability
    DenialOfService,
}

/// Issue severity levels
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// Bytecode instruction representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instruction {
    /// Program counter offset
    pub pc: usize,
    /// Opcode
    pub opcode: u8,
    /// Opcode name
    pub opcode_name: String,
    /// Operand data (for PUSH instructions)
    pub operand: Option<Vec<u8>>,
    /// Gas cost for this instruction
    pub gas_cost: u64,
    /// Stack effect (items consumed, items produced)
    pub stack_effect: (i8, i8),
}

/// Bytecode optimization options
#[derive(Debug, Clone)]
pub struct OptimizationOptions {
    /// Remove unreachable code
    pub remove_dead_code: bool,
    /// Optimize jump sequences
    pub optimize_jumps: bool,
    /// Merge duplicate code blocks
    pub merge_duplicates: bool,
    /// Optimize PUSH/POP sequences
    pub optimize_stack_ops: bool,
    /// Inline simple functions
    pub inline_functions: bool,
}

impl Default for OptimizationOptions {
    fn default() -> Self {
        Self {
            remove_dead_code: true,
            optimize_jumps: true,
            merge_duplicates: false, // Can be expensive
            optimize_stack_ops: true,
            inline_functions: false, // Can increase code size
        }
    }
}

/// Bytecode parser and analyzer
pub struct BytecodeAnalyzer {
    /// Optimization options
    optimization_options: OptimizationOptions,
}

impl BytecodeAnalyzer {
    /// Create a new bytecode analyzer
    pub fn new() -> Self {
        Self {
            optimization_options: OptimizationOptions::default(),
        }
    }

    /// Create analyzer with custom optimization options
    pub fn with_options(options: OptimizationOptions) -> Self {
        Self {
            optimization_options: options,
        }
    }

    /// Analyze bytecode and return analysis results
    pub fn analyze(&self, bytecode: &[u8]) -> BytecodeAnalysis {
        let mut analysis = BytecodeAnalysis {
            size: bytecode.len(),
            instruction_count: 0,
            opcode_frequencies: HashMap::new(),
            jump_destinations: HashSet::new(),
            security_issues: Vec::new(),
            estimated_gas: 0,
            complexity_score: 0,
            is_valid: true,
            errors: Vec::new(),
        };

        // Parse instructions
        let instructions = match self.parse_instructions(bytecode) {
            Ok(instructions) => instructions,
            Err(e) => {
                analysis.is_valid = false;
                analysis
                    .errors
                    .push(format!("Failed to parse bytecode: {}", e));
                return analysis;
            }
        };

        analysis.instruction_count = instructions.len();

        // Analyze each instruction
        for instruction in &instructions {
            // Count opcode frequencies
            *analysis
                .opcode_frequencies
                .entry(instruction.opcode)
                .or_insert(0) += 1;

            // Accumulate gas estimation
            analysis.estimated_gas += instruction.gas_cost;

            // Find jump destinations
            if instruction.opcode == Opcode::JUMPDEST as u8 {
                analysis.jump_destinations.insert(instruction.pc);
            }
        }

        // Perform security analysis
        analysis.security_issues = self.analyze_security_issues(&instructions);

        // Calculate complexity score
        analysis.complexity_score = self.calculate_complexity(&instructions, &analysis);

        // Validate bytecode structure
        if let Err(validation_errors) = self.validate_bytecode(&instructions) {
            analysis.is_valid = false;
            analysis.errors.extend(validation_errors);
        }

        debug!(
            "Bytecode analysis complete: {} instructions, {} gas, complexity {}",
            analysis.instruction_count, analysis.estimated_gas, analysis.complexity_score
        );

        analysis
    }

    /// Parse bytecode into instructions
    pub fn parse_instructions(&self, bytecode: &[u8]) -> Result<Vec<Instruction>, VMError> {
        let mut instructions = Vec::new();
        let mut pc = 0;

        while pc < bytecode.len() {
            let opcode_byte = bytecode[pc];
            let opcode = Opcode::from(opcode_byte);
            let opcode_name = format!("{:?}", opcode);

            let mut operand = None;
            let mut instruction_size = 1;

            // Handle PUSH instructions (0x60-0x7f)
            if opcode_byte >= 0x60 && opcode_byte <= 0x7f {
                let push_size = (opcode_byte - 0x60 + 1) as usize;
                let operand_start = pc + 1;
                let operand_end = (operand_start + push_size).min(bytecode.len());

                if operand_end > operand_start {
                    operand = Some(bytecode[operand_start..operand_end].to_vec());
                }

                instruction_size = 1 + push_size;
            }

            // Get gas cost and stack effect
            let gas_cost = self.get_instruction_gas_cost(opcode_byte);
            let stack_effect = self.get_stack_effect(opcode_byte);

            let instruction = Instruction {
                pc,
                opcode: opcode_byte,
                opcode_name,
                operand,
                gas_cost,
                stack_effect,
            };

            instructions.push(instruction);
            pc += instruction_size;
        }

        Ok(instructions)
    }

    /// Disassemble bytecode to human-readable format
    pub fn disassemble(&self, bytecode: &[u8]) -> Result<String, VMError> {
        let instructions = self.parse_instructions(bytecode)?;
        let mut output = String::new();

        output.push_str("Address  Opcode    Operand                          Gas   Stack\n");
        output.push_str("-------  --------  ------------------------------  ----  -----\n");

        for instruction in instructions {
            let operand_str = if let Some(ref operand) = instruction.operand {
                let hex_str = hex::encode(operand);
                if hex_str.len() > 30 {
                    format!("{}...", &hex_str[..27])
                } else {
                    hex_str
                }
            } else {
                String::new()
            };

            let stack_str = format!(
                "{:+}→{:+}",
                -instruction.stack_effect.0, instruction.stack_effect.1
            );

            output.push_str(&format!(
                "{:07x}  {:8}  {:30}  {:4}  {}\n",
                instruction.pc,
                instruction.opcode_name,
                operand_str,
                instruction.gas_cost,
                stack_str
            ));
        }

        Ok(output)
    }

    /// Optimize bytecode based on optimization options
    pub fn optimize(&self, bytecode: &[u8]) -> Result<Vec<u8>, VMError> {
        let mut instructions = self.parse_instructions(bytecode)?;

        if self.optimization_options.remove_dead_code {
            instructions = self.remove_dead_code(instructions)?;
        }

        if self.optimization_options.optimize_jumps {
            instructions = self.optimize_jumps(instructions)?;
        }

        if self.optimization_options.optimize_stack_ops {
            instructions = self.optimize_stack_operations(instructions)?;
        }

        if self.optimization_options.merge_duplicates {
            instructions = self.merge_duplicate_blocks(instructions)?;
        }

        // Reassemble optimized instructions back to bytecode
        self.assemble_instructions(&instructions)
    }

    /// Validate bytecode structure
    fn validate_bytecode(&self, instructions: &[Instruction]) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        // Check for valid jump destinations
        let jump_destinations: HashSet<usize> = instructions
            .iter()
            .filter(|i| i.opcode == Opcode::JUMPDEST as u8)
            .map(|i| i.pc)
            .collect();

        // Validate JUMP and JUMPI targets
        for instruction in instructions {
            match instruction.opcode {
                op if op == Opcode::JUMP as u8 || op == Opcode::JUMPI as u8 => {
                    // This is a simplified check - in reality we'd need to track stack values
                    // For now, just ensure JUMPDEST instructions exist
                    if jump_destinations.is_empty() {
                        errors.push("JUMP instruction found but no JUMPDEST defined".to_string());
                    }
                }
                _ => {}
            }
        }

        // Check for proper instruction boundaries
        let mut expected_pc = 0;
        for instruction in instructions {
            if instruction.pc != expected_pc {
                errors.push(format!(
                    "Instruction PC mismatch: expected {}, found {}",
                    expected_pc, instruction.pc
                ));
            }

            expected_pc = instruction.pc + 1;
            if let Some(ref operand) = instruction.operand {
                expected_pc += operand.len();
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Analyze security issues in bytecode
    fn analyze_security_issues(&self, instructions: &[Instruction]) -> Vec<SecurityIssue> {
        let mut issues = Vec::new();

        for (i, instruction) in instructions.iter().enumerate() {
            match instruction.opcode {
                // Check for dangerous DELEGATECALL usage
                op if op == Opcode::DELEGATECALL as u8 => {
                    issues.push(SecurityIssue {
                        issue_type: SecurityIssueType::DangerousDelegatecall,
                        location: instruction.pc,
                        severity: Severity::High,
                        description: "DELEGATECALL can be dangerous if target is not trusted"
                            .to_string(),
                    });
                }

                // Check for SELFDESTRUCT usage
                op if op == Opcode::SELFDESTRUCT as u8 => {
                    issues.push(SecurityIssue {
                        issue_type: SecurityIssueType::SelfdestructUsage,
                        location: instruction.pc,
                        severity: Severity::Medium,
                        description: "SELFDESTRUCT usage detected - ensure proper access control"
                            .to_string(),
                    });
                }

                // Check for potential reentrancy patterns
                op if op == Opcode::CALL as u8 => {
                    // Look for SSTORE after CALL (simplified reentrancy check)
                    if let Some(next_instruction) = instructions.get(i + 1) {
                        if next_instruction.opcode == Opcode::SSTORE as u8 {
                            issues.push(SecurityIssue {
                                issue_type: SecurityIssueType::Reentrancy,
                                location: instruction.pc,
                                severity: Severity::High,
                                description:
                                    "Potential reentrancy: storage write after external call"
                                        .to_string(),
                            });
                        }
                    }
                }

                // Check for unchecked arithmetic (simplified)
                op if op == Opcode::ADD as u8 || op == Opcode::MUL as u8 => {
                    // In a more sophisticated analysis, we'd check if overflow protection exists
                    if i > 0
                        && instructions.get(i - 1).map(|prev| prev.opcode)
                            != Some(Opcode::DUP1 as u8)
                    {
                        issues.push(SecurityIssue {
                            issue_type: SecurityIssueType::IntegerOverflow,
                            location: instruction.pc,
                            severity: Severity::Medium,
                            description: "Potential integer overflow - consider using SafeMath"
                                .to_string(),
                        });
                    }
                }

                _ => {}
            }
        }

        issues
    }

    /// Calculate complexity score (0-100)
    fn calculate_complexity(
        &self,
        instructions: &[Instruction],
        analysis: &BytecodeAnalysis,
    ) -> u8 {
        let mut score = 0;

        // Base complexity from instruction count
        score += (instructions.len() / 10).min(30) as u8;

        // Complexity from unique opcodes
        score += (analysis.opcode_frequencies.len() / 5).min(20) as u8;

        // Complexity from control flow
        let control_flow_ops = analysis
            .opcode_frequencies
            .iter()
            .filter(|(op, _)| {
                matches!(
                    **op,
                    op if op == Opcode::JUMP as u8
                        || op == Opcode::JUMPI as u8
                        || op == Opcode::CALL as u8
                        || op == Opcode::DELEGATECALL as u8
                )
            })
            .map(|(_, count)| *count)
            .sum::<usize>();

        score += (control_flow_ops / 2).min(25) as u8;

        // Complexity from security issues
        let critical_issues = analysis
            .security_issues
            .iter()
            .filter(|issue| issue.severity >= Severity::High)
            .count();
        score += (critical_issues * 5).min(25) as u8;

        score.min(100)
    }

    /// Get gas cost for an instruction
    fn get_instruction_gas_cost(&self, opcode: u8) -> u64 {
        use crate::vm::GAS_COSTS;

        match opcode {
            0x00 => GAS_COSTS.base,            // STOP
            0x01 => GAS_COSTS.add,             // ADD
            0x02 => GAS_COSTS.mul,             // MUL
            0x03 => GAS_COSTS.sub,             // SUB
            0x04 => GAS_COSTS.div,             // DIV
            0x06 => GAS_COSTS.mod_op,          // MOD
            0x10..=0x1d => GAS_COSTS.very_low, // Comparison ops
            0x20 => GAS_COSTS.keccak256,       // KECCAK256
            0x21 => GAS_COSTS.blake3,          // BLAKE3
            0x30..=0x3e => GAS_COSTS.low,      // Environmental ops
            0x40..=0x46 => GAS_COSTS.base,     // Block info ops
            0x50 => GAS_COSTS.pop,             // POP
            0x51 => GAS_COSTS.mload,           // MLOAD
            0x52 => GAS_COSTS.mstore,          // MSTORE
            0x54 => GAS_COSTS.sload,           // SLOAD
            0x55 => GAS_COSTS.sstore,          // SSTORE
            0x56 => GAS_COSTS.jump,            // JUMP
            0x57 => GAS_COSTS.jumpi,           // JUMPI
            0x60..=0x7f => GAS_COSTS.push1,    // PUSH ops
            0x80..=0x8f => GAS_COSTS.dup1,     // DUP ops
            0x90..=0x9f => GAS_COSTS.swap1,    // SWAP ops
            0xa0..=0xa4 => GAS_COSTS.log_base, // LOG ops
            0xf0 => GAS_COSTS.create,          // CREATE
            0xf1 => GAS_COSTS.call,            // CALL
            0xf3 => GAS_COSTS.return_op,       // RETURN
            0xf4 => GAS_COSTS.delegatecall,    // DELEGATECALL
            0xff => GAS_COSTS.selfdestruct,    // SELFDESTRUCT
            _ => GAS_COSTS.base,               // Default
        }
    }

    /// Get stack effect for an instruction (items consumed, items produced)
    fn get_stack_effect(&self, opcode: u8) -> (i8, i8) {
        match opcode {
            0x00 => (0, 0),        // STOP
            0x01 => (2, 1),        // ADD
            0x02 => (2, 1),        // MUL
            0x03 => (2, 1),        // SUB
            0x04 => (2, 1),        // DIV
            0x06 => (2, 1),        // MOD
            0x10..=0x1d => (2, 1), // Comparison ops
            0x20 => (2, 1),        // KECCAK256
            0x21 => (2, 1),        // BLAKE3
            0x30 => (0, 1),        // ADDRESS
            0x31 => (1, 1),        // BALANCE
            0x32 => (0, 1),        // ORIGIN
            0x33 => (0, 1),        // CALLER
            0x34 => (0, 1),        // CALLVALUE
            0x35 => (1, 1),        // CALLDATALOAD
            0x36 => (0, 1),        // CALLDATASIZE
            0x37 => (3, 0),        // CALLDATACOPY
            0x50 => (1, 0),        // POP
            0x51 => (1, 1),        // MLOAD
            0x52 => (2, 0),        // MSTORE
            0x54 => (1, 1),        // SLOAD
            0x55 => (2, 0),        // SSTORE
            0x56 => (1, 0),        // JUMP
            0x57 => (2, 0),        // JUMPI
            0x58 => (0, 1),        // PC
            0x59 => (0, 1),        // MSIZE
            0x5a => (0, 1),        // GAS
            0x5b => (0, 0),        // JUMPDEST
            0x60..=0x7f => (0, 1), // PUSH ops
            0x80..=0x8f => {
                // DUP ops
                let dup_n = (opcode - 0x80 + 1) as i8;
                (0, 1) // Duplicates nth item
            }
            0x90..=0x9f => (2, 2), // SWAP ops
            0xa0 => (2, 0),        // LOG0
            0xa1 => (3, 0),        // LOG1
            0xa2 => (4, 0),        // LOG2
            0xa3 => (5, 0),        // LOG3
            0xa4 => (6, 0),        // LOG4
            0xf0 => (3, 1),        // CREATE
            0xf1 => (7, 1),        // CALL
            0xf2 => (7, 1),        // CALLCODE
            0xf3 => (2, 0),        // RETURN
            0xf4 => (6, 1),        // DELEGATECALL
            0xfa => (6, 1),        // STATICCALL
            0xfd => (2, 0),        // REVERT
            0xff => (1, 0),        // SELFDESTRUCT
            _ => (0, 0),           // Default/unknown
        }
    }

    /// Remove unreachable code
    fn remove_dead_code(
        &self,
        mut instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, VMError> {
        let mut reachable = HashSet::new();
        let mut to_visit = vec![0]; // Start from first instruction

        // Build instruction index
        let mut pc_to_index = HashMap::new();
        for (i, instruction) in instructions.iter().enumerate() {
            pc_to_index.insert(instruction.pc, i);
        }

        // Mark reachable instructions
        while let Some(pc) = to_visit.pop() {
            if reachable.contains(&pc) {
                continue;
            }
            reachable.insert(pc);

            if let Some(&index) = pc_to_index.get(&pc) {
                if let Some(instruction) = instructions.get(index) {
                    match instruction.opcode {
                        op if op == Opcode::JUMP as u8 => {
                            // Would need stack analysis to determine target
                            // For now, mark all JUMPDEST as potentially reachable
                            for inst in &instructions {
                                if inst.opcode == Opcode::JUMPDEST as u8 {
                                    to_visit.push(inst.pc);
                                }
                            }
                        }
                        op if op == Opcode::JUMPI as u8 => {
                            // Conditional jump - both paths reachable
                            if let Some(next_inst) = instructions.get(index + 1) {
                                to_visit.push(next_inst.pc);
                            }
                            // Also mark JUMPDEST targets as reachable
                            for inst in &instructions {
                                if inst.opcode == Opcode::JUMPDEST as u8 {
                                    to_visit.push(inst.pc);
                                }
                            }
                        }
                        op if op == Opcode::RETURN as u8
                            || op == Opcode::REVERT as u8
                            || op == Opcode::STOP as u8 =>
                        {
                            // Terminal instructions - don't continue
                        }
                        _ => {
                            // Regular instruction - continue to next
                            if let Some(next_inst) = instructions.get(index + 1) {
                                to_visit.push(next_inst.pc);
                            }
                        }
                    }
                }
            }
        }

        // Keep only reachable instructions
        instructions.retain(|inst| reachable.contains(&inst.pc));

        info!(
            "Dead code removal: kept {} reachable instructions",
            instructions.len()
        );
        Ok(instructions)
    }

    /// Optimize jump sequences
    fn optimize_jumps(&self, instructions: Vec<Instruction>) -> Result<Vec<Instruction>, VMError> {
        // Placeholder for jump optimization
        // Could optimize jump chains, remove unnecessary jumps, etc.
        Ok(instructions)
    }

    /// Optimize stack operations
    fn optimize_stack_operations(
        &self,
        mut instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, VMError> {
        // Remove redundant POP after PUSH of same value
        let mut optimized = Vec::new();
        let mut i = 0;

        while i < instructions.len() {
            let current = &instructions[i];

            // Look for PUSH followed by POP
            if current.opcode >= 0x60 && current.opcode <= 0x7f {
                if let Some(next) = instructions.get(i + 1) {
                    if next.opcode == Opcode::POP as u8 {
                        // Skip both PUSH and POP
                        i += 2;
                        continue;
                    }
                }
            }

            optimized.push(current.clone());
            i += 1;
        }

        info!(
            "Stack optimization: {} -> {} instructions",
            instructions.len(),
            optimized.len()
        );
        Ok(optimized)
    }

    /// Merge duplicate code blocks
    fn merge_duplicate_blocks(
        &self,
        instructions: Vec<Instruction>,
    ) -> Result<Vec<Instruction>, VMError> {
        // This is a complex optimization that would require control flow analysis
        // For now, return unchanged
        Ok(instructions)
    }

    /// Assemble instructions back to bytecode
    fn assemble_instructions(&self, instructions: &[Instruction]) -> Result<Vec<u8>, VMError> {
        let mut bytecode = Vec::new();

        for instruction in instructions {
            bytecode.push(instruction.opcode);

            if let Some(ref operand) = instruction.operand {
                bytecode.extend_from_slice(operand);
            }
        }

        Ok(bytecode)
    }
}

/// Bytecode compiler utilities
pub struct BytecodeCompiler {
    /// Target optimization level
    optimization_level: u8,
}

impl BytecodeCompiler {
    /// Create a new bytecode compiler
    pub fn new(optimization_level: u8) -> Self {
        Self { optimization_level }
    }

    /// Compile high-level operations to bytecode
    pub fn compile_operations(&self, operations: &[Operation]) -> Result<Vec<u8>, VMError> {
        let mut bytecode = Vec::new();

        for operation in operations {
            let op_bytecode = self.compile_operation(operation)?;
            bytecode.extend(op_bytecode);
        }

        // Apply optimizations if requested
        if self.optimization_level > 0 {
            let analyzer = BytecodeAnalyzer::new();
            bytecode = analyzer.optimize(&bytecode)?;
        }

        Ok(bytecode)
    }

    /// Compile a single operation
    fn compile_operation(&self, operation: &Operation) -> Result<Vec<u8>, VMError> {
        let mut bytecode = Vec::new();

        match operation {
            Operation::Push(value) => {
                let value_bytes = value.to_be_bytes();
                let push_size = self.minimal_push_size(&value_bytes);
                bytecode.push((0x60 + push_size - 1) as u8); // PUSH1-PUSH32
                bytecode.extend_from_slice(&value_bytes[value_bytes.len() - push_size..]);
            }
            Operation::Add => bytecode.push(Opcode::ADD as u8),
            Operation::Sub => bytecode.push(Opcode::SUB as u8),
            Operation::Mul => bytecode.push(Opcode::MUL as u8),
            Operation::Div => bytecode.push(Opcode::DIV as u8),
            Operation::Store => bytecode.push(Opcode::SSTORE as u8),
            Operation::Load => bytecode.push(Opcode::SLOAD as u8),
            Operation::Jump(target) => {
                // Push target address
                let target_bytes = target.to_be_bytes();
                let push_size = self.minimal_push_size(&target_bytes);
                bytecode.push((0x60 + push_size - 1) as u8);
                bytecode.extend_from_slice(&target_bytes[target_bytes.len() - push_size..]);
                bytecode.push(Opcode::JUMP as u8);
            }
            Operation::JumpIf(target) => {
                let target_bytes = target.to_be_bytes();
                let push_size = self.minimal_push_size(&target_bytes);
                bytecode.push((0x60 + push_size - 1) as u8);
                bytecode.extend_from_slice(&target_bytes[target_bytes.len() - push_size..]);
                bytecode.push(Opcode::JUMPI as u8);
            }
            Operation::Return => bytecode.push(Opcode::RETURN as u8),
            Operation::Revert => bytecode.push(Opcode::REVERT as u8),
        }

        Ok(bytecode)
    }

    /// Calculate minimal PUSH size needed for a value
    fn minimal_push_size(&self, bytes: &[u8]) -> usize {
        // Find first non-zero byte
        for (i, &byte) in bytes.iter().enumerate() {
            if byte != 0 {
                return bytes.len() - i;
            }
        }
        1 // At least PUSH1 for zero
    }
}

/// High-level operations for compilation
#[derive(Debug, Clone)]
pub enum Operation {
    Push(u64),
    Add,
    Sub,
    Mul,
    Div,
    Store,
    Load,
    Jump(u64),
    JumpIf(u64),
    Return,
    Revert,
}

/// Bytecode validation utilities
pub fn validate_bytecode_format(bytecode: &[u8]) -> Result<(), VMError> {
    if bytecode.is_empty() {
        return Err(VMError::InvalidTransaction("Empty bytecode".to_string()));
    }

    // Check maximum bytecode size (24KB limit like Ethereum)
    const MAX_BYTECODE_SIZE: usize = 24 * 1024;
    if bytecode.len() > MAX_BYTECODE_SIZE {
        return Err(VMError::InvalidTransaction(format!(
            "Bytecode too large: {} bytes (max: {})",
            bytecode.len(),
            MAX_BYTECODE_SIZE
        )));
    }

    let analyzer = BytecodeAnalyzer::new();
    let analysis = analyzer.analyze(bytecode);

    if !analysis.is_valid {
        return Err(VMError::InvalidTransaction(format!(
            "Invalid bytecode: {}",
            analysis.errors.join(", ")
        )));
    }

    Ok(())
}

/// Calculate bytecode hash for verification
pub fn calculate_bytecode_hash(bytecode: &[u8]) -> String {
    hex::encode(hash_data_blake3(bytecode))
}

/// Extract function selectors from bytecode
pub fn extract_function_selectors(bytecode: &[u8]) -> Result<Vec<[u8; 4]>, VMError> {
    let analyzer = BytecodeAnalyzer::new();
    let instructions = analyzer.parse_instructions(bytecode)?;
    let mut selectors = Vec::new();

    let mut i = 0;
    while i < instructions.len() {
        // Look for PUSH4 instructions that might be function selectors
        if let Some(instruction) = instructions.get(i) {
            if instruction.opcode == 0x63 {
                // PUSH4
                if let Some(ref operand) = instruction.operand {
                    if operand.len() == 4 {
                        let mut selector = [0u8; 4];
                        selector.copy_from_slice(operand);
                        selectors.push(selector);
                    }
                }
            }
        }
        i += 1;
    }

    Ok(selectors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytecode_analysis() {
        let analyzer = BytecodeAnalyzer::new();

        // Simple bytecode: PUSH1 1, PUSH1 2, ADD, STOP
        let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
        let analysis = analyzer.analyze(&bytecode);

        assert!(analysis.is_valid);
        assert_eq!(analysis.size, 6);
        assert!(analysis.instruction_count > 0);
        assert!(analysis.estimated_gas > 0);
    }

    #[test]
    fn test_instruction_parsing() {
        let analyzer = BytecodeAnalyzer::new();

        // PUSH1 0x42, POP
        let bytecode = vec![0x60, 0x42, 0x50];
        let instructions = analyzer.parse_instructions(&bytecode).unwrap();

        assert_eq!(instructions.len(), 2);
        assert_eq!(instructions[0].opcode, 0x60); // PUSH1
        assert_eq!(instructions[0].operand, Some(vec![0x42]));
        assert_eq!(instructions[1].opcode, 0x50); // POP
    }

    #[test]
    fn test_disassembly() {
        let analyzer = BytecodeAnalyzer::new();

        // Simple bytecode
        let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01];
        let disassembly = analyzer.disassemble(&bytecode).unwrap();

        assert!(disassembly.contains("PUSH1"));
        assert!(disassembly.contains("ADD"));
    }

    #[test]
    fn test_security_analysis() {
        let analyzer = BytecodeAnalyzer::new();

        // Bytecode with DELEGATECALL
        let bytecode = vec![0xf4]; // DELEGATECALL
        let analysis = analyzer.analyze(&bytecode);

        assert!(!analysis.security_issues.is_empty());
        assert!(
            analysis
                .security_issues
                .iter()
                .any(|issue| matches!(issue.issue_type, SecurityIssueType::DangerousDelegatecall))
        );
    }

    #[test]
    fn test_bytecode_optimization() {
        let analyzer = BytecodeAnalyzer::new();

        // PUSH1 42, POP (should be optimized away)
        let bytecode = vec![0x60, 0x42, 0x50];
        let optimized = analyzer.optimize(&bytecode).unwrap();

        // Should be shorter after optimization
        assert!(optimized.len() < bytecode.len());
    }

    #[test]
    fn test_bytecode_validation() {
        // Valid bytecode
        let valid_bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00];
        assert!(validate_bytecode_format(&valid_bytecode).is_ok());

        // Empty bytecode
        let empty_bytecode = vec![];
        assert!(validate_bytecode_format(&empty_bytecode).is_err());

        // Too large bytecode
        let large_bytecode = vec![0x00; 25 * 1024]; // 25KB
        assert!(validate_bytecode_format(&large_bytecode).is_err());
    }

    #[test]
    fn test_bytecode_compiler() {
        let compiler = BytecodeCompiler::new(0);

        let operations = vec![
            Operation::Push(42),
            Operation::Push(1),
            Operation::Add,
            Operation::Return,
        ];

        let bytecode = compiler.compile_operations(&operations).unwrap();
        assert!(!bytecode.is_empty());

        // Should contain PUSH, ADD, RETURN opcodes
        assert!(bytecode.contains(&0x60)); // PUSH1
        assert!(bytecode.contains(&0x01)); // ADD
        assert!(bytecode.contains(&0xf3)); // RETURN
    }

    #[test]
    fn test_function_selector_extraction() {
        // Bytecode with PUSH4 function selector
        let bytecode = vec![0x63, 0xa9, 0x05, 0x9c, 0xbb]; // PUSH4 0xa9059cbb (transfer)
        let selectors = extract_function_selectors(&bytecode).unwrap();

        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0], [0xa9, 0x05, 0x9c, 0xbb]);
    }

    #[test]
    fn test_bytecode_hash() {
        let bytecode = vec![0x60, 0x01, 0x60, 0x02, 0x01];
        let hash = calculate_bytecode_hash(&bytecode);

        assert_eq!(hash.len(), 64); // Blake3 produces 32 bytes = 64 hex chars

        // Same bytecode should produce same hash
        let hash2 = calculate_bytecode_hash(&bytecode);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_complexity_calculation() {
        let analyzer = BytecodeAnalyzer::new();

        // Simple bytecode
        let simple_bytecode = vec![0x60, 0x01, 0x00]; // PUSH1 1, STOP
        let simple_analysis = analyzer.analyze(&simple_bytecode);

        // Complex bytecode with loops and calls
        let complex_bytecode = vec![
            0x60, 0x01, // PUSH1 1
            0x60, 0x02, // PUSH1 2
            0x01, // ADD
            0x56, // JUMP
            0x5b, // JUMPDEST
            0xf1, // CALL
            0xf4, // DELEGATECALL
            0x00, // STOP
        ];
        let complex_analysis = analyzer.analyze(&complex_bytecode);

        assert!(complex_analysis.complexity_score > simple_analysis.complexity_score);
    }
}
