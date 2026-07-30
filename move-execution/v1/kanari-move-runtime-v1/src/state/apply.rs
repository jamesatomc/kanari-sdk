use super::*;
use kanari_types::transaction::ObjectOwnerKind;

impl StateManager {
    fn is_system_clock_object(&self, object_id: &str, type_name: &str) -> Result<bool> {
        if Self::is_system_clock_type(type_name).is_some_and(|is_clock| !is_clock) {
            return Ok(false);
        }
        if Self::is_system_clock_type(type_name).unwrap_or(false) {
            let object_address = AccountAddress::from_hex_literal(object_id).ok();
            let configured_clock = self.get_system_clock_object_id()?;
            return Ok(configured_clock.is_none() || configured_clock == object_address);
        }
        Ok(false)
    }

    fn is_system_clock_type(type_name: &str) -> Option<bool> {
        let Ok(struct_tag) = StructTag::from_str(type_name) else {
            return None;
        };
        Some(
            struct_tag.module.as_str() == ClockModule::CLOCK_MODULE
                && struct_tag.name.as_str() == ClockModule::CLOCK_STRUCT
                && struct_tag.type_params.is_empty(),
        )
    }

    fn is_object_locked_coin_holder_type(type_name: &str) -> bool {
        let Ok(struct_tag) = StructTag::from_str(type_name) else {
            return false;
        };

        if Self::is_balance_struct(&struct_tag) {
            return false;
        }

        let module_name = struct_tag.module.as_str();
        let struct_name = struct_tag.name.as_str();
        !(module_name == CoinModule::COIN_MODULE
            && (struct_name == CoinModule::TREASURY_CAP_STRUCT || struct_name == "CoinMetadata"))
    }

    fn supply_tracking_token_types(&self, changeset: &ChangeSet) -> BTreeSet<String> {
        let mut token_types: BTreeSet<String> =
            self.global_token_supplies.keys().cloned().collect();
        token_types.insert(GAS_COIN.to_string());

        for (_, token_type, _) in &changeset.treasuries {
            token_types.insert(Self::normalize_token_type(token_type));
        }
        for (_, token_type, _) in &changeset.token_balance_sets {
            token_types.insert(Self::normalize_token_type(token_type));
        }
        for (_, created) in &changeset.created_objects {
            if let Some((token_type, _)) = Self::balance_token_amount(&created.type_, &created.data)
            {
                token_types.insert(token_type);
            }
        }

        token_types
    }

    fn visible_supply_snapshot(&self, token_type: &str) -> Result<u64> {
        let token_type = Self::normalize_token_type(token_type);
        let cached = self
            .global_token_supplies
            .get(&token_type)
            .copied()
            .unwrap_or(0);
        Ok(cached.max(self.indexed_wallet_supply(&token_type)?))
    }

    fn capture_supply_snapshots(
        &self,
        token_types: BTreeSet<String>,
    ) -> Result<(BTreeMap<String, u64>, BTreeMap<String, u64>)> {
        let mut issued = BTreeMap::new();
        let mut visible = BTreeMap::new();

        for token_type in token_types {
            issued.insert(
                token_type.clone(),
                self.issued_supply_for_token(&token_type)?,
            );
            visible.insert(
                token_type.clone(),
                self.visible_supply_snapshot(&token_type)?,
            );
        }

        Ok((issued, visible))
    }

    fn add_locked_coin_record(
        records: &mut Vec<ObjectLockedCoinRecord>,
        holder: &(String, CreatedObject),
        token_type: &str,
        amount: u64,
    ) -> Result<()> {
        if amount == 0 {
            return Ok(());
        }

        if let Some(existing) = records
            .iter_mut()
            .find(|record| record.holder_object_id == holder.0 && record.token_type == token_type)
        {
            existing.amount = existing
                .amount
                .checked_add(amount)
                .require("Object-locked coin record overflow")?;
            existing.holder_type = holder.1.type_.clone();
            existing.owner = holder.1.owner;
            return Ok(());
        }

        records.push(ObjectLockedCoinRecord {
            holder_object_id: holder.0.clone(),
            holder_type: holder.1.type_.clone(),
            owner: holder.1.owner,
            token_type: token_type.to_string(),
            amount,
        });
        Ok(())
    }

    fn release_locked_coin_records(
        records: &mut Vec<ObjectLockedCoinRecord>,
        holder_ids: &HashSet<String>,
        token_type: &str,
        amount: u64,
    ) {
        let mut remaining = amount;

        for prefer_holder in [true, false] {
            if remaining == 0 {
                break;
            }

            for record in records.iter_mut() {
                if remaining == 0 {
                    break;
                }
                if record.token_type != token_type {
                    continue;
                }
                if prefer_holder && !holder_ids.contains(&record.holder_object_id) {
                    continue;
                }

                let release = record.amount.min(remaining);
                record.amount -= release;
                remaining -= release;
            }
        }

        records.retain(|record| record.amount > 0);
    }

    fn reconcile_object_locked_coin_records(
        &mut self,
        changeset: &ChangeSet,
        issued_before: &BTreeMap<String, u64>,
        visible_before: &BTreeMap<String, u64>,
    ) -> Result<()> {
        let holder_candidates: Vec<(String, CreatedObject)> = changeset
            .created_objects
            .iter()
            .filter(|(_, created)| Self::is_object_locked_coin_holder_type(&created.type_))
            .map(|(id, created)| (id.clone(), created.clone()))
            .collect();

        if holder_candidates.is_empty() && issued_before.is_empty() {
            return Ok(());
        }

        let holder_ids: HashSet<String> =
            holder_candidates.iter().map(|(id, _)| id.clone()).collect();
        let deleted_ids: HashSet<String> = changeset.deleted_objects.iter().cloned().collect();
        let mut records = self.load_object_locked_coin_records()?;
        let original_records = records.clone();

        if !deleted_ids.is_empty() {
            records.retain(|record| !deleted_ids.contains(&record.holder_object_id));
        }

        let token_types: BTreeSet<String> = issued_before
            .keys()
            .chain(visible_before.keys())
            .cloned()
            .collect();

        for token_type in token_types {
            let issued_before_value = issued_before.get(&token_type).copied().unwrap_or(0);
            let visible_before_value = visible_before.get(&token_type).copied().unwrap_or(0);
            let issued_after_value = self.issued_supply_for_token(&token_type)?;
            let visible_after_value = self.visible_supply_snapshot(&token_type)?;

            let issued_delta = issued_after_value as i128 - issued_before_value as i128;
            let visible_delta = visible_after_value as i128 - visible_before_value as i128;
            let locked_delta = issued_delta - visible_delta;

            if locked_delta > 0 {
                if let Some(holder) = holder_candidates.first() {
                    Self::add_locked_coin_record(
                        &mut records,
                        holder,
                        &token_type,
                        locked_delta as u64,
                    )?;
                }
            } else if locked_delta < 0 {
                Self::release_locked_coin_records(
                    &mut records,
                    &holder_ids,
                    &token_type,
                    (-locked_delta) as u64,
                );
            }
        }

        if records != original_records {
            self.save_object_locked_coin_records(&records)?;
        }

        Ok(())
    }

    /// Apply ChangeSet from Move VM execution
    /// This is the ONLY way to modify state - all changes must come from Move VM
    pub fn apply_changeset(&mut self, changeset: &ChangeSet) -> Result<()> {
        if changeset.treasuries.is_empty()
            && self
                .global_token_supplies
                .get(GAS_COIN)
                .copied()
                .unwrap_or(0)
                > self.total_supply
        {
            self.validate_supply_invariants()?;
        }
        self.apply_changeset_with_options(changeset, true)
    }

    pub fn apply_changeset_without_supply_validation(
        &mut self,
        changeset: &ChangeSet,
    ) -> Result<()> {
        self.apply_changeset_with_options(changeset, false)
    }

    fn needs_object_locked_reconciliation(&self, changeset: &ChangeSet) -> Result<bool> {
        if !changeset.treasuries.is_empty()
            || !changeset.token_balance_sets.is_empty()
            || !changeset.deleted_objects.is_empty()
        {
            return Ok(true);
        }

        changeset
            .created_objects
            .iter()
            .map(|(object_id, created)| {
                Ok(!self.is_system_clock_object(object_id, &created.type_)?
                    && (Self::is_object_locked_coin_holder_type(&created.type_)
                        || Self::treasury_cap_token_supply(&created.type_, &created.data)
                            .is_some()))
            })
            .try_fold(false, |acc, item| item.map(|value| acc || value))
    }

    fn mark_owner_for_object_balance_recompute(
        owners_to_recompute: &mut BTreeSet<AccountAddress>,
        native_object_changed_owners: &mut BTreeSet<AccountAddress>,
        non_native_object_changed_owners: &mut BTreeSet<AccountAddress>,
        owner: AccountAddress,
        token_type: &str,
    ) {
        owners_to_recompute.insert(owner);
        if Self::normalize_token_type(token_type) == GAS_COIN {
            native_object_changed_owners.insert(owner);
        } else {
            non_native_object_changed_owners.insert(owner);
        }
    }

    fn record_object_balance_owner_if_needed(
        owners_to_recompute: &mut BTreeSet<AccountAddress>,
        native_object_changed_owners: &mut BTreeSet<AccountAddress>,
        non_native_object_changed_owners: &mut BTreeSet<AccountAddress>,
        owner: AccountAddress,
        type_name: &str,
        data: &[u8],
    ) {
        if let Some((token_type, _)) = Self::balance_token_amount(type_name, data) {
            Self::mark_owner_for_object_balance_recompute(
                owners_to_recompute,
                native_object_changed_owners,
                non_native_object_changed_owners,
                owner,
                &token_type,
            );
        }
    }

    fn object_balance_token_for_type_and_data(type_name: &str, data: &[u8]) -> Option<String> {
        Self::balance_token_amount(type_name, data)
            .map(|(token_type, _)| Self::normalize_token_type(&token_type))
    }

    fn changeset_creates_object_balance_for_owner(
        changeset: &ChangeSet,
        owner: AccountAddress,
        token_type: &str,
    ) -> bool {
        changeset.created_objects.iter().any(|(_, created)| {
            created.owner == owner
                && Self::object_balance_token_for_type_and_data(&created.type_, &created.data)
                    .is_some_and(|created_token| created_token == token_type)
        })
    }

    fn changeset_deletes_object_balance_for_owner(
        &self,
        changeset: &ChangeSet,
        owner: AccountAddress,
        token_type: &str,
    ) -> Result<bool> {
        for object_id in &changeset.deleted_objects {
            let Some((_, existing)) = self.load_stored_object_by_any_id(object_id)? else {
                continue;
            };
            if existing.owner != owner {
                continue;
            }
            if Self::object_balance_token_for_type_and_data(&existing.type_name, &existing.data)
                .is_some_and(|existing_token| existing_token == token_type)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn owner_has_object_backed_token_balance(
        &self,
        owner: AccountAddress,
        token_type: &str,
    ) -> Result<bool> {
        Ok(self
            .compute_owned_token_balances(owner, None)?
            .contains_key(token_type))
    }

    fn changeset_touches_object_backed_token_for_owner(
        &self,
        changeset: &ChangeSet,
        owner: AccountAddress,
        token_type: &str,
    ) -> Result<bool> {
        Ok(
            self.owner_has_object_backed_token_balance(owner, token_type)?
                || Self::changeset_creates_object_balance_for_owner(changeset, owner, token_type)
                || self.changeset_deletes_object_balance_for_owner(changeset, owner, token_type)?,
        )
    }

    fn persist_treasury_cap_state(
        &mut self,
        token_type: &str,
        owner: AccountAddress,
        total_supply: u64,
    ) -> Result<bool> {
        let key = Self::supply_key(token_type);
        self.save_internal(&key, &TreasuryCap { total_supply })?;

        let mut key_owner = b"treasury:".to_vec();
        key_owner.extend_from_slice(token_type.as_bytes());
        self.save_internal(&key_owner, &owner)?;
        self.add_to_index_list(b"treasury_index", format!("treasury:{}", token_type))?;

        if Self::normalize_token_type(token_type) == GAS_COIN {
            self.save_native_total_supply(total_supply)?;
        }

        Ok(true)
    }

    fn canonical_owned_object_id(object_id: &str) -> String {
        canonical_object_id(object_id).unwrap_or_else(|| object_id.to_string())
    }

    fn remove_owned_object_index_variants(
        &mut self,
        owner: AccountAddress,
        object_id: &str,
        stored_id: Option<&str>,
    ) -> Result<()> {
        let owner_key = owned_objects_key(&owner);
        self.remove_from_index_list(&owner_key, object_id)?;

        if let Some(stored_id) = stored_id
            && stored_id != object_id
        {
            self.remove_from_index_list(&owner_key, stored_id)?;
        }

        let canonical_id = Self::canonical_owned_object_id(object_id);
        if canonical_id != object_id {
            self.remove_from_index_list(&owner_key, &canonical_id)?;
        }
        if let Some(stored_id) = stored_id
            && canonical_id != stored_id
        {
            self.remove_from_index_list(&owner_key, &canonical_id)?;
        }

        Ok(())
    }

    fn refresh_owned_object_index(
        &mut self,
        owner: AccountAddress,
        object_id: &str,
        stored_id: Option<&str>,
    ) -> Result<()> {
        self.remove_owned_object_index_variants(owner, object_id, stored_id)?;
        self.add_to_index_list(
            &owned_objects_key(&owner),
            Self::canonical_owned_object_id(object_id),
        )
    }

    fn add_many_owned_object_index(
        &mut self,
        owner: AccountAddress,
        object_ids: Vec<String>,
    ) -> Result<()> {
        self.add_many_to_index_list(&owned_objects_key(&owner), object_ids)
    }

    fn apply_changeset_with_options(
        &mut self,
        changeset: &ChangeSet,
        validate_supply: bool,
    ) -> Result<()> {
        let profile_apply = matches!(
            std::env::var("KANARI_STATE_APPLY_PROFILE").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES")
        );
        let apply_started_at = std::time::Instant::now();
        if validate_supply {
            // Validate on a cloned snapshot so rejected transactions cannot poison live state.
            let mut candidate = self.clone();
            candidate.apply_changeset_with_options(changeset, false)?;
            candidate.repair_legacy_native_wallet_overcount()?;
            if let Err(error) = candidate.validate_cached_supply_invariants() {
                Self::report_supply_invariant_violation("after apply_changeset", &error)?;
                return Err(error);
            }
            *self = candidate;
            return Ok(());
        }

        let supply_delta = changeset
            .owner_deltas
            .values()
            .try_fold(0i128, |total, change| {
                total
                    .checked_add(change.balance_delta)
                    .require("Native supply delta overflow")
            })?;
        let next_total_supply = if supply_delta > 0 {
            let mint_amount = u64::try_from(supply_delta)
                .require("Native supply delta overflowed u64 total supply")?;
            Some(
                self.total_supply
                    .checked_add(mint_amount)
                    .require("Native total supply overflow")?,
            )
        } else if supply_delta < 0 {
            let burn_amount = u64::try_from(supply_delta.unsigned_abs())
                .require("Native supply delta overflowed u64 total supply")?;
            ensure!(
                self.total_supply >= burn_amount,
                "Native total supply underflow: tried to burn {} from {}",
                burn_amount,
                self.total_supply
            );
            Some(self.total_supply - burn_amount)
        } else {
            None
        };

        // Capture native balances once before mutating the state. Debit validation and
        // post-object recomputation both need this snapshot; independently resolving the
        // same owner here used to scan every owned Coin object twice before the write pass.
        let mut native_snapshot_owners = changeset
            .owner_deltas
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        for (_, created) in &changeset.created_objects {
            if Self::object_balance_token_for_type_and_data(&created.type_, &created.data)
                .is_some_and(|token_type| token_type == GAS_COIN)
            {
                native_snapshot_owners.insert(created.owner);
            }
        }
        for object_id in &changeset.deleted_objects {
            if let Some((_, existing)) = self.load_stored_object_by_any_id(object_id)?
                && Self::object_balance_token_for_type_and_data(&existing.type_name, &existing.data)
                    .is_some_and(|token_type| token_type == GAS_COIN)
            {
                native_snapshot_owners.insert(existing.owner);
            }
        }
        let mut native_balances_before = BTreeMap::new();
        for owner in native_snapshot_owners {
            let ledger_balance = self
                .load_owner_state(&owner)?
                .map(|state| state.native_balance())
                .unwrap_or(0);
            let native_debit = changeset
                .owner_deltas
                .get(&owner)
                .map(|change| change.balance_delta)
                .filter(|delta| *delta < 0)
                .map(|delta| {
                    u64::try_from(delta.unsigned_abs())
                        .require("Native debit overflowed u64 owner balance")
                })
                .transpose()?
                .unwrap_or(0);
            let object_balance = if native_debit > 0 && ledger_balance >= native_debit {
                0
            } else {
                self.compute_owned_token_balances(owner, None)?
                    .get(GAS_COIN)
                    .copied()
                    .unwrap_or(0)
            };
            native_balances_before.insert(owner, (ledger_balance, object_balance));
        }
        let snapshots_at = std::time::Instant::now();

        for (address, change) in &changeset.owner_deltas {
            if change.balance_delta >= 0 {
                continue;
            }
            let debit = u64::try_from(change.balance_delta.unsigned_abs())
                .require("Native debit overflowed u64 owner balance")?;
            let (ledger_balance, object_balance) = native_balances_before
                .get(address)
                .copied()
                .unwrap_or((0, 0));
            let balance = if ledger_balance > 0 {
                ledger_balance
            } else {
                object_balance
            };
            ensure!(
                balance >= debit,
                "Insufficient native balance for {}: need {}, have {}",
                address.to_hex_literal(),
                debit,
                balance
            );
        }

        let mut supplies_dirty = false;
        let mut owner_index_additions = Vec::with_capacity(changeset.owner_deltas.len());
        let reconcile_object_locked = self.needs_object_locked_reconciliation(changeset)?;
        let (issued_before, visible_before) = if reconcile_object_locked {
            self.capture_supply_snapshots(self.supply_tracking_token_types(changeset))?
        } else {
            (BTreeMap::new(), BTreeMap::new())
        };
        let mut owners_to_recompute: BTreeSet<AccountAddress> = BTreeSet::new();
        let mut native_object_changed_owners: BTreeSet<AccountAddress> = BTreeSet::new();
        let mut non_native_object_changed_owners: BTreeSet<AccountAddress> = BTreeSet::new();
        let mut native_object_owner_changed_owners: BTreeSet<AccountAddress> = BTreeSet::new();
        let mut native_object_created_owners: BTreeSet<AccountAddress> = BTreeSet::new();
        let mut native_object_gas_adjusted: BTreeMap<AccountAddress, u64> = BTreeMap::new();
        let mut native_object_before_from_changes: BTreeMap<AccountAddress, u64> = BTreeMap::new();
        let mut native_object_after_from_changes: BTreeMap<AccountAddress, u64> = BTreeMap::new();
        let mut native_balance_after_owner_deltas: BTreeMap<AccountAddress, u64> = BTreeMap::new();
        let mut object_owner_index_additions = Vec::with_capacity(
            changeset
                .created_objects
                .len()
                .saturating_add(changeset.gas_object_refs.len()),
        );
        let mut object_index_additions = Vec::with_capacity(
            changeset
                .created_objects
                .len()
                .saturating_add(changeset.gas_object_refs.len()),
        );
        let mut owned_object_index_additions: BTreeMap<AccountAddress, Vec<String>> =
            BTreeMap::new();
        let gas_object_ids = changeset
            .gas_object_refs
            .iter()
            .map(|gas_ref| Self::canonical_owned_object_id(&gas_ref.object_id))
            .collect::<BTreeSet<_>>();
        let created_object_owners_by_id = changeset
            .created_objects
            .iter()
            .map(|(object_id, created)| (Self::canonical_owned_object_id(object_id), created.owner))
            .collect::<BTreeMap<_, _>>();

        for (address, change) in &changeset.owner_deltas {
            let mut owner_state = self.load_owner_state_or_default(*address)?;
            let old_balances = owner_state.token_balances.clone();
            let native_token = GAS_COIN.to_string();

            if change.balance_delta > 0 {
                let amount = u64::try_from(change.balance_delta)
                    .require("Native credit overflowed u64 owner balance")?;
                let next = owner_state
                    .native_balance()
                    .checked_add(amount)
                    .require("Native owner balance overflow")?;
                owner_state.set_token_balance_value(&native_token, next);
            } else if change.balance_delta < 0 {
                let debit = u64::try_from(change.balance_delta.unsigned_abs())
                    .require("Native debit overflowed u64 owner balance")?;
                let next = owner_state.native_balance() - debit;
                owner_state.set_token_balance_value(&native_token, next);
            }
            native_balance_after_owner_deltas
                .insert(owner_state.address, owner_state.native_balance());
            for module_name in &change.modules_added {
                owner_state.add_module(module_name.clone());
            }
            if owner_state.is_empty() {
                self.overlay
                    .insert(Self::owner_state_key(&owner_state.address), None);
                self.remove_from_index_list(
                    OWNER_INDEX_KEY,
                    &owner_state.address.to_hex_literal(),
                )?;
            } else {
                self.save_owner_record(&owner_state)?;
                owner_index_additions.push(owner_state.address.to_hex_literal());
            }
            supplies_dirty |= self.capture_supply_changed(&owner_state, &old_balances)?;
        }

        self.add_many_to_index_list(OWNER_INDEX_KEY, owner_index_additions)?;
        let owner_deltas_at = std::time::Instant::now();

        // Update total supply if there was mint/burn (supply_delta != 0)
        if let Some(next_total_supply) = next_total_supply {
            self.save_native_total_supply(next_total_supply)?;
        }

        // Apply treasury creations/updates
        for (owner, token_type, total_supply) in &changeset.treasuries {
            let key = Self::supply_key(token_type);
            self.save_internal(&key, total_supply)?;

            let mut key_owner = b"treasury:".to_vec();
            key_owner.extend_from_slice(token_type.as_bytes());
            self.save_internal(&key_owner, owner)?;

            self.add_to_index_list(b"treasury_index", format!("treasury:{}", token_type))?;

            if Self::normalize_token_type(token_type) == GAS_COIN {
                self.save_native_total_supply(total_supply.total_supply)?;
            }
        }

        // Apply NFT capability creations/updates
        for (_owner, token_type, nft_cap) in &changeset.nft_caps {
            let mut key = b"nft:".to_vec();
            key.extend_from_slice(token_type.as_bytes());
            self.save_internal(&key, nft_cap)?;
        }

        // Record Global Token Supplies to database only once after all processing
        // Treat token balance hints as recompute triggers, not as canonical balance writes.
        // Non-native balances must come from owned objects so queries always reflect object
        // inventory instead of a parallel per-owner cache.
        for (owner, token_type, amount) in &changeset.token_balance_sets {
            let normalized_token_type = Self::normalize_token_type(token_type);
            if normalized_token_type == GAS_COIN {
                // Native KANARI balance is applied from owner deltas so gas and
                // transfers remain exact. Object-derived hints must not double count it.
                continue;
            }

            let _ = amount;
            let object_backed = self.changeset_touches_object_backed_token_for_owner(
                changeset,
                *owner,
                &normalized_token_type,
            )?;
            if object_backed {
                owners_to_recompute.insert(*owner);
            }
        }

        for obj_id in &changeset.deleted_objects {
            if let Some((stored_id, existing)) = self.load_stored_object_by_any_id(obj_id)? {
                let obj_key = object_key(&stored_id);
                Self::record_object_balance_owner_if_needed(
                    &mut owners_to_recompute,
                    &mut native_object_changed_owners,
                    &mut non_native_object_changed_owners,
                    existing.owner,
                    &existing.type_name,
                    &existing.data,
                );
                if Self::object_balance_token_for_type_and_data(&existing.type_name, &existing.data)
                    .is_some_and(|token_type| token_type == GAS_COIN)
                {
                    if let Some((_, amount)) =
                        Self::balance_token_amount(&existing.type_name, &existing.data)
                    {
                        let owner_balance = native_object_before_from_changes
                            .entry(existing.owner)
                            .or_insert(0);
                        *owner_balance = owner_balance
                            .checked_add(amount)
                            .require("Native object balance snapshot overflow")?;
                        native_object_after_from_changes
                            .entry(existing.owner)
                            .or_insert(0);
                    }
                    native_object_owner_changed_owners.insert(existing.owner);
                }
                if matches!(existing.owner_kind, ObjectOwnerKind::AddressOwner(_)) {
                    self.remove_owned_object_index_variants(
                        existing.owner,
                        obj_id,
                        Some(&stored_id),
                    )?;
                }
                self.overlay.insert(obj_key, None);
                self.remove_from_index_list(b"object_index", &stored_id)?;
                self.remove_from_index_list(
                    b"object_index",
                    &Self::canonical_owned_object_id(obj_id),
                )?;
            } else {
                let obj_key = object_key(obj_id);
                self.overlay.insert(obj_key, None);
                self.remove_from_index_list(
                    b"object_index",
                    &Self::canonical_owned_object_id(obj_id),
                )?;
            }
        }

        // 1. Check for newly created Objects to index Collections
        for (obj_id, created) in &changeset.created_objects {
            // If this is a Collection type Object, record it in the global index
            if created.type_.contains("::collection::Collection") {
                self.add_to_index_list(b"nft_collection_index", obj_id.clone())?;
            }
        }

        // 2. Check Events to index NFT <-> Collection relationships
        for event in &changeset.events {
            // Check if this is a MintLog from james::nft
            if event.type_tag.to_string().contains("::nft::MintLog") {
                // MintLog data in nft.move contains: object_id(32), creator(32), collection_id(32)
                if event.event_data.len() >= 96 {
                    let nft_id_bytes = &event.event_data[0..32];
                    let coll_id_bytes = &event.event_data[64..96];

                    let (Ok(nft_id), Ok(coll_id)) = (
                        AccountAddress::from_bytes(nft_id_bytes).map(|addr| addr.to_hex_literal()),
                        AccountAddress::from_bytes(coll_id_bytes).map(|addr| addr.to_hex_literal()),
                    ) else {
                        log::warn!(
                            "Skipping malformed MintLog object ids while indexing collection members"
                        );
                        continue;
                    };

                    // Record in Collection member index (O(1) Access)
                    let key = metadata_key(b"collection_members:", &coll_id);
                    self.add_to_index_list(&key, nft_id)?;
                }
            }
        }

        for (obj_id, created) in &changeset.created_objects {
            let mut new_obj = created.clone();
            let existing_obj = self.load_stored_object_by_any_id(obj_id)?;
            let existing_stored_id = existing_obj
                .as_ref()
                .map(|(stored_id, _)| stored_id.clone());
            let existing_owner = existing_obj.as_ref().map(|(_, existing)| existing.owner);
            let canonical_obj_id = Self::canonical_owned_object_id(obj_id);
            let obj_key = object_key(&canonical_obj_id);

            if let Some((stored_id, existing)) = existing_obj {
                // Canonical object versions must be derived from the visible state snapshot,
                // not the runtime object cache, so every authority commits the same version.
                // Replaying the consensus clock prologue for the same timestamp is idempotent.
                // Some checkpoint paths can observe the same prologue more than once; allowing
                // an identical replay to bump only the version would split state roots even
                // though the canonical clock value is unchanged.
                let identical_clock_replay = self
                    .is_system_clock_object(obj_id, &existing.type_name)?
                    && existing.type_name == new_obj.type_
                    && existing.data == new_obj.data
                    && existing.owner == new_obj.owner
                    && existing.owner_kind == new_obj.owner_kind;
                new_obj.version = if identical_clock_replay {
                    existing.version
                } else {
                    existing.version + 1
                };
                if new_obj.owner.to_hex_literal() == *obj_id {
                    new_obj.owner = existing.owner;
                }
                let existing_balance =
                    Self::balance_token_amount(&existing.type_name, &existing.data);
                let existing_native_coin =
                    existing_balance.as_ref().is_some_and(|(token_type, _)| {
                        Self::normalize_token_type(token_type) == GAS_COIN
                    });
                if let Some((token_type, amount)) = existing_balance
                    && Self::normalize_token_type(&token_type) == GAS_COIN
                {
                    let owner_balance = native_object_before_from_changes
                        .entry(existing.owner)
                        .or_insert(0);
                    *owner_balance = owner_balance
                        .checked_add(amount)
                        .require("Native object balance snapshot overflow")?;
                }
                let is_designated_gas_object = gas_object_ids.contains(&canonical_obj_id);
                // Older/internal ChangeSets do not carry an explicit gas object. In that
                // case, charge a native coin already being written by the transaction so
                // object-backed balance reconciliation cannot erase the owner debit.
                // Explicit gas references always take precedence when they are present.
                let is_implicit_gas_object = changeset.gas_object_refs.is_empty()
                    && changeset
                        .owner_deltas
                        .get(&existing.owner)
                        .is_some_and(|change| change.balance_delta < 0);
                if existing_native_coin && (is_designated_gas_object || is_implicit_gas_object) {
                    let sender_native_debit: u64 = changeset
                        .owner_deltas
                        .get(&existing.owner)
                        .map(|change| change.balance_delta)
                        .filter(|delta| *delta < 0)
                        .map(|delta| {
                            u64::try_from(delta.unsigned_abs())
                                .require("Native debit overflowed u64 object gas adjustment")
                        })
                        .transpose()?
                        .unwrap_or(0);
                    let already_adjusted = native_object_gas_adjusted
                        .get(&existing.owner)
                        .copied()
                        .unwrap_or(0);
                    let remaining_debit = sender_native_debit.saturating_sub(already_adjusted);
                    if remaining_debit > 0 {
                        let new_struct_tag = StructTag::from_str(&new_obj.type_)
                            .context("Invalid transferred native coin object type")?;
                        let object_amount =
                            Self::extract_balance_from_object_bytes(&new_obj.data, &new_struct_tag)
                                .context("Invalid transferred native coin object data")?;
                        ensure!(
                            object_amount >= remaining_debit,
                            "Insufficient native coin object balance for gas: balance={}, debit={}",
                            object_amount,
                            remaining_debit
                        );
                        let next_amount = object_amount - remaining_debit;
                        ensure!(
                            Self::write_balance_to_object_bytes(
                                &mut new_obj.data,
                                &new_struct_tag,
                                next_amount,
                            ),
                            "Failed to write adjusted native coin object balance"
                        );
                        native_object_gas_adjusted.insert(existing.owner, sender_native_debit);
                    }
                }

                if existing.owner != new_obj.owner
                    && matches!(existing.owner_kind, ObjectOwnerKind::AddressOwner(_))
                {
                    if existing_native_coin {
                        native_object_owner_changed_owners.insert(existing.owner);
                        native_object_owner_changed_owners.insert(new_obj.owner);
                    }
                    self.remove_owned_object_index_variants(
                        existing.owner,
                        obj_id,
                        Some(&stored_id),
                    )?;
                }
                if stored_id != canonical_obj_id {
                    self.overlay.insert(object_key(&stored_id), None);
                }
                Self::record_object_balance_owner_if_needed(
                    &mut owners_to_recompute,
                    &mut native_object_changed_owners,
                    &mut non_native_object_changed_owners,
                    existing.owner,
                    &existing.type_name,
                    &existing.data,
                );
            } else {
                // For new objects, use version from ChangeSet or default to 1
                if new_obj.version == 0 {
                    new_obj.version = 1;
                }
            }
            Self::record_object_balance_owner_if_needed(
                &mut owners_to_recompute,
                &mut native_object_changed_owners,
                &mut non_native_object_changed_owners,
                new_obj.owner,
                &new_obj.type_,
                &new_obj.data,
            );
            if let Some((token_type, amount)) =
                Self::balance_token_amount(&new_obj.type_, &new_obj.data)
                && Self::normalize_token_type(&token_type) == GAS_COIN
            {
                if existing_stored_id.is_none() {
                    native_object_created_owners.insert(new_obj.owner);
                }
                let owner_balance = native_object_after_from_changes
                    .entry(new_obj.owner)
                    .or_insert(0);
                *owner_balance = owner_balance
                    .checked_add(amount)
                    .require("Native object balance snapshot overflow")?;
            }
            if let Some((token_type, total_supply)) =
                Self::treasury_cap_token_supply(&new_obj.type_, &new_obj.data)
            {
                supplies_dirty |=
                    self.persist_treasury_cap_state(&token_type, new_obj.owner, total_supply)?;
            }

            let stored_obj = StoredObject {
                id: canonical_obj_id.clone(),
                owner: new_obj.owner,
                owner_kind: new_obj.owner_kind.clone(),
                type_name: new_obj.type_.clone(),
                data: new_obj.data.clone(),
                version: new_obj.version,
            };
            self.save_internal(&obj_key, &stored_obj)?;
            object_index_additions.push(canonical_obj_id.clone());

            if matches!(new_obj.owner_kind, ObjectOwnerKind::AddressOwner(_)) {
                object_owner_index_additions.push(new_obj.owner.to_hex_literal());
                if existing_stored_id.is_none() {
                    owned_object_index_additions
                        .entry(new_obj.owner)
                        .or_default()
                        .push(canonical_obj_id.clone());
                } else if existing_stored_id.as_deref() != Some(canonical_obj_id.as_str())
                    || existing_owner.is_some_and(|owner| owner != new_obj.owner)
                {
                    self.refresh_owned_object_index(
                        new_obj.owner,
                        obj_id,
                        existing_stored_id.as_deref(),
                    )?;
                }
            }

            if new_obj.type_.contains("::coin::CoinMetadata<")
                && let Some(start) = new_obj.type_.find('<')
                && let Some(end) = new_obj.type_.rfind('>')
            {
                let token_type = &new_obj.type_[start + 1..end];
                self.persist_coin_metadata(token_type, &new_obj.data)?;
            }
        }
        let created_objects_at = std::time::Instant::now();

        // A separate gas coin is normally not passed into the Move function, so
        // it will not appear in `created_objects`. Debit the explicitly declared
        // gas object here instead of charging whichever native coin happened to
        // be mutated first by the transfer. In particular, a transfer coin whose
        // entire balance was split has amount zero and must never receive gas.
        for gas_ref in &changeset.gas_object_refs {
            let canonical_gas_id = Self::canonical_owned_object_id(&gas_ref.object_id);
            if let Some(owner) = created_object_owners_by_id.get(&canonical_gas_id) {
                let sender_native_debit = changeset
                    .owner_deltas
                    .get(owner)
                    .map(|change| change.balance_delta)
                    .filter(|delta| *delta < 0)
                    .map(|delta| {
                        u64::try_from(delta.unsigned_abs())
                            .require("Native debit overflowed u64 gas object adjustment")
                    })
                    .transpose()?
                    .unwrap_or(0);
                let already_adjusted = native_object_gas_adjusted.get(owner).copied().unwrap_or(0);
                if sender_native_debit > 0 && already_adjusted >= sender_native_debit {
                    continue;
                }
            }
            let Some((stored_id, existing)) =
                self.load_stored_object_by_any_id(&gas_ref.object_id)?
            else {
                continue;
            };
            let sender_native_debit = changeset
                .owner_deltas
                .get(&existing.owner)
                .map(|change| change.balance_delta)
                .filter(|delta| *delta < 0)
                .map(|delta| {
                    u64::try_from(delta.unsigned_abs())
                        .require("Native debit overflowed u64 gas object adjustment")
                })
                .transpose()?
                .unwrap_or(0);
            let already_adjusted = native_object_gas_adjusted
                .get(&existing.owner)
                .copied()
                .unwrap_or(0);
            let remaining_debit = sender_native_debit.saturating_sub(already_adjusted);
            if remaining_debit == 0 {
                continue;
            }
            let Some((token_type, object_amount)) =
                Self::balance_token_amount(&existing.type_name, &existing.data)
            else {
                continue;
            };
            if Self::normalize_token_type(&token_type) != GAS_COIN {
                continue;
            }
            ensure!(
                object_amount >= remaining_debit,
                "Insufficient native gas coin balance: balance={}, debit={}",
                object_amount,
                remaining_debit
            );
            let struct_tag = StructTag::from_str(&existing.type_name)
                .context("Invalid native gas coin object type")?;
            let mut next_data = existing.data.clone();
            ensure!(
                Self::write_balance_to_object_bytes(
                    &mut next_data,
                    &struct_tag,
                    object_amount - remaining_debit,
                ),
                "Failed to write adjusted native gas coin balance"
            );
            let next_object = StoredObject {
                id: canonical_gas_id.clone(),
                owner: existing.owner,
                owner_kind: existing.owner_kind.clone(),
                type_name: existing.type_name.clone(),
                data: next_data,
                version: existing.version.saturating_add(1),
            };
            self.save_internal(&object_key(&canonical_gas_id), &next_object)?;
            if stored_id != canonical_gas_id {
                self.overlay.insert(object_key(&stored_id), None);
            }
            object_index_additions.push(canonical_gas_id.clone());
            if matches!(next_object.owner_kind, ObjectOwnerKind::AddressOwner(_)) {
                object_owner_index_additions.push(next_object.owner.to_hex_literal());
                self.refresh_owned_object_index(
                    next_object.owner,
                    &gas_ref.object_id,
                    Some(&stored_id),
                )?;
            }
            Self::record_object_balance_owner_if_needed(
                &mut owners_to_recompute,
                &mut native_object_changed_owners,
                &mut non_native_object_changed_owners,
                next_object.owner,
                &next_object.type_name,
                &next_object.data,
            );
            if let Some((token_type, amount)) =
                Self::balance_token_amount(&next_object.type_name, &next_object.data)
                && Self::normalize_token_type(&token_type) == GAS_COIN
            {
                let owner_balance = native_object_after_from_changes
                    .entry(next_object.owner)
                    .or_insert(0);
                *owner_balance = owner_balance
                    .checked_add(amount)
                    .require("Native object balance snapshot overflow")?;
            }
            native_object_gas_adjusted.insert(existing.owner, sender_native_debit);
        }
        let gas_objects_at = std::time::Instant::now();

        for (owner, object_ids) in owned_object_index_additions {
            self.add_many_owned_object_index(owner, object_ids)?;
        }
        self.add_many_to_index_list(b"object_index", object_index_additions)?;
        self.add_many_to_index_list(OWNER_INDEX_KEY, object_owner_index_additions)?;
        let indexes_at = std::time::Instant::now();

        for owner in owners_to_recompute {
            let native_delta = changeset
                .owner_deltas
                .get(&owner)
                .map(|change| change.balance_delta)
                .unwrap_or(0i128);
            let native_object_changed = native_object_changed_owners.contains(&owner);
            let adjusted_gas = native_object_gas_adjusted.get(&owner).copied().unwrap_or(0);
            let native_gas_credit = changeset
                .native_gas_credits
                .get(&owner)
                .copied()
                .unwrap_or(0);
            let (native_ledger_before, native_object_before) = native_balances_before
                .get(&owner)
                .copied()
                .unwrap_or((0, 0));
            let native_object_before = if native_object_before > 0 {
                native_object_before
            } else {
                native_object_before_from_changes
                    .get(&owner)
                    .copied()
                    .unwrap_or(0)
            };
            if (native_delta <= 0 || native_gas_credit == 0)
                && native_object_changed
                && !non_native_object_changed_owners.contains(&owner)
                && !native_object_owner_changed_owners.contains(&owner)
                && !native_object_created_owners.contains(&owner)
                && let Some(native_object_after) = native_object_after_from_changes.get(&owner)
                && let Some(current_native_balance) =
                    native_balance_after_owner_deltas.get(&owner).copied()
                && current_native_balance > 0
            {
                let prior_non_object_balance =
                    native_ledger_before.saturating_sub(native_object_before);
                let object_backed_with_ledger = native_object_after
                    .checked_add(prior_non_object_balance)
                    .and_then(|amount| amount.checked_add(native_gas_credit))
                    .require("Native owner balance overflow during object reconciliation")?;
                let expected_native_balance = if native_delta < 0 {
                    object_backed_with_ledger.min(current_native_balance)
                } else {
                    object_backed_with_ledger
                };
                if expected_native_balance == current_native_balance {
                    continue;
                }
            }
            let changed = self.recompute_token_balances_for_owner(
                owner,
                native_delta,
                native_object_changed,
                adjusted_gas,
                native_gas_credit,
                native_ledger_before,
                native_object_before,
            )?;
            if changed {
                supplies_dirty = true;
            }
        }
        let recompute_at = std::time::Instant::now();

        if reconcile_object_locked {
            self.reconcile_object_locked_coin_records(changeset, &issued_before, &visible_before)?;
        }

        if supplies_dirty {
            let supplies_clone = self.global_token_supplies.clone();
            self.save_internal(b"global_token_supplies", &supplies_clone)?;
        }

        // Move modules/resources are part of the same canonical overlay as objects and
        // balances. Keeping them here prevents speculative VM execution from mutating the
        // shared database before the checkpoint is validated and committed.
        for (key, value) in &changeset.move_writes {
            match value {
                Some(bytes) => self.save_internal(key, bytes)?,
                None => {
                    self.overlay.insert(key.clone(), None);
                }
            }

            if key.starts_with(b"module:") {
                let module_key =
                    String::from_utf8(key.clone()).context("Move module key is not valid UTF-8")?;
                if value.is_some() {
                    self.add_to_index_list(b"module_index", module_key)?;
                } else {
                    self.remove_from_index_list(b"module_index", &module_key)?;
                }
            }
        }
        let move_writes_at = std::time::Instant::now();

        // =====================================================================
        // Process Dynamic Fields into State Overlay.
        // =====================================================================
        for (object_id, name_bytes, value_bytes) in &changeset.added_dynamic_fields {
            let df_key = Self::dynamic_field_key(object_id, name_bytes);
            self.save_internal(&df_key, value_bytes)?;
        }

        for (object_id, name_bytes) in &changeset.removed_dynamic_fields {
            let df_key = Self::dynamic_field_key(object_id, name_bytes);
            // Record as None so commit() will delete it from RocksDB
            self.overlay.insert(df_key, None);
        }
        let dynamic_fields_at = std::time::Instant::now();

        self.advance_access_versions(&changeset.deterministic_access_set())?;
        let access_versions_at = std::time::Instant::now();
        if profile_apply {
            eprintln!(
                "state apply profile: owners={} created={} gas_refs={} move_writes={} snapshots={:.6}s owner_deltas={:.6}s created_objects={:.6}s gas_objects={:.6}s indexes={:.6}s recompute={:.6}s move_writes={:.6}s dynamic={:.6}s access_versions={:.6}s total={:.6}s",
                changeset.owner_deltas.len(),
                changeset.created_objects.len(),
                changeset.gas_object_refs.len(),
                changeset.move_writes.len(),
                snapshots_at.duration_since(apply_started_at).as_secs_f64(),
                owner_deltas_at.duration_since(snapshots_at).as_secs_f64(),
                created_objects_at
                    .duration_since(owner_deltas_at)
                    .as_secs_f64(),
                gas_objects_at
                    .duration_since(created_objects_at)
                    .as_secs_f64(),
                indexes_at.duration_since(gas_objects_at).as_secs_f64(),
                recompute_at.duration_since(indexes_at).as_secs_f64(),
                move_writes_at.duration_since(recompute_at).as_secs_f64(),
                dynamic_fields_at
                    .duration_since(move_writes_at)
                    .as_secs_f64(),
                access_versions_at
                    .duration_since(dynamic_fields_at)
                    .as_secs_f64(),
                access_versions_at
                    .duration_since(apply_started_at)
                    .as_secs_f64(),
            );
        }

        Ok(())
    }

    pub fn try_apply_owned_native_burn_batch_without_supply_validation(
        &mut self,
        changesets: &[ChangeSet],
    ) -> Result<bool> {
        if changesets.is_empty() {
            return Ok(true);
        }

        let mut owner_deltas: BTreeMap<AccountAddress, i128> = BTreeMap::new();
        let mut object_updates = Vec::with_capacity(changesets.len());
        let mut access = StateAccessSet::default();
        let mut supply_delta = 0i128;

        for changeset in changesets {
            if !changeset.success
                || !changeset.events.is_empty()
                || !changeset.treasuries.is_empty()
                || !changeset.nft_caps.is_empty()
                || !changeset.token_balance_sets.is_empty()
                || !changeset.shared_inputs.is_empty()
                || !changeset.immutable_inputs.is_empty()
                || changeset.gas_payment.is_none()
                || changeset.gas_object_refs.len() != 1
                || changeset.created_objects.len() != 1
                || !changeset.deleted_objects.is_empty()
                || !changeset.added_dynamic_fields.is_empty()
                || !changeset.removed_dynamic_fields.is_empty()
                || !changeset.move_writes.is_empty()
                || !changeset.resolver_reads.is_empty()
            {
                return Ok(false);
            }

            let gas_ref = &changeset.gas_object_refs[0];
            let (object_id, created) = &changeset.created_objects[0];
            let canonical_object_id = Self::canonical_owned_object_id(object_id);
            if Self::canonical_owned_object_id(&gas_ref.object_id) != canonical_object_id {
                return Ok(false);
            }
            if !Self::balance_token_amount(&created.type_, &created.data)
                .is_some_and(|(token_type, _)| Self::normalize_token_type(&token_type) == GAS_COIN)
            {
                return Ok(false);
            }

            let Some(payment) = &changeset.gas_payment else {
                return Ok(false);
            };
            let payment_owner = AccountAddress::from_hex_literal(&payment.owner)
                .context("Native owned batch gas payment owner is invalid")?;
            if payment_owner != created.owner {
                return Ok(false);
            }

            let sender_delta = changeset
                .owner_deltas
                .get(&created.owner)
                .map(|delta| delta.balance_delta)
                .unwrap_or(0);
            if sender_delta >= 0 {
                return Ok(false);
            }
            let sender_debit = u64::try_from(sender_delta.unsigned_abs())
                .context("Native owned batch sender debit overflowed u64")?;
            let gas_credit_total =
                changeset
                    .native_gas_credits
                    .values()
                    .try_fold(0u64, |acc, value| {
                        acc.checked_add(*value)
                            .context("Native gas credit overflow in owned batch")
                    })?;
            if gas_credit_total > sender_debit {
                return Ok(false);
            }

            let (token_type, object_balance) =
                Self::balance_token_amount(&created.type_, &created.data)
                    .context("Native owned batch coin object has invalid balance bytes")?;
            if Self::normalize_token_type(&token_type) != GAS_COIN || object_balance < sender_debit
            {
                return Ok(false);
            }

            let mut next_data = created.data.clone();
            let struct_tag = StructTag::from_str(&created.type_)
                .context("Native owned batch coin object type is invalid")?;
            if !Self::write_balance_to_object_bytes(
                &mut next_data,
                &struct_tag,
                object_balance - sender_debit,
            ) {
                return Ok(false);
            }

            for (owner, delta) in &changeset.owner_deltas {
                if !delta.modules_added.is_empty() {
                    return Ok(false);
                }
                let entry = owner_deltas.entry(*owner).or_insert(0);
                *entry = entry
                    .checked_add(delta.balance_delta)
                    .context("Native owned batch owner delta overflow")?;
                supply_delta = supply_delta
                    .checked_add(delta.balance_delta)
                    .context("Native owned batch supply delta overflow")?;
            }

            object_updates.push(StoredObject {
                id: canonical_object_id.clone(),
                owner: created.owner,
                owner_kind: created.owner_kind.clone(),
                type_name: created.type_.clone(),
                data: next_data,
                version: created.version.saturating_add(1),
            });
            access
                .writes
                .insert(format!("object:{canonical_object_id}").into_bytes());
        }

        if supply_delta > 0 {
            return Ok(false);
        }
        if supply_delta < 0 {
            let burn_amount = u64::try_from(supply_delta.unsigned_abs())
                .context("Native owned batch burn overflowed u64")?;
            ensure!(
                self.total_supply >= burn_amount,
                "Native total supply underflow: tried to burn {} from {}",
                burn_amount,
                self.total_supply
            );
            self.save_native_total_supply(self.total_supply - burn_amount)?;
            let current_visible = self
                .global_token_supplies
                .get(GAS_COIN)
                .copied()
                .unwrap_or(0);
            ensure!(
                current_visible >= burn_amount,
                "Native visible supply underflow in owned batch: visible={}, burn={}",
                current_visible,
                burn_amount
            );
            let next_visible = current_visible - burn_amount;
            if next_visible == 0 {
                self.global_token_supplies.remove(GAS_COIN);
            } else {
                self.global_token_supplies
                    .insert(GAS_COIN.to_string(), next_visible);
            }
            let supplies_clone = self.global_token_supplies.clone();
            self.save_internal(b"global_token_supplies", &supplies_clone)?;
        }

        let mut owner_index_additions = Vec::with_capacity(owner_deltas.len());
        for (owner, delta) in owner_deltas {
            let mut owner_state = self.load_owner_state_or_default(owner)?;
            let current = owner_state.native_balance();
            let next = if delta >= 0 {
                current
                    .checked_add(u64::try_from(delta).context("Native credit overflowed u64")?)
                    .context("Native owner balance overflow")?
            } else {
                let debit =
                    u64::try_from(delta.unsigned_abs()).context("Native debit overflowed u64")?;
                ensure!(
                    current >= debit,
                    "Insufficient native balance for {}: need {}, have {}",
                    owner.to_hex_literal(),
                    debit,
                    current
                );
                current - debit
            };
            owner_state.set_token_balance_value(GAS_COIN, next);
            if owner_state.is_empty() {
                self.overlay.insert(Self::owner_state_key(&owner), None);
                self.remove_from_index_list(OWNER_INDEX_KEY, &owner.to_hex_literal())?;
            } else {
                self.save_owner_record(&owner_state)?;
                owner_index_additions.push(owner.to_hex_literal());
            }
        }

        for object in object_updates {
            self.save_internal(&object_key(&object.id), &object)?;
        }
        self.add_many_to_index_list(OWNER_INDEX_KEY, owner_index_additions)?;
        self.advance_access_versions(&access)?;
        Ok(true)
    }
}
