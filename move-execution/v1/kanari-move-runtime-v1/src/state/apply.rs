use super::*;

impl StateManager {
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
        token_types.insert(KANARI_TOKEN_TYPE.to_string());

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
                self.issued_supply_for_token(&token_type),
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
    ) {
        if amount == 0 {
            return;
        }

        if let Some(existing) = records
            .iter_mut()
            .find(|record| record.holder_object_id == holder.0 && record.token_type == token_type)
        {
            existing.amount = existing.amount.saturating_add(amount);
            existing.holder_type = holder.1.type_.clone();
            existing.owner = holder.1.owner;
            return;
        }

        records.push(ObjectLockedCoinRecord {
            holder_object_id: holder.0.clone(),
            holder_type: holder.1.type_.clone(),
            owner: holder.1.owner,
            token_type: token_type.to_string(),
            amount,
        });
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
            let issued_after_value = self.issued_supply_for_token(&token_type);
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
                    );
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
        self.apply_changeset_with_options(changeset, true)
    }

    pub fn apply_changeset_without_supply_validation(
        &mut self,
        changeset: &ChangeSet,
    ) -> Result<()> {
        self.apply_changeset_with_options(changeset, false)
    }

    fn needs_object_locked_reconciliation(changeset: &ChangeSet) -> bool {
        !changeset.treasuries.is_empty()
            || !changeset.token_balance_sets.is_empty()
            || !changeset.created_objects.is_empty()
            || !changeset.deleted_objects.is_empty()
    }

    fn should_repair_legacy_native_overcount(changeset: &ChangeSet) -> bool {
        changeset
            .treasuries
            .iter()
            .any(|(_, token_type, _)| Self::normalize_token_type(token_type) != KANARI_TOKEN_TYPE)
            || changeset
                .token_balance_sets
                .iter()
                .any(|(_, token_type, _)| {
                    Self::normalize_token_type(token_type) != KANARI_TOKEN_TYPE
                })
            || changeset.created_objects.iter().any(|(_, created)| {
                Self::balance_token_amount(&created.type_, &created.data)
                    .is_some_and(|(token_type, _)| token_type != KANARI_TOKEN_TYPE)
                    || Self::treasury_cap_token_supply(&created.type_, &created.data)
                        .is_some_and(|(token_type, _)| token_type != KANARI_TOKEN_TYPE)
                    || (created.type_.contains("::coin::CoinMetadata<")
                        && created
                            .type_
                            .split('<')
                            .nth(1)
                            .and_then(|inner| inner.strip_suffix('>'))
                            .map(Self::normalize_token_type)
                            .is_some_and(|token_type| token_type != KANARI_TOKEN_TYPE))
            })
    }

    fn mark_owner_for_object_balance_recompute(
        owners_to_recompute: &mut BTreeSet<AccountAddress>,
        native_object_changed_owners: &mut BTreeSet<AccountAddress>,
        owner: AccountAddress,
        token_type: &str,
    ) {
        owners_to_recompute.insert(owner);
        if token_type == KANARI_TOKEN_TYPE {
            native_object_changed_owners.insert(owner);
        }
    }

    fn record_object_balance_owner_if_needed(
        owners_to_recompute: &mut BTreeSet<AccountAddress>,
        native_object_changed_owners: &mut BTreeSet<AccountAddress>,
        owner: AccountAddress,
        type_name: &str,
        data: &[u8],
    ) {
        if let Some((token_type, _)) = Self::balance_token_amount(type_name, data) {
            Self::mark_owner_for_object_balance_recompute(
                owners_to_recompute,
                native_object_changed_owners,
                owner,
                &token_type,
            );
        }
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

        if token_type == KANARI_TOKEN_TYPE {
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

    fn apply_changeset_with_options(
        &mut self,
        changeset: &ChangeSet,
        validate_supply: bool,
    ) -> Result<()> {
        if validate_supply {
            // Validate on a cloned snapshot so rejected transactions cannot poison live state.
            let mut candidate = self.clone();
            candidate.apply_changeset_with_options(changeset, false)?;
            if Self::should_repair_legacy_native_overcount(changeset) {
                candidate.repair_legacy_native_wallet_overcount()?;
            }
            candidate.sync_native_visible_supply_cache()?;
            if let Err(error) = candidate.validate_supply_invariants() {
                Self::report_supply_invariant_violation("after apply_changeset", &error)?;
                return Err(error);
            }
            *self = candidate;
            return Ok(());
        }

        let supply_delta = changeset
            .account_changes
            .values()
            .try_fold(0i128, |total, change| {
                total
                    .checked_add(change.balance_delta)
                    .require("Native supply delta overflow")
            })?;
        let next_total_supply = if supply_delta > 0 {
            let mint_amount = u64::try_from(supply_delta)
                .expect("Native supply delta overflowed u64 total supply");
            Some(
                self.total_supply
                    .checked_add(mint_amount)
                    .require("Native total supply overflow")?,
            )
        } else if supply_delta < 0 {
            let burn_amount = u64::try_from(supply_delta.unsigned_abs())
                .expect("Native supply delta overflowed u64 total supply");
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

        for (address, change) in &changeset.account_changes {
            if change.balance_delta >= 0 {
                continue;
            }
            let debit = u64::try_from(change.balance_delta.unsigned_abs())
                .expect("Native debit overflowed u64 account balance");
            let balance = self.load_account_or_default(*address)?.native_balance();
            ensure!(
                balance >= debit,
                "Insufficient native balance for {}: need {}, have {}",
                address.to_hex_literal(),
                debit,
                balance
            );
        }

        let mut supplies_dirty = false;
        let mut account_index_additions = Vec::with_capacity(changeset.account_changes.len());
        let reconcile_object_locked = Self::needs_object_locked_reconciliation(changeset);
        let (issued_before, visible_before) = if reconcile_object_locked {
            self.capture_supply_snapshots(self.supply_tracking_token_types(changeset))?
        } else {
            (BTreeMap::new(), BTreeMap::new())
        };

        for (address, change) in &changeset.account_changes {
            let mut account = self.load_account_or_default(*address)?;
            let old_balances = account.token_balances.clone();
            let native_token = KANARI_TOKEN_TYPE.to_string();

            if change.balance_delta > 0 {
                let amount = u64::try_from(change.balance_delta)
                    .expect("Native credit overflowed u64 account balance");
                let next = account
                    .native_balance()
                    .checked_add(amount)
                    .require("Native account balance overflow")?;
                account.set_token_balance_value(&native_token, next);
            } else if change.balance_delta < 0 {
                let debit = u64::try_from(change.balance_delta.unsigned_abs())
                    .expect("Native debit overflowed u64 account balance");
                let next = account.native_balance() - debit;
                account.set_token_balance_value(&native_token, next);
            }
            account.sequence_number = account
                .sequence_number
                .checked_add(change.sequence_increment)
                .require("Account sequence number overflow")?;
            for module_name in &change.modules_added {
                account.add_module(module_name.clone());
            }
            self.save_account_record(&account)?;
            account_index_additions.push(account.address.to_hex_literal());
            supplies_dirty |= self.capture_supply_changed(&account, &old_balances);
        }

        self.add_many_to_index_list(ACCOUNT_INDEX_KEY, account_index_additions)?;

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

            if token_type == KANARI_TOKEN_TYPE {
                self.save_native_total_supply(total_supply.total_supply)?;
            }
        }

        // Apply NFT capability creations/updates
        for (_owner, token_type, nft_cap) in &changeset.nft_caps {
            let mut key = b"nft:".to_vec();
            key.extend_from_slice(token_type.as_bytes());
            self.save_internal(&key, nft_cap)?;
        }

        // Apply incremental token balance hints first; exact owner totals are recomputed
        // from owned coin objects after object mutations are applied.
        for (owner, token_type, amount) in &changeset.token_balance_sets {
            let mut account = self.load_account_or_default(*owner)?;
            let normalized_token_type = Self::normalize_token_type(token_type);
            if normalized_token_type == KANARI_TOKEN_TYPE {
                // Native KANARI balance is applied from account_changes so gas and
                // transfers remain exact. Object-derived hints must not double count it.
                continue;
            }

            let old_balances = account.token_balances.clone();
            let current = account.get_token_balance(&normalized_token_type);
            let next = current.saturating_add(amount.value());
            account.set_token_balance_value(&normalized_token_type, next);
            self.save_account(&account)?;

            supplies_dirty |= self.capture_supply_changed(&account, &old_balances);
        }

        // Record Global Token Supplies to database only once after all processing
        let mut owners_to_recompute: BTreeSet<AccountAddress> = BTreeSet::new();
        let mut native_object_changed_owners: BTreeSet<AccountAddress> = BTreeSet::new();
        let mut native_object_gas_adjusted: BTreeMap<AccountAddress, u64> = BTreeMap::new();

        for obj_id in &changeset.deleted_objects {
            if let Some((stored_id, existing)) = self.load_stored_object_by_any_id(obj_id)? {
                let obj_key = object_key(&stored_id);
                Self::record_object_balance_owner_if_needed(
                    &mut owners_to_recompute,
                    &mut native_object_changed_owners,
                    existing.owner,
                    &existing.type_name,
                    &existing.data,
                );
                self.remove_owned_object_index_variants(existing.owner, obj_id, Some(&stored_id))?;
                self.overlay.insert(obj_key, None);
            } else {
                let obj_key = object_key(obj_id);
                self.overlay.insert(obj_key, None);
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
            let obj_key = object_key(obj_id);

            if let Some((stored_id, existing)) = existing_obj {
                // Canonical object versions must be derived from the visible state snapshot,
                // not the runtime object cache, so every authority commits the same version.
                new_obj.version = existing.version + 1;
                if new_obj.owner.to_hex_literal() == *obj_id {
                    new_obj.owner = existing.owner;
                }
                let existing_native_coin =
                    Self::balance_token_amount(&existing.type_name, &existing.data)
                        .is_some_and(|(token_type, _)| token_type == KANARI_TOKEN_TYPE);
                if existing_native_coin {
                    let sender_native_debit: u64 = changeset
                        .account_changes
                        .get(&existing.owner)
                        .map(|change| change.balance_delta)
                        .filter(|delta| *delta < 0)
                        .map(|delta| {
                            u64::try_from(delta.unsigned_abs()).expect(
                                "Native debit overflowed u64 object gas adjustment",
                            )
                        })
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

                if existing.owner != new_obj.owner {
                    self.remove_owned_object_index_variants(
                        existing.owner,
                        obj_id,
                        Some(&stored_id),
                    )?;
                }
                if stored_id != *obj_id {
                    self.overlay.insert(object_key(&stored_id), None);
                }
                Self::record_object_balance_owner_if_needed(
                    &mut owners_to_recompute,
                    &mut native_object_changed_owners,
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
                new_obj.owner,
                &new_obj.type_,
                &new_obj.data,
            );
            if let Some((token_type, total_supply)) =
                Self::treasury_cap_token_supply(&new_obj.type_, &new_obj.data)
            {
                supplies_dirty |=
                    self.persist_treasury_cap_state(&token_type, new_obj.owner, total_supply)?;
            }

            let stored_obj = StoredObject {
                id: obj_id.clone(),
                owner: new_obj.owner,
                type_name: new_obj.type_.clone(),
                data: new_obj.data.clone(),
                version: new_obj.version,
            };
            self.save_internal(&obj_key, &stored_obj)?;

            self.refresh_owned_object_index(new_obj.owner, obj_id, existing_stored_id.as_deref())?;

            if new_obj.type_.contains("::coin::CoinMetadata<")
                && let Some(start) = new_obj.type_.find('<')
                && let Some(end) = new_obj.type_.rfind('>')
            {
                let token_type = &new_obj.type_[start + 1..end];
                self.persist_coin_metadata(token_type, &new_obj.data)?;
            }
        }

        for owner in owners_to_recompute {
            let native_delta = changeset
                .account_changes
                .get(&owner)
                .map(|change| change.balance_delta)
                .unwrap_or(0i128);
            let native_object_changed = native_object_changed_owners.contains(&owner);
            let adjusted_gas = native_object_gas_adjusted.get(&owner).copied().unwrap_or(0);
            if self.recompute_token_balances_for_owner(
                owner,
                native_delta,
                native_object_changed,
                adjusted_gas,
            )? {
                supplies_dirty = true;
            }
        }

        if reconcile_object_locked {
            self.reconcile_object_locked_coin_records(changeset, &issued_before, &visible_before)?;
        }

        if self.sync_native_visible_supply_cache()? {
            supplies_dirty = true;
        }

        if supplies_dirty {
            let supplies_clone = self.global_token_supplies.clone();
            self.save_internal(b"global_token_supplies", &supplies_clone)?;
        }

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

        Ok(())
    }
}
