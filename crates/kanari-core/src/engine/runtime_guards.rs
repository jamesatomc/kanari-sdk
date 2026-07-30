// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;

pub(crate) fn strict_guard_required(network: &str, override_value: Option<&str>) -> bool {
    override_value
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or_else(|| {
            !(network.eq_ignore_ascii_case("local") || network.eq_ignore_ascii_case("test"))
        })
}

#[derive(Debug, Clone)]
pub struct RuntimeGuardConfig {
    pub network: String,
    pub fail_fast_supply_enabled: bool,
    pub strict_persistence_required: bool,
    pub strict_checkpoint_roots: bool,
    pub persistent_storage_available: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeHealthReport {
    pub guards: RuntimeGuardConfig,
    pub supply_invariants_ok: bool,
    pub supply_invariant_error: Option<String>,
}

impl RuntimeHealthReport {
    pub fn status(&self) -> &'static str {
        if self.supply_invariants_ok {
            "ok"
        } else {
            "degraded"
        }
    }
}

impl BlockchainEngine {
    pub fn network_name() -> String {
        env::var("KANARI_NETWORK").unwrap_or_else(|_| "testnet".to_string())
    }

    pub fn strict_persistence_required() -> bool {
        strict_guard_required(
            &Self::network_name(),
            env::var("KANARI_REQUIRE_PERSISTENT_STORAGE")
                .ok()
                .as_deref(),
        )
    }

    pub fn strict_checkpoint_roots_required() -> bool {
        strict_guard_required(
            &Self::network_name(),
            env::var("KANARI_STRICT_CHECKPOINT_ROOTS").ok().as_deref(),
        )
    }

    pub fn fail_fast_supply_enabled() -> bool {
        StateManager::supply_invariant_fail_fast_enabled()
    }

    pub fn runtime_guard_config(&self) -> RuntimeGuardConfig {
        RuntimeGuardConfig {
            network: Self::network_name(),
            fail_fast_supply_enabled: Self::fail_fast_supply_enabled(),
            strict_persistence_required: Self::strict_persistence_required(),
            strict_checkpoint_roots: Self::strict_checkpoint_roots_required(),
            persistent_storage_available: self.persistent_store.is_some(),
        }
    }

    pub fn runtime_health_report(&self) -> RuntimeHealthReport {
        let supply_invariant_error = self
            .state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .validate_supply_invariants()
            .err()
            .map(|e| e.to_string());

        RuntimeHealthReport {
            guards: self.runtime_guard_config(),
            supply_invariants_ok: supply_invariant_error.is_none(),
            supply_invariant_error,
        }
    }

    pub fn validate_runtime_health(&self) -> Result<()> {
        let report = self.runtime_health_report();
        if let Some(error) = report.supply_invariant_error {
            anyhow::bail!(error);
        }

        if report.guards.strict_persistence_required && !report.guards.persistent_storage_available
        {
            anyhow::bail!("persistent storage is required but engine is running in-memory");
        }

        Ok(())
    }
}
