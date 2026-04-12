// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Light Client Support for DAG Consensus
//!
//! Enables lightweight clients to:
//! - Verify transactions without full DAG sync
//! - Verify checkpoints with minimal data
//! - Query account state with Merkle proofs
//!
//! Inspired by Sui's light client design.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use smt::compute_merkle_root;
use std::collections::BTreeMap;

use super::{AuthorityId, Checkpoint, Round};

/// Maximum checkpoints to retain in light client (keep last N)
const MAX_CHECKPOINTS: usize = 1000;

/// Light client checkpoint verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightCheckpoint {
    /// Checkpoint sequence number
    pub sequence: u64,

    /// State root hash
    pub state_root: Vec<u8>,

    /// Transaction root hash
    pub tx_root: Vec<u8>,

    /// Round/epoch
    pub epoch: Round,

    /// Quorum signatures (2f+1 authorities)
    pub signatures: Vec<CheckpointSignature>,
}

/// Authority signature on checkpoint
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointSignature {
    /// Authority ID
    pub authority: AuthorityId,

    /// Signature bytes
    pub signature: Vec<u8>,

    /// Authority's stake/weight
    pub stake: u64,
}

/// State proof for light client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateProof {
    /// Account address
    pub address: String,

    /// Account state data (value)
    /// If is_membership is false, this should be empty
    pub state_data: Vec<u8>,

    /// Whether this is a membership proof (true) or non-membership proof (false)
    pub is_membership: bool,

    /// Merkle proof siblings (bottom-up from leaf)
    pub siblings: Vec<[u8; 32]>,

    /// State root hash
    pub state_root: Vec<u8>,

    /// Checkpoint this proof is based on
    pub checkpoint: LightCheckpoint,
}

/// Transaction inclusion proof
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionProof {
    /// Transaction hash
    pub tx_hash: Vec<u8>,

    /// Transaction index in the block/checkpoint
    pub tx_index: usize,

    /// Transaction data
    pub tx_data: Vec<u8>,

    /// Merkle proof path
    pub path: Vec<Vec<u8>>,

    /// Transaction root hash
    pub tx_root: Vec<u8>,

    /// Checkpoint containing this transaction
    pub checkpoint: LightCheckpoint,
}

/// Light client
pub struct LightClient {
    /// Verified checkpoints (sequence -> checkpoint)
    verified_checkpoints: BTreeMap<u64, LightCheckpoint>,

    /// Latest verified checkpoint
    latest_checkpoint: u64,

    /// Authority public keys for signature verification
    authority_keys: BTreeMap<AuthorityId, Vec<u8>>,

    /// Total stake
    total_stake: u64,
}

impl LightClient {
    /// Create new light client
    pub fn new(authority_keys: BTreeMap<AuthorityId, Vec<u8>>) -> Self {
        let total_stake: u64 = authority_keys.len() as u64; // Simplified: 1 stake per authority

        Self {
            verified_checkpoints: BTreeMap::new(),
            latest_checkpoint: 0,
            authority_keys,
            total_stake,
        }
    }

    /// Verify and add checkpoint
    pub fn verify_checkpoint(&mut self, checkpoint: LightCheckpoint) -> Result<()> {
        // Check if we already have this checkpoint
        if self.verified_checkpoints.contains_key(&checkpoint.sequence) {
            return Ok(());
        }

        // Verify quorum: need 2f+1 signatures
        let f = (self.authority_keys.len() - 1) / 3;
        let quorum = 2 * f + 1;

        if checkpoint.signatures.len() < quorum {
            return Err(anyhow!(
                "Insufficient signatures: {} < {} (quorum)",
                checkpoint.signatures.len(),
                quorum
            ));
        }

        // Verify each signature
        let checkpoint_hash = self.hash_checkpoint(&checkpoint);
        let mut total_stake = 0u64;

        for sig in &checkpoint.signatures {
            // Verify authority exists
            if !self.authority_keys.contains_key(&sig.authority) {
                return Err(anyhow!("Unknown authority: {}", sig.authority));
            }

            // Verify signature (simplified)
            self.verify_signature(&checkpoint_hash, &sig.signature, &sig.authority)?;

            total_stake += sig.stake;
        }

        // Verify stake exceeds 2f+1
        if total_stake < (self.total_stake * 2 / 3) {
            return Err(anyhow!("Insufficient stake in quorum"));
        }

        // Add verified checkpoint
        self.verified_checkpoints
            .insert(checkpoint.sequence, checkpoint.clone());

        if checkpoint.sequence > self.latest_checkpoint {
            self.latest_checkpoint = checkpoint.sequence;
        }

        tracing::info!(
            "Verified checkpoint {} with {} signatures",
            checkpoint.sequence,
            checkpoint.signatures.len()
        );

        Ok(())
    }

    /// Verify state proof using SMT verification
    pub fn verify_state_proof(&self, proof: &StateProof) -> Result<()> {
        // Check if checkpoint is verified
        if !self
            .verified_checkpoints
            .contains_key(&proof.checkpoint.sequence)
        {
            return Err(anyhow!(
                "Checkpoint {} not verified",
                proof.checkpoint.sequence
            ));
        }

        // Verify state root matches checkpoint
        if proof.state_root != proof.checkpoint.state_root {
            return Err(anyhow!("State root mismatch"));
        }

        // Verify Merkle proof using SMT library
        let mut root_arr = [0u8; 32];
        if proof.state_root.len() == 32 {
            root_arr.copy_from_slice(&proof.state_root);
        } else {
            return Err(anyhow!("Invalid state root length"));
        }

        // Reconstruct leaf hash (consistent with SMT implementation)
        let key_hash = smt::digest(proof.address.as_bytes());
        let leaf_hash = if proof.is_membership {
            smt::hash_leaf(&key_hash, &proof.state_data)
        } else {
            // Use default leaf hash for non-membership
            // The default leaf is H(0x00 || 32 zero key || 32 zero value)
            smt::default_hashes()[256]
        };

        // verify_proof returns true if valid
        // Proof tuple: (is_member, leaf_hash, siblings)
        if !smt::verify_proof(
            &root_arr,
            proof.address.as_bytes(),
            (proof.is_membership, leaf_hash, proof.siblings.clone()),
        ) {
            return Err(anyhow!("Invalid Merkle proof for state"));
        }

        tracing::debug!(
            "Verified state proof for address {} at checkpoint {}",
            proof.address,
            proof.checkpoint.sequence
        );

        Ok(())
    }

    /// Verify transaction proof using Merkle proof verification
    pub fn verify_transaction_proof(&self, proof: &TransactionProof) -> Result<()> {
        // Check if checkpoint is verified
        if !self
            .verified_checkpoints
            .contains_key(&proof.checkpoint.sequence)
        {
            return Err(anyhow!(
                "Checkpoint {} not verified",
                proof.checkpoint.sequence
            ));
        }

        // Verify transaction hash matches data
        let tx_hash = kanari_crypto::hash_data_blake3(&proof.tx_data);
        if tx_hash != proof.tx_hash {
            return Err(anyhow!("Transaction hash mismatch"));
        }

        // Verify tx root matches checkpoint
        if proof.tx_root != proof.checkpoint.tx_root {
            return Err(anyhow!("Transaction root mismatch"));
        }

        // Verify Merkle inclusion proof using smt library
        if !smt::verify_merkle_proof(&proof.tx_hash, proof.tx_index, &proof.path, &proof.tx_root) {
            return Err(anyhow!("Invalid Merkle proof for transaction inclusion"));
        }

        tracing::debug!(
            "Verified transaction inclusion at checkpoint {}",
            proof.checkpoint.sequence
        );

        Ok(())
    }

    /// Get latest verified checkpoint
    pub fn get_latest_checkpoint(&self) -> Option<&LightCheckpoint> {
        self.verified_checkpoints.get(&self.latest_checkpoint)
    }

    /// Get checkpoint by sequence
    pub fn get_checkpoint(&self, sequence: u64) -> Option<&LightCheckpoint> {
        self.verified_checkpoints.get(&sequence)
    }

    /// Prune old checkpoints to prevent memory leak
    pub fn prune_old_checkpoints(&mut self) {
        if self.verified_checkpoints.len() > MAX_CHECKPOINTS {
            // Keep only the most recent MAX_CHECKPOINTS
            let cutoff_seq = self
                .latest_checkpoint
                .saturating_sub(MAX_CHECKPOINTS as u64);
            self.verified_checkpoints
                .retain(|&seq, _| seq >= cutoff_seq);

            tracing::debug!(
                "Pruned old checkpoints, retained {} recent checkpoints",
                self.verified_checkpoints.len()
            );
        }
    }

    /// Get memory usage statistics
    pub fn get_checkpoint_count(&self) -> usize {
        self.verified_checkpoints.len()
    }

    /// Hash checkpoint for signing
    fn hash_checkpoint(&self, checkpoint: &LightCheckpoint) -> Vec<u8> {
        let data = format!(
            "{}:{}:{}:{}",
            checkpoint.sequence,
            hex::encode(&checkpoint.state_root),
            hex::encode(&checkpoint.tx_root),
            checkpoint.epoch
        );
        kanari_crypto::hash_data_blake3(data.as_bytes())
    }

    /// Verify signature (simplified - in production use proper crypto)
    fn verify_signature(&self, _message: &[u8], signature: &[u8], authority: &str) -> Result<()> {
        // Simplified verification
        // In production: use ed25519, secp256k1, or BLS signatures

        let _pubkey = self
            .authority_keys
            .get(authority)
            .ok_or_else(|| anyhow!("Authority not found: {}", authority))?;

        // Placeholder: assume signature is valid if it's non-empty
        if signature.is_empty() {
            return Err(anyhow!("Empty signature"));
        }

        Ok(())
    }
}

/// Light client query interface
pub struct LightClientQuery {
    /// Light client instance
    client: LightClient,
}

impl LightClientQuery {
    /// Create new query interface
    pub fn new(client: LightClient) -> Self {
        Self { client }
    }

    /// Query account state with proof
    pub fn query_account_state(&mut self, _address: String, proof: StateProof) -> Result<Vec<u8>> {
        self.client.verify_state_proof(&proof)?;
        Ok(proof.state_data)
    }

    /// Verify transaction was included
    pub fn verify_transaction_inclusion(
        &mut self,
        tx_hash: Vec<u8>,
        proof: TransactionProof,
    ) -> Result<bool> {
        if proof.tx_hash != tx_hash {
            return Ok(false);
        }

        self.client.verify_transaction_proof(&proof)?;
        Ok(true)
    }

    /// Get latest checkpoint sequence
    pub fn get_latest_checkpoint_sequence(&self) -> u64 {
        self.client.latest_checkpoint
    }

    /// Sync to new checkpoint
    pub fn sync_checkpoint(&mut self, checkpoint: LightCheckpoint) -> Result<()> {
        self.client.verify_checkpoint(checkpoint)
    }
}

/// Checkpoint builder (full node only)
pub struct CheckpointBuilder {
    /// Current checkpoint sequence
    sequence: u64,

    /// Authority ID
    authority_id: AuthorityId,

    /// Authority secret key
    secret_key: Vec<u8>,
}

impl CheckpointBuilder {
    /// Create new checkpoint builder
    pub fn new(sequence: u64, authority_id: AuthorityId, secret_key: Vec<u8>) -> Self {
        Self {
            sequence,
            authority_id,
            secret_key,
        }
    }

    /// Build light checkpoint from full checkpoint
    pub fn build_light_checkpoint(
        &self,
        checkpoint: &Checkpoint,
        signatures: Vec<CheckpointSignature>,
    ) -> LightCheckpoint {
        // Calculate tx_root from actual transactions
        let tx_hashes: Vec<Vec<u8>> = checkpoint
            .transactions
            .iter()
            .map(|tx| tx.hash().0.0.to_vec())
            .collect();
        let tx_root = if tx_hashes.is_empty() {
            vec![0u8; 32]
        } else {
            compute_merkle_root(&tx_hashes)
        };

        LightCheckpoint {
            sequence: checkpoint.sequence,
            state_root: checkpoint.state_root.clone(),
            tx_root,
            epoch: checkpoint.sequence, // Use sequence as epoch
            signatures,
        }
    }

    /// Sign checkpoint
    pub fn sign_checkpoint(&self, checkpoint: &LightCheckpoint) -> CheckpointSignature {
        // Verify sequence is expected
        if checkpoint.sequence != self.sequence {
            tracing::warn!(
                "Checkpoint sequence mismatch: expected {}, got {}",
                self.sequence,
                checkpoint.sequence
            );
        }

        let message = format!(
            "{}:{}:{}:{}",
            checkpoint.sequence,
            hex::encode(&checkpoint.state_root),
            hex::encode(&checkpoint.tx_root),
            checkpoint.epoch
        );

        // Use secret_key for signing (simplified - just hash with key)
        let key_hash = kanari_crypto::hash_data_blake3(&self.secret_key);
        let combined = [message.as_bytes(), &key_hash].concat();
        let signature = kanari_crypto::hash_data_blake3(&combined);

        CheckpointSignature {
            authority: self.authority_id.clone(),
            signature,
            stake: 1, // Simplified: 1 stake per authority
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_authorities() -> BTreeMap<AuthorityId, Vec<u8>> {
        let mut authorities = BTreeMap::new();
        for i in 0..4 {
            authorities.insert(
                format!("auth{}", i),
                vec![i as u8; 32], // Mock public key
            );
        }
        authorities
    }

    fn create_test_checkpoint(sequence: u64) -> LightCheckpoint {
        LightCheckpoint {
            sequence,
            state_root: vec![1u8; 32],
            tx_root: vec![2u8; 32],
            epoch: sequence,
            signatures: vec![],
        }
    }

    #[test]
    fn test_light_client_quorum() {
        let authorities = create_test_authorities();
        let mut client = LightClient::new(authorities);

        let mut checkpoint = create_test_checkpoint(1);

        // Add 3 signatures (quorum for 4 authorities: 2f+1 = 3)
        for i in 0..3 {
            checkpoint.signatures.push(CheckpointSignature {
                authority: format!("auth{}", i),
                signature: vec![i; 64],
                stake: 1,
            });
        }

        assert!(client.verify_checkpoint(checkpoint).is_ok());
    }

    #[test]
    fn test_light_client_insufficient_signatures() {
        let authorities = create_test_authorities();
        let mut client = LightClient::new(authorities);

        let mut checkpoint = create_test_checkpoint(1);

        // Only 2 signatures (insufficient)
        for i in 0..2 {
            checkpoint.signatures.push(CheckpointSignature {
                authority: format!("auth{}", i),
                signature: vec![i; 64],
                stake: 1,
            });
        }

        assert!(client.verify_checkpoint(checkpoint).is_err());
    }

    #[test]
    fn test_checkpoint_builder() {
        let builder = CheckpointBuilder::new(1, "auth1".to_string(), vec![0u8; 32]);

        let checkpoint = create_test_checkpoint(1);
        let signature = builder.sign_checkpoint(&checkpoint);

        assert_eq!(signature.authority, "auth1");
        assert!(!signature.signature.is_empty());
    }
}
