// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Kanari RPC Client
//!
//! HTTP client for interacting with Kanari RPC server

use anyhow::{Context, Result};
use kanari_rpc_api::*;
use kanari_types::error::KanariUnwrapExt;
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use reqwest::Client;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Deserialize)]
pub struct ObjectRefInfo {
    pub id: String,
    pub owner: String,
    pub type_: String,
    pub data: Vec<u8>,
    pub version: u64,
    pub digest: String,
}

/// RPC client
pub struct RpcClient {
    client: Client,
    url: String,
    request_id: AtomicU64,
}

impl RpcClient {
    /// Create new RPC client
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            url: url.into(),
            request_id: AtomicU64::new(1),
        }
    }

    /// Get next request ID
    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send RPC request
    async fn request(&self, method: &str, params: serde_json::Value) -> Result<RpcResponse> {
        let request = RpcRequest {
            jsonrpc: "2.0".to_string(),
            method: method.to_string(),
            params,
            id: self.next_id(),
        };

        let response = self
            .client
            .post(&self.url)
            .json(&request)
            .send()
            .await
            .context("Failed to send request")?;

        let rpc_response: RpcResponse =
            response.json().await.context("Failed to parse response")?;

        if let Some(error) = rpc_response.error {
            anyhow::bail!("RPC error: {} (code: {})", error.message, error.code);
        }

        Ok(rpc_response)
    }

    /// Get account information
    pub async fn get_account(&self, address: &str) -> Result<AccountInfo> {
        let response = self
            .request(methods::GET_ACCOUNT, serde_json::json!(address))
            .await?;

        let result = response.result.context("No result in response")?;
        serde_json::from_value(result).context("Failed to parse account info")
    }

    /// Get block by height
    pub async fn get_block(&self, height: u64) -> Result<BlockInfo> {
        let response = self
            .request(methods::GET_BLOCK, serde_json::json!(height))
            .await?;

        let result = response.result.context("No result in response")?;
        serde_json::from_value(result).context("Failed to parse block info")
    }

    /// Get full block by height (including transactions and vertices)
    pub async fn get_full_block(&self, height: u64) -> Result<FullBlockData> {
        let response = self
            .request(methods::GET_FULL_BLOCK, serde_json::json!(height))
            .await?;

        let result = response.result.context("No result in response")?;
        serde_json::from_value(result).context("Failed to parse full block data")
    }

    /// Get current block height
    pub async fn get_block_height(&self) -> Result<u64> {
        let response = self
            .request(methods::GET_BLOCK_HEIGHT, serde_json::json!(null))
            .await?;

        let result = response.result.context("No result in response")?;
        serde_json::from_value(result).context("Failed to parse block height")
    }

    /// Get blockchain statistics
    pub async fn get_stats(&self) -> Result<BlockchainStats> {
        let response = self
            .request(methods::GET_STATS, serde_json::json!(null))
            .await?;

        let result = response.result.context("No result in response")?;
        serde_json::from_value(result).context("Failed to parse stats")
    }

    /// Get exact, unaggregated object references owned by an address.
    pub async fn get_owned_object_refs(
        &self,
        owner: &str,
        object_type: Option<String>,
    ) -> Result<Vec<ObjectRefInfo>> {
        let response = self
            .request(
                "kanari_getOwnedObjects",
                serde_json::json!({ "owner": owner, "object_type": object_type }),
            )
            .await?;
        let result = response.result.context("No result in response")?;
        serde_json::from_value(
            result
                .get("objects")
                .cloned()
                .context("Owned object response is missing objects")?,
        )
        .context("Failed to parse exact owned object references")
    }

    pub async fn submit_object_transaction(
        &self,
        transaction: SignedObjectTransaction,
    ) -> Result<TransactionStatus> {
        let response = self
            .request(
                "kanari_submitObjectTransaction",
                serde_json::to_value(transaction)?,
            )
            .await?;
        let result = response.result.context("No result in response")?;
        Ok(TransactionStatus {
            hash: result["digest"]
                .as_str()
                .require("missing object transaction digest")?
                .to_string(),
            status: result["status"]
                .as_str()
                .unwrap_or("pending_consensus")
                .to_string(),
            block_height: None,
            gas_used: None,
        })
    }

    /// Submit signed transaction
    pub async fn submit_transaction(&self, tx: SignedTransactionData) -> Result<TransactionStatus> {
        let response = self
            .request(methods::SUBMIT_TRANSACTION, serde_json::to_value(tx)?)
            .await?;

        let result = response.result.context("No result in response")?;

        let status = TransactionStatus {
            hash: result["hash"]
                .as_str()
                .require("missing transaction hash")?
                .to_string(),
            status: result["status"]
                .as_str()
                .require("missing transaction status")?
                .to_string(),
            block_height: None,
            gas_used: None,
        };

        Ok(status)
    }

    /// Publish Move module
    pub async fn publish_module(&self, request: PublishModuleRequest) -> Result<TransactionStatus> {
        let response = self
            .request(methods::PUBLISH_MODULE, serde_json::to_value(request)?)
            .await?;

        let result = response.result.context("No result in response")?;

        let status = TransactionStatus {
            hash: result["hash"]
                .as_str()
                .require("missing transaction hash")?
                .to_string(),
            status: result["status"]
                .as_str()
                .require("missing transaction status")?
                .to_string(),
            block_height: None,
            gas_used: None,
        };

        Ok(status)
    }

    /// Call Move function
    pub async fn call_function(&self, request: CallFunctionRequest) -> Result<TransactionStatus> {
        let response = self
            .request(methods::CALL_FUNCTION, serde_json::to_value(request)?)
            .await?;

        let result = response.result.context("No result in response")?;

        let status = TransactionStatus {
            hash: result["hash"]
                .as_str()
                .require("missing transaction hash")?
                .to_string(),
            status: result["status"]
                .as_str()
                .require("missing transaction status")?
                .to_string(),
            block_height: None,
            gas_used: None,
        };

        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_creation() {
        let client = RpcClient::new("http://localhost:19001");
        assert_eq!(client.url, "http://localhost:19001");
    }
}
