// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::{
    build_blocking_client, get_account_sequence, get_rpc_endpoint, get_sender_for_tx,
    load_wallet_for, normalize_addr, resolve_sender,
};
use anyhow::{Context, Result};
use clap::*;
use kanari_rpc_api::{CallFunctionRequest, RpcRequest, RpcResponse, methods};
use kanari_types::gas_v2::{GasEstimate, GasOperation};
use kanari_types::transaction::{SignedTransaction, Transaction};
use log::error;
use move_core_types::{parser, runtime_value::MoveValue};

/// Call a Move function on the blockchain
#[derive(Parser)]
#[clap(
    name = "call",
    about = "Call a Move function on the blockchain",
    after_help = "Examples:\n  kanari call --package 0x1 --module coin --function transfer --args 0x2 1000 --sender 0x1\n"
)]
pub struct Call {
    /// Package address (hex)
    /// Example: 0x840512ff... (use `--module <NAME>` separately)
    #[clap(long = "package")]
    pub package: String,

    /// Module name (required). Use `--module <NAME>` with `--package <ADDRESS>`.
    #[clap(long = "module", value_name = "MODULE")]
    pub module: String,

    /// Function name in module
    #[clap(long = "function")]
    pub function: String,

    /// Type arguments to the generic function being called.
    /// All must be specified, or the call will fail.
    /// Example: 0x1::coin::KANARI
    #[clap(long = "type-args")]
    pub type_args: Vec<String>,

    /// Simplified ordered args like in the function syntax.
    /// ObjectIDs, Addresses must be hex strings.
    /// Example: 0x123 1000 true
    #[clap(long = "args", num_args = 0..)]
    pub args: Vec<String>,

    /// Gas limit for the transaction
    #[clap(long = "gas-limit", default_value = "100000")]
    pub gas_limit: u64,

    /// Gas price in Mist
    #[clap(long = "gas-price", default_value = "0")]
    pub gas_price: u64,

    /// RPC endpoint
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
    // `immediate` execution option removed — always submit normally.
}

impl Call {
    pub fn execute(self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());

        eprintln!("Preparing function call...");

        // Normalize and validate addresses

        // Expect separate flags: `--package <ADDRESS>` and `--module <MODULE>`.
        if self.package.contains("::") {
            anyhow::bail!(
                "Combined form `address::module` is not supported. Use `--package <ADDRESS> --module <MODULE>` instead."
            );
        }

        let package_str = &self.package;
        let module_name = &self.module;

        let sender_normalized = resolve_sender(None)?;
        let package_normalized = normalize_addr(package_str)
            .with_context(|| format!("Invalid package address: {}", package_str))?;

        eprintln!("Call Details:");
        eprintln!("   Package: {}::{}", package_normalized, module_name);
        eprintln!("   Function: {}", self.function);
        eprintln!("   Sender: {}", sender_normalized);
        eprintln!("   Gas Limit: {}", self.gas_limit);
        eprintln!("   Gas Price: {}", self.gas_price);

        // Load wallet (signing is required).
        let wallet = load_wallet_for(&sender_normalized, None)?;
        eprintln!(
            "   Wallet loaded: {} (curve: {})",
            sender_normalized, wallet.curve_type
        );

        let sender_for_tx = get_sender_for_tx(&wallet, &sender_normalized)?;

        // Parse and validate type arguments (keep original string values for RPC but validate them)
        if !self.type_args.is_empty() {
            for type_arg in &self.type_args {
                parser::parse_type_tag(type_arg.trim())
                    .with_context(|| format!("Invalid type argument: {}", type_arg))?;
            }
            eprintln!("   Type Args: {}", self.type_args.join(", "));
        }
        let parsed_type_args = self.type_args.clone();

        // Parse arguments (support typed args and JSON arrays).
        let parsed_args: Vec<Vec<u8>> = self
            .args
            .iter()
            .map(|arg| self.parse_arg_flexible(arg))
            .collect::<Result<_>>()?;

        if !parsed_args.is_empty() {
            eprintln!("   Arguments: {} args provided", parsed_args.len());
        }

        // Estimate gas using runtime `ExecuteFunction`
        let operation = GasOperation::ExecuteFunction { complexity: 1 };
        let estimate = GasEstimate::from_operation(operation, self.gas_price);
        eprintln!("Gas estimation:");
        eprintln!("   Estimated: {} units", estimate.gas_units);
        eprintln!("   Limit: {} units", self.gas_limit);
        eprintln!(
            "   Total Cost: {} Mist ({:.9} KANARI)",
            estimate.total_cost_mist, estimate.total_cost_kanari
        );

        // Create transaction
        eprintln!("Creating transaction...");

        // Query account sequence number so signature and RPC include it (fail-fast)
        let client = build_blocking_client(30)?;
        let seq_num: u64 = get_account_sequence(&client, &rpc, &sender_normalized)?;

        // Sign transaction using the loaded wallet via SignedTransaction
        let signed_tx = {
            // Format module as "address::module_name" for runtime compatibility
            let module_full = format!("{}::{}", package_normalized, module_name);

            // Create proper Transaction to match server's expectation (use parsed args/type_args)
            let transaction = Transaction::ExecuteFunction {
                sender: sender_for_tx.clone(),
                module: module_full.clone(),
                function: self.function.clone(),
                type_args: parsed_type_args.clone(),
                args: parsed_args.clone(),
                gas_limit: self.gas_limit,
                gas_price: self.gas_price,
                sequence_number: seq_num,
            };

            // Wrap and sign using SignedTransaction helper
            let mut stx = SignedTransaction::new(transaction);
            stx.sign(&wallet.private_key, wallet.curve_type)
                .map_err(|e| anyhow::anyhow!("Failed to sign transaction: {}", e))?;
            eprintln!("   Transaction signed (curve: {})", wallet.curve_type);
            stx
        };

        let call_req = CallFunctionRequest {
            sender: sender_for_tx.clone(),
            package: package_normalized.clone(),
            module: module_name.clone(),
            function: self.function.clone(),
            type_args: parsed_type_args.clone(),
            args: parsed_args.clone(),
            gas_limit: self.gas_limit,
            gas_price: self.gas_price,
            sequence_number: seq_num,
            signature: Some(signed_tx.signature.clone()),
            execute_immediate: Some(true),
        };

        let rpc_request = RpcRequest {
            jsonrpc: "2.0".to_string(),
            method: methods::CALL_FUNCTION.to_string(),
            params: serde_json::to_value(call_req).unwrap_or(serde_json::json!(null)),
            id: 1,
        };

        eprintln!("Sending RPC request to {}...", rpc);

        let client = build_blocking_client(30)?;
        match client.post(&rpc).json(&rpc_request).send() {
            Ok(resp) => match resp.json::<RpcResponse>() {
                Ok(rpc_resp) => {
                    if let Some(err) = rpc_resp.error {
                        error!("RPC error: {} (code {})", err.message, err.code);
                    } else if let Some(result) = rpc_resp.result {
                        // Parse result as `TransactionResult` if possible to print summary
                        if let Ok(tx_result) = serde_json::from_value::<
                            kanari_rpc_api::TransactionResult,
                        >(result.clone())
                        {
                            eprintln!("Transaction: {}", tx_result.hash);
                            eprintln!("Status: {}", tx_result.status);
                            eprintln!("Gas used: {} Mist", tx_result.gas_used);

                            if let Some(ref error_msg) = tx_result.error_message {
                                error!("Transaction failed: {}", error_msg);
                            }
                        }

                        match serde_json::to_string_pretty(&result) {
                            Ok(s) => eprintln!("RPC result:\n{}", s),
                            Err(_) => eprintln!("RPC result: {}", result),
                        }
                    } else {
                        eprintln!("RPC response has no result and no error");
                    }
                }
                Err(e) => error!("Failed to parse RPC response: {}", e),
            },
            Err(e) => error!("Failed to send RPC request: {}", e),
        }

        eprintln!("Function call prepared and RPC sent.");
        eprintln!("Next steps:");
        eprintln!(" - Check transaction status");
        eprintln!(" - View execution results on explorer");

        Ok(())
    }

    /// 🛠️ ฟังก์ชันแก้ปัญหา Deserialization
    fn parse_arg_flexible(&self, arg: &str) -> Result<Vec<u8>> {
        let s = arg.trim();

        // 1. จัดการ Vector (สำหรับ NFT Attributes: vector<String>)
        if s.starts_with('[') && s.ends_with(']') {
            let inner = s[1..s.len() - 1].trim();
            let mut elements = Vec::new();
            if !inner.is_empty() {
                for part in inner.split(',') {
                    let clean = part.trim().trim_matches('"').trim_matches('\'');
                    // แปลงเป็น vector<u8> (BCS ของ String) แล้วยัดลง Vector ใหญ่
                    elements.push(MoveValue::Vector(
                        clean.as_bytes().iter().map(|&b| MoveValue::U8(b)).collect(),
                    ));
                }
            }
            // ผลลัพธ์คือ vector<vector<u8>> ซึ่งตรงกับ vector<String> ใน Move
            return MoveValue::Vector(elements)
                .simple_serialize()
                .ok_or_else(|| anyhow::anyhow!("Fail to serialize vector<String>"));
        }

        // 2. จัดการ Address (Hex 0x...)
        if s.starts_with("0x")
            && s.len() > 10
            && let Ok(addr) = parser::parse_transaction_argument(s)
        {
            return MoveValue::from(addr)
                .simple_serialize()
                .context("addr fail");
        }

        // 3. จัดการตัวเลข (u64)
        // กรอง "001" ออกไปให้กลายเป็น String (Fallback)
        if (!s.starts_with('0') || s == "0")
            && let Ok(val) = s.parse::<u64>()
        {
            return MoveValue::U64(val).simple_serialize().context("u64 fail");
        }

        // 4. Fallback: ทุกอย่างที่เหลือให้มองเป็น vector<u8> (Move String)
        // เช่น "KariKid #1", "First NFT", "001", "https://..."
        let bytes = s.as_bytes().to_vec();
        Ok(bcs::to_bytes(&bytes)?)
    }
}
