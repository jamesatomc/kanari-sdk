// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::reroot_path;
use crate::command::common::resolve_transaction_gas;
use crate::command::common::{
    build_blocking_client, get_account_sequence, get_rpc_endpoint, get_sender_for_tx,
    load_wallet_for, normalize_addr, resolve_sender,
};
use anyhow::{Result, bail};
use clap::*;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::transaction::{SignedTransaction, Transaction};
use kanari_types::{GasEstimate, GasOperation};
use log::error;
use move_package::BuildConfig;
use std::path::PathBuf;

/// Publish the Move module to the blockchain
#[derive(Parser, Clone)]
#[clap(name = "publish")]
pub struct Publish {
    #[clap(flatten)]
    pub build_config: BuildConfig,
    /// Path to the Move package (defaults to current directory)
    #[clap(long = "package-path")]
    pub package_path: Option<PathBuf>,

    /// Gas limit for the transaction
    #[clap(long = "gas-limit")]
    pub gas_limit: Option<u64>,

    /// Gas price in Mist
    #[clap(long = "gas-price")]
    pub gas_price: Option<u64>,

    /// RPC endpoint
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
    // `immediate` execution option removed — always submit normally.
}

impl Publish {
    pub fn execute(self, path: Option<PathBuf>, config: BuildConfig) -> Result<()> {
        let rerooted_path = reroot_path(path.or(self.package_path.clone()))?;
        let (gas_limit, gas_price) = resolve_transaction_gas(self.gas_limit, self.gas_price);

        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());

        // Normalize and validate sender address
        let sender_normalized = resolve_sender(None)?;

        eprintln!("Building Move package...");

        // Build the package
        let compiled_package = config.compile_package(&rerooted_path, &mut std::io::stderr())?;

        // Get compiled modules (collect once to avoid double iteration)
        let modules: Vec<_> = compiled_package.all_modules().collect();
        eprintln!("Package compiled successfully!");
        eprintln!("   Modules: {}", modules.len());

        // Build a blocking HTTP client with a timeout to avoid hanging RPC calls
        let client = build_blocking_client(30)?;

        // Precompute modules that actually belong to the sender (will be published)
        let modules_to_publish: Vec<_> = modules
            .iter()
            .filter(|module_unit| {
                let module_address = module_unit.unit.module.self_id().address().to_string();
                let module_addr_normalized =
                    normalize_addr(&module_address).unwrap_or(module_address);
                module_addr_normalized.to_lowercase() == sender_normalized.to_lowercase()
            })
            .collect();

        // Optionally show total estimated gas for all modules that will be published
        let mut total_estimated_gas: u64 = 0;
        for mu in &modules_to_publish {
            let mut bytes = vec![];
            mu.unit.module.serialize(&mut bytes)?;
            let op = GasOperation::PublishModule {
                module_size: bytes.len(),
            };
            let est = GasEstimate::from_operation(op, gas_price);
            total_estimated_gas = total_estimated_gas.saturating_add(est.gas_units);
        }
        if modules_to_publish.len() > 1 {
            eprintln!(
                "Total estimated gas for all modules: {}",
                total_estimated_gas
            );
        }

        if modules.is_empty() {
            bail!("No modules found in package");
        }

        // Load wallet (signing is required).
        let wallet = {
            let w = load_wallet_for(&sender_normalized, None)?;
            eprintln!(
                "   Wallet loaded: {} (curve: {})",
                sender_normalized, w.curve_type
            );
            w
        };

        // If it's a PQC or Hybrid wallet, use the tagged address (Curve:PublicKey)
        // for signing and sender identity to ensure the RPC server can verify it.
        let sender_for_tx = get_sender_for_tx(&wallet, &sender_normalized)?;

        eprintln!("Publishing modules to blockchain...");
        eprintln!("   RPC: {}", rpc);
        eprintln!("   Sender: {}", sender_for_tx);

        let mut published_count = 0;
        let mut skipped_count = 0;

        // === Fetch base sequence number once to avoid race conditions ===
        let base_seq: u64 = get_account_sequence(&client, &rpc, &sender_for_tx)?;

        // Next sequence to use for publishing modules (increment only when a module is actually published)
        let mut next_seq = base_seq;

        for module_unit in &modules {
            let module = &module_unit.unit.module;
            let module_name = module.self_id().name().to_string();
            let module_address = module.self_id().address().to_string();

            // Normalize module address for comparison
            let module_addr_normalized = normalize_addr(&module_address).unwrap_or(module_address);

            // Only publish modules where the module address matches the sender
            if module_addr_normalized.to_lowercase() != sender_normalized.to_lowercase() {
                // Quietly skip dependency modules that don't belong to the sender.
                skipped_count += 1;
                continue;
            }

            let module_bytecode = {
                let mut bytes = vec![];
                module.serialize(&mut bytes)?;
                bytes
            };

            eprintln!("   Module: {}", module_name);
            eprintln!("     Size: {} bytes", module_bytecode.len());
            eprintln!("     Address: {}", module.self_id().address());
            eprintln!("     Functions: {}", module.function_defs.len());

            // Estimate gas using runtime GasOperation::PublishModule and GasEstimate
            let operation = GasOperation::PublishModule {
                module_size: module_bytecode.len(),
            };
            let estimate = GasEstimate::from_operation(operation, gas_price);
            eprintln!("   Estimated: {} units", estimate.gas_units);
            eprintln!("   Limit: {} units", gas_limit);
            eprintln!(
                "   Total Cost: {} Mist ({:.9} KANARI)",
                estimate.total_cost_mist, estimate.total_cost_kanari
            );

            // Create PublishModuleRequest and submit to RPC endpoint
            use kanari_rpc_api::{PublishModuleRequest, RpcRequest, RpcResponse, methods};

            // Use a monotonic sequence number reserved from base_seq for this publish
            let seq_num = next_seq;

            // Wrap and sign transaction using SignedTransaction (fatal on failure)
            let signed_tx = {
                let transaction = Transaction::PublishModule {
                    sender: sender_for_tx.clone(),
                    module_bytes: module_bytecode.clone(),
                    module_name: module_name.clone(),
                    gas_limit,
                    gas_price,
                    sequence_number: seq_num,
                };

                let mut stx = SignedTransaction::new(transaction);
                stx.sign(&wallet.private_key, wallet.curve_type)
                    .require("Failed to sign module")?;
                stx
            };

            let pub_req = PublishModuleRequest {
                sender: sender_for_tx.clone(),
                module_bytes: module_bytecode.clone(),
                module_name: module_name.clone(),
                gas_limit,
                gas_price,
                sequence_number: seq_num,
                signature: Some(signed_tx.signature.clone()),
                execute_immediate: Some(true),
            };

            let rpc_request = RpcRequest {
                jsonrpc: "2.0".to_string(),
                method: methods::PUBLISH_MODULE.to_string(),
                params: serde_json::to_value(pub_req)
                    .require("Failed to serialize publish request")?,
                id: 1,
            };

            eprintln!("     Sending publish RPC to {}...", rpc);
            match client.post(&rpc).json(&rpc_request).send() {
                Ok(resp) => match resp.json::<RpcResponse>() {
                    Ok(rpc_resp) => {
                        if let Some(err) = rpc_resp.error {
                            error!("     RPC error: {} (code {})", err.message, err.code);
                        } else {
                            // No RPC error -> consider request accepted by node
                            if let Some(result) = rpc_resp.result {
                                // Try to parse as TransactionResult
                                if let Ok(tx_result) =
                                    serde_json::from_value::<kanari_rpc_api::TransactionResult>(
                                        result.clone(),
                                    )
                                {
                                    eprintln!("     Transaction: {}", tx_result.hash);
                                    eprintln!("     Status: {}", tx_result.status);
                                    eprintln!("     Gas used: {} Mist", tx_result.gas_used);

                                    // Show error message if transaction failed
                                    if let Some(ref error_msg) = tx_result.error_message {
                                        error!("     Transaction failed: {}", error_msg);
                                    }
                                }

                                // Always print the RPC result to show any side effects (like created objects)
                                match serde_json::to_string_pretty(&result) {
                                    Ok(s) => eprintln!("     RPC result:\n{}", s),
                                    Err(_) => eprintln!("     RPC result: {}", result),
                                }
                            } else {
                                eprintln!("     RPC response has no result and no error");
                            }

                            // RPC accepted request: advance sequence and published count
                            published_count += 1;
                            next_seq = next_seq.wrapping_add(1);
                        }
                    }
                    Err(e) => error!("     Failed to parse RPC response: {}", e),
                },
                Err(e) => error!("     Failed to send RPC request: {}", e),
            }
        }

        if published_count == 0 {
            error!(
                "⚠️  Warning: No modules were published. All modules were skipped (likely dependencies)."
            );
        }

        eprintln!("Package build and validation complete!");
        eprintln!("   Published: {} modules", published_count);
        eprintln!("   Skipped: {} dependency modules", skipped_count);

        Ok(())
    }
}
