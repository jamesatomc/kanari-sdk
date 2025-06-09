use serde::{Deserialize, Serialize};
use crate::address::Address;

/// Information about the transaction currently being executed.
/// Corresponds to `kanari_framework::tx_context::TxContext` in Move
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxContext {
    /// The address of the user that signed the current transaction
    pub sender: Address,
    /// Hash of the current transaction
    pub tx_hash: Vec<u8>,
    /// The current epoch number
    pub epoch: u64,
    /// Timestamp that the epoch started at
    pub epoch_timestamp_ms: u64,
    /// Counter recording the number of fresh id's created while executing this transaction
    pub ids_created: u64,
}

impl TxContext {
    /// Create a new TxContext
    pub fn new(
        sender: Address,
        tx_hash: Vec<u8>,
        epoch: u64,
        epoch_timestamp_ms: u64,
        ids_created: u64,
    ) -> Result<Self, TxContextError> {
        if tx_hash.len() != TX_HASH_LENGTH {
            return Err(TxContextError::BadTxHashLength);
        }

        Ok(Self {
            sender,
            tx_hash,
            epoch,
            epoch_timestamp_ms,
            ids_created,
        })
    }

    /// Return the address of the user that signed the current transaction
    pub fn sender(&self) -> Address {
        self.sender
    }

    /// Return the transaction digest (hash of transaction inputs)
    pub fn digest(&self) -> &[u8] {
        &self.tx_hash
    }

    /// Return the current epoch
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Return the epoch start time as a unix timestamp in milliseconds
    pub fn epoch_timestamp_ms(&self) -> u64 {
        self.epoch_timestamp_ms
    }

    /// Generate a fresh object address
    pub fn fresh_object_address(&mut self) -> Address {
        let id = derive_id(&self.tx_hash, self.ids_created);
        self.ids_created += 1;
        id
    }

    /// Return the number of IDs created by the current transaction
    pub fn ids_created(&self) -> u64 {
        self.ids_created
    }
}

/// Derive an ID via hash(tx_hash || ids_created)
fn derive_id(tx_hash: &[u8], ids_created: u64) -> Address {
    use sha2::{Digest, Sha256};
    
    let mut hasher = Sha256::new();
    hasher.update(tx_hash);
    hasher.update(ids_created.to_le_bytes());
    let hash = hasher.finalize();
    
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hash[..32]);
    Address::new(bytes)
}

/// Transaction context errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TxContextError {
    #[error("Expected tx hash of length {}, but found length {}", TX_HASH_LENGTH, "actual")]
    BadTxHashLength,
    #[error("No IDs have been created in this transaction")]
    NoIDsCreated,
}

/// Constants
pub const TX_HASH_LENGTH: usize = 32;

/// TxContext error constants matching Move constants
pub mod error_constants {
    /// Expected an tx hash of length 32, but found a different length
    pub const E_BAD_TX_HASH_LENGTH: u64 = 0;
    /// Attempt to get the most recent created object ID when none has been created
    pub const E_NO_IDS_CREATED: u64 = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tx_context_creation() {
        let sender = Address::zero();
        let tx_hash = vec![0u8; TX_HASH_LENGTH];
        let epoch = 1;
        let epoch_timestamp_ms = 1000;
        let ids_created = 0;

        let ctx = TxContext::new(sender, tx_hash, epoch, epoch_timestamp_ms, ids_created);
        assert!(ctx.is_ok());
        
        let ctx = ctx.unwrap();
        assert_eq!(ctx.sender(), sender);
        assert_eq!(ctx.epoch(), epoch);
        assert_eq!(ctx.epoch_timestamp_ms(), epoch_timestamp_ms);
        assert_eq!(ctx.ids_created(), ids_created);
    }

    #[test]
    fn test_fresh_object_address() {
        let sender = Address::zero();
        let tx_hash = vec![1u8; TX_HASH_LENGTH];
        let mut ctx = TxContext::new(sender, tx_hash, 1, 1000, 0).unwrap();

        let addr1 = ctx.fresh_object_address();
        let addr2 = ctx.fresh_object_address();
        
        // Addresses should be different
        assert_ne!(addr1, addr2);
        assert_eq!(ctx.ids_created(), 2);
    }

    #[test]
    fn test_bad_tx_hash_length() {
        let sender = Address::zero();
        let tx_hash = vec![0u8; 16]; // Wrong length
        
        let result = TxContext::new(sender, tx_hash, 1, 1000, 0);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), TxContextError::BadTxHashLength));
    }
}
