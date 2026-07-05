// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::{build_blocking_client, get_rpc_endpoint};
use crate::command::move_cli::reroot_path;

use anyhow::{Context, Result, bail};
use clap::Parser;
use kanari_types::error::KanariUnwrapExt;
use move_package::BuildConfig;
use std::path::PathBuf;

/// Verify a Move module's bytecode via RPC verifier
#[derive(Parser)]
#[clap(name = "verify")]
pub struct Verify {
    /// Path to a compiled module file (.mv). If omitted, `--package-path` will be built and first module verified.
    #[clap(long = "file")]
    pub file: Option<PathBuf>,

    /// Path to the Move package to build and verify the first module found
    #[clap(long = "package-path")]
    pub package_path: Option<PathBuf>,

    /// RPC endpoint
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
}

impl Verify {
    pub fn execute(self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());

        // Build blocking client
        let client = build_blocking_client(30)?;

        // Determine module bytes either from file or by building package
        let module_bytes: Vec<u8> = if let Some(fp) = self.file {
            std::fs::read(&fp).with_context(|| format!("Failed to read module file: {:?}", fp))?
        } else if let Some(pkg) = self.package_path {
            // Build package and take first compiled module bytes
            let rooted = reroot_path(Some(pkg))?;
            let config = BuildConfig::default();
            let compiled = config.compile_package(&rooted, &mut std::io::stderr())?;
            let mut found = None;
            if let Some(mu) = compiled.all_modules().next() {
                let mut b = vec![];
                mu.unit.module.serialize(&mut b)?;
                found = Some(b);
            }
            found.require("No modules found in package")?
        } else {
            bail!("Either --file or --package-path must be provided")
        };

        use kanari_rpc_api::{RpcRequest, RpcResponse, methods};

        let req = RpcRequest {
            jsonrpc: "2.0".to_string(),
            method: methods::VERIFY_MODULE.to_string(),
            params: serde_json::json!({ "module_bytes": module_bytes }),
            id: 1,
        };

        eprintln!("Sending verify request to {}...", rpc);
        let resp = client
            .post(&rpc)
            .json(&req)
            .send()
            .context("Failed to send RPC request")?;

        let rpc_resp: RpcResponse = resp.json().context("Failed to parse RPC response")?;
        if let Some(err) = rpc_resp.error {
            bail!("RPC error {}: {}", err.code, err.message);
        }

        if let Some(result) = rpc_resp.result {
            eprintln!("Verify result: {}", result);
        } else {
            eprintln!("No result returned from verifier");
        }

        Ok(())
    }
}
