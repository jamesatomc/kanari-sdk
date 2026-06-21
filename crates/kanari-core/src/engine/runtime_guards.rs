// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::*;

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
        env::var("KANARI_REQUIRE_PERSISTENT_STORAGE")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or_else(|_| Self::network_name().eq_ignore_ascii_case("mainnet"))
    }

    pub fn strict_checkpoint_roots_required() -> bool {
        env::var("KANARI_STRICT_CHECKPOINT_ROOTS")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or_else(|_| Self::network_name().eq_ignore_ascii_case("mainnet"))
    }

    pub fn allow_in_memory_fallback() -> bool {
        env::var("KANARI_ALLOW_IN_MEMORY_FALLBACK")
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    }

    pub fn checkpoint_chain_id() -> String {
        env::var("KANARI_CHAIN_ID")
            .unwrap_or_else(|_| format!("kanari-{}", Self::network_name().to_ascii_lowercase()))
    }

    pub fn current_epoch() -> u64 {
        env::var("KANARI_EPOCH")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0)
    }

    pub fn checkpoint_protocol_version() -> u64 {
        env::var("KANARI_PROTOCOL_VERSION")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1)
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

    /// Execute a public, bounded and read-only Move function.
    pub fn execute_runtime_view(
        &self,
        package_addr: &str,
        module_name: &str,
        function_name: &str,
        type_args: &[String],
        args: &[Vec<u8>],
    ) -> Result<serde_json::Value> {
        let runtime = self
            .runtime_pool
            .first()
            .ok_or_else(|| anyhow::anyhow!("No Move runtime is available"))?;
        runtime.execute_safe_view_function(
            package_addr,
            module_name,
            function_name,
            type_args,
            args,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_view_rejects_oversized_input_before_loading_module() {
        let engine = BlockchainEngine::new_in_memory().unwrap();
        let oversized = vec![vec![0u8; 64 * 1024 + 1]];
        let error = engine
            .execute_runtime_view("0x2", "coin", "value", &[], &oversized)
            .unwrap_err();
        assert!(error.to_string().contains("input"));
    }
}
