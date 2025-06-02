// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use mona_blockchain::blockchain::{BLOCKCHAIN_DATA, submit_transaction};
use move_core_types::account_address::AccountAddress;
use serde_json::Value as JsonValue;
use sha3::{Digest, Sha3_256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::utils::generate_random_hex;
use crate::vm_module::{VMModule, serialize_module_to_mvsm};
use crate::vm_state::VM_STATE;

pub struct DeploymentResult {
    pub transaction_id: String,
    pub status: String,
    pub gas_used: u64,
    pub execution_time_ms: u64,
    pub block_height: u64,
    pub modules_deployed: usize,
    pub mvsm_files: Vec<String>,
}

pub fn submit_deployment_to_blockchain(
    package: &move_package::compilation::compiled_package::CompiledPackage,
    address: &AccountAddress,
    deployment_info: &JsonValue,
    signature: Option<Vec<u8>>,
    signer_address: Option<String>,
) -> anyhow::Result<DeploymentResult> {
    let start = std::time::Instant::now();

    let modules = deployment_info["modules"].as_array().unwrap();
    let mut total_gas_used = 0;
    let mut modules_deployed = 0;

    let blockchain = BLOCKCHAIN_DATA.iter();
    let block_height = match blockchain.last() {
        Some(block) => block.index,
        None => 0,
    };

    let mut vm_state = match VM_STATE.try_write() {
        Ok(state) => state,
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to lock VM state for writing: {}",
                e
            ));
        }
    };

    let deploy_tx_id = format!("deploy_tx_{}", generate_random_hex(32));

    if modules.is_empty() {
        return Err(anyhow::anyhow!("No modules to deploy"));
    }

    let mut blockchain_transactions = Vec::new();
    let mut mvsm_storage_keys = Vec::new();

    for (_idx, module_json) in modules.iter().enumerate() {
        let module_name = module_json["name"].as_str().unwrap_or("unknown");

        let bytecode = match package
            .root_compiled_units
            .iter()
            .find(|unit| unit.unit.name().to_string() == module_name)
        {
            Some(unit) => {
                let bytecode = unit.unit.serialize(None);
                bytecode
            }
            None => {
                let size_bytes = module_json["size_bytes"].as_u64().unwrap_or(1024) as usize;
                vec![0u8; size_bytes]
            }
        };

        let size_bytes = bytecode.len() as u64;

        let public_funcs = module_json["public_functions"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|f| f["name"].as_str().map(|s| s.to_string()))
            .collect::<Vec<String>>();

        let vm_module = VMModule::new(
            *address,
            module_name.to_string(),
            bytecode.clone(),
            public_funcs.clone(),
            block_height,
        );

        // Store module in secure storage instead of file system
        let storage_key = match serialize_module_to_mvsm(
            &vm_module,
            &package.compiled_package_info.package_name.to_string(),
            None,
        ) {
            Ok(key) => {
                println!("Stored .mvsm module in secure storage: {}", key);
                mvsm_storage_keys.push(key.clone());
                Some(key)
            }
            Err(e) => {
                eprintln!(
                    "Warning: Failed to store .mvsm module in secure storage: {}",
                    e
                );
                None
            }
        };

        // Use more reasonable gas calculation for deployments
        let base_gas: u64 = 100_000; // 0.0001 KARI
        let size_factor: u64 = size_bytes.saturating_mul(10); // 10 gas per byte
        let gas_used = base_gas.saturating_add(size_factor).min(500_000); // Cap at 0.0005 KARI
        total_gas_used += gas_used;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut tx_data = Vec::new();
        let module_hash = {
            let mut hasher = Sha3_256::new();
            hasher.update(&bytecode);
            hex::encode(hasher.finalize())
        };

        // Include storage key in transaction data if available
        let data_str = if let Some(key) = storage_key {
            format!(
                "VM_MODULE:{}:{}:{}:{}",
                module_name,
                bytecode.len(),
                module_hash,
                key
            )
        } else {
            format!(
                "VM_MODULE:{}:{}:{}",
                module_name,
                bytecode.len(),
                module_hash
            )
        };

        tx_data.extend_from_slice(data_str.as_bytes());

        let blockchain_tx = mona_blockchain::block::Transaction {
            transaction_id: format!("{}_{}", deploy_tx_id, module_name),
            sender: (*address).into(),
            receiver: (*address).into(),
            amount: 0, // No token transfer for deployment
            timestamp,
            gas_fee: gas_used,
            signature: signature.clone().unwrap_or_default(),
            data: Some(tx_data),
        };

        blockchain_transactions.push(blockchain_tx);

        vm_state.register_module(vm_module.clone());

        let padded_addr = format!("{:0>64}", address.to_hex());
        let full_module_id = format!("0x{}::{}", padded_addr, module_name);

        let mut vm_module_copy = vm_module.clone();
        vm_module_copy.module_id = full_module_id;
        vm_state.register_module(vm_module_copy);

        modules_deployed += 1;
    }

    if let (Some(sig), Some(signer)) = (&signature, &signer_address) {
        vm_state.last_signature = Some(hex::encode(sig));
        vm_state.last_signer = Some(signer.clone());
    }

    let execution_time = start.elapsed().as_millis();

    vm_state.execution_count += 1;
    vm_state.last_execution = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    drop(vm_state);

    for tx in blockchain_transactions {
        let _tx_id = tx.transaction_id.clone();
        if let Err(e) = submit_transaction(tx) {
            println!("Warning: Failed to submit transaction to blockchain: {}", e);
            // Don't fail the entire deployment for blockchain submission issues
        }
    }

    let result = DeploymentResult {
        transaction_id: deploy_tx_id,
        status: "COMMITTED".to_string(),
        gas_used: total_gas_used,
        execution_time_ms: execution_time as u64,
        block_height: block_height as u64,
        modules_deployed,
        mvsm_files: mvsm_storage_keys,
    };

    Ok(result)
}
