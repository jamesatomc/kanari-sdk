// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::{
    build_blocking_client, get_rpc_endpoint, get_sender_for_tx, load_wallet_for, normalize_addr,
    resolve_sender, resolve_transaction_gas,
};
use crate::command::rpc_helpers::{
    render_transaction_submission, require_rpc_result, sign_call_function_request,
    submit_blocking_rpc,
};
use anyhow::{Context, Result};
use clap::*;
use kanari_rpc_api::{BuildCallFunctionRequest, CallFunctionRequest, methods};
use kanari_types::error::KanariUnwrapExt;
use kanari_types::{GasEstimate, GasOperation};
use move_core_types::{account_address::AccountAddress, parser, runtime_value::MoveValue};

/// Call a Move function on the blockchain
#[derive(Parser)]
#[clap(
    name = "call",
    about = "Call a Move function on the blockchain",
    after_help = "Examples:\n  kanari call --package 0x1 --module coin --function transfer --args 0x2 1000\n  kanari call --package 0x1 --module nft --function mutate --args 0xabc \"new name\"\n"
)]
pub struct Call {
    #[clap(long = "package")]
    pub package: String,
    #[clap(long = "module", value_name = "MODULE")]
    pub module: String,
    #[clap(long = "function")]
    pub function: String,
    #[clap(long = "type-args")]
    pub type_args: Vec<String>,
    #[clap(long = "args", num_args = 0..)]
    pub args: Vec<String>,
    #[clap(long = "gas-limit")]
    pub gas_limit: Option<u64>,
    #[clap(long = "gas-price")]
    pub gas_price: Option<u64>,
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
}

impl Call {
    pub fn execute(self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let (gas_limit, gas_price) = resolve_transaction_gas(self.gas_limit, self.gas_price);

        if self.package.contains("::") {
            anyhow::bail!(
                "Combined form `address::module` is not supported. Use `--package <ADDRESS> --module <MODULE>` instead."
            );
        }

        eprintln!("Preparing function call...");

        let sender_normalized = resolve_sender(None)?;
        let package_normalized = normalize_addr(&self.package)
            .with_context(|| format!("Invalid package address: {}", self.package))?;
        let client = build_blocking_client(30)?;

        eprintln!("Call Details:");
        eprintln!("   Package: {}::{}", package_normalized, self.module);
        eprintln!("   Function: {}", self.function);
        eprintln!("   Sender: {}", sender_normalized);
        eprintln!("   Gas Limit: {}", gas_limit);
        eprintln!("   Gas Price: {}", gas_price);

        let wallet = load_wallet_for(&sender_normalized, None)?;
        eprintln!(
            "   Wallet loaded: {} (curve: {})",
            sender_normalized, wallet.curve_type
        );
        let sender_for_tx = get_sender_for_tx(&wallet, &sender_normalized)?;

        if !self.type_args.is_empty() {
            for type_arg in &self.type_args {
                parser::parse_type_tag(type_arg.trim())
                    .with_context(|| format!("Invalid type argument: {}", type_arg))?;
            }
            eprintln!("   Type Args: {}", self.type_args.join(", "));
        }

        let parsed_args: Vec<Vec<u8>> = self
            .args
            .iter()
            .map(|arg| self.parse_arg_flexible(arg))
            .collect::<Result<_>>()?;
        if !parsed_args.is_empty() {
            eprintln!("   Arguments: {} args provided", parsed_args.len());
        }

        let estimate =
            GasEstimate::from_operation(GasOperation::ExecuteFunction { complexity: 1 }, gas_price);
        eprintln!("Gas estimation:");
        eprintln!("   Estimated: {} units", estimate.gas_units);
        eprintln!("   Limit: {} units", gas_limit);
        eprintln!(
            "   Total Cost: {} Mist ({:.9} KANARI)",
            estimate.total_cost_mist, estimate.total_cost_kanari
        );

        eprintln!("Creating transaction via API...");
        let prepared = submit_blocking_rpc(
            &client,
            &rpc,
            methods::BUILD_CALL_FUNCTION,
            serde_json::to_value(BuildCallFunctionRequest {
                sender: sender_for_tx.clone(),
                package: package_normalized,
                module: self.module.clone(),
                function: self.function.clone(),
                type_args: self.type_args.clone(),
                args: parsed_args,
                object_inputs: None,
                gas_limit,
                gas_price,
                nonce: None,
                execute_immediate: Some(true),
            })
            .context("Failed to serialize build call request")?,
        )?;
        let prepared: CallFunctionRequest = serde_json::from_value(require_rpc_result(
            prepared,
            "RPC did not return prepared call request",
        )?)
        .context("Failed to decode prepared call request")?;

        if let Some(object_inputs) = &prepared.object_inputs
            && !object_inputs.is_empty()
        {
            eprintln!(
                "   Object Inputs: {} API-resolved object ref(s)",
                object_inputs.len()
            );
        }
        if let Some(gas_payment) = &prepared.gas_payment
            && let Some(gas_object) = gas_payment.payment_objects.first()
        {
            eprintln!("   Gas payment object: {}", gas_object.object_id);
        }

        let signed = sign_call_function_request(prepared, &wallet)?;
        eprintln!("   Transaction signed (curve: {})", wallet.curve_type);

        eprintln!("Sending RPC request to {}...", rpc);
        let rpc_response = submit_blocking_rpc(
            &client,
            &rpc,
            methods::CALL_FUNCTION,
            serde_json::to_value(signed).context("Failed to serialize call request")?,
        )?;
        render_transaction_submission(&client, &rpc, rpc_response, "", true)?;
        Ok(())
    }

    fn parse_arg_flexible(&self, arg: &str) -> Result<Vec<u8>> {
        let s = arg.trim();

        if s.starts_with('[') && s.ends_with(']') {
            let inner = s[1..s.len() - 1].trim();
            let mut elements = Vec::new();
            if !inner.is_empty() {
                for part in inner.split(',') {
                    let clean = part.trim().trim_matches('"').trim_matches('\'');
                    elements.push(MoveValue::Vector(
                        clean.as_bytes().iter().map(|&b| MoveValue::U8(b)).collect(),
                    ));
                }
            }
            return MoveValue::Vector(elements)
                .simple_serialize()
                .require("Fail to serialize vector<String>");
        }

        let hex_part = if let Some(stripped) = s.strip_prefix('x') {
            if !stripped.starts_with("0x") {
                Some(stripped)
            } else {
                None
            }
        } else {
            s.strip_prefix("0x")
        };

        if let Some(raw_hex) = hex_part
            && raw_hex.len() == AccountAddress::LENGTH * 2
            && raw_hex.chars().all(|c| c.is_ascii_hexdigit())
        {
            let obj_id = AccountAddress::from_hex_literal(&format!("0x{}", raw_hex))
                .require("Invalid object ID format")?;
            eprintln!("[CLI] object-id argument detected: 0x{}", raw_hex);
            return Ok(obj_id.to_vec());
        }

        if s.starts_with("0x")
            && s.len() > 10
            && let Ok(addr) = parser::parse_transaction_argument(s)
        {
            return MoveValue::from(addr)
                .simple_serialize()
                .context("addr fail");
        }

        if (!s.starts_with('0') || s == "0")
            && let Ok(val) = s.parse::<u64>()
        {
            return MoveValue::U64(val).simple_serialize().context("u64 fail");
        }

        Ok(bcs::to_bytes(&s.as_bytes().to_vec())?)
    }
}
