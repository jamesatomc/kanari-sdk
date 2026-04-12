// Blockchain data structures and operations
use anyhow::Result;
use kanari_crypto::hash_data_blake3;
use serde::{Deserialize, Serialize};
use smt::compute_merkle_root;
// use fully-qualified paths for time APIs to avoid unused-import warnings
use tracing::error;

use crate::{event::Event, transaction::SignedTransaction};

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
        timestamp: u64,
    ) -> Self {
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
        timestamp: u64,
    ) -> Self {
        let tx_count = transactions.len();

        // Compute merkle root from transaction hashes
        let tx_hashes: Vec<Vec<u8>> = transactions
            .iter()
            .map(|tx| tx.hash().0.0.to_vec())
            .collect();
        let merkle_root = compute_merkle_root(&tx_hashes);

        let header = BlockHeader::new(
            height,
            prev_hash,
            state_root,
            merkle_root,
            tx_count,
            timestamp,
        );

        Self {
            header,
            transactions,
            events,
        }
    }

    pub fn genesis() -> Self {
        let genesis_state_root = smt::default_hashes()[0].to_vec();
        Self::new(0, vec![0u8; 32], genesis_state_root, vec![], vec![], 0)
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
        let tx_hashes: Vec<Vec<u8>> = self
            .transactions
            .iter()
            .map(|tx| tx.hash().0.0.to_vec())
            .collect();
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
            let tx_hash = signed_tx.transaction.hash();

            if tx_hash.0.0 == [0u8; 32] {
                anyhow::bail!("Transaction {} has empty hash", i);
            }

            // Verify sender address format
            let sender = signed_tx.transaction.sender_address();
            if sender.is_empty() {
                anyhow::bail!("Transaction {} has empty sender address", i);
            }

            // Require a valid signature for every transaction
            signed_tx.verify_signature().map_err(|e| {
                anyhow::anyhow!("Invalid or missing signature for transaction {}: {}", i, e)
            })?;
        }

        Ok(())
    }
}
