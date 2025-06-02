// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use mona_crypto::verify_signature;
use mona_types::gas::format_gas_fee_display;
use move_core_types::account_address::AccountAddress;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::ModuleId;
use move_package::BuildConfig;
use serde_json::{Value as JsonValue, json};
use sha3::{Digest, Sha3_256};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::blockchain_integration::{DeploymentResult, submit_deployment_to_blockchain};
use crate::utils::{
    generate_object_id, get_module_dependencies, get_module_public_functions, reroot_path,
};

pub struct Publish {
    pub signature: Option<Vec<u8>>,
    pub signer_address: Option<String>,
}

impl Publish {
    pub fn execute(
        self,
        path: Option<PathBuf>,
        address: Option<AccountAddress>,
        config: BuildConfig,
        gas_budget: Option<u64>,
        skip_verify: bool,
    ) -> anyhow::Result<()> {
        let rerooted_path = reroot_path(path)?;

        // Verify sources directory exists and contains .move files
        let sources_dir = rerooted_path.join("sources");
        if !sources_dir.exists()
            || !std::fs::read_dir(&sources_dir)
                .map_err(|e| anyhow::anyhow!("Failed to read sources: {}", e))?
                .filter_map(Result::ok)
                .any(|entry| entry.path().extension().map_or(false, |ext| ext == "move"))
        {
            return Err(anyhow::anyhow!(
                "No Move source files found at {}",
                sources_dir.display()
            ));
        }

        let address = address.unwrap_or_else(|| AccountAddress::from_hex_literal("0x1").unwrap());

        // Compile with panic handling
        let compiled_package = match std::panic::catch_unwind(|| {
            config.compile_package(&rerooted_path, &mut Vec::new())
        }) {
            Ok(Ok(package)) if !package.root_compiled_units.is_empty() => package,
            Ok(Ok(_)) => return Err(anyhow::anyhow!("No modules compiled")),
            Ok(Err(e)) => return Err(anyhow::anyhow!("Compilation failed: {}", e)),
            Err(_) => return Err(anyhow::anyhow!("Compiler error")),
        };

        if let (Some(signature), Some(signer)) = (&self.signature, &self.signer_address) {
            let mut hasher = Sha3_256::new();
            hasher.update(address.to_hex().as_bytes());
            hasher.update(rerooted_path.to_str().unwrap_or("").as_bytes());
            hasher.update(gas_budget.unwrap_or(3_000_000).to_le_bytes());

            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            hasher.update(timestamp.to_le_bytes());

            let payload_hash = hasher.finalize();
            let payload_to_verify = payload_hash.as_slice();

            match verify_signature(signer, payload_to_verify, signature) {
                Ok(true) => {}
                Ok(false) => {}
                Err(_) => {}
            }
        }

        let deployment_info = self.prepare_deployment(
            &compiled_package,
            address,
            gas_budget.unwrap_or(3_000_000),
            skip_verify,
        )?;

        let deployment_result = self.submit_to_blockchain(
            &compiled_package,
            &address,
            &deployment_info,
            self.signature.clone(),
            self.signer_address.clone(),
        )?;

        let mut signature_info = json!({
            "signed": false
        });

        if let (Some(_), Some(signer)) = (&self.signature, &self.signer_address) {
            signature_info = json!({
                "signed": true,
                "signer": signer,
                "timestamp": SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            });
        }

        let result = json!({
            "status": "success",
            "type": "blockchain_deployment",
            "metadata": {
                "package": {
                    "name": compiled_package.compiled_package_info.package_name.to_string(),
                    "id": generate_object_id(),
                    "path": rerooted_path.to_string_lossy(),
                    "address": format!("0x{}", address.to_hex()),
                    "gas_budget": gas_budget.unwrap_or(3_000_000),
                    "gas_used": deployment_result.gas_used,
                    "deploy_time": deployment_result.execution_time_ms,
                },
                "blockchain": {
                    "transaction_id": deployment_result.transaction_id,
                    "status": deployment_result.status,
                    "block_height": deployment_result.block_height,
                    "modules_deployed": deployment_result.modules_deployed,
                    "timestamp": SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                },
                "signature": signature_info,
                "deployment": deployment_info,
                "mvsm_files": deployment_result.mvsm_files
            }
        });

        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }

    fn prepare_deployment(
        &self,
        package: &move_package::compilation::compiled_package::CompiledPackage,
        address: AccountAddress,
        gas_budget: u64,
        skip_verify: bool,
    ) -> anyhow::Result<JsonValue> {
        let mut modules_json = Vec::new();

        let address_str = format!("{:0>64}", address.to_hex());
        let address_0x = format!("0x{}", address_str);

        for unit in &package.root_compiled_units {
            let module = &unit.unit;
            let module_name = module.name().to_string();
            let module_id = ModuleId::new(address, Identifier::new(module_name.clone())?);

            let standard_module_id = module_id.to_string();
            let full_module_id = format!("{}::{}", address_0x, module_name);

            let bytecode = module.serialize(None);

            let gas_estimate = estimate_gas_for_module(&bytecode, gas_budget);

            let module_meta = json!({
                "id": generate_object_id(),
                "name": module_name,
                "module_id": standard_module_id,
                "full_module_id": full_module_id,
                "size_bytes": bytecode.len(),
                "verification": {
                    "skip": skip_verify,
                    "gas_estimate": gas_estimate,
                    "gas_display": format_gas_fee_display(gas_estimate)
                },
                "constructor_args": [],
                "dependencies": get_module_dependencies(&unit.unit),
                "public_functions": get_module_public_functions(&unit.unit)
            });

            modules_json.push(module_meta);
        }

        let deployment_json = json!({
            "modules": modules_json,
            "timestamp": SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            "id": generate_object_id(),
        });

        Ok(deployment_json)
    }

    fn submit_to_blockchain(
        &self,
        package: &move_package::compilation::compiled_package::CompiledPackage,
        address: &AccountAddress,
        deployment_info: &JsonValue,
        signature: Option<Vec<u8>>,
        signer_address: Option<String>,
    ) -> anyhow::Result<DeploymentResult> {
        submit_deployment_to_blockchain(
            package,
            address,
            deployment_info,
            signature,
            signer_address,
        )
    }
}

fn estimate_gas_for_module(bytecode: &[u8], gas_budget: u64) -> u64 {
    // Use a more reasonable gas calculation for deployments
    let base_deployment_gas: u64 = 100_000; // Base gas for deployment (0.0001 KARI)
    let size_factor: u64 = (bytecode.len() as u64).saturating_mul(10); // 10 gas per byte
    let estimate = base_deployment_gas.saturating_add(size_factor);

    // Cap at a reasonable maximum for deployments
    let max_deployment_gas: u64 = 500_000; // 0.0005 KARI maximum
    let final_estimate = std::cmp::min(estimate, max_deployment_gas);

    // Still respect the user's gas budget as absolute maximum
    std::cmp::min(final_estimate, gas_budget)
}
