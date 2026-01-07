// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Blockchain data structures and operations
use anyhow::Result;
use kanari_crypto::hash_data_blake3;
use kanari_crypto::keys::CurveType;
use kanari_move_runtime::changeset::Event;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::error;

mod merkle;
pub use merkle::{
    CompressedMerkleProof, batch_verify_merkle_proofs, compute_merkle_root,
    generate_merkle_multiproof, generate_merkle_proof, verify_merkle_proof,
};

/// Signed transaction wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedTransaction {
    pub transaction: Transaction,
    pub signature: Option<Vec<u8>>,
}

impl SignedTransaction {
    pub fn new(transaction: Transaction) -> Self {
        Self {
            transaction,
            signature: None,
        }
    }

    pub fn sign(&mut self, private_key: &str, curve_type: CurveType) -> Result<()> {
        let tx_hash = self.transaction.hash();
        let signature = kanari_crypto::sign_message(private_key, &tx_hash, curve_type)
            .map_err(|e| anyhow::anyhow!("Failed to sign transaction: {}", e))?;
        self.signature = Some(signature);
        Ok(())
    }

    pub fn verify_signature(&self) -> Result<bool> {
        let signature = self
            .signature
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Transaction not signed"))?;

        let tx_hash = self.transaction.hash();
        let sender = self.transaction.sender_address();

        kanari_crypto::verify_signature(sender, &tx_hash, signature)
            .map_err(|e| anyhow::anyhow!("Signature verification failed: {}", e))
    }

    pub fn hash(&self) -> Vec<u8> {
        let serialized = match bcs::to_bytes(self) {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to serialize SignedTransaction for hashing: {}", e);
                Vec::new()
            }
        };
        hash_data_blake3(&serialized)
    }
}

/// Block header containing metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockHeader {
    pub height: u64,
    pub timestamp: u64,
    pub prev_hash: Vec<u8>,
    pub state_root: Vec<u8>,
    pub merkle_root: Vec<u8>,
    pub tx_count: usize,
}

impl BlockHeader {
    pub fn new(
        height: u64,
        prev_hash: Vec<u8>,
        state_root: Vec<u8>,
        merkle_root: Vec<u8>,
        tx_count: usize,
    ) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Self {
            height,
            timestamp,
            prev_hash,
            state_root,
            merkle_root,
            tx_count,
        }
    }

    pub fn hash(&self) -> Vec<u8> {
        let serialized = match bcs::to_bytes(self) {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to serialize BlockHeader for hashing: {}", e);
                Vec::new()
            }
        };
        hash_data_blake3(&serialized)
    }
}

/// Transaction types in Kanari blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transaction {
    /// Publish a Move module
    PublishModule {
        sender: String,
        module_bytes: Vec<u8>,
        module_name: String,
        gas_limit: u64,
        gas_price: u64,
        sequence_number: u64,
    },
    /// Execute a Move function
    ExecuteFunction {
        sender: String,
        module: String,
        function: String,
        type_args: Vec<String>,
        args: Vec<Vec<u8>>,
        gas_limit: u64,
        gas_price: u64,
        sequence_number: u64,
    },
    /// Transfer coins
    Transfer {
        from: String,
        to: String,
        amount: u64,
        gas_limit: u64,
        gas_price: u64,
        sequence_number: u64,
    },
    /// Burn coins (remove from total supply)
    Burn {
        from: String,
        amount: u64,
        gas_limit: u64,
        gas_price: u64,
        sequence_number: u64,
    },
}

impl Transaction {
    pub fn hash(&self) -> Vec<u8> {
        let serialized = match bcs::to_bytes(self) {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to serialize Transaction for hashing: {}", e);
                Vec::new()
            }
        };
        hash_data_blake3(&serialized)
    }

    pub fn sender(&self) -> &str {
        match self {
            Transaction::PublishModule { sender, .. } => sender,
            Transaction::ExecuteFunction { sender, .. } => sender,
            Transaction::Transfer { from, .. } => from,
            Transaction::Burn { from, .. } => from,
        }
    }

    pub fn sender_address(&self) -> &str {
        self.sender()
    }

    pub fn sequence_number(&self) -> u64 {
        match self {
            Transaction::PublishModule {
                sequence_number, ..
            } => *sequence_number,
            Transaction::ExecuteFunction {
                sequence_number, ..
            } => *sequence_number,
            Transaction::Transfer {
                sequence_number, ..
            } => *sequence_number,
            Transaction::Burn {
                sequence_number, ..
            } => *sequence_number,
        }
    }

    pub fn gas_limit(&self) -> u64 {
        match self {
            Transaction::PublishModule { gas_limit, .. } => *gas_limit,
            Transaction::ExecuteFunction { gas_limit, .. } => *gas_limit,
            Transaction::Transfer { gas_limit, .. } => *gas_limit,
            Transaction::Burn { gas_limit, .. } => *gas_limit,
        }
    }

    pub fn gas_price(&self) -> u64 {
        match self {
            Transaction::PublishModule { gas_price, .. } => *gas_price,
            Transaction::ExecuteFunction { gas_price, .. } => *gas_price,
            Transaction::Transfer { gas_price, .. } => *gas_price,
            Transaction::Burn { gas_price, .. } => *gas_price,
        }
    }

    /// Create a transfer transaction with default gas settings
    pub fn new_transfer(from: String, to: String, amount: u64) -> Self {
        Self::Transfer {
            from,
            to,
            amount,
            gas_limit: 100_000, // Default gas limit
            gas_price: 1000,    // Default gas price (1000 Mist)
            sequence_number: 0,
        }
    }

    /// Create a burn transaction with default gas settings
    pub fn new_burn(from: String, amount: u64) -> Self {
        Self::Burn {
            from,
            amount,
            gas_limit: 100_000,
            gas_price: 1000,
            sequence_number: 0,
        }
    }
}

/// Block containing transactions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub transactions: Vec<SignedTransaction>,
    pub events: Vec<Event>,
}

impl Block {
    pub fn new(
        height: u64,
        prev_hash: Vec<u8>,
        state_root: Vec<u8>,
        transactions: Vec<SignedTransaction>,
        events: Vec<Event>,
    ) -> Self {
        let tx_count = transactions.len();

        // Compute merkle root from transaction hashes
        let tx_hashes: Vec<Vec<u8>> = transactions.iter().map(|tx| tx.hash()).collect();
        let merkle_root = compute_merkle_root(&tx_hashes);

        let header = BlockHeader::new(height, prev_hash, state_root, merkle_root, tx_count);

        Self {
            header,
            transactions,
            events,
        }
    }

    pub fn genesis() -> Self {
        Self::new(0, vec![0u8; 32], vec![0u8; 32], vec![], vec![])
    }

    pub fn hash(&self) -> Vec<u8> {
        self.header.hash()
    }

    pub fn verify(&self, prev_block: &Block) -> Result<()> {
        // Verify height
        if self.header.height != prev_block.header.height + 1 {
            anyhow::bail!(
                "Invalid block height: expected {}, got {}",
                prev_block.header.height + 1,
                self.header.height
            );
        }

        // Verify prev_hash
        let expected_prev_hash = prev_block.hash();
        if self.header.prev_hash != expected_prev_hash {
            anyhow::bail!(
                "Invalid previous hash: expected {}, got {}",
                hex::encode(&expected_prev_hash),
                hex::encode(&self.header.prev_hash)
            );
        }

        // Verify block hash integrity (recompute and compare)
        let computed_hash = self.hash();
        let header_hash = self.header.hash();
        if computed_hash != header_hash {
            anyhow::bail!(
                "Block hash mismatch: computed {}, header {}",
                hex::encode(&computed_hash),
                hex::encode(&header_hash)
            );
        }

        // Verify timestamp (must be >= previous block's timestamp)
        if self.header.height > 1 && self.header.timestamp < prev_block.header.timestamp {
            anyhow::bail!(
                "Invalid timestamp: {} < {}",
                self.header.timestamp,
                prev_block.header.timestamp
            );
        }

        // Verify transaction count matches header
        if self.transactions.len() != self.header.tx_count {
            anyhow::bail!(
                "Transaction count mismatch: header says {}, actual {}",
                self.header.tx_count,
                self.transactions.len()
            );
        }

        // Verify merkle root
        let tx_hashes: Vec<Vec<u8>> = self.transactions.iter().map(|tx| tx.hash()).collect();
        let computed_merkle_root = compute_merkle_root(&tx_hashes);
        if self.header.merkle_root != computed_merkle_root {
            anyhow::bail!(
                "Merkle root mismatch: header {}, computed {}",
                hex::encode(&self.header.merkle_root),
                hex::encode(&computed_merkle_root)
            );
        }

        // Verify each transaction has valid structure
        for (i, signed_tx) in self.transactions.iter().enumerate() {
            // Verify transaction hash is valid
            let tx_hash = signed_tx.transaction.hash();
            if tx_hash.is_empty() {
                anyhow::bail!("Transaction {} has empty hash", i);
            }

            // Verify sender address format
            let sender = signed_tx.transaction.sender_address();
            if sender.is_empty() {
                anyhow::bail!("Transaction {} has empty sender address", i);
            }

            // Verify signature if present
            if signed_tx.signature.is_some() {
                signed_tx.verify_signature().map_err(|e| {
                    anyhow::anyhow!("Invalid signature for transaction {}: {}", i, e)
                })?;
            }
        }

        Ok(())
    }
}

/// Blockchain state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blockchain {
    pub blocks: Vec<Block>,
    #[serde(skip)]
    executed_tx_hashes: std::collections::HashSet<String>,
    /// Store alternative chains (forks) - each fork is a chain starting from divergence point
    #[serde(skip)]
    pub forks: Vec<Vec<Block>>,
}

impl Blockchain {
    pub fn new() -> Self {
        let genesis = Block::genesis();
        Self {
            blocks: vec![genesis],
            executed_tx_hashes: std::collections::HashSet::new(),
            forks: Vec::new(),
        }
    }

    pub fn latest_block(&self) -> &Block {
        self.blocks
            .last()
            .expect("blockchain must contain at least the genesis block")
    }

    pub fn height(&self) -> u64 {
        self.latest_block().header.height
    }

    /// Check if transaction hash has been executed before (deduplication)
    pub fn is_transaction_executed(&self, tx_hash: &str) -> bool {
        self.executed_tx_hashes.contains(tx_hash)
    }

    /// Mark transaction as executed (for deduplication tracking)
    pub fn mark_transaction_executed(&mut self, tx_hash: String) {
        self.executed_tx_hashes.insert(tx_hash);
    }

    /// Rebuild executed transaction hash set from blockchain history
    /// Call this after loading blockchain from disk
    pub fn rebuild_tx_hash_index(&mut self) {
        self.executed_tx_hashes.clear();
        for block in &self.blocks {
            for signed_tx in &block.transactions {
                let tx_hash = hex::encode(signed_tx.hash());
                self.executed_tx_hashes.insert(tx_hash);
            }
        }
    }

    pub fn add_block(&mut self, block: Block) -> Result<()> {
        self.add_block_with_validation(block, true)
    }

    pub fn add_block_with_validation(&mut self, block: Block, validate: bool) -> Result<()> {
        if validate {
            let prev_block = self.latest_block();
            block.verify(prev_block)?;

            // Check for duplicate transactions in this block
            for signed_tx in &block.transactions {
                let tx_hash = hex::encode(signed_tx.hash());
                if self.is_transaction_executed(&tx_hash) {
                    anyhow::bail!("Duplicate transaction detected: {}", tx_hash);
                }
            }
        }

        // Add transaction hashes to executed set
        for signed_tx in &block.transactions {
            let tx_hash = hex::encode(signed_tx.hash());
            self.mark_transaction_executed(tx_hash);
        }

        self.blocks.push(block);
        Ok(())
    }

    pub fn get_block(&self, height: u64) -> Option<&Block> {
        self.blocks.iter().find(|b| b.header.height == height)
    }

    pub fn get_transaction_count(&self) -> usize {
        self.blocks.iter().map(|b| b.transactions.len()).sum()
    }

    /// Calculate total difficulty/work of a chain (longest chain rule)
    fn calculate_chain_work(blocks: &[Block]) -> u64 {
        blocks.len() as u64
    }

    /// Handle potential fork by comparing with incoming chain
    /// Returns true if we switched to the new chain
    pub fn handle_fork(&mut self, fork_blocks: Vec<Block>) -> Result<bool> {
        if fork_blocks.is_empty() {
            return Ok(false);
        }

        // Find common ancestor by checking from the beginning of fork
        let mut common_height = 0;
        for block in fork_blocks.iter() {
            if let Some(our_block) = self.get_block(block.header.height) {
                if our_block.hash() == block.hash() {
                    // This block is the same in both chains
                    common_height = block.header.height;
                } else {
                    // Found divergence point
                    break;
                }
            } else {
                // Fork extends beyond our chain
                break;
            }
        }

        // Get divergent portion of the fork (blocks after common ancestor)
        let fork_divergent: Vec<Block> = fork_blocks
            .into_iter()
            .filter(|b| b.header.height > common_height)
            .collect();

        if fork_divergent.is_empty() {
            return Ok(false); // No new blocks
        }

        // Compare chain work (longest chain rule)
        let our_work = Self::calculate_chain_work(&self.blocks);
        let mut fork_chain = self.blocks[..=common_height as usize].to_vec();
        fork_chain.extend(fork_divergent.clone());
        let fork_work = Self::calculate_chain_work(&fork_chain);

        if fork_work > our_work {
            // Fork is longer - reorganize to fork chain
            tracing::info!(
                "Fork detected! Reorganizing chain from height {} (our_work: {}, fork_work: {})",
                common_height + 1,
                our_work,
                fork_work
            );

            // Save old chain as fork for potential rollback
            let old_chain = self.blocks[(common_height as usize + 1)..].to_vec();
            if !old_chain.is_empty() {
                self.forks.push(old_chain);
            }

            // Replace with new chain
            self.blocks.truncate((common_height + 1) as usize);
            for block in fork_divergent {
                // Don't validate against previous block since we already validated the fork
                self.add_block_with_validation(block, false)?;
            }

            // Rebuild transaction hash index
            self.rebuild_tx_hash_index();

            Ok(true)
        } else {
            // Our chain is longer or equal - keep it and store fork
            tracing::debug!(
                "Fork detected but our chain is longer (our_work: {}, fork_work: {})",
                our_work,
                fork_work
            );
            self.forks.push(fork_divergent);
            Ok(false)
        }
    }

    /// Get canonical chain (main chain)
    pub fn get_canonical_chain(&self) -> &[Block] {
        &self.blocks
    }

    /// Get all stored forks
    pub fn get_forks(&self) -> &[Vec<Block>] {
        &self.forks
    }

    /// Prune old forks (keep only recent forks within depth)
    pub fn prune_forks(&mut self, max_fork_depth: u64) {
        let current_height = self.height();
        self.forks.retain(|fork| {
            if let Some(first_block) = fork.first() {
                current_height - first_block.header.height <= max_fork_depth
            } else {
                false
            }
        });
    }

}

impl Default for Blockchain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_genesis_block() {
        let genesis = Block::genesis();
        assert_eq!(genesis.header.height, 0);
        assert_eq!(genesis.transactions.len(), 0);
    }

    #[test]
    fn test_blockchain_creation() {
        let chain = Blockchain::new();
        assert_eq!(chain.height(), 0);
        assert_eq!(chain.blocks.len(), 1);
    }

    #[test]
    fn test_add_block() {
        let mut chain = Blockchain::new();
        let prev_hash = chain.latest_block().hash();

        let block = Block::new(1, prev_hash, vec![0u8; 32], vec![], vec![]);
        chain.add_block(block).unwrap();

        assert_eq!(chain.height(), 1);
        assert_eq!(chain.blocks.len(), 2);
    }

    #[test]
    fn test_block_verification() {
        let chain = Blockchain::new();
        let prev_block = chain.latest_block();

        let valid_block = Block::new(1, prev_block.hash(), vec![0u8; 32], vec![], vec![]);
        assert!(valid_block.verify(prev_block).is_ok());
        let invalid_block = Block::new(2, prev_block.hash(), vec![0u8; 32], vec![], vec![]);
        assert!(invalid_block.verify(prev_block).is_err());
    }

    #[test]
    fn test_transaction_hash() {
        let tx = Transaction::new_transfer("0x1".to_string(), "0x2".to_string(), 1000);

        let hash1 = tx.hash();
        let hash2 = tx.hash();
        assert_eq!(hash1, hash2);
    }
}
