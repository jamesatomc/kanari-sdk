// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use hex;
use kanari_crypto::hash_data_blake3;
use kanari_types::coin::TreasuryCap;
use kanari_types::object::IDRecord;
use kanari_types::object::UIDRecord;
use kanari_types::{balance::BalanceRecord, event::Event};
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::{ModuleId, StructTag};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Created object information captured from Move VM write-sets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedObject {
    pub owner: AccountAddress,
    /// Optional UIDRecord when object follows UID pattern (for ownership tracking)
    pub uid: Option<UIDRecord>,
    /// Optional IDRecord for DEX/DeFi objects that need copyable IDs
    pub id: Option<IDRecord>,
    #[serde(rename = "type")]
    pub type_: String,
    pub data: Vec<u8>,
    pub version: u64,
}

/// Represents changes to account state from Move VM execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountChange {
    pub address: AccountAddress,
    pub balance_delta: i128, // Positive = credit, Negative = debit
    pub sequence_increment: u64,
    pub modules_added: BTreeSet<String>,
}

impl AccountChange {
    fn new(address: AccountAddress) -> Self {
        Self {
            address,
            balance_delta: 0,
            sequence_increment: 0,
            modules_added: BTreeSet::new(),
        }
    }

    pub fn debit(&mut self, amount: u64) {
        self.balance_delta -= i128::from(amount);
    }

    pub fn credit(&mut self, amount: u64) {
        self.balance_delta += i128::from(amount);
    }

    pub fn increment_sequence(&mut self) {
        self.sequence_increment += 1;
    }

    fn add_module(&mut self, module_name: String) {
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
    /// Per-account token balance additions: (owner, token_type, BalanceRecord)
    pub token_balance_sets: Vec<(AccountAddress, String, BalanceRecord)>,
    /// Objects created during execution. Each entry is (object_id, CreatedObject)
    pub created_objects: Vec<(String, CreatedObject)>,
    /// Objects deleted during execution. Each entry is object_id
    pub deleted_objects: Vec<String>,
    /// (object_id, name_bytes, value_bytes)
    pub added_dynamic_fields: Vec<(String, Vec<u8>, Vec<u8>)>,
    /// (object_id, name_bytes)
    pub removed_dynamic_fields: Vec<(String, Vec<u8>)>,
    /// Canonical Move module writes captured from the VM session.
    pub module_writes: Vec<(ModuleId, Vec<u8>)>,
    /// Canonical Move resource writes. None represents deletion.
    pub resource_writes: Vec<(AccountAddress, StructTag, Option<Vec<u8>>)>,
    pub gas_used: u64,
    pub success: bool,
    pub error_message: Option<String>,
}

impl ChangeSet {
    fn with_status(gas_used: u64, success: bool, error_message: Option<String>) -> Self {
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
            module_writes: Vec::new(),
            resource_writes: Vec::new(),
            gas_used,
            success,
            error_message,
        }
    }

    pub fn new() -> Self {
        Self::with_status(0, true, None)
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
            && self.module_writes.is_empty()
            && self.resource_writes.is_empty()
            && self.gas_used == 0
            && self.success
            && self.error_message.is_none()
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
        self.module_writes.append(&mut other.module_writes);
        self.resource_writes.append(&mut other.resource_writes);

        self.gas_used = self.gas_used.saturating_add(other.gas_used);
        if !other.success {
            self.success = false;
            self.error_message = other.error_message;
        }
    }

    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn add_module_write(&mut self, module_id: ModuleId, bytes: Vec<u8>) {
        self.module_writes
            .retain(|(existing, _)| existing != &module_id);
        self.module_writes.push((module_id, bytes));
    }

    pub fn add_resource_write(
        &mut self,
        address: AccountAddress,
        tag: StructTag,
        bytes: Option<Vec<u8>>,
    ) {
        self.resource_writes
            .retain(|(existing_address, existing_tag, _)| {
                existing_address != &address || existing_tag != &tag
            });
        self.resource_writes.push((address, tag, bytes));
    }

    pub fn add_deleted_object(&mut self, object_id: String) {
        let canonical_id = Self::canonicalize_object_id(&object_id);
        self.deleted_objects.push(canonical_id);
    }

    fn canonicalize_object_id(object_id: &str) -> String {
        AccountAddress::from_hex_literal(object_id)
            .map(|addr| addr.to_hex_literal())
            .unwrap_or_else(|_| object_id.to_string())
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
        id: Option<IDRecord>,
        object_id: Option<String>,
    ) {
        let canonical_id = if let Some(id) = &object_id {
            Self::canonicalize_object_id(id)
        } else if let Some(ref u) = uid {
            u.address().to_hex_literal()
        } else if let Some(ref i) = id {
            i.address().to_hex_literal()
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
            // Prevent accidental Owner overwrite
            if owner.to_hex_literal() != canonical_id {
                existing_obj.owner = owner;
            }
            existing_obj.data = data;
            existing_obj.version = version;
            existing_obj.type_ = type_;
            if let Some(u) = uid {
                existing_obj.uid = Some(u);
            }
            if let Some(i) = id {
                existing_obj.id = Some(i);
            }
        } else {
            self.created_objects.push((
                canonical_id,
                CreatedObject {
                    owner,
                    uid,
                    id,
                    type_,
                    data,
                    version,
                },
            ));
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/changeset_tests.rs"]
mod tests;
