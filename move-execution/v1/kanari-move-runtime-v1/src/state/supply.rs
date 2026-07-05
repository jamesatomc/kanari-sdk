use super::*;

impl StateManager {
    pub(super) fn supply_key(token_type: &str) -> Vec<u8> {
        let mut key = b"supply:".to_vec();
        key.extend_from_slice(token_type.as_bytes());
        key
    }

    pub(super) fn load_persisted_supply_from_store(
        store: &PersistentStore,
        token_type: &str,
    ) -> Option<u64> {
        let key = Self::supply_key(token_type);
        store
            .load::<TreasuryCap>(&key)
            .ok()
            .flatten()
            .map(|cap| cap.total_supply)
            .or_else(|| store.load::<u64>(&key).ok().flatten())
    }

    pub(super) fn save_native_total_supply(&mut self, total_supply: u64) -> Result<()> {
        self.total_supply = total_supply;
        self.save_internal(b"total_supply", &total_supply)?;

        let supply_key = Self::supply_key(KANARI_TOKEN_TYPE);
        if self
            .load_internal::<TreasuryCap>(&supply_key)
            .ok()
            .flatten()
            .is_some()
        {
            self.save_internal(&supply_key, &TreasuryCap { total_supply })?;
        } else if self
            .load_internal::<u64>(&supply_key)
            .ok()
            .flatten()
            .is_some()
        {
            self.save_internal(&supply_key, &total_supply)?;
        }

        Ok(())
    }

    pub(super) fn issued_supply_for_token(&self, token_type: &str) -> u64 {
        if token_type == KANARI_TOKEN_TYPE {
            return self.total_supply;
        }

        let supply_key = Self::supply_key(token_type);
        self.load_internal::<TreasuryCap>(&supply_key)
            .ok()
            .flatten()
            .map(|cap| cap.total_supply)
            .or_else(|| self.load_internal::<u64>(&supply_key).ok().flatten())
            .or_else(|| self.global_token_supplies.get(token_type).copied())
            .unwrap_or(0)
    }

    pub(super) fn indexed_wallet_supply(&self, token_type: &str) -> Result<u64> {
        let token_type = Self::normalize_token_type(token_type);
        Ok(self
            .load_account_addresses()?
            .into_iter()
            .filter_map(|address| self.load_account(&address).ok().flatten())
            .map(|account| account.get_token_balance(&token_type))
            .fold(0u64, |acc, balance| acc.saturating_add(balance)))
    }

    pub(super) fn sync_native_visible_supply_cache(&mut self) -> Result<bool> {
        let indexed_visible = self.indexed_wallet_supply(KANARI_TOKEN_TYPE)?;
        let current = self
            .global_token_supplies
            .get(KANARI_TOKEN_TYPE)
            .copied()
            .unwrap_or(0);
        if current == indexed_visible {
            return Ok(false);
        }

        if indexed_visible == 0 {
            self.global_token_supplies.remove(KANARI_TOKEN_TYPE);
        } else {
            self.global_token_supplies
                .insert(KANARI_TOKEN_TYPE.to_string(), indexed_visible);
        }
        Ok(true)
    }

    /// Repair legacy native wallet-visible overcount before startup/checkpoint work.
    ///
    /// Do not call this from normal transaction application. New bad transaction
    /// effects must be rejected by supply invariants instead of being silently
    /// normalized.
    pub fn repair_legacy_native_wallet_overcount(&mut self) -> Result<bool> {
        let indexed_visible = self.indexed_wallet_supply(KANARI_TOKEN_TYPE)?;
        if indexed_visible <= self.total_supply {
            let changed = self.sync_native_visible_supply_cache()?;
            if changed {
                let supplies_clone = self.global_token_supplies.clone();
                self.save_internal(b"global_token_supplies", &supplies_clone)?;
            }
            return Ok(changed);
        }

        let mut excess = indexed_visible - self.total_supply;
        let mut accounts = self
            .load_account_addresses()?
            .into_iter()
            .filter_map(|address| self.load_account(&address).ok().flatten())
            .filter(|account| account.native_balance() > 0)
            .collect::<Vec<_>>();

        accounts.sort_by_key(|account| account.address.to_hex_literal());
        accounts.reverse();

        for mut account in accounts {
            if excess == 0 {
                break;
            }

            let current = account.native_balance();
            let debit = current.min(excess);
            let next = current - debit;
            account.set_token_balance_value(KANARI_TOKEN_TYPE, next);
            self.save_account_record(&account)?;
            excess -= debit;
        }

        ensure!(
            excess == 0,
            "Unable to repair legacy native supply overcount: remaining_excess={}",
            excess
        );
        log::warn!(
            "[StateManager] Repaired legacy native wallet overcount: indexed_visible={} total_supply={} repaired_excess={}",
            indexed_visible,
            self.total_supply,
            indexed_visible - self.total_supply
        );
        self.sync_native_visible_supply_cache()?;
        let supplies_clone = self.global_token_supplies.clone();
        self.save_internal(b"global_token_supplies", &supplies_clone)?;
        Ok(true)
    }

    pub(super) fn load_object_locked_coin_records(&self) -> Result<Vec<ObjectLockedCoinRecord>> {
        Ok(self
            .load_internal(OBJECT_LOCKED_COIN_RECORDS_KEY)?
            .unwrap_or_default())
    }

    pub(super) fn save_object_locked_coin_records(
        &mut self,
        records: &[ObjectLockedCoinRecord],
    ) -> Result<()> {
        self.save_internal(OBJECT_LOCKED_COIN_RECORDS_KEY, records)
    }

    fn object_locked_supply_for_token(&self, token_type: &str) -> Result<u64> {
        let token_type = Self::normalize_token_type(token_type);
        Ok(self
            .load_object_locked_coin_records()?
            .into_iter()
            .filter(|record| record.token_type == token_type)
            .map(|record| record.amount)
            .fold(0u64, |acc, amount| acc.saturating_add(amount)))
    }

    pub fn token_supply_summary(&self, token_type: &str) -> Result<TokenSupplySummary> {
        let token_type = Self::normalize_token_type(token_type);
        let total_supply = self.issued_supply_for_token(&token_type);
        let cached_visible = self
            .global_token_supplies
            .get(&token_type)
            .copied()
            .unwrap_or(0);
        let indexed_visible = self.indexed_wallet_supply(&token_type)?;
        let wallet_visible_supply = if token_type == KANARI_TOKEN_TYPE {
            indexed_visible
        } else {
            cached_visible.max(indexed_visible)
        };
        let ledger_locked_supply = self.object_locked_supply_for_token(&token_type)?;
        let inferred_locked_supply = total_supply.saturating_sub(wallet_visible_supply);
        let object_locked_supply = ledger_locked_supply.max(inferred_locked_supply);
        let accounted_supply = wallet_visible_supply.saturating_add(object_locked_supply);

        Ok(TokenSupplySummary {
            token_type,
            total_supply,
            wallet_visible_supply,
            object_locked_supply,
            accounted_supply,
            untracked_supply: total_supply.saturating_sub(accounted_supply),
        })
    }

    pub fn supply_invariant_fail_fast_enabled() -> bool {
        std::env::var("KANARI_FAIL_FAST_ON_SUPPLY_MISMATCH")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or_else(|_| {
                matches!(
                    std::env::var("KANARI_NETWORK")
                        .unwrap_or_else(|_| "testnet".to_string())
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "mainnet"
                )
            })
    }

    pub(super) fn report_supply_invariant_violation(
        context: &str,
        error: &anyhow::Error,
    ) -> Result<()> {
        log::error!(
            "[StateManager] Supply invariant check failed {}: {}",
            context,
            error
        );

        if Self::supply_invariant_fail_fast_enabled() {
            anyhow::bail!("Supply invariant check failed {}: {}", context, error);
        }

        Ok(())
    }
    fn save_token_metadata_field<T: Serialize + ?Sized>(
        &mut self,
        prefix: &[u8],
        token_type: &str,
        value: &T,
    ) -> Result<()> {
        let key = metadata_key(prefix, token_type);
        self.save_internal(&key, value)
    }

    fn load_token_metadata_field<T: DeserializeOwned>(
        &self,
        prefix: &[u8],
        token_type: &str,
    ) -> Result<Option<T>> {
        let key = metadata_key(prefix, token_type);
        self.load_internal(&key)
    }
    pub(super) fn normalize_token_type(token_type: &str) -> String {
        if let Ok(TypeTag::Struct(st)) = TypeTag::from_str(token_type) {
            return format!("{}", st);
        }
        token_type.to_string()
    }
    pub(super) fn persist_coin_metadata(&mut self, token_type: &str, data: &[u8]) -> Result<()> {
        #[derive(Deserialize)]
        struct MoveString {
            bytes: Vec<u8>,
        }
        #[derive(Deserialize)]
        struct MoveUrl {
            inner: MoveString,
        }
        #[derive(Deserialize)]
        struct MoveOption<T> {
            vec: Vec<T>,
        }
        #[derive(Deserialize)]
        struct ParsedCoinMetadata {
            _id: AccountAddress,
            decimals: u8,
            symbol: MoveString,
            name: MoveString,
            description: MoveString,
            icon_url: MoveOption<MoveUrl>,
        }

        if let Ok(meta) = bcs::from_bytes::<ParsedCoinMetadata>(data) {
            self.save_token_metadata_field(b"metadata_decimals:", token_type, &meta.decimals)?;

            if let Ok(name) = String::from_utf8(meta.name.bytes) {
                self.save_token_metadata_field(b"metadata_name:", token_type, &name)?;
            }
            if let Ok(symbol) = String::from_utf8(meta.symbol.bytes) {
                self.save_token_metadata_field(b"metadata_symbol:", token_type, &symbol)?;
            }
            if let Ok(description) = String::from_utf8(meta.description.bytes) {
                self.save_token_metadata_field(b"metadata_description:", token_type, &description)?;
            }
            if let Some(url_obj) = meta.icon_url.vec.into_iter().next()
                && let Ok(url) = String::from_utf8(url_obj.inner.bytes)
            {
                self.save_token_metadata_field(b"metadata_icon_url:", token_type, &url)?;
            }
        } else if data.len() > 32 {
            self.save_token_metadata_field(b"metadata_decimals:", token_type, &data[32])?;
        }
        Ok(())
    }

    pub(super) fn adjust_global_supplies_for_account_delta(
        &mut self,
        old_balances: &BTreeMap<String, BalanceRecord>,
        new_balances: &BTreeMap<String, BalanceRecord>,
    ) -> bool {
        let mut changed = false;
        let mut tokens = BTreeSet::new();
        tokens.extend(old_balances.keys().cloned());
        tokens.extend(new_balances.keys().cloned());

        for token_type in tokens {
            let old_amount = old_balances
                .get(&token_type)
                .map(|x| x.value())
                .unwrap_or(0);
            let new_amount = new_balances
                .get(&token_type)
                .map(|x| x.value())
                .unwrap_or(0);

            if old_amount == new_amount {
                continue;
            }

            changed = true;
            let current_supply = self
                .global_token_supplies
                .get(&token_type)
                .copied()
                .unwrap_or(0);

            let updated_supply = if new_amount >= old_amount {
                current_supply.saturating_add(new_amount - old_amount)
            } else {
                current_supply.saturating_sub(old_amount - new_amount)
            };

            if updated_supply == 0 {
                self.global_token_supplies.remove(&token_type);
            } else {
                self.global_token_supplies
                    .insert(token_type, updated_supply);
            }
        }

        changed
    }

    fn apply_native_delta(balance: u64, delta: i128) -> Result<u64> {
        if delta >= 0 {
            let amount = u64::try_from(delta)
                .expect("Native object delta overflowed u64 account balance");
            return balance
                .checked_add(amount)
                .require("Native object balance overflow after account delta");
        }

        let debit = u64::try_from(delta.unsigned_abs())
            .expect("Native object delta overflowed u64 account balance");
        ensure!(
            balance >= debit,
            "Native object balance underflow after account delta: balance={}, debit={}",
            balance,
            debit
        );
        Ok(balance - debit)
    }

    pub(super) fn capture_supply_changed(
        &mut self,
        account: &Account,
        old_balances: &BTreeMap<String, BalanceRecord>,
    ) -> bool {
        self.adjust_global_supplies_for_account_delta(old_balances, &account.token_balances)
    }

    pub(super) fn recompute_token_balances_for_owner(
        &mut self,
        owner: AccountAddress,
        native_delta: i128,
        native_object_changed: bool,
        native_object_gas_adjusted: u64,
    ) -> Result<bool> {
        let mut account = self.load_account_or_default(owner)?;
        let old_balances = account.token_balances.clone();
        let native_balance_after_account_changes = account.native_balance();
        let mut aggregated: BTreeMap<String, u64> = BTreeMap::new();

        for object_id in self.get_owned_objects(&owner)? {
            let Some(obj) = self.get_object(&object_id)? else {
                continue;
            };

            let Ok(struct_tag) = StructTag::from_str(&obj.type_) else {
                continue;
            };

            if !Self::is_balance_struct(&struct_tag) {
                continue;
            }

            let Some(amount) = Self::extract_balance_from_object_bytes(&obj.data, &struct_tag)
            else {
                continue;
            };

            let Some(token_type) = Self::token_type_from_balance_struct(&struct_tag) else {
                continue;
            };

            let token_type = Self::normalize_token_type(&token_type);
            let entry = aggregated.entry(token_type).or_insert(0);
            *entry = entry.saturating_add(amount);
        }

        let native_object_balance = aggregated.remove(KANARI_TOKEN_TYPE);
        account.token_balances = aggregated
            .into_iter()
            .map(|(token_type, amount)| (token_type, BalanceRecord::new(amount)))
            .collect();

        let adjusted_native_delta = native_delta.saturating_add(i128::from(native_object_gas_adjusted));
        let native_balance = if let Some(object_balance) = native_object_balance {
            if native_object_changed {
                Self::apply_native_delta(object_balance, adjusted_native_delta)?
            } else {
                Self::apply_native_delta(object_balance, native_delta)?
            }
        } else if native_object_changed {
            0
        } else {
            native_balance_after_account_changes
        };
        account.set_token_balance_value(KANARI_TOKEN_TYPE, native_balance);
        self.save_account(&account)?;

        Ok(self.capture_supply_changed(&account, &old_balances))
    }
    /// Get token decimals for a specific token type
    pub fn get_token_decimals(&self, token_type: &str) -> Result<Option<u8>> {
        self.load_token_metadata_field(b"metadata_decimals:", token_type)
    }

    ///  Get token name for a specific token type
    pub fn get_token_name(&self, token_type: &str) -> Result<Option<String>> {
        self.load_token_metadata_field(b"metadata_name:", token_type)
    }

    ///  Get token symbol for a specific token type
    pub fn get_token_symbol(&self, token_type: &str) -> Result<Option<String>> {
        self.load_token_metadata_field(b"metadata_symbol:", token_type)
    }

    /// Get token description for a specific token type
    pub fn get_token_description(&self, token_type: &str) -> Result<Option<String>> {
        self.load_token_metadata_field(b"metadata_description:", token_type)
    }

    /// Get token icon URL for a specific token type
    pub fn get_token_icon_url(&self, token_type: &str) -> Result<Option<String>> {
        self.load_token_metadata_field(b"metadata_icon_url:", token_type)
    }

    pub fn validate_supply_invariants(&self) -> Result<()> {
        let supply_key = Self::supply_key(KANARI_TOKEN_TYPE);
        let persisted_native_supply = self
            .load_internal::<TreasuryCap>(&supply_key)
            .ok()
            .flatten()
            .map(|cap| cap.total_supply)
            .or_else(|| self.load_internal::<u64>(&supply_key).ok().flatten());
        if let Some(persisted) = persisted_native_supply
            && persisted != self.total_supply
        {
            anyhow::bail!(
                "native total supply mismatch: state.total_supply={} persisted_treasury={}",
                self.total_supply,
                persisted
            );
        }

        let native_supply = self.token_supply_summary(KANARI_TOKEN_TYPE)?;
        // Wallet-visible balance caches only reflect top-level wallet-owned
        // coin objects. Coins can also be held inside DeFi objects (for
        // example escrow funds), so visible supply may be lower than issued
        // supply without implying a burn. It must never exceed total supply.
        if native_supply.wallet_visible_supply > native_supply.total_supply {
            anyhow::bail!(
                "native supply overcount: total_supply={} wallet_visible_supply={} object_locked_supply={}",
                native_supply.total_supply,
                native_supply.wallet_visible_supply,
                native_supply.object_locked_supply
            );
        }
        if native_supply.accounted_supply > native_supply.total_supply {
            anyhow::bail!(
                "native supply overcount: total_supply={} accounted_supply={} wallet_visible_supply={} object_locked_supply={}",
                native_supply.total_supply,
                native_supply.accounted_supply,
                native_supply.wallet_visible_supply,
                native_supply.object_locked_supply
            );
        }

        Ok(())
    }
}
