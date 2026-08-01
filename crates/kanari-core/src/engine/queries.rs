// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use kanari_rpc_api::ObjectInfo;
use kanari_rpc_api::{
    BlockData, BlockchainStats, CanonicalStateDiffResponse, CanonicalStateEntry,
    CanonicalStateSnapshotResponse, CompareCanonicalStateSnapshotRequest, FullBlockData, OwnerInfo,
    RpcObjectOwnerKindFilter, SmtStatusResponse,
};
use kanari_types::address::Address as KanariAddress;
use kanari_types::transaction::{ObjectOwnerKind, ObjectRef};
use log::info;

use super::*;
use crate::{BlockchainEngine, Checkpoint, CheckpointSyncData};
use std::sync::RwLockReadGuard;

impl BlockchainEngine {
    // Checkpoint sync is sequential, so the receiver normally already has the
    // previous round. Crash/restart catch-up can be much further behind and
    // needs enough signed Mysticeti evidence to rebuild the local DAG before it
    // applies a checkpoint. Keep this bounded and operator-tunable because
    // vertices carry transaction payloads.
    const DEFAULT_DAG_VERTICES_PER_CHECKPOINT_SYNC: usize = 128;
    const MAX_DAG_VERTICES_PER_CHECKPOINT_SYNC: usize = 512;

    fn dag_vertices_per_checkpoint_sync() -> usize {
        std::env::var("KANARI_DAG_VERTICES_PER_CHECKPOINT_SYNC")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .map(|value| value.clamp(16, Self::MAX_DAG_VERTICES_PER_CHECKPOINT_SYNC))
            .unwrap_or(Self::DEFAULT_DAG_VERTICES_PER_CHECKPOINT_SYNC)
    }
    pub fn smt_status(&self, audit: bool) -> Result<SmtStatusResponse> {
        let diagnostics = self.state_read().smt_diagnostics(audit)?;
        let stats = self.try_get_stats()?;
        Ok(SmtStatusResponse {
            height: stats.height,
            checkpoint_state_root: stats.state_root,
            enabled: diagnostics.enabled,
            persisted_root: diagnostics.persisted_root,
            effective_root: diagnostics.effective_root,
            overlay_entries: diagnostics.overlay_entries,
            overlay_updates: diagnostics.overlay_updates,
            overlay_deletes: diagnostics.overlay_deletes,
            canonical_membership_changed: diagnostics.canonical_membership_changed,
            runtime_schema_version: diagnostics.runtime_schema_version,
            expected_runtime_schema_version: diagnostics.expected_runtime_schema_version,
            wallet_supply_index_version: diagnostics.wallet_supply_index_version,
            expected_wallet_supply_index_version: diagnostics.expected_wallet_supply_index_version,
            audit_requested: diagnostics.audit_requested,
            audit_performed: diagnostics.audit_performed,
            persisted_leaf_count: diagnostics.persisted_leaf_count,
            consistent: diagnostics.consistent,
            consistency_error: diagnostics.consistency_error,
        })
    }

    pub fn canonical_state_snapshot_entries(
        &self,
        limit: Option<usize>,
        prefix: Option<&str>,
    ) -> Result<Vec<CanonicalStateEntry>> {
        let state = self.state_read();
        let mut entries = state
            .try_canonical_state_snapshot()?
            .into_iter()
            .map(|(key, value)| CanonicalStateEntry {
                key: String::from_utf8_lossy(&key).into_owned(),
                value: hex::encode(value),
            })
            .filter(|entry| match prefix {
                Some(prefix) => entry.key.starts_with(prefix),
                None => true,
            })
            .collect::<Vec<_>>();
        if let Some(limit) = limit {
            entries.truncate(limit);
        }
        Ok(entries)
    }

    pub fn canonical_state_snapshot_response(
        &self,
        limit: Option<usize>,
        prefix: Option<&str>,
    ) -> Result<CanonicalStateSnapshotResponse> {
        let entries = self.canonical_state_snapshot_entries(limit, prefix)?;
        Ok(CanonicalStateSnapshotResponse {
            height: self.try_get_stats()?.height,
            state_root: self.latest_checkpoint_state_root_hex(),
            entry_count: entries.len(),
            entries,
        })
    }

    pub fn compare_canonical_state_snapshot(
        &self,
        req: &CompareCanonicalStateSnapshotRequest,
    ) -> Result<CanonicalStateDiffResponse> {
        use std::collections::BTreeMap;

        let local_entries = self.canonical_state_snapshot_entries(None, None)?;
        let local_map = local_entries
            .iter()
            .map(|entry| (entry.key.clone(), entry.value.clone()))
            .collect::<BTreeMap<_, _>>();
        let remote_map = req
            .entries
            .iter()
            .map(|entry| (entry.key.clone(), entry.value.clone()))
            .collect::<BTreeMap<_, _>>();

        let mut first_divergence = None;
        for key in local_map.keys().chain(remote_map.keys()) {
            match (local_map.get(key), remote_map.get(key)) {
                (Some(left), Some(right)) if left == right => {}
                (Some(left), Some(right)) => {
                    first_divergence = Some(format!("key={} left={} right={}", key, left, right));
                    break;
                }
                (Some(left), None) => {
                    first_divergence = Some(format!("key={} missing_on_right left={}", key, left));
                    break;
                }
                (None, Some(right)) => {
                    first_divergence = Some(format!("key={} missing_on_left right={}", key, right));
                    break;
                }
                (None, None) => {}
            }
        }

        Ok(CanonicalStateDiffResponse {
            height: self.try_get_stats()?.height,
            state_root: self.latest_checkpoint_state_root_hex(),
            local_entry_count: local_map.len(),
            remote_entry_count: remote_map.len(),
            first_divergence,
        })
    }

    pub fn try_canonical_state_snapshot_dump(
        &self,
        limit: Option<usize>,
    ) -> Result<Vec<(String, String)>> {
        let mut entries = self
            .canonical_state_snapshot_entries(limit, None)?
            .into_iter()
            .map(|entry| (entry.key, entry.value))
            .collect::<Vec<_>>();
        if let Some(limit) = limit {
            entries.truncate(limit);
        }
        Ok(entries)
    }

    pub fn canonical_state_snapshot_dump(&self, limit: Option<usize>) -> Vec<(String, String)> {
        self.try_canonical_state_snapshot_dump(limit)
            .expect("canonical snapshot dump requires readable, well-formed persistent state")
    }

    pub fn try_first_canonical_state_divergence(&self, other: &Self) -> Result<Option<String>> {
        let left = self.state_read().try_canonical_state_snapshot()?;
        let right = other.state_read().try_canonical_state_snapshot()?;

        for key in left.keys().chain(right.keys()) {
            match (left.get(key), right.get(key)) {
                (Some(left_value), Some(right_value)) if left_value == right_value => {}
                (Some(left_value), Some(right_value)) => {
                    return Ok(Some(format!(
                        "key={} left={} right={}",
                        String::from_utf8_lossy(key),
                        hex::encode(left_value),
                        hex::encode(right_value)
                    )));
                }
                (Some(left_value), None) => {
                    return Ok(Some(format!(
                        "key={} missing_on_right left={}",
                        String::from_utf8_lossy(key),
                        hex::encode(left_value)
                    )));
                }
                (None, Some(right_value)) => {
                    return Ok(Some(format!(
                        "key={} missing_on_left right={}",
                        String::from_utf8_lossy(key),
                        hex::encode(right_value)
                    )));
                }
                (None, None) => {}
            }
        }

        Ok(None)
    }

    pub fn first_canonical_state_divergence(&self, other: &Self) -> Option<String> {
        self.try_first_canonical_state_divergence(other).expect(
            "canonical state divergence check requires readable, well-formed persistent state",
        )
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
        match self.try_get_stats() {
            Ok(stats) => stats,
            Err(error) => {
                log::error!("Failed to compute blockchain stats: {error:#}");
                let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
                BlockchainStats {
                    height: chain.height(),
                    total_blocks: chain.dag_checkpoints.len(),
                    total_transactions: chain.get_transaction_count(),
                    pending_transactions: self.pending_transaction_len(),
                    total_owners: 0,
                    total_supply: self.state_read().total_supply,
                    state_root: hex::encode(&chain.latest_checkpoint().state_root),
                }
            }
        }
    }

    pub fn try_get_stats(&self) -> Result<BlockchainStats> {
        let state = self.state_read();
        let chain = match self.blockchain.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!("Blockchain lock poisoned in get_stats, recovering...");
                poisoned.into_inner()
            }
        };
        let pending_transactions = self.pending_transaction_len();

        Ok(BlockchainStats {
            height: chain.height(),
            total_blocks: chain.dag_checkpoints.len(),
            total_transactions: chain.get_transaction_count(),
            pending_transactions,
            total_owners: state.try_owner_count()?,
            total_supply: state.total_supply,
            state_root: hex::encode(&chain.latest_checkpoint().state_root),
        })
    }

    pub fn get_owner_info(&self, owner: &str) -> Option<OwnerInfo> {
        match self.try_get_owner_info(owner) {
            Ok(info) => info,
            Err(error) => {
                log::error!("Failed to query owner info for {owner}: {error:#}");
                None
            }
        }
    }

    pub fn try_get_owner_info(&self, owner: &str) -> Result<Option<OwnerInfo>> {
        let state = self.state_read();
        let Some(acc) = state.try_get_owner_state_by_hex(owner)? else {
            return Ok(None);
        };
        let final_owned_objects = self.resolve_account_objects(&state, &acc.address)?;
        let balances = state
            .resolve_owner_token_balances(acc.address)
            .with_context(|| format!("Failed to resolve token balances for owner {owner}"))?;

        Ok(Some(OwnerInfo {
            owner: format!("{:#x}", acc.owner_address()),
            nonce: Some(self.get_expected_nonce(owner)),
            modules: acc.modules.iter().cloned().collect(),
            balances,
            owned_object_count: Some(final_owned_objects.len()),
            owned_objects: Some(final_owned_objects),
        }))
    }

    pub fn get_objects_by_type(&self, object_type: &str) -> Result<Vec<ObjectInfo>> {
        self.query_objects(None, None, Some(object_type), None, None)
    }

    pub(crate) fn object_info_from_created_object(id: String, obj: CreatedObject) -> ObjectInfo {
        let digest = obj.digest();
        ObjectInfo {
            id,
            owner: format!("{:#x}", obj.owner),
            owner_kind: obj.owner_kind,
            type_: obj.type_,
            data: obj.data,
            version: obj.version,
            digest: Some(digest),
        }
    }

    pub fn query_objects(
        &self,
        owner: Option<&str>,
        owner_kind: Option<RpcObjectOwnerKindFilter>,
        object_type: Option<&str>,
        min_version: Option<u64>,
        max_version: Option<u64>,
    ) -> Result<Vec<ObjectInfo>> {
        let state = self.state_read();
        let owner_addr = owner
            .map(KanariAddress::parse_to_account_address)
            .transpose()?;
        let owner_kind_filter = owner_kind.as_ref().map(|kind| match kind {
            RpcObjectOwnerKindFilter::Address => ObjectOwnerKind::AddressOwner("".to_string()),
            RpcObjectOwnerKindFilter::Shared => ObjectOwnerKind::Shared,
            RpcObjectOwnerKindFilter::Immutable => ObjectOwnerKind::Immutable,
        });
        Ok(state
            .query_objects(
                owner_addr,
                owner_kind_filter.as_ref(),
                object_type,
                min_version,
                max_version,
            )?
            .into_iter()
            .map(|(id, obj)| Self::object_info_from_created_object(id, obj))
            .collect())
    }

    pub fn get_object_by_ref(&self, object_ref: &ObjectRef) -> Result<Option<ObjectInfo>> {
        let state = self.state_read();
        let Some(obj) = state.get_object(&object_ref.object_id)? else {
            return Ok(None);
        };
        if let Some(version) = object_ref.version
            && obj.version != version
        {
            return Ok(None);
        }
        if let Some(digest) = &object_ref.digest
            && obj.digest() != *digest
        {
            return Ok(None);
        }
        Ok(Some(Self::object_info_from_created_object(
            object_ref.object_id.clone(),
            obj,
        )))
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

    fn blockchain_read_for_query(&self, context: &str) -> RwLockReadGuard<'_, Blockchain> {
        self.blockchain.read().unwrap_or_else(|poisoned| {
            log::error!("Blockchain lock poisoned in {context}, recovering...");
            poisoned.into_inner()
        })
    }

    fn block_data_from_checkpoint(checkpoint: &Checkpoint) -> BlockData {
        let (transaction_effects, object_changes, object_graph_edges) =
            Self::checkpoint_detail_collections(checkpoint);
        BlockData {
            height: checkpoint.sequence,
            timestamp: checkpoint.timestamp,
            hash: Self::checkpoint_hash_hex(checkpoint),
            prev_hash: hex::encode(&checkpoint.prev_checkpoint_hash),
            state_root: hex::encode(&checkpoint.state_root),
            tx_count: checkpoint.transactions.len(),
            events: Vec::new(),
            transaction_effects,
            object_changes,
            object_graph_edges,
        }
    }

    fn full_block_data_from_checkpoint(checkpoint: &Checkpoint) -> FullBlockData {
        let (transaction_effects, object_changes, object_graph_edges) =
            Self::checkpoint_detail_collections(checkpoint);
        FullBlockData {
            height: checkpoint.sequence,
            timestamp: checkpoint.timestamp,
            hash: Self::checkpoint_hash_hex(checkpoint),
            prev_hash: hex::encode(&checkpoint.prev_checkpoint_hash),
            state_root: hex::encode(&checkpoint.state_root),
            tx_count: checkpoint.transactions.len(),
            events: Vec::new(),
            transactions: checkpoint.transactions.iter().cloned().collect(),
            transaction_effects,
            object_changes,
            object_graph_edges,
            vertices: checkpoint.vertices.iter().map(hex::encode).collect(),
        }
    }

    fn checkpoint_detail_collections(
        checkpoint: &Checkpoint,
    ) -> (
        Vec<kanari_types::transaction::TransactionEffects>,
        Vec<kanari_types::transaction::ObjectChange>,
        Vec<kanari_types::transaction::ObjectGraphEdge>,
    ) {
        (
            checkpoint.transaction_effects.iter().cloned().collect(),
            if checkpoint.object_changes.is_empty() {
                checkpoint
                    .transaction_effects
                    .iter()
                    .flat_map(|effects| effects.object_changes.iter().cloned())
                    .collect()
            } else {
                checkpoint.object_changes.iter().cloned().collect()
            },
            if checkpoint.object_graph_edges.is_empty() {
                checkpoint
                    .transaction_effects
                    .iter()
                    .flat_map(|effects| effects.causal_edges.iter().cloned())
                    .collect()
            } else {
                checkpoint.object_graph_edges.iter().cloned().collect()
            },
        )
    }

    pub fn get_block(&self, height: u64) -> Option<BlockData> {
        let chain = self.blockchain_read_for_query("get_block");
        chain
            .get_checkpoint(height)
            .map(Self::block_data_from_checkpoint)
    }

    pub fn get_full_block(&self, height: u64) -> Option<FullBlockData> {
        let chain = self.blockchain_read_for_query("get_full_block");
        chain
            .get_checkpoint(height)
            .map(Self::full_block_data_from_checkpoint)
    }

    pub fn get_checkpoint_sync(&self, sequence: u64) -> Result<Option<CheckpointSyncData>> {
        let chain = self.blockchain_read_for_query("get_checkpoint_sync");
        let Some(checkpoint) = chain.get_checkpoint(sequence).cloned() else {
            return Ok(None);
        };
        drop(chain);
        let dag_vertices = self.dag_vertices_for_checkpoint_sync(
            &checkpoint.vertices,
            Self::dag_vertices_per_checkpoint_sync(),
        )?;
        Ok(Some(CheckpointSyncData {
            checkpoint,
            dag_vertices,
        }))
    }

    pub fn checkpoint_hash(&self, sequence: u64) -> Result<Option<Vec<u8>>> {
        let chain = self.blockchain.read().unwrap_or_else(|e| e.into_inner());
        chain
            .get_checkpoint(sequence)
            .map(Checkpoint::hash)
            .transpose()
    }

    fn decode_block_hash_field(label: &str, value: &str) -> Result<Vec<u8>> {
        let decoded = decode_hex_exact(label, value, 32)?;
        Ok(decoded)
    }

    pub fn try_block_from_full_data(
        full_block: &FullBlockData,
    ) -> Result<kanari_types::block::Block> {
        use kanari_types::block::{Block, BlockHeader};
        use smt::compute_merkle_root as compute_transaction_merkle_root;

        let tx_hashes: Vec<Vec<u8>> = full_block.transactions.iter().map(|tx| tx.hash()).collect();
        let merkle_root = compute_transaction_merkle_root(&tx_hashes);

        let header = BlockHeader::new(
            full_block.height,
            Self::decode_block_hash_field("block prev_hash", &full_block.prev_hash)?,
            Self::decode_block_hash_field("block state_root", &full_block.state_root)?,
            merkle_root,
            full_block.tx_count,
            full_block.timestamp,
        );

        Ok(Block {
            header,
            transactions: full_block.transactions.clone(),
            events: full_block.events.clone(),
        })
    }

    pub fn block_from_full_data(full_block: &FullBlockData) -> kanari_types::block::Block {
        Self::try_block_from_full_data(full_block)
            .expect("FullBlockData must contain valid 32-byte hex prev_hash and state_root")
    }

    pub fn sync_checkpoint_from_data(&self, checkpoint_data: &CheckpointSyncData) -> Result<()> {
        let stats = self.try_get_stats()?;
        let checkpoint = &checkpoint_data.checkpoint;
        info!(
            "[SYNC] Attempting to sync checkpoint #{} (our height: {})",
            checkpoint.sequence, stats.height
        );

        if checkpoint.sequence <= stats.height {
            let local = self
                .get_checkpoint_sync(checkpoint.sequence)?
                .ok_or_else(|| {
                    anyhow::anyhow!("Missing local checkpoint #{}", checkpoint.sequence)
                })?;
            anyhow::ensure!(
                local.checkpoint.hash()? == checkpoint.hash()?,
                "Conflicting checkpoint #{}",
                checkpoint.sequence
            );
            return Ok(());
        }

        anyhow::bail!(
            "Checkpoint #{} has not been committed by the local Mysticeti DAG; apply its signed DAG evidence first",
            checkpoint.sequence
        )
    }

    pub fn execute_view_function(
        &self,
        package_addr: &str,
        module_name: &str,
        function_name: &str,
        type_args: &[String],
        args: &[Vec<u8>],
        object_inputs: &[kanari_types::transaction::ObjectInput],
    ) -> Result<serde_json::Value> {
        let runtime = &self.runtime_pool[0];
        runtime.execute_view_function(
            package_addr,
            module_name,
            function_name,
            type_args,
            args,
            object_inputs,
        )
    }
}

#[cfg(test)]
#[path = "../../tests/unit/queries_tests.rs"]
mod tests;
