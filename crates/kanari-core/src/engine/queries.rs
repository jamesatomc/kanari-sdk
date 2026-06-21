// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_rpc_api::{AccountInfo, BlockData, BlockchainStats, FullBlockData};
use kanari_types::address::Address as KanariAddress;
use log::{info, warn};

use super::*;
use crate::{BlockchainEngine, Checkpoint, CheckpointSyncData};

impl BlockchainEngine {
    pub fn latest_checkpoint_hash_hex(&self) -> String {
        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain
            .latest_checkpoint()
            .hash()
            .map(hex::encode)
            .unwrap_or_default()
    }

    pub fn latest_checkpoint_state_root_hex(&self) -> String {
        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        hex::encode(&chain.latest_checkpoint().state_root)
    }

    pub fn get_stats(&self) -> BlockchainStats {
        let state = self.state_read();
        let chain = match self.blockchain.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Blockchain lock poisoned in get_stats, recovering...");
                poisoned.into_inner()
            }
        };
        let pending_transactions = self.pending_transaction_len();

        BlockchainStats {
            height: chain.height(),
            total_blocks: chain.dag_checkpoints.len(),
            total_transactions: chain.get_transaction_count(),
            pending_transactions,
            total_accounts: state.account_count(),
            total_supply: state.total_supply,
            state_root: hex::encode(&chain.latest_checkpoint().state_root),
        }
    }

    pub fn get_account_info(&self, address: &str) -> Option<AccountInfo> {
        let state = self.state_read();

        state.get_account_by_hex(address).map(|acc| {
            let final_owned_objects = self.resolve_account_objects(&state, &acc.address);
            let sequence_number = self.get_expected_sequence(address);
            let mut actual_token_balances = std::collections::BTreeMap::new();

            for obj in &final_owned_objects {
                if !obj.type_.contains("::coin::Coin<") || obj.data.len() < 40 {
                    continue;
                }

                let Some(start) = obj.type_.find('<') else {
                    continue;
                };
                let Some(end) = obj.type_.rfind('>') else {
                    continue;
                };

                let token_type = obj.type_[start + 1..end].to_string();
                let mut amount_bytes = [0u8; 8];
                amount_bytes.copy_from_slice(&obj.data[32..40]);
                let amount = u64::from_le_bytes(amount_bytes);

                let entry = actual_token_balances.entry(token_type).or_insert(0u64);
                *entry = entry.saturating_add(amount);
            }

            for (token_type, balance) in &acc.token_balances {
                actual_token_balances
                    .entry(token_type.clone())
                    .or_insert_with(|| balance.value());
            }

            AccountInfo {
                address: format!("{:#x}", acc.address),
                sequence_number,
                modules: acc.modules.iter().cloned().collect(),
                token_balances: actual_token_balances,
                owned_objects: Some(final_owned_objects),
            }
        })
    }

    pub fn get_module_bytecode(&self, address: &str, module_name: &str) -> Option<Vec<u8>> {
        use move_core_types::{identifier::Identifier, language_storage::ModuleId};

        let addr = match KanariAddress::parse_to_account_address(address) {
            Ok(a) => a,
            Err(_) => return None,
        };

        let ident = match Identifier::new(module_name) {
            Ok(i) => i,
            Err(_) => return None,
        };

        let module_id = ModuleId::new(addr, ident);
        let runtime = &self.runtime_pool[0];
        runtime.get_module_bytes(&module_id)
    }

    pub fn list_all_modules(&self) -> Vec<(String, String)> {
        let runtime = &self.runtime_pool[0];
        runtime
            .list_modules()
            .into_iter()
            .map(|module_id| {
                (
                    format!("0x{}", module_id.address()),
                    module_id.name().to_string(),
                )
            })
            .collect()
    }

    fn checkpoint_hash_hex(checkpoint: &Checkpoint) -> String {
        checkpoint.hash().map(hex::encode).unwrap_or_default()
    }

    fn block_data_from_checkpoint(checkpoint: &Checkpoint) -> BlockData {
        BlockData {
            height: checkpoint.sequence,
            timestamp: checkpoint.timestamp,
            hash: Self::checkpoint_hash_hex(checkpoint),
            prev_hash: hex::encode(&checkpoint.prev_checkpoint_hash),
            state_root: hex::encode(&checkpoint.state_root),
            tx_count: checkpoint.transactions.len(),
            events: Vec::new(),
        }
    }

    fn full_block_data_from_checkpoint(checkpoint: &Checkpoint) -> FullBlockData {
        FullBlockData {
            height: checkpoint.sequence,
            timestamp: checkpoint.timestamp,
            hash: Self::checkpoint_hash_hex(checkpoint),
            prev_hash: hex::encode(&checkpoint.prev_checkpoint_hash),
            state_root: hex::encode(&checkpoint.state_root),
            tx_count: checkpoint.transactions.len(),
            events: Vec::new(),
            transactions: checkpoint.transactions.iter().cloned().collect(),
            vertices: checkpoint.vertices.iter().map(hex::encode).collect(),
        }
    }

    pub fn get_block(&self, height: u64) -> Option<BlockData> {
        let chain = match self.blockchain.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Blockchain lock poisoned in get_block, recovering...");
                poisoned.into_inner()
            }
        };
        chain
            .get_checkpoint(height)
            .map(Self::block_data_from_checkpoint)
    }

    pub fn get_full_block(&self, height: u64) -> Option<FullBlockData> {
        let chain = match self.blockchain.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Blockchain lock poisoned in get_full_block, recovering...");
                poisoned.into_inner()
            }
        };
        chain
            .get_checkpoint(height)
            .map(Self::full_block_data_from_checkpoint)
    }

    pub fn get_checkpoint_sync(&self, sequence: u64) -> Option<CheckpointSyncData> {
        let chain = match self.blockchain.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Blockchain lock poisoned in get_checkpoint_sync, recovering...");
                poisoned.into_inner()
            }
        };
        chain
            .get_checkpoint(sequence)
            .cloned()
            .map(|checkpoint| CheckpointSyncData {
                checkpoint,
                certificate: None,
            })
    }

    pub fn block_from_full_data(full_block: &FullBlockData) -> kanari_types::block::Block {
        use kanari_types::block::{Block, BlockHeader};
        use smt::compute_merkle_root as compute_transaction_merkle_root;

        let tx_hashes: Vec<Vec<u8>> = full_block.transactions.iter().map(|tx| tx.hash()).collect();
        let merkle_root = compute_transaction_merkle_root(&tx_hashes);

        let header = BlockHeader::new(
            full_block.height,
            hex::decode(&full_block.prev_hash).unwrap_or_default(),
            hex::decode(&full_block.state_root).unwrap_or_default(),
            merkle_root,
            full_block.tx_count,
            full_block.timestamp,
        );

        Block {
            header,
            transactions: full_block.transactions.clone(),
            events: full_block.events.clone(),
        }
    }

    fn checkpoint_committee_digest(&self) -> Result<Vec<u8>> {
        let mut entries = self
            .consensus_public_keys
            .iter()
            .map(|(authority_id, public_key)| (authority_id.clone(), public_key.clone()))
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(kanari_crypto::hash_data_blake3(&bcs::to_bytes(&(
            b"kanari:checkpoint-committee:v1".as_slice(),
            entries,
        ))?))
    }

    fn checkpoint_committee_total_voting_power(&self) -> u64 {
        self.consensus_public_keys.len() as u64
    }

    fn checkpoint_quorum_threshold(&self) -> u64 {
        let total = self.checkpoint_committee_total_voting_power();
        if total == 0 {
            return 0;
        }
        (2 * total) / 3 + 1
    }

    fn validate_checkpoint_certificate(
        &self,
        checkpoint: &Checkpoint,
        certificate: &crate::consensus::CheckpointCertificate,
    ) -> Result<()> {
        if self.consensus_public_keys.is_empty() {
            anyhow::bail!("No consensus committee is configured for checkpoint verification");
        }

        if certificate.chain_id != Self::checkpoint_chain_id() {
            anyhow::bail!("Checkpoint certificate chain id mismatch");
        }
        if certificate.epoch != Self::current_epoch() {
            anyhow::bail!("Checkpoint certificate epoch mismatch");
        }
        if certificate.protocol_version != Self::checkpoint_protocol_version() {
            anyhow::bail!("Checkpoint certificate protocol version mismatch");
        }
        if certificate.sequence != checkpoint.sequence {
            anyhow::bail!("Checkpoint certificate sequence mismatch");
        }
        if certificate.prev_checkpoint_hash != checkpoint.prev_checkpoint_hash {
            anyhow::bail!("Checkpoint certificate previous hash mismatch");
        }
        if certificate.state_root != checkpoint.state_root {
            anyhow::bail!("Checkpoint certificate state root mismatch");
        }
        if certificate.checkpoint_hash != checkpoint.hash()? {
            anyhow::bail!("Checkpoint certificate checkpoint hash mismatch");
        }
        if certificate.committee_digest != self.checkpoint_committee_digest()? {
            anyhow::bail!("Checkpoint certificate committee digest mismatch");
        }

        let signing_bytes = certificate.signing_bytes()?;
        let mut seen_signers = std::collections::HashSet::new();
        let mut voting_power = 0u64;
        for signer in &certificate.signatures {
            if !seen_signers.insert(signer.authority_id.clone()) {
                anyhow::bail!("Checkpoint certificate contains duplicate signer");
            }
            let public_key = self
                .consensus_public_keys
                .get(&signer.authority_id)
                .ok_or_else(|| {
                    anyhow::anyhow!("Checkpoint certificate signer is not in the active committee")
                })?;
            if signer.voting_power != 1 {
                anyhow::bail!("Checkpoint certificate signer has invalid voting power");
            }
            let public_key: [u8; 32] = public_key.as_slice().try_into().map_err(|_| {
                anyhow::anyhow!(
                    "Invalid consensus public key length for {}",
                    signer.authority_id
                )
            })?;
            let verifying_key =
                ed25519_dalek::VerifyingKey::from_bytes(&public_key).map_err(|e| {
                    anyhow::anyhow!(
                        "Invalid consensus public key for {}: {}",
                        signer.authority_id,
                        e
                    )
                })?;
            let signature_bytes: [u8; 64] =
                signer.signature.as_slice().try_into().map_err(|_| {
                    anyhow::anyhow!(
                        "Invalid checkpoint certificate signature length for {}",
                        signer.authority_id
                    )
                })?;
            let signature = ed25519_dalek::Signature::from_bytes(&signature_bytes);
            use ed25519_dalek::Verifier;
            verifying_key
                .verify(&signing_bytes, &signature)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Invalid checkpoint certificate signature for {}: {}",
                        signer.authority_id,
                        e
                    )
                })?;
            voting_power = voting_power.saturating_add(signer.voting_power);
        }

        if certificate.total_voting_power != voting_power {
            anyhow::bail!("Checkpoint certificate voting power total mismatch");
        }
        if voting_power < self.checkpoint_quorum_threshold() {
            anyhow::bail!("Checkpoint certificate has insufficient voting power");
        }

        Ok(())
    }

    pub fn sync_checkpoint_from_data(&self, checkpoint_data: &CheckpointSyncData) -> Result<()> {
        let stats = self.get_stats();
        let checkpoint = &checkpoint_data.checkpoint;
        info!(
            "[SYNC] Attempting to sync checkpoint #{} (our height: {})",
            checkpoint.sequence, stats.height
        );

        if checkpoint.sequence <= stats.height {
            info!(
                "[SYNC] Already have checkpoint #{}, skipping",
                checkpoint.sequence
            );
            return Ok(());
        }

        if checkpoint.sequence != stats.height + 1 {
            warn!(
                "[SYNC] Checkpoint #{} is not consecutive (need {})",
                checkpoint.sequence,
                stats.height + 1
            );
            anyhow::bail!(
                "Cannot sync checkpoint #{}: current height is {}",
                checkpoint.sequence,
                stats.height
            );
        }

        let certificate = checkpoint_data.certificate.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "Refusing to sync checkpoint #{} without a certificate",
                checkpoint.sequence
            )
        })?;
        self.validate_checkpoint_certificate(checkpoint, certificate)?;

        info!(
            "[SYNC] Verifying {} transaction signatures from checkpoint #{}",
            checkpoint.transactions.len(),
            checkpoint.sequence
        );
        for (i, signed_tx) in checkpoint.transactions.iter().enumerate() {
            signed_tx.verified_transaction_hash().map_err(|e| {
                anyhow::anyhow!(
                    "Invalid or missing signature for transaction {} in checkpoint #{}: {}",
                    i + 1,
                    checkpoint.sequence,
                    e
                )
            })?;
        }

        if checkpoint.transactions.is_empty() {
            anyhow::bail!(
                "Refusing to sync empty checkpoint #{} from network",
                checkpoint.sequence
            );
        }

        let checkpoint_to_apply = checkpoint.clone();
        let (computed_root, verified_state, to_execute) =
            self.prepare_checkpoint_state(&checkpoint_to_apply)?;

        if !self.checkpoint_root_matches(
            checkpoint_to_apply.sequence,
            &computed_root,
            &checkpoint_to_apply.state_root,
        )? {
            anyhow::bail!(
                "Checkpoint #{} state root mismatch: advertised={}, computed={}",
                checkpoint_to_apply.sequence,
                hex::encode(&checkpoint_to_apply.state_root),
                hex::encode(&computed_root)
            );
        }

        self.apply_prepared_checkpoint(checkpoint_to_apply, verified_state, to_execute, true)?;

        info!(
            "Synced checkpoint #{} with {} transactions",
            checkpoint.sequence,
            checkpoint.transactions.len()
        );

        Ok(())
    }

    /// Public view execution is disabled until the runtime provides both normal
    /// Move visibility enforcement and bounded gas/memory/return-size accounting.
    /// The previous path called `execute_function_bypass_visibility` with an
    /// unmetered gas meter, which was not safe to expose through RPC.
    pub fn execute_view_function(
        &self,
        _package_addr: &str,
        _module_name: &str,
        _function_name: &str,
        _type_args: &[String],
        _args: &[Vec<u8>],
    ) -> Result<serde_json::Value> {
        anyhow::bail!(
            "Move view execution is temporarily disabled pending metered, visibility-safe execution"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CheckpointSyncData,
        consensus::{Checkpoint, CheckpointCertificate, CheckpointSignature},
    };
    use ed25519_dalek::{Signer, SigningKey};
    use kanari_crypto::keys::{CurveType, generate_keypair};
    use kanari_move_runtime_v1::state::Account;
    use kanari_types::transaction::{SignedTransaction, Transaction};
    use kanari_types::{
        address::Address as KanariAddress, balance::BalanceRecord, kanari::KANARI_TOKEN_TYPE,
    };
    use std::collections::BTreeMap;

    fn signed_transfer(sequence_number: u64) -> SignedTransaction {
        let sender = generate_keypair(CurveType::Ed25519).unwrap();
        let recipient = generate_keypair(CurveType::Ed25519).unwrap();
        let tx = Transaction::new_transfer(
            sender.tagged_address(),
            recipient.address,
            1,
            sequence_number,
        );
        let mut signed_tx = SignedTransaction::new(tx);
        signed_tx
            .sign(&sender.private_key, sender.curve_type)
            .unwrap();
        signed_tx
    }

    fn fund_sender(engine: &BlockchainEngine, address: &str, balance: u64) {
        let addr = KanariAddress::parse_to_account_address(address).unwrap();
        let mut account = Account::with_native_balance(addr, balance);
        account.set_token_balance(KANARI_TOKEN_TYPE.to_string(), BalanceRecord::new(balance));
        engine
            .state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .save_account(&account)
            .unwrap();
    }

    fn authority_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn configured_engine() -> (BlockchainEngine, BTreeMap<String, SigningKey>) {
        let mut engine = BlockchainEngine::new_in_memory().unwrap();
        let authorities = vec![
            "0x1".to_string(),
            "0x2".to_string(),
            "0x3".to_string(),
            "0x4".to_string(),
        ];
        let mut signing_keys = BTreeMap::new();
        signing_keys.insert("0x1".to_string(), authority_key(1));
        signing_keys.insert("0x2".to_string(), authority_key(2));
        signing_keys.insert("0x3".to_string(), authority_key(3));
        signing_keys.insert("0x4".to_string(), authority_key(4));
        let public_keys = signing_keys
            .iter()
            .map(|(authority, key)| (authority.clone(), key.verifying_key().to_bytes().to_vec()))
            .collect::<BTreeMap<_, _>>();
        engine.set_authorities("0x1".to_string(), authorities);
        engine
            .set_consensus_signing_key(signing_keys.get("0x1").unwrap().clone(), public_keys)
            .unwrap();
        (engine, signing_keys)
    }

    fn build_certificate(
        engine: &BlockchainEngine,
        checkpoint: &Checkpoint,
        signing_keys: &BTreeMap<String, SigningKey>,
        signer_ids: &[&str],
    ) -> CheckpointCertificate {
        let mut certificate = CheckpointCertificate {
            epoch: BlockchainEngine::current_epoch(),
            chain_id: BlockchainEngine::checkpoint_chain_id(),
            protocol_version: BlockchainEngine::checkpoint_protocol_version(),
            sequence: checkpoint.sequence,
            checkpoint_hash: checkpoint.hash().unwrap(),
            prev_checkpoint_hash: checkpoint.prev_checkpoint_hash.clone(),
            state_root: checkpoint.state_root.clone(),
            certified_vertex: checkpoint.vertices.first().copied(),
            committee_digest: engine.checkpoint_committee_digest().unwrap(),
            signatures: Vec::new(),
            total_voting_power: 0,
        };
        let signing_bytes = certificate.signing_bytes().unwrap();
        for signer_id in signer_ids {
            let key = signing_keys.get(*signer_id).unwrap();
            certificate.signatures.push(CheckpointSignature {
                authority_id: (*signer_id).to_string(),
                signature: key.sign(&signing_bytes).to_bytes().to_vec(),
                voting_power: 1,
            });
        }
        certificate.total_voting_power = certificate
            .signatures
            .iter()
            .map(|signature| signature.voting_power)
            .sum();
        certificate
    }

    #[test]
    fn sync_checkpoint_from_data_rejects_missing_certificate() {
        let (engine, _) = configured_engine();
        let prev_hash = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let checkpoint = Checkpoint::new(
            1,
            vec![],
            vec![signed_transfer(0)],
            vec![9u8; 32],
            42,
            prev_hash,
        );
        let sync_data = CheckpointSyncData {
            checkpoint,
            certificate: None,
        };

        let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
        assert!(error.to_string().contains("without a certificate"));
    }

    #[test]
    fn sync_checkpoint_from_data_rejects_empty_checkpoint() {
        let (engine, signing_keys) = configured_engine();
        let prev_hash = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let state_root = engine
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .compute_state_root();
        let checkpoint = Checkpoint::new(1, vec![], vec![], state_root, 42, prev_hash);
        let certificate =
            build_certificate(&engine, &checkpoint, &signing_keys, &["0x1", "0x2", "0x3"]);
        let sync_data = CheckpointSyncData {
            checkpoint,
            certificate: Some(certificate),
        };

        let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Refusing to sync empty checkpoint")
        );
        assert_eq!(engine.get_stats().height, 0);
    }

    #[test]
    fn sync_checkpoint_from_data_rejects_root_mismatch() {
        let (engine, signing_keys) = configured_engine();
        let prev_hash = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let signed_tx = signed_transfer(0);
        fund_sender(&engine, signed_tx.transaction.sender_address(), 1_000_000);
        let checkpoint = Checkpoint::new(1, vec![], vec![signed_tx], vec![9u8; 32], 42, prev_hash);
        let certificate =
            build_certificate(&engine, &checkpoint, &signing_keys, &["0x1", "0x2", "0x3"]);
        let sync_data = CheckpointSyncData {
            checkpoint,
            certificate: Some(certificate),
        };

        let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
        assert!(error.to_string().contains("state root mismatch"));
        assert_eq!(engine.get_stats().height, 0);
    }

    #[test]
    fn sync_checkpoint_from_data_rejects_stale_committee_certificate() {
        let (engine, signing_keys) = configured_engine();
        let prev_hash = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let checkpoint = Checkpoint::new(
            1,
            vec![],
            vec![signed_transfer(0)],
            vec![9u8; 32],
            42,
            prev_hash,
        );
        let mut certificate =
            build_certificate(&engine, &checkpoint, &signing_keys, &["0x1", "0x2", "0x3"]);
        certificate.committee_digest = vec![7u8; 32];
        let sync_data = CheckpointSyncData {
            checkpoint,
            certificate: Some(certificate),
        };

        let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
        assert!(error.to_string().contains("committee digest mismatch"));
    }

    #[test]
    fn sync_checkpoint_from_data_rejects_insufficient_voting_power() {
        let (engine, signing_keys) = configured_engine();
        let prev_hash = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let checkpoint = Checkpoint::new(
            1,
            vec![],
            vec![signed_transfer(0)],
            vec![9u8; 32],
            42,
            prev_hash,
        );
        let certificate = build_certificate(&engine, &checkpoint, &signing_keys, &["0x1", "0x2"]);
        let sync_data = CheckpointSyncData {
            checkpoint,
            certificate: Some(certificate),
        };

        let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
        assert!(error.to_string().contains("insufficient voting power"));
    }

    #[test]
    fn sync_checkpoint_from_data_rejects_duplicate_signers() {
        let (engine, signing_keys) = configured_engine();
        let prev_hash = {
            let chain = engine.blockchain.read().unwrap_or_else(|e| e.into_inner());
            chain.latest_checkpoint().hash().unwrap()
        };
        let checkpoint = Checkpoint::new(
            1,
            vec![],
            vec![signed_transfer(0)],
            vec![9u8; 32],
            42,
            prev_hash,
        );
        let certificate =
            build_certificate(&engine, &checkpoint, &signing_keys, &["0x1", "0x1", "0x2"]);
        let sync_data = CheckpointSyncData {
            checkpoint,
            certificate: Some(certificate),
        };

        let error = engine.sync_checkpoint_from_data(&sync_data).unwrap_err();
        assert!(error.to_string().contains("duplicate signer"));
    }

    #[test]
    fn view_execution_fails_closed() {
        let engine = BlockchainEngine::new_in_memory().unwrap();
        let error = engine
            .execute_view_function("0x2", "coin", "value", &[], &[])
            .unwrap_err();
        assert!(error.to_string().contains("temporarily disabled"));
    }
}
