// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use hex;
use kanari_crypto::hash_data_blake3;
use kanari_types::coin::TreasuryCap;
use kanari_types::digest::{Digest, ObjectDigest, TransactionDigest};
use kanari_types::effects::{ExecutionStatus, GasCostSummary, ObjectRef, TransactionEffects};
use kanari_types::object::UIDRecord;
use kanari_types::{balance::BalanceRecord, event::Event};
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Created object information captured from Move VM write-sets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedObject {
    pub owner: AccountAddress,
    /// Optional UIDRecord when object follows UID pattern
    pub uid: Option<UIDRecord>,
    #[serde(rename = "type")]
    pub type_: String,
    pub data: Vec<u8>,
    pub version: u64,
}

/// Represents changes to account state from Move VM execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountChange {
    pub address: AccountAddress,
    pub balance_delta: i64, // Positive = credit, Negative = debit
    pub sequence_increment: u64,
    pub modules_added: BTreeSet<String>,
}

impl AccountChange {
    pub fn new(address: AccountAddress) -> Self {
        Self {
            address,
            balance_delta: 0,
            sequence_increment: 0,
            modules_added: BTreeSet::new(),
        }
    }

    pub fn debit(&mut self, amount: u64) {
        self.balance_delta -= amount as i64;
    }

    pub fn credit(&mut self, amount: u64) {
        self.balance_delta += amount as i64;
    }

    pub fn increment_sequence(&mut self) {
        self.sequence_increment += 1;
    }

    pub fn add_module(&mut self, module_name: String) {
        self.modules_added.insert(module_name);
    }
}

/// ChangeSet represents all state changes from Move VM execution
/// This is the canonical output from Move VM that StateManager will apply
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChangeSet {
    pub account_changes: BTreeMap<AccountAddress, AccountChange>,
    pub events: Vec<Event>,
    /// Treasury creations or updates: (owner, token_type, TreasuryCap)
    pub treasuries: Vec<(AccountAddress, String, TreasuryCap)>,
    /// NFT capability creations or updates: (owner, token_type, NftCapRecord)
    pub nft_caps: Vec<(
        AccountAddress,
        String,
        kanari_types::collection::NftCapRecord,
    )>,
    /// Per-account token balances (absolute set): (owner, token_type, BalanceRecord)
    pub token_balance_sets: Vec<(AccountAddress, String, BalanceRecord)>,
    /// Objects created during execution. Each entry is (object_id, CreatedObject)
    pub created_objects: Vec<(String, CreatedObject)>,
    /// Objects deleted during execution. Each entry is object_id
    pub deleted_objects: Vec<String>,
    /// (object_id, name_bytes, value_bytes)
    pub added_dynamic_fields: Vec<(String, Vec<u8>, Vec<u8>)>,
    /// (object_id, name_bytes)
    pub removed_dynamic_fields: Vec<(String, Vec<u8>)>,
    pub gas_used: u64,
    pub success: bool,
    pub error_message: Option<String>,
}

impl ChangeSet {
    pub fn new() -> Self {
        Self {
            account_changes: BTreeMap::new(),
            events: Vec::new(),
            treasuries: Vec::new(),
            nft_caps: Vec::new(),
            token_balance_sets: Vec::new(),
            created_objects: Vec::new(),
            deleted_objects: Vec::new(),
            added_dynamic_fields: Vec::new(),
            removed_dynamic_fields: Vec::new(),
            gas_used: 0,
            success: true,
            error_message: None,
        }
    }

    pub fn with_gas(gas_used: u64) -> Self {
        Self {
            account_changes: BTreeMap::new(),
            events: Vec::new(),
            treasuries: Vec::new(),
            nft_caps: Vec::new(),
            token_balance_sets: Vec::new(),
            created_objects: Vec::new(),
            deleted_objects: Vec::new(),
            added_dynamic_fields: Vec::new(),
            removed_dynamic_fields: Vec::new(),
            gas_used,
            success: true,
            error_message: None,
        }
    }

    pub fn failed(error: String, gas_used: u64) -> Self {
        Self {
            account_changes: BTreeMap::new(),
            events: Vec::new(),
            treasuries: Vec::new(),
            nft_caps: Vec::new(),
            token_balance_sets: Vec::new(),
            created_objects: Vec::new(),
            deleted_objects: Vec::new(),
            added_dynamic_fields: Vec::new(),
            removed_dynamic_fields: Vec::new(),
            gas_used,
            success: false,
            error_message: Some(error),
        }
    }

    pub fn get_or_create_change(&mut self, address: AccountAddress) -> &mut AccountChange {
        self.account_changes
            .entry(address)
            .or_insert_with(|| AccountChange::new(address))
    }

    /// Transfer operation: debit sender, credit receiver
    pub fn transfer(&mut self, from: AccountAddress, to: AccountAddress, amount: u64) {
        let sender = self.get_or_create_change(from);
        sender.debit(amount);
        sender.increment_sequence();

        let receiver = self.get_or_create_change(to);
        receiver.credit(amount);
    }

    /// Mint operation: create new tokens
    pub fn mint(&mut self, to: AccountAddress, amount: u64) {
        let receiver = self.get_or_create_change(to);
        receiver.credit(amount);
    }

    /// Burn operation: destroy tokens
    pub fn burn(&mut self, from: AccountAddress, amount: u64) {
        let sender = self.get_or_create_change(from);
        sender.debit(amount);
    }

    /// Module publish operation
    /// Note: Does NOT increment sequence - that's handled by the engine layer
    pub fn publish_module(&mut self, publisher: AccountAddress, module_name: String) {
        let account = self.get_or_create_change(publisher);
        account.add_module(module_name);
    }

    /// Collect gas fees to DAO
    pub fn collect_gas(&mut self, dao_address: AccountAddress, gas_amount: u64) {
        let dao = self.get_or_create_change(dao_address);
        dao.credit(gas_amount);
    }

    pub fn set_gas_used(&mut self, gas: u64) {
        self.gas_used = gas;
    }

    pub fn mark_failed(&mut self, error: String) {
        self.success = false;
        self.error_message = Some(error);
    }

    pub fn is_empty(&self) -> bool {
        self.account_changes.is_empty()
            && self.events.is_empty()
            && self.treasuries.is_empty()
            && self.token_balance_sets.is_empty()
            && self.created_objects.is_empty()
            && self.deleted_objects.is_empty()
            && self.added_dynamic_fields.is_empty()
            && self.removed_dynamic_fields.is_empty()
            && self.gas_used == 0
            && self.success
            && self.error_message.is_none()
    }

    pub fn account_count(&self) -> usize {
        self.account_changes.len()
    }

    /// Merge another ChangeSet into this one
    /// Used to combine Move VM changes with gas/sequence changes
    /// Token balance sets are consolidated to prevent duplicates
    pub fn merge(&mut self, mut other: ChangeSet) {
        for (addr, other_change) in other.account_changes {
            let existing = self.get_or_create_change(addr);
            existing.balance_delta += other_change.balance_delta;
            existing.sequence_increment += other_change.sequence_increment;

            // Merge modules_added without duplicates
            for module in other_change.modules_added {
                existing.modules_added.insert(module);
            }
        }
        self.events.extend(other.events);
        self.treasuries.extend(other.treasuries);
        self.nft_caps.extend(other.nft_caps);

        // Consolidate token_balance_sets to prevent duplicates during merge
        for (owner, token_type, amount) in other.token_balance_sets {
            self.add_token_balance_set(owner, token_type, amount.value());
        }

        self.created_objects.extend(other.created_objects);
        self.deleted_objects.extend(other.deleted_objects);

        self.added_dynamic_fields
            .append(&mut other.added_dynamic_fields);
        self.removed_dynamic_fields
            .append(&mut other.removed_dynamic_fields);

        self.gas_used += other.gas_used;
        if !other.success {
            self.success = false;
            self.error_message = other.error_message;
        }
    }

    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn add_deleted_object(&mut self, object_id: String) {
        let canonical_id = if let Ok(addr) = AccountAddress::from_hex_literal(&object_id) {
            addr.to_hex_literal()
        } else {
            object_id
        };
        self.deleted_objects.push(canonical_id);
    }

    /// Record an NftCap creation/update for a given token type
    pub fn add_nftcap(
        &mut self,
        owner: AccountAddress,
        token_type: String,
        cap: kanari_types::collection::NftCapRecord,
    ) {
        self.nft_caps.push((owner, token_type, cap));
    }

    /// Record a treasury (TreasuryCap) creation/update for a given token type
    pub fn add_treasury(&mut self, owner: AccountAddress, token_type: String, total_supply: u64) {
        self.treasuries
            .push((owner, token_type, TreasuryCap { total_supply }));
    }

    /// Record a token balance update for an account
    /// When multiple balance updates occur for the same (owner, token_type) pair in the same changeset,
    /// consolidates them by summing amounts. This handles multiple coins of the same type
    /// being transferred to the same account in one transaction.
    pub fn add_token_balance_set(
        &mut self,
        owner: AccountAddress,
        token_type: String,
        amount: u64,
    ) {
        // Check if entry already exists for this (owner, token_type) pair
        if let Some((_, _, existing_balance)) = self
            .token_balance_sets
            .iter_mut()
            .find(|(o, t, _)| o == &owner && t == &token_type)
        {
            // Sum amounts: multiple coins of same type to same account
            let existing_amount = existing_balance.value();
            let combined_amount = existing_amount.saturating_add(amount);
            *existing_balance = BalanceRecord::new(combined_amount);
        } else {
            // Add new entry
            self.token_balance_sets
                .push((owner, token_type, BalanceRecord::new(amount)));
        }
    }

    /// Record a created object discovered in Move write-sets
    /// Uses O(n) linear search - acceptable for small object counts per transaction
    /// For high-frequency object creation, consider using HashMap-based storage
    pub fn add_created_object(
        &mut self,
        owner: AccountAddress,
        type_: String,
        data: Vec<u8>,
        version: u64,
        uid: Option<UIDRecord>,
        object_id: Option<String>,
    ) {
        let canonical_id = if let Some(id) = &object_id {
            if let Ok(addr) = AccountAddress::from_hex_literal(id) {
                addr.to_hex_literal()
            } else {
                id.clone()
            }
        } else if let Some(ref u) = uid {
            u.address().to_hex_literal()
        } else {
            let mut input = Vec::new();
            input.extend_from_slice(owner.as_ref());
            input.extend_from_slice(type_.as_bytes());
            let hash = hash_data_blake3(&input);
            format!("0x{}", hex::encode(&hash[0..32]))
        };

        if let Some((_, existing_obj)) = self
            .created_objects
            .iter_mut()
            .find(|(id, _)| id == &canonical_id)
        {
            // ป้องกัน Overwrite ชื่อ Owner ผิดพลาด
            if owner.to_hex_literal() != canonical_id {
                existing_obj.owner = owner;
            }
            existing_obj.data = data;
            existing_obj.version = version;
            existing_obj.type_ = type_;
            if let Some(u) = uid {
                existing_obj.uid = Some(u);
            }
        } else {
            self.created_objects.push((
                canonical_id,
                CreatedObject {
                    owner,
                    uid,
                    type_,
                    data,
                    version,
                },
            ));
        }
    }

    /// Compute the canonical id for an object using the same logic as
    /// `add_created_object`. Exposed so external callers can persist objects
    /// under the identical id before or after calling into the ChangeSet.
    pub fn compute_canonical_id(
        owner: &AccountAddress,
        type_: &str,
        data: &[u8],
        uid: &Option<UIDRecord>,
    ) -> String {
        if let Some(u) = uid {
            format!("{:#x}", u.address())
        } else {
            let mut input = Vec::new();
            input.extend_from_slice(owner.as_ref());
            input.extend_from_slice(type_.as_bytes());
            input.extend_from_slice(data);
            let hash = hash_data_blake3(&input);
            format!("0x{}", hex::encode(&hash[0..32]))
        }
    }

    /// แปลง ChangeSet ภายใน ให้เป็นใบเสร็จ TransactionEffects อย่างเป็นทางการ
    pub fn into_effects(self, tx_digest: TransactionDigest) -> TransactionEffects {
        // 1. จัดการสถานะ (Success / Failure)
        let status = if self.success {
            ExecutionStatus::Success
        } else {
            ExecutionStatus::Failure {
                error: self
                    .error_message
                    .unwrap_or_else(|| "Unknown Execution Error".to_string()),
            }
        };

        // 2. จัดการค่า Gas (ปัจจุบันเรารวมไว้อยู่ใน computation_cost ก่อน)
        let gas_used = GasCostSummary {
            computation_cost: self.gas_used,
            storage_cost: 0,   // TODO: คำนวณจากขนาด byte ของ created_objects ในอนาคต
            storage_rebate: 0, // TODO: คำนวณจาก deleted_objects
        };

        // 3. แมป Object ที่ถูกสร้างใหม่
        let mut created = Vec::new();
        for (id_str, obj) in self.created_objects {
            if let Ok(addr) = AccountAddress::from_hex_literal(&id_str) {
                // ทำการ Hash ตัวข้อมูล (data) เพื่อสร้าง ObjectDigest แบบสไตล์ Sui
                let hash_bytes = kanari_crypto::hash_data_blake3(&obj.data);
                let digest = ObjectDigest(Digest::from_bytes(&hash_bytes).unwrap());

                created.push(ObjectRef {
                    object_id: addr,
                    version: obj.version,
                    digest,
                });
            }
        }

        // 4. แมป Object ที่ถูกลบ (Tombstone)
        let mut deleted = Vec::new();
        for id_str in self.deleted_objects {
            if let Ok(addr) = AccountAddress::from_hex_literal(&id_str) {
                deleted.push(ObjectRef {
                    object_id: addr,
                    version: 0, // สำหรับ Object ที่ลบ อาจจะใช้ version สุดท้าย แต่ใส่ 0 ไปก่อน
                    digest: ObjectDigest(Digest([0u8; 32])), // ใช้ 0 ล้วนเป็นสัญลักษณ์การลบ
                });
            }
        }

        // 5. แมป Object ที่ถูกแก้ไข (ตอนนี้เว้นไว้ก่อน จะมาดึงจาก account_changes ภายหลัง)
        let mutated = Vec::new();

        TransactionEffects {
            transaction_digest: tx_digest,
            status,
            gas_used,
            created,
            mutated,
            deleted,
        }
    }
}

#[cfg(test)]
mod tests {
    use kanari_types::address::Address as KanariAddress;

    use super::*;

    #[test]
    fn test_changeset_transfer() {
        let mut cs = ChangeSet::new();
        let from = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();
        let to = AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS).unwrap();

        cs.transfer(from, to, 100);

        assert_eq!(cs.account_changes.len(), 2);
        assert_eq!(cs.account_changes.get(&from).unwrap().balance_delta, -100);
        assert_eq!(cs.account_changes.get(&to).unwrap().balance_delta, 100);
        assert_eq!(cs.account_changes.get(&from).unwrap().sequence_increment, 1);
    }

    #[test]
    fn test_changeset_mint() {
        let mut cs = ChangeSet::new();
        let to = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();

        cs.mint(to, 1000);

        assert_eq!(cs.account_changes.len(), 1);
        assert_eq!(cs.account_changes.get(&to).unwrap().balance_delta, 1000);
    }

    #[test]
    fn test_changeset_burn() {
        let mut cs = ChangeSet::new();
        let from = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS).unwrap();

        cs.burn(from, 500);

        assert_eq!(cs.account_changes.len(), 1);
        assert_eq!(cs.account_changes.get(&from).unwrap().balance_delta, -500);
    }

    #[test]
    fn test_changeset_module_publish() {
        let mut cs = ChangeSet::new();
        let publisher =
            AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS).unwrap();

        cs.publish_module(publisher, "kanari".to_string());

        let change = cs.account_changes.get(&publisher).unwrap();
        assert_eq!(change.modules_added.len(), 1);
        assert!(change.modules_added.contains("kanari"));
        // Note: sequence_increment is NOT set by publish_module - it's handled by engine
        assert_eq!(change.sequence_increment, 0);
    }
}
