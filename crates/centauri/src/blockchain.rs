// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Blockchain data structures and operations
use anyhow::Result;
use kanari_types::block::Block;
use serde::{Deserialize, Serialize};

use crate::consensus::Checkpoint;

/// Blockchain state - DAG-based consensus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blockchain {
    /// Blocks for P2P sync compatibility
    pub blocks: Vec<Block>,

    /// DAG checkpoints (committed state)
    #[serde(default = "default_dag_checkpoints")]
    pub dag_checkpoints: Vec<Checkpoint>,

    /// Track executed transactions (for deduplication)
    #[serde(skip)]
    executed_tx_hashes: std::collections::HashSet<String>,

    /// Always in DAG mode
    #[serde(default = "default_dag_mode")]
    pub dag_mode: bool,
}

fn default_dag_mode() -> bool {
    true
}

fn default_dag_checkpoints() -> Vec<Checkpoint> {
    vec![Checkpoint::genesis()]
}

impl Blockchain {
    pub fn new() -> Self {
        let genesis = Block::genesis();
        Self {
            blocks: vec![genesis],
            dag_checkpoints: vec![Checkpoint::genesis()],
            executed_tx_hashes: std::collections::HashSet::new(),
            dag_mode: true, // Always DAG mode
        }
    }

    /// Create blockchain in DAG mode (same as new())
    pub fn new_with_dag() -> Self {
        Self::new()
    }

    /// Enable DAG mode (always enabled, kept for compatibility)
    pub fn enable_dag_mode(&mut self) {
        // DAG mode is always enabled
        if self.dag_checkpoints.is_empty() {
            self.dag_checkpoints.push(Checkpoint::genesis());
        }
    }

    pub fn latest_block(&self) -> &Block {
        self.blocks
            .last()
            .expect("blockchain must contain at least the genesis block")
    }

    /// Get latest checkpoint (for DAG mode)
    pub fn latest_checkpoint(&self) -> &Checkpoint {
        if self.dag_checkpoints.is_empty() {
            // This should never happen as we initialize with genesis
            panic!("DAG checkpoints is empty - should contain at least genesis");
        }
        self.dag_checkpoints.last().unwrap()
    }

    pub fn height(&self) -> u64 {
        // Height = checkpoint sequence number (always in DAG mode)
        self.latest_checkpoint().sequence
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

        // Rebuild from blocks (compatibility)
        for block in &self.blocks {
            for signed_tx in &block.transactions {
                let tx_hash = hex::encode(signed_tx.hash().0.0);
                self.executed_tx_hashes.insert(tx_hash);
            }
        }

        // Rebuild from DAG checkpoints (primary mode)
        for checkpoint in &self.dag_checkpoints {
            for signed_tx in &checkpoint.transactions {
                let tx_hash = hex::encode(signed_tx.hash().0.0);
                self.executed_tx_hashes.insert(tx_hash);
            }
        }
    }

    pub fn add_block(&mut self, block: Block) -> Result<()> {
        self.add_block_with_validation(block, true)
    }

    pub fn add_block_with_validation(&mut self, _block: Block, _validate: bool) -> Result<()> {
        // Blocks are only used for P2P sync compatibility in DAG mode
        // Direct block addition is deprecated - use add_checkpoint instead
        anyhow::bail!(
            "Direct block addition is not supported in DAG mode. Use add_checkpoint instead."
        );
    }

    /// Add checkpoint (for DAG mode)
    pub fn add_checkpoint(&mut self, checkpoint: Checkpoint) -> Result<()> {
        self.add_checkpoint_with_validation(checkpoint, true)
    }

    /// Add checkpoint with optional validation. When `validate` is false,
    /// the sequence and previous-checkpoint-hash checks are skipped. This is
    /// useful for trusted sync paths which reconstruct checkpoints from
    /// external data where full checkpoint metadata may not be present.
    pub fn add_checkpoint_with_validation(
        &mut self,
        checkpoint: Checkpoint,
        validate: bool,
    ) -> Result<()> {
        if validate {
            // Verify checkpoint sequence
            let expected_seq = self.latest_checkpoint().sequence + 1;
            if checkpoint.sequence != expected_seq {
                anyhow::bail!(
                    "Invalid checkpoint sequence: expected {}, got {}",
                    expected_seq,
                    checkpoint.sequence
                );
            }

            // Verify previous checkpoint hash
            let prev_hash = self.latest_checkpoint().hash();
            if checkpoint.prev_checkpoint_hash != prev_hash {
                anyhow::bail!("Invalid previous checkpoint hash");
            }
        }

        // Mark transactions as executed (for state-level deduplication)
        for signed_tx in &checkpoint.transactions {
            let tx_hash = hex::encode(signed_tx.hash().0.0);
            self.mark_transaction_executed(tx_hash);
        }

        // Create a block for each checkpoint for P2P sync compatibility
        let prev_hash = if self.blocks.is_empty() {
            vec![0u8; 32]
        } else {
            self.blocks.last().unwrap().hash()
        };

        let block = Block::new(
            checkpoint.sequence, // Use checkpoint sequence for block height
            prev_hash,
            checkpoint.state_root.clone(),
            checkpoint.transactions.clone(),
            Vec::new(), // events handled separately
            checkpoint.timestamp,
        );

        self.blocks.push(block);
        self.dag_checkpoints.push(checkpoint);
        Ok(())
    }

    pub fn get_block(&self, height: u64) -> Option<&Block> {
        self.blocks.iter().find(|b| b.header.height == height)
    }

    /// Get checkpoint by sequence number (for DAG mode)
    pub fn get_checkpoint(&self, sequence: u64) -> Option<&Checkpoint> {
        self.dag_checkpoints
            .iter()
            .find(|cp| cp.sequence == sequence)
    }

    pub fn get_transaction_count(&self) -> usize {
        // Count transactions in checkpoints
        self.dag_checkpoints
            .iter()
            .map(|cp| cp.transactions.len())
            .sum()
    }
}

impl Default for Blockchain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use kanari_types::transaction::Transaction;

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
        assert!(chain.dag_mode);
    }

    #[test]
    fn test_add_checkpoint() {
        let mut chain = Blockchain::new();

        let prev_cp_hash = chain.latest_checkpoint().hash();
        let checkpoint = Checkpoint::new(1, Vec::new(), Vec::new(), vec![0u8; 32], 0, prev_cp_hash);

        chain.add_checkpoint(checkpoint).unwrap();
        assert_eq!(chain.height(), 1);
        assert_eq!(chain.dag_checkpoints.len(), 2);
    }

    #[test]
    fn test_block_verification() {
        let chain = Blockchain::new();
        let prev_block = chain.latest_block();

        let valid_block = Block::new(1, prev_block.hash(), vec![0u8; 32], vec![], vec![], 0);
        assert!(valid_block.verify(prev_block).is_ok());
        let invalid_block = Block::new(2, prev_block.hash(), vec![0u8; 32], vec![], vec![], 0);
        assert!(invalid_block.verify(prev_block).is_err());
    }

    #[test]
    fn test_transaction_hash() {
        let tx = Transaction::new_transfer("0x1".to_string(), "0x2".to_string(), 1000, 0);

        let hash1 = tx.hash();
        let hash2 = tx.hash();
        assert_eq!(hash1, hash2);
    }
}
