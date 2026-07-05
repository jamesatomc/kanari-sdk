// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_rpc_api::{AccountInfo, BlockData, BlockchainStats, FullBlockData};
use kanari_types::address::Address as KanariAddress;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use log::{info, warn};

use super::*;
use crate::{BlockchainEngine, Checkpoint, CheckpointSyncData};

impl BlockchainEngine {
    fn committed_transaction_count_for_stats(&self, fallback_count: usize) -> usize {
        let Some(store) = &self.persistent_store else {
            return fallback_count;
        };

        match store.logical_entries() {
            Ok(entries) => {
                let mut payload_hashes = std::collections::HashSet::new();
                let mut indexed_hashes = std::collections::HashSet::new();
                let mut executed_object_hashes = std::collections::HashSet::new();

                for (key, _) in &entries {
                    if let Some(hash) = key.strip_prefix(b"tx_payload/") {
                        payload_hashes.insert(hash.to_vec());
                    } else if let Some(hash) = key.strip_prefix(b"tx_index/") {
                        indexed_hashes.insert(hash.to_vec());
                    } else if let Some(hash) = key.strip_prefix(b"object_tx:executed:") {
                        executed_object_hashes.insert(hash.to_vec());
                    }
                }

                payload_hashes
                    .intersection(&indexed_hashes)
                    .count()
                    .saturating_add(executed_object_hashes.len())
                    .max(fallback_count)
            }
            Err(error) => {
                warn!(
                    "Failed to scan committed transaction history for stats: {}",
                    error
                );
                fallback_count
            }
        }
    }

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
        let object_pending = self.pending_object_transaction_len().unwrap_or_else(|error| {
            warn!("Failed to read object transaction pending count: {}", error);
            0
        });
        let pending_transactions = self
            .pending_transaction_len()
            .saturating_add(object_pending);
        let total_transactions =
            self.committed_transaction_count_for_stats(chain.get_transaction_count());

        BlockchainStats {
            height: chain.height(),
            total_blocks: chain.dag_checkpoints.len(),
            total_transactions,
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
                if token_type == KANARI_TOKEN_TYPE {
                    actual_token_balances.insert(token_type.clone(), balance.value());
                } else {
                    actual_token_balances
                        .entry(token_type.clone())
                        .or_insert_with(|| balance.value());
                }
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
            .map(|checkpoint| CheckpointSyncData { checkpoint })
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

    pub fn sync_checkpoint_from_data(&self, checkpoint_data: &CheckpointSyncData) -> Result<()> {
        let stats = self.get_stats();
        let checkpoint = &checkpoint_data.checkpoint;
        if checkpoint.sequence <= stats.height {
            return Ok(());
        }
        if checkpoint.sequence != stats.height.saturating_add(1) {
            anyhow::bail!(
                "Cannot sync checkpoint {} while local height is {}",
                checkpoint.sequence,
                stats.height
            );
        }
        self.apply_checkpoint(checkpoint.clone())?;
        info!("Synced checkpoint {}", checkpoint.sequence);
        Ok(())
    }
}
