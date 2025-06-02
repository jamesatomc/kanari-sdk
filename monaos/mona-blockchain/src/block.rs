// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use crate::chain_id::CHAIN_ID;
use consensus_pos::HashAlgorithm;
use log;
use mona_crypto::verify_signature; // Add mona-crypto dependency
use mona_types::address::Address;
use serde::{Deserialize, Serialize};

// Define the Transaction struct
#[derive(Serialize, Deserialize, Clone)]
pub struct Transaction {
    pub transaction_id: String, // New field for transaction ID
    pub sender: Address,
    pub receiver: Address,
    pub amount: u64,
    pub gas_fee: u64, // Add gas fee field
    pub timestamp: u64,
    pub signature: Vec<u8>,    // Changed from Option<String> to Vec<u8>
    pub data: Option<Vec<u8>>, // Re-add data field as Option<Vec<u8>>
}

// Add methods to the Transaction struct
impl Transaction {
    // Create a message representation of the transaction for signing/verification
    pub fn to_signable_message(&self) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(self.transaction_id.as_bytes());
        message.extend_from_slice(self.sender.to_string().as_bytes());
        message.extend_from_slice(self.receiver.to_string().as_bytes());
        message.extend_from_slice(&self.amount.to_le_bytes());
        message.extend_from_slice(&self.gas_fee.to_le_bytes()); // Include gas fee in the signed message
        message.extend_from_slice(&self.timestamp.to_le_bytes());

        // For debugging
        log::debug!(
            "Generated message for signing/verification: tx_id={}, len={}",
            self.transaction_id,
            message.len()
        );

        message
    }

    // Verify the transaction signature
    pub fn verify(&self) -> bool {
        // Check if signature exists
        if self.signature.is_empty() {
            log::warn!("Transaction {} has no signature", self.transaction_id);
            return false;
        }

        // Generate the message that was originally signed
        let message = self.to_signable_message();

        // Verify signature using mona-crypto
        match verify_signature(&self.sender.to_string(), &message, &self.signature) {
            Ok(is_valid) => {
                if !is_valid {
                    log::warn!("Invalid signature for transaction {}", self.transaction_id);
                }
                is_valid
            }
            Err(err) => {
                log::error!(
                    "Error verifying signature for transaction {}: {}",
                    self.transaction_id,
                    err
                );
                false
            }
        }
    }

    // Check if the transaction is a VM transaction
    pub fn is_vm_transaction(&self) -> bool {
        if let Some(data) = &self.data {
            if let Ok(data_str) = std::str::from_utf8(data) {
                return data_str.starts_with("VM:") || data_str.starts_with("VM_MODULE");
            }
        }
        false
    }

    // Check if the transaction is a VM module deployment
    pub fn is_vm_module_deployment(&self) -> bool {
        if let Some(data) = &self.data {
            if let Ok(data_str) = std::str::from_utf8(data) {
                return data_str.starts_with("VM_MODULE");
            }
        }
        false
    }

    // Get transaction type as a string for better logging
    pub fn get_transaction_type(&self) -> &'static str {
        if self.is_vm_module_deployment() {
            "VM_MODULE_DEPLOYMENT"
        } else if self.is_vm_transaction() {
            "VM_FUNCTION_CALL"
        } else if self.data.is_some() {
            "DATA_TRANSACTION"
        } else {
            "TOKEN_TRANSFER"
        }
    }

    // Extract VM module information from deployment transaction
    pub fn get_vm_module_info(&self) -> Option<(String, String)> {
        if let Some(data) = &self.data {
            if let Ok(data_str) = std::str::from_utf8(data) {
                if data_str.starts_with("VM_MODULE_DEPLOYMENT:") {
                    let parts: Vec<&str> = data_str.split(':').collect();
                    if parts.len() >= 3 {
                        return Some((parts[1].to_string(), parts[2].to_string())); // (address, module_name)
                    }
                }
            }
        }
        None
    }

    // Extract VM function call information
    pub fn get_vm_function_info(&self) -> Option<(String, String)> {
        if let Some(data) = &self.data {
            if let Ok(data_str) = std::str::from_utf8(data) {
                if data_str.starts_with("VM:") {
                    let parts: Vec<&str> = data_str.split(':').collect();
                    if parts.len() >= 3 {
                        return Some((parts[1].to_string(), parts[2].to_string())); // (module_id, function)
                    }
                }
            }
        }
        None
    }
}

// Define the Block struct
#[derive(Serialize, Deserialize, Clone)]
pub struct Block<T: HashAlgorithm> {
    pub chain_id: String,
    pub index: u32,
    pub timestamp: u64,
    pub data: Vec<u8>,
    pub hash: String,
    pub prev_hash: String,
    pub tokens: u64,
    pub transactions: Vec<Transaction>,
    pub address: String,
    pub hasher: T,
}

// Implement the Block struct
impl<T: HashAlgorithm> Block<T> {
    pub fn new(
        index: u32,
        data: Vec<u8>,
        prev_hash: String,
        tokens: u64,
        transactions: Vec<Transaction>,
        address: String,
        hasher: T,
    ) -> Block<T> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut block = Block {
            chain_id: CHAIN_ID.to_string(),
            index,
            timestamp,
            data,
            hash: String::new(),
            prev_hash,
            tokens,
            transactions,
            address,
            hasher,
        };
        block.hash = block.calculate_hash();
        block
    }

    // Add a method to calculate the hash of the block
    pub fn calculate_hash(&self) -> String {
        let mut input = Vec::new();
        input.extend_from_slice(self.chain_id.as_bytes());
        input.extend_from_slice(&self.index.to_le_bytes());
        input.extend_from_slice(&self.timestamp.to_le_bytes());
        input.extend_from_slice(&self.data);
        input.extend_from_slice(self.prev_hash.as_bytes());
        input.extend_from_slice(&self.tokens.to_le_bytes());

        // Serialize transactions
        let transactions_serialized = serde_json::to_string(&self.transactions).unwrap();
        input.extend_from_slice(transactions_serialized.as_bytes());

        // self.hasher.log_input(&input);
        self.hasher.hash(&input)
    }

    // Add a method to verify the block
    pub fn verify(&self, prev_block: &Block<T>) -> bool {
        if self.index != prev_block.index + 1 {
            return false;
        }
        if self.prev_hash != prev_block.hash {
            return false;
        }

        let calculated_hash = self.calculate_hash();
        if self.hash != calculated_hash {
            return false;
        }
        true
    }
}
