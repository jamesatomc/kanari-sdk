// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use hex;
use kanari_crypto::hash_data_blake3;
use kanari_types::coin::TreasuryCap;
use kanari_types::object::IDRecord;
use kanari_types::object::UIDRecord;
use kanari_types::transaction::{
    GasPayment, ObjectChange, ObjectChangeKind, ObjectGraphEdge, ObjectGraphEdgeKind, ObjectInput,
    ObjectOwnerKind, ObjectRef, TransactionEffects,
};
use kanari_types::{balance::BalanceRecord, event::Event};
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Created object information captured from Move VM write-sets
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatedObject {
    pub owner: AccountAddress,
    pub owner_kind: ObjectOwnerKind,
    /// Optional UIDRecord when object follows UID pattern (for ownership tracking)
    pub uid: Option<UIDRecord>,
    /// Optional IDRecord for DEX/DeFi objects that need copyable IDs
    pub id: Option<IDRecord>,
    #[serde(rename = "type")]
    pub type_: String,
    pub data: Vec<u8>,
    pub version: u64,
}

impl CreatedObject {
    pub fn owner_kind(&self) -> ObjectOwnerKind {
        self.owner_kind.clone()
    }

    pub fn digest(&self) -> String {
        format!("0x{}", hex::encode(hash_data_blake3(&self.data)))
    }

    pub fn object_ref(&self, object_id: &str) -> ObjectRef {
        ObjectRef::new(
            object_id.to_string(),
            Some(self.version),
            Some(self.digest()),
        )
    }
}

/// Represents owner-state deltas from Move VM execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerDelta {
    pub address: AccountAddress,
    /// Positive = credit, negative = debit. i128 prevents lossy u64 -> i64 casts.
    pub balance_delta: i128,
    pub modules_added: BTreeSet<String>,
}

/// Canonical state keys observed or modified by one transaction execution.
///
/// Keys are deterministic byte strings rather than storage pointers so every
/// validator derives exactly the same conflict decision. The manifest is
/// intentionally conservative: an unclassified write is represented by a
/// dedicated key, which serializes it rather than risking a false negative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateAccessSet {
    pub reads: BTreeSet<Vec<u8>>,
    pub writes: BTreeSet<Vec<u8>>,
}

impl StateAccessSet {
    pub fn conflicts_with(&self, other: &Self) -> bool {
        self.writes
            .iter()
            .any(|key| other.reads.contains(key) || other.writes.contains(key))
            || other.writes.iter().any(|key| self.reads.contains(key))
    }

    fn read(&mut self, key: impl Into<Vec<u8>>) {
        self.reads.insert(key.into());
    }

    fn write(&mut self, key: impl Into<Vec<u8>>) {
        self.writes.insert(key.into());
    }
}

impl OwnerDelta {
    fn new(address: AccountAddress) -> Self {
        Self {
            address,
            balance_delta: 0,
            modules_added: BTreeSet::new(),
        }
    }

    pub fn debit(&mut self, amount: u64) {
        self.balance_delta -= amount as i128;
    }

    pub fn credit(&mut self, amount: u64) {
        self.balance_delta += amount as i128;
    }

    fn add_module(&mut self, module_name: String) {
        self.modules_added.insert(module_name);
    }
}

/// ChangeSet represents all state changes from Move VM execution.
/// This is the canonical output from Move VM that StateManager will apply.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChangeSet {
    pub owner_deltas: BTreeMap<AccountAddress, OwnerDelta>,
    /// Native gas credits keyed by collector. This is tracked separately from
    /// owner deltas because object-backed balance recomputation must add gas
    /// credits without double-counting transfer or mint deltas already
    /// represented by Coin objects.
    #[serde(default)]
    pub native_gas_credits: BTreeMap<AccountAddress, u64>,
    pub events: Vec<Event>,
    /// Treasury creations or updates: (owner, token_type, TreasuryCap)
    pub treasuries: Vec<(AccountAddress, String, TreasuryCap)>,
    /// NFT capability creations or updates: (owner, token_type, NftCapRecord)
    pub nft_caps: Vec<(
        AccountAddress,
        String,
        kanari_types::collection::NftCapRecord,
    )>,
    /// Per-owner token balances (absolute set): (owner, token_type, BalanceRecord)
    pub token_balance_sets: Vec<(AccountAddress, String, BalanceRecord)>,
    pub input_objects: Vec<ObjectInput>,
    pub shared_inputs: Vec<ObjectRef>,
    pub immutable_inputs: Vec<ObjectRef>,
    pub gas_payment: Option<GasPayment>,
    pub gas_object_refs: Vec<ObjectRef>,
    /// Objects created during execution. Each entry is (object_id, CreatedObject)
    pub created_objects: Vec<(String, CreatedObject)>,
    /// Objects deleted during execution. Each entry is object_id
    pub deleted_objects: Vec<String>,
    pub explicit_object_changes: Vec<ObjectChange>,
    /// (object_id, name_bytes, value_bytes)
    pub added_dynamic_fields: Vec<(String, Vec<u8>, Vec<u8>)>,
    /// (object_id, name_bytes)
    pub removed_dynamic_fields: Vec<(String, Vec<u8>)>,
    /// Canonical Move module/resource operations keyed exactly as the shared store.
    /// Some(bytes) is create/modify and None is delete.
    pub move_writes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    /// Canonical storage keys read by the Move resolver during this execution.
    /// This is execution metadata and is intentionally not persisted as chain state.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub resolver_reads: BTreeSet<Vec<u8>>,
    pub gas_used: u64,
    pub success: bool,
    pub error_message: Option<String>,
}

impl ChangeSet {
    /// Produce a deterministic, conservative access manifest from canonical VM
    /// effects. Resolver read tracing is added separately; this manifest already
    /// covers every object, owner, resource, dynamic field and gas write that
    /// reaches StateManager.
    pub fn deterministic_access_set(&self) -> StateAccessSet {
        let mut access = StateAccessSet::default();

        access.reads.extend(self.resolver_reads.iter().cloned());
        // Dynamic-field natives use a separate resolver extension. Until that
        // extension exposes its precise read key, serialize only transactions
        // that write a dynamic field against this conservative read fence.
        access.read(b"df:*".to_vec());

        for input in &self.input_objects {
            access.read(format!(
                "object:{}",
                Self::canonicalize_object_id(&input.object_ref.object_id)
            ));
        }
        for input in &self.shared_inputs {
            access.read(format!(
                "object:{}",
                Self::canonicalize_object_id(&input.object_id)
            ));
        }
        for input in &self.immutable_inputs {
            access.read(format!(
                "object:{}",
                Self::canonicalize_object_id(&input.object_id)
            ));
        }
        for input in &self.gas_object_refs {
            access.write(format!(
                "object:{}",
                Self::canonicalize_object_id(&input.object_id)
            ));
        }
        if let Some(payment) = &self.gas_payment {
            for input in &payment.payment_objects {
                access.write(format!(
                    "object:{}",
                    Self::canonicalize_object_id(&input.object_id)
                ));
            }
        }
        for (owner, delta) in &self.owner_deltas {
            if self.is_parallel_safe_native_owner_delta(owner, delta) {
                continue;
            }
            access.write(format!("owner:{}", owner.to_hex_literal()));
        }
        for (_, token_type, _) in &self.treasuries {
            access.write(format!("supply:{token_type}"));
            access.write(format!("treasury:{token_type}"));
            access.write(b"global_token_supplies".to_vec());
        }
        for (_, token_type, _) in &self.nft_caps {
            access.write(format!("nft:{token_type}"));
        }
        for (owner, token_type, _) in &self.token_balance_sets {
            access.write(format!(
                "token_balance:{}:{}",
                owner.to_hex_literal(),
                token_type
            ));
        }
        for (object_id, _) in &self.created_objects {
            access.write(format!(
                "object:{}",
                Self::canonicalize_object_id(object_id)
            ));
        }
        for object_id in &self.deleted_objects {
            access.write(format!(
                "object:{}",
                Self::canonicalize_object_id(object_id)
            ));
        }
        for change in &self.explicit_object_changes {
            access.write(format!(
                "object:{}",
                Self::canonicalize_object_id(&change.object_ref.object_id)
            ));
        }
        for (object_id, name, _) in &self.added_dynamic_fields {
            access.write(b"df:*".to_vec());
            access.write(format!(
                "dynamic:{}:{}",
                Self::canonicalize_object_id(object_id),
                hex::encode(name)
            ));
        }
        for (object_id, name) in &self.removed_dynamic_fields {
            access.write(b"df:*".to_vec());
            access.write(format!(
                "dynamic:{}:{}",
                Self::canonicalize_object_id(object_id),
                hex::encode(name)
            ));
        }
        for key in self.move_writes.keys() {
            access.write(key.clone());
        }
        for event in &self.events {
            if event.type_tag.to_string().contains("::nft::MintLog") && event.event_data.len() >= 96
            {
                access.write(format!(
                    "collection_members:0x{}",
                    hex::encode(&event.event_data[64..96])
                ));
            }
        }
        access
    }

    fn is_parallel_safe_native_owner_delta(
        &self,
        owner: &AccountAddress,
        delta: &OwnerDelta,
    ) -> bool {
        if !delta.modules_added.is_empty() {
            return false;
        }
        if delta.balance_delta >= 0 {
            return false;
        }

        let Some(payment) = &self.gas_payment else {
            return false;
        };
        if payment.payment_objects.is_empty() {
            return false;
        }
        let Ok(payment_owner) = AccountAddress::from_hex_literal(&payment.owner) else {
            return false;
        };
        payment_owner == *owner
    }

    pub fn record_resolver_reads<I>(&mut self, reads: I)
    where
        I: IntoIterator<Item = Vec<u8>>,
    {
        self.resolver_reads.extend(reads);
    }

    fn input_edge_relation(change_type: &ObjectChangeKind) -> ObjectGraphEdgeKind {
        match change_type {
            ObjectChangeKind::Created => ObjectGraphEdgeKind::InputCreate,
            ObjectChangeKind::Mutated => ObjectGraphEdgeKind::InputMutate,
            ObjectChangeKind::Deleted => ObjectGraphEdgeKind::InputDelete,
            ObjectChangeKind::Transferred => ObjectGraphEdgeKind::InputTransfer,
        }
    }

    fn shared_input_edge_relation(change_type: &ObjectChangeKind) -> ObjectGraphEdgeKind {
        match change_type {
            ObjectChangeKind::Created => ObjectGraphEdgeKind::SharedInputCreate,
            ObjectChangeKind::Mutated => ObjectGraphEdgeKind::SharedInputMutate,
            ObjectChangeKind::Deleted => ObjectGraphEdgeKind::SharedInputDelete,
            ObjectChangeKind::Transferred => ObjectGraphEdgeKind::SharedInputTransfer,
        }
    }

    fn immutable_input_edge_relation(change_type: &ObjectChangeKind) -> ObjectGraphEdgeKind {
        match change_type {
            ObjectChangeKind::Created => ObjectGraphEdgeKind::ImmutableInputCreate,
            ObjectChangeKind::Mutated => ObjectGraphEdgeKind::ImmutableInputMutate,
            ObjectChangeKind::Deleted => ObjectGraphEdgeKind::ImmutableInputDelete,
            ObjectChangeKind::Transferred => ObjectGraphEdgeKind::ImmutableInputTransfer,
        }
    }

    fn gas_edge_relation(change_type: &ObjectChangeKind) -> ObjectGraphEdgeKind {
        match change_type {
            ObjectChangeKind::Created => ObjectGraphEdgeKind::GasCreate,
            ObjectChangeKind::Mutated => ObjectGraphEdgeKind::GasMutate,
            ObjectChangeKind::Deleted => ObjectGraphEdgeKind::GasDelete,
            ObjectChangeKind::Transferred => ObjectGraphEdgeKind::GasTransfer,
        }
    }

    fn with_status(gas_used: u64, success: bool, error_message: Option<String>) -> Self {
        Self {
            owner_deltas: BTreeMap::new(),
            native_gas_credits: BTreeMap::new(),
            events: Vec::new(),
            treasuries: Vec::new(),
            nft_caps: Vec::new(),
            token_balance_sets: Vec::new(),
            input_objects: Vec::new(),
            shared_inputs: Vec::new(),
            immutable_inputs: Vec::new(),
            gas_payment: None,
            gas_object_refs: Vec::new(),
            created_objects: Vec::new(),
            deleted_objects: Vec::new(),
            explicit_object_changes: Vec::new(),
            added_dynamic_fields: Vec::new(),
            removed_dynamic_fields: Vec::new(),
            move_writes: BTreeMap::new(),
            resolver_reads: BTreeSet::new(),
            gas_used,
            success,
            error_message,
        }
    }

    pub fn new() -> Self {
        Self::with_status(0, true, None)
    }

    pub fn get_or_create_owner_delta(&mut self, address: AccountAddress) -> &mut OwnerDelta {
        self.owner_deltas
            .entry(address)
            .or_insert_with(|| OwnerDelta::new(address))
    }

    /// Transfer operation: debit sender, credit receiver
    pub fn transfer(&mut self, from: AccountAddress, to: AccountAddress, amount: u64) {
        let sender = self.get_or_create_owner_delta(from);
        sender.debit(amount);

        let receiver = self.get_or_create_owner_delta(to);
        receiver.credit(amount);
    }

    /// Mint operation: create new tokens
    pub fn mint(&mut self, to: AccountAddress, amount: u64) {
        self.get_or_create_owner_delta(to).credit(amount);
    }

    /// Burn operation: destroy tokens
    pub fn burn(&mut self, from: AccountAddress, amount: u64) {
        self.get_or_create_owner_delta(from).debit(amount);
    }

    /// Module publish operation. Sequence handling remains in the engine layer.
    pub fn publish_module(&mut self, publisher: AccountAddress, module_name: String) {
        self.get_or_create_owner_delta(publisher)
            .add_module(module_name);
    }

    /// Collect gas fees to DAO
    pub fn collect_gas(&mut self, dao_address: AccountAddress, gas_amount: u64) {
        self.get_or_create_owner_delta(dao_address)
            .credit(gas_amount);
        let collected = self.native_gas_credits.entry(dao_address).or_insert(0);
        *collected = collected.saturating_add(gas_amount);
    }

    pub fn set_gas_used(&mut self, gas: u64) {
        self.gas_used = gas;
    }

    pub fn mark_failed(&mut self, error: String) {
        self.success = false;
        self.error_message = Some(error);
    }

    pub fn is_empty(&self) -> bool {
        self.owner_deltas.is_empty()
            && self.native_gas_credits.is_empty()
            && self.events.is_empty()
            && self.treasuries.is_empty()
            && self.token_balance_sets.is_empty()
            && self.input_objects.is_empty()
            && self.shared_inputs.is_empty()
            && self.immutable_inputs.is_empty()
            && self.gas_payment.is_none()
            && self.gas_object_refs.is_empty()
            && self.created_objects.is_empty()
            && self.deleted_objects.is_empty()
            && self.explicit_object_changes.is_empty()
            && self.added_dynamic_fields.is_empty()
            && self.removed_dynamic_fields.is_empty()
            && self.move_writes.is_empty()
            && self.gas_used == 0
            && self.success
            && self.error_message.is_none()
    }

    /// Merge another ChangeSet into this one. Later Move writes replace earlier writes
    /// for the same canonical key, matching serial transaction execution semantics.
    pub fn merge(&mut self, mut other: ChangeSet) {
        for (addr, other_change) in other.owner_deltas {
            let existing = self.get_or_create_owner_delta(addr);
            existing.balance_delta = existing
                .balance_delta
                .saturating_add(other_change.balance_delta);
            existing.modules_added.extend(other_change.modules_added);
        }
        for (collector, amount) in other.native_gas_credits {
            let collected = self.native_gas_credits.entry(collector).or_insert(0);
            *collected = collected.saturating_add(amount);
        }
        self.events.extend(other.events);
        self.treasuries.extend(other.treasuries);
        self.nft_caps.extend(other.nft_caps);
        self.input_objects.append(&mut other.input_objects);
        self.shared_inputs.append(&mut other.shared_inputs);
        self.immutable_inputs.append(&mut other.immutable_inputs);
        if self.gas_payment.is_none() {
            self.gas_payment = other.gas_payment.take();
        }
        self.gas_object_refs.append(&mut other.gas_object_refs);

        for (owner, token_type, amount) in other.token_balance_sets {
            self.add_token_balance_set(owner, token_type, amount.value());
        }

        self.created_objects.extend(other.created_objects);
        self.deleted_objects.extend(other.deleted_objects);
        self.explicit_object_changes
            .append(&mut other.explicit_object_changes);
        self.added_dynamic_fields
            .append(&mut other.added_dynamic_fields);
        self.removed_dynamic_fields
            .append(&mut other.removed_dynamic_fields);
        for (key, value) in other.move_writes {
            self.move_writes.insert(key, value);
        }
        self.resolver_reads.append(&mut other.resolver_reads);

        self.gas_used = self.gas_used.saturating_add(other.gas_used);
        if !other.success {
            self.success = false;
            self.error_message = other.error_message;
        }
    }

    /// Merge another ChangeSet by reference.
    ///
    /// This keeps high-throughput checkpoint preparation from cloning an entire
    /// vector of transaction effects before the merged batch is actually built.
    /// Later Move writes still replace earlier writes for the same canonical key,
    /// matching `merge`.
    pub fn merge_from(&mut self, other: &ChangeSet) {
        self.events.reserve(other.events.len());
        self.treasuries.reserve(other.treasuries.len());
        self.nft_caps.reserve(other.nft_caps.len());
        self.input_objects.reserve(other.input_objects.len());
        self.shared_inputs.reserve(other.shared_inputs.len());
        self.immutable_inputs.reserve(other.immutable_inputs.len());
        self.gas_object_refs.reserve(other.gas_object_refs.len());
        self.created_objects.reserve(other.created_objects.len());
        self.deleted_objects.reserve(other.deleted_objects.len());
        self.explicit_object_changes
            .reserve(other.explicit_object_changes.len());
        self.added_dynamic_fields
            .reserve(other.added_dynamic_fields.len());
        self.removed_dynamic_fields
            .reserve(other.removed_dynamic_fields.len());

        for (addr, other_change) in &other.owner_deltas {
            let existing = self.get_or_create_owner_delta(*addr);
            existing.balance_delta = existing
                .balance_delta
                .saturating_add(other_change.balance_delta);
            existing
                .modules_added
                .extend(other_change.modules_added.iter().cloned());
        }
        for (collector, amount) in &other.native_gas_credits {
            let collected = self.native_gas_credits.entry(*collector).or_insert(0);
            *collected = collected.saturating_add(*amount);
        }
        self.events.extend(other.events.iter().cloned());
        self.treasuries.extend(other.treasuries.iter().cloned());
        self.nft_caps.extend(other.nft_caps.iter().cloned());
        self.input_objects
            .extend(other.input_objects.iter().cloned());
        self.shared_inputs
            .extend(other.shared_inputs.iter().cloned());
        self.immutable_inputs
            .extend(other.immutable_inputs.iter().cloned());
        if self.gas_payment.is_none() {
            self.gas_payment = other.gas_payment.clone();
        }
        self.gas_object_refs
            .extend(other.gas_object_refs.iter().cloned());

        for (owner, token_type, amount) in &other.token_balance_sets {
            self.add_token_balance_set(*owner, token_type.clone(), amount.value());
        }

        self.created_objects
            .extend(other.created_objects.iter().cloned());
        self.deleted_objects
            .extend(other.deleted_objects.iter().cloned());
        self.explicit_object_changes
            .extend(other.explicit_object_changes.iter().cloned());
        self.added_dynamic_fields
            .extend(other.added_dynamic_fields.iter().cloned());
        self.removed_dynamic_fields
            .extend(other.removed_dynamic_fields.iter().cloned());
        for (key, value) in &other.move_writes {
            self.move_writes.insert(key.clone(), value.clone());
        }
        self.resolver_reads
            .extend(other.resolver_reads.iter().cloned());

        self.gas_used = self.gas_used.saturating_add(other.gas_used);
        if !other.success {
            self.success = false;
            self.error_message = other.error_message.clone();
        }
    }

    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn record_move_write(&mut self, key: Vec<u8>, value: Option<Vec<u8>>) {
        self.move_writes.insert(key, value);
    }

    pub fn add_deleted_object(&mut self, object_id: String) {
        self.deleted_objects
            .push(Self::canonicalize_object_id(&object_id));
    }

    fn canonicalize_object_id(object_id: &str) -> String {
        AccountAddress::from_hex_literal(object_id)
            .map(|addr| addr.to_hex_literal())
            .unwrap_or_else(|_| object_id.to_string())
    }

    pub fn add_treasury(&mut self, owner: AccountAddress, token_type: String, total_supply: u64) {
        self.treasuries
            .push((owner, token_type, TreasuryCap { total_supply }));
    }

    /// Record an absolute token balance for a given account/token pair. Multiple coin
    /// fragments observed in one VM session are summed to one absolute owner total.
    pub fn add_token_balance_set(
        &mut self,
        owner: AccountAddress,
        token_type: String,
        amount: u64,
    ) {
        if let Some((_, _, existing_balance)) = self
            .token_balance_sets
            .iter_mut()
            .find(|(o, t, _)| o == &owner && t == &token_type)
        {
            *existing_balance = BalanceRecord::new(existing_balance.value().saturating_add(amount));
        } else {
            self.token_balance_sets
                .push((owner, token_type, BalanceRecord::new(amount)));
        }
    }

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
                    owner_kind: ObjectOwnerKind::AddressOwner(owner.to_hex_literal()),
                    type_,
                    data,
                    version,
                },
            ));
        }
    }

    pub fn object_changes(&self) -> Vec<ObjectChange> {
        if !self.explicit_object_changes.is_empty() {
            return self.explicit_object_changes.clone();
        }
        let mut changes = Vec::new();

        for (object_id, created) in &self.created_objects {
            let change_type = if created.version <= 1 {
                ObjectChangeKind::Created
            } else {
                ObjectChangeKind::Mutated
            };

            changes.push(ObjectChange {
                change_type: change_type.clone(),
                object_ref: created.object_ref(object_id),
                previous_object_ref: match change_type {
                    ObjectChangeKind::Created => None,
                    _ => Some(ObjectRef::new(
                        object_id.clone(),
                        created.version.checked_sub(1),
                        None,
                    )),
                },
                type_: Some(created.type_.clone()),
                owner: Some(created.owner_kind()),
                previous_owner: None,
                previous_version: match change_type {
                    ObjectChangeKind::Created => None,
                    _ => created.version.checked_sub(1),
                },
            });
        }

        for object_id in &self.deleted_objects {
            changes.push(ObjectChange {
                change_type: ObjectChangeKind::Deleted,
                object_ref: ObjectRef::new(object_id.clone(), None, None),
                previous_object_ref: None,
                type_: None,
                owner: None,
                previous_owner: None,
                previous_version: None,
            });
        }

        changes
    }

    pub fn effects(&self, gas_payment: Option<GasPayment>) -> TransactionEffects {
        let object_changes = self.object_changes();
        let mut created = Vec::new();
        let mut mutated = Vec::new();
        let mut deleted = Vec::new();
        let mut transferred = Vec::new();
        let mut causal_edges = Vec::new();
        let input_refs = self
            .input_objects
            .iter()
            .map(|input| input.object_ref.clone())
            .collect::<Vec<_>>();

        for change in &object_changes {
            match change.change_type {
                ObjectChangeKind::Created => created.push(change.clone()),
                ObjectChangeKind::Mutated => mutated.push(change.clone()),
                ObjectChangeKind::Deleted => deleted.push(change.clone()),
                ObjectChangeKind::Transferred => transferred.push(change.clone()),
            }

            if let Some(previous_object_ref) = &change.previous_object_ref {
                causal_edges.push(ObjectGraphEdge {
                    source_object_ref: previous_object_ref.clone(),
                    target_object_ref: change.object_ref.clone(),
                    relation: if matches!(change.change_type, ObjectChangeKind::Deleted) {
                        ObjectGraphEdgeKind::Delete
                    } else {
                        ObjectGraphEdgeKind::VersionSuccessor
                    },
                });

                if matches!(change.change_type, ObjectChangeKind::Transferred)
                    && change.previous_owner != change.owner
                {
                    causal_edges.push(ObjectGraphEdge {
                        source_object_ref: previous_object_ref.clone(),
                        target_object_ref: change.object_ref.clone(),
                        relation: ObjectGraphEdgeKind::OwnershipTransfer,
                    });
                }
            }

            for input in &self.input_objects {
                causal_edges.push(ObjectGraphEdge {
                    source_object_ref: input.object_ref.clone(),
                    target_object_ref: change.object_ref.clone(),
                    relation: Self::input_edge_relation(&change.change_type),
                });
            }
            for input in &self.shared_inputs {
                causal_edges.push(ObjectGraphEdge {
                    source_object_ref: input.clone(),
                    target_object_ref: change.object_ref.clone(),
                    relation: Self::shared_input_edge_relation(&change.change_type),
                });
            }
            for input in &self.immutable_inputs {
                causal_edges.push(ObjectGraphEdge {
                    source_object_ref: input.clone(),
                    target_object_ref: change.object_ref.clone(),
                    relation: Self::immutable_input_edge_relation(&change.change_type),
                });
            }
            for input in &self.gas_object_refs {
                causal_edges.push(ObjectGraphEdge {
                    source_object_ref: input.clone(),
                    target_object_ref: change.object_ref.clone(),
                    relation: Self::gas_edge_relation(&change.change_type),
                });
                if matches!(change.change_type, ObjectChangeKind::Created) {
                    causal_edges.push(ObjectGraphEdge {
                        source_object_ref: input.clone(),
                        target_object_ref: change.object_ref.clone(),
                        relation: ObjectGraphEdgeKind::CallContextCreate,
                    });
                }
            }
        }

        for gas_ref in &self.gas_object_refs {
            for input_ref in &input_refs {
                causal_edges.push(ObjectGraphEdge {
                    source_object_ref: gas_ref.clone(),
                    target_object_ref: input_ref.clone(),
                    relation: ObjectGraphEdgeKind::GasMutate,
                });
            }
            for input_ref in &self.shared_inputs {
                causal_edges.push(ObjectGraphEdge {
                    source_object_ref: gas_ref.clone(),
                    target_object_ref: input_ref.clone(),
                    relation: ObjectGraphEdgeKind::GasMutate,
                });
            }
            for input_ref in &self.immutable_inputs {
                causal_edges.push(ObjectGraphEdge {
                    source_object_ref: gas_ref.clone(),
                    target_object_ref: input_ref.clone(),
                    relation: ObjectGraphEdgeKind::GasMutate,
                });
            }
        }
        causal_edges.sort_by(|a, b| {
            (
                a.source_object_ref.object_id.as_str(),
                a.target_object_ref.object_id.as_str(),
                format!("{:?}", a.relation),
            )
                .cmp(&(
                    b.source_object_ref.object_id.as_str(),
                    b.target_object_ref.object_id.as_str(),
                    format!("{:?}", b.relation),
                ))
        });
        causal_edges.dedup();

        TransactionEffects {
            status: if self.success {
                "success".to_string()
            } else {
                "failed".to_string()
            },
            gas_used: self.gas_used,
            gas_payment: gas_payment.or_else(|| self.gas_payment.clone()),
            input_objects: self
                .input_objects
                .iter()
                .map(|input| input.object_ref.clone())
                .collect(),
            shared_inputs: self.shared_inputs.clone(),
            immutable_inputs: self.immutable_inputs.clone(),
            gas_object_refs: self.gas_object_refs.clone(),
            object_changes,
            created,
            mutated,
            deleted,
            transferred,
            causal_edges,
            error_message: self.error_message.clone(),
        }
    }

    pub fn set_transaction_context(
        &mut self,
        object_inputs: Vec<ObjectInput>,
        gas_payment: Option<GasPayment>,
    ) {
        self.input_objects = object_inputs.clone();
        self.shared_inputs = object_inputs
            .iter()
            .filter(|input| matches!(input.owner, Some(ObjectOwnerKind::Shared)))
            .map(|input| input.object_ref.clone())
            .collect();
        self.immutable_inputs = object_inputs
            .iter()
            .filter(|input| matches!(input.owner, Some(ObjectOwnerKind::Immutable)))
            .map(|input| input.object_ref.clone())
            .collect();
        self.gas_object_refs = gas_payment
            .as_ref()
            .map(|payment| payment.payment_objects.clone())
            .unwrap_or_default();
        self.gas_payment = gas_payment;
    }

    pub fn set_explicit_object_changes(&mut self, object_changes: Vec<ObjectChange>) {
        self.explicit_object_changes = object_changes;
    }
}

#[cfg(test)]
#[path = "../tests/unit/changeset_tests.rs"]
mod tests;
