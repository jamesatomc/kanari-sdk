// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Kanari RPC API Definitions
//!
//! Defines request/response types and RPC methods for Kanari blockchain
use kanari_types::event::Event;
use kanari_types::transaction::SignedTransaction;
use serde::{Deserialize, Serialize};

/// RPC request wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: serde_json::Value,
    pub id: u64,
}

/// RPC response wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    pub id: u64,
}

/// RPC error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl RpcError {
    pub fn internal_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: msg.into(),
            data: None,
        }
    }

    pub fn invalid_params(msg: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: impl Into<String>) -> Self {
        Self {
            code: -32601,
            message: format!("Method not found: {}", method.into()),
            data: None,
        }
    }

    pub fn parse_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32700,
            message: msg.into(),
            data: None,
        }
    }

    pub fn invalid_request(msg: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: msg.into(),
            data: None,
        }
    }

    pub fn module_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32001,
            message: format!("Module error: {}", msg.into()),
            data: None,
        }
    }

    pub fn transaction_error(msg: impl Into<String>) -> Self {
        Self {
            code: -32002,
            message: format!("Transaction error: {}", msg.into()),
            data: None,
        }
    }
}

/// Account info response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub address: String,
    pub sequence_number: u64,
    pub modules: Vec<String>,
    /// Token balances: token_type -> amount
    pub token_balances: std::collections::BTreeMap<String, u64>,
    /// Owned objects discovered by the runtime (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_objects: Option<Vec<ObjectInfo>>,
}

/// Token balance info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBalance {
    pub token_type: String,
    pub amount: u64,
    pub decimals: u8,
    pub symbol: String,
}

/// Checkpoint-backed block view response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInfo {
    pub height: u64,
    pub timestamp: u64,
    pub hash: String,
    pub prev_hash: String,
    pub tx_count: usize,
    pub state_root: String,
    pub events: Vec<RpcEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockchainStats {
    pub height: u64,
    pub total_blocks: usize,
    pub total_transactions: usize,
    pub pending_transactions: usize,
    pub total_accounts: usize,
    pub total_supply: u64,
    pub state_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub id: String,
    pub owner: String,
    pub type_: String,
    pub data: Vec<u8>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockData {
    pub height: u64,
    pub timestamp: u64,
    pub hash: String,
    pub prev_hash: String,
    pub state_root: String,
    pub tx_count: usize,
    pub events: Vec<Event>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullBlockData {
    pub height: u64,
    pub timestamp: u64,
    pub hash: String,
    pub prev_hash: String,
    pub state_root: String,
    pub tx_count: usize,
    pub events: Vec<Event>,
    pub transactions: Vec<SignedTransaction>,
    pub vertices: Vec<String>,
}

/// Request for state root (optional height). If `height` is None, latest root is returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetStateRootRequest {
    pub height: Option<u64>,
}

/// Account state proof returned to light clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProof {
    pub state_root: String,
    pub is_member: bool,
    pub leaf_hash: Vec<u8>,
    pub siblings: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAccountProofRequest {
    pub address: String,
    pub height: Option<u64>,
}

/// Event emitted by Move runtime (RPC representation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEvent {
    pub key: Vec<u8>,
    pub sequence_number: u64,
    pub type_tag: String,
    pub event_data: Vec<u8>,
}

/// Transaction status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionStatus {
    pub hash: String,
    pub status: String,
    pub block_height: Option<u64>,
    pub gas_used: Option<u64>,
}

/// Detailed transaction information returned by `getTransaction` and `getAllTransactions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionDetails {
    pub hash: String,
    pub status: String,
    pub block_height: Option<u64>,
    pub gas_used: Option<u64>,
    pub tx_type: String,
    pub sender: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_address: Option<String>,
    pub sequence_number: u64,
    pub gas_limit: u64,
    pub gas_price: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_functions: Option<Vec<String>>,
}

/// Transaction result with created objects
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionResult {
    pub hash: String,
    pub status: String,
    pub gas_used: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Submit transaction request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitTransactionRequest {
    pub transaction: SignedTransactionData,
}

/// Signed transaction data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedTransactionData {
    pub sender: String,
    pub recipient: Option<String>,
    pub amount: Option<u64>,
    pub gas_limit: u64,
    pub gas_price: u64,
    pub signature: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

/// Publish module request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishModuleRequest {
    pub sender: String,
    pub module_bytes: Vec<u8>,
    pub module_name: String,
    pub gas_limit: u64,
    pub gas_price: u64,
    pub signature: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

/// Call function request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallFunctionRequest {
    pub sender: String,
    pub package: String,
    pub module: String,
    pub function: String,
    pub type_args: Vec<String>,
    pub args: Vec<Vec<u8>>,
    pub gas_limit: u64,
    pub gas_price: u64,
    pub signature: Option<Vec<u8>>,
    pub execute_immediate: Option<bool>,
}

/// View function request (read-only, no transaction submission)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewFunctionRequest {
    pub package: String,
    pub module: String,
    pub function: String,
    pub type_args: Vec<String>,
    #[serde(deserialize_with = "deserialize_args")]
    pub args: Vec<Vec<u8>>,
}

/// Custom deserializer for args that supports both hex strings and byte arrays
fn deserialize_args<'de, D>(deserializer: D) -> Result<Vec<Vec<u8>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

    struct ArgsVisitor;

    impl<'de> Visitor<'de> for ArgsVisitor {
        type Value = Vec<Vec<u8>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("array of hex strings or byte arrays")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut args = Vec::new();
            while let Some(value) = seq.next_element::<serde_json::Value>()? {
                // Try to parse as hex string first
                if let Some(hex_str) = value.as_str() {
                    let hex_str = hex_str.trim_start_matches("0x");
                    let bytes = hex::decode(hex_str).map_err(de::Error::custom)?;
                    args.push(bytes);
                } else if let Some(arr) = value.as_array() {
                    // Parse as array of numbers
                    let bytes: Result<Vec<u8>, _> = arr
                        .iter()
                        .map(|v| {
                            v.as_u64()
                                .ok_or_else(|| de::Error::custom("Expected number"))
                        })
                        .map(|r| r.map(|n| n as u8))
                        .collect();
                    args.push(bytes?);
                } else {
                    return Err(de::Error::custom("Expected string or array"));
                }
            }
            Ok(args)
        }
    }

    deserializer.deserialize_seq(ArgsVisitor)
}

/// Module query response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub address: String,
    pub name: String,
    pub bytecode_hash: String,
    pub size: usize,
    pub dependencies: Vec<String>,
}

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub sync_status: String,
    pub network: String,
    pub supply_invariants_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supply_invariant_error: Option<String>,
    pub fail_fast_enabled: bool,
    pub strict_persistence_required: bool,
    pub strict_checkpoint_roots: bool,
    pub persistent_storage_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkAuthorityStatus {
    pub authority_id: String,
    pub local: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub local_authority_id: String,
    pub authority_count: usize,
    pub authorities: Vec<NetworkAuthorityStatus>,
}

/// Get object request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetObjectRequest {
    pub object_id: String,
}

/// Get owned objects request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOwnedObjectsRequest {
    pub owner: String,
    pub object_type: Option<String>,
}

/// Get objects by type request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetObjectsByTypeRequest {
    pub object_type: String,
}

/// Get token balance request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTokenBalanceRequest {
    pub address: String,
    pub token_type: String,
}

/// Get all balances request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetAllBalancesRequest {
    pub address: String,
}

/// RPC Methods
#[kanari_open_rpc::open_rpc]
pub mod methods {
    use kanari_open_rpc::{
        object_schema, optional_schema, schema_array, schema_integer, schema_object, schema_string,
    };

    // Account & Balance
    #[open_rpc_method(
        summary = "Get account info",
        description = "Returns account data including sequence number, balances, modules, and owned objects.",
        params = [("address", "Target account address.", true, schema_string())],
        result = ("account", "Account information object.", schema_object()),
        tags = ["account", "balance"]
    )]
    pub const GET_ACCOUNT: &str = "kanari_getAccount";
    #[open_rpc_method(
        summary = "Get one token balance",
        description = "Returns the balance of a specific token type for one account.",
        params = [(
            "request",
            "Address and token type payload.",
            true,
            object_schema(&[("address", schema_string()), ("token_type", schema_string())])
        )],
        result = ("balance", "Token balance payload.", schema_object()),
        tags = ["account", "balance"]
    )]
    pub const GET_TOKEN_BALANCE: &str = "kanari_getTokenBalance";
    #[open_rpc_method(
        summary = "Get all balances",
        description = "Returns all visible balances for one account.",
        params = [(
            "request",
            "Address payload.",
            true,
            object_schema(&[("address", schema_string())])
        )],
        result = ("balances", "All account balances.", schema_object()),
        tags = ["account", "balance"]
    )]
    pub const GET_ALL_BALANCES: &str = "kanari_getAllBalances";
    #[open_rpc_method(
        summary = "List tokens",
        description = "Returns visible token metadata and supply summaries.",
        params = [],
        result = ("tokens", "List of tracked token summaries.", schema_array(schema_object())),
        tags = ["balance"]
    )]
    pub const LIST_TOKENS: &str = "kanari_listTokens";

    // Blocks & Transactions
    #[open_rpc_method(
        summary = "Get block",
        description = "Returns a simplified block view by height.",
        params = [("height", "Block height.", true, schema_integer())],
        result = ("block", "Block summary.", schema_object()),
        tags = ["block"]
    )]
    pub const GET_BLOCK: &str = "kanari_getBlock";
    #[open_rpc_method(
        summary = "Get full block",
        description = "Returns full block contents including transactions.",
        params = [("height", "Block height.", true, schema_integer())],
        result = ("block", "Full block payload.", schema_object()),
        tags = ["block"]
    )]
    pub const GET_FULL_BLOCK: &str = "kanari_getFullBlock";
    #[open_rpc_method(
        summary = "Get current block height",
        description = "Returns the latest committed block height.",
        params = [],
        result = ("height", "Current chain height.", schema_integer()),
        tags = ["block"]
    )]
    pub const GET_BLOCK_HEIGHT: &str = "kanari_getBlockHeight";
    #[open_rpc_method(
        summary = "Get transaction",
        description = "Returns a committed or pending transaction by hash.",
        params = [(
            "hash",
            "Transaction hash string, with or without 0x prefix.",
            true,
            schema_string()
        )],
        result = ("transaction", "Transaction details.", schema_object()),
        tags = ["transaction"]
    )]
    pub const GET_TRANSACTION: &str = "kanari_getTransaction";
    #[open_rpc_method(
        summary = "List transactions",
        description = "Returns recent committed and pending transactions, with optional filtering.",
        params = [(
            "request",
            "Optional list options such as limit or account.",
            false,
            schema_object()
        )],
        result = ("transactions", "Transaction detail list.", schema_array(schema_object())),
        tags = ["transaction"]
    )]
    pub const GET_ALL_TRANSACTIONS: &str = "kanari_getAllTransactions";
    #[open_rpc_method(
        summary = "Submit transaction",
        description = "Submits a transfer or burn transaction to the mempool.",
        params = [("transaction", "Signed transaction payload.", true, schema_object())],
        result = ("submission", "Submission status payload.", schema_object()),
        tags = ["transaction"]
    )]
    pub const SUBMIT_TRANSACTION: &str = "kanari_submitTransaction";

    // Stats & Info
    #[open_rpc_method(
        summary = "Get chain stats",
        description = "Returns aggregate blockchain statistics.",
        params = [],
        result = ("stats", "Blockchain statistics.", schema_object()),
        tags = ["system"]
    )]
    pub const GET_STATS: &str = "kanari_getStats";
    pub const ESTIMATE_GAS: &str = "kanari_estimateGas";
    #[open_rpc_method(
        summary = "Health check",
        description = "Returns runtime health and guard configuration.",
        params = [],
        result = ("health", "Health status payload.", schema_object()),
        tags = ["system"]
    )]
    pub const HEALTH: &str = "kanari_health";

    #[open_rpc_method(
        summary = "Get network authority status",
        description = "Returns the configured validator authority set for this node.",
        params = [],
        result = ("network", "Network authority status payload.", schema_object()),
        tags = ["system"]
    )]
    pub const GET_NETWORK_STATUS: &str = "kanari_getNetworkStatus";

    // Module operations
    #[open_rpc_method(
        summary = "Publish module",
        description = "Publishes Move module bytecode.",
        params = [("module", "Module publication payload.", true, schema_object())],
        result = ("publication", "Publish execution or pending status.", schema_object()),
        tags = ["module"]
    )]
    pub const PUBLISH_MODULE: &str = "kanari_publishModule";
    #[open_rpc_method(
        summary = "Get module",
        description = "Returns module metadata by address and module name.",
        params = [(
            "request",
            "Address and module name payload.",
            true,
            object_schema(&[("address", schema_string()), ("name", schema_string())])
        )],
        result = ("module", "Module metadata.", schema_object()),
        tags = ["module"]
    )]
    pub const GET_MODULE: &str = "kanari_getModule";
    #[open_rpc_method(
        summary = "List modules",
        description = "Lists all known published modules.",
        params = [],
        result = ("modules", "Module metadata list.", schema_array(schema_object())),
        tags = ["module"]
    )]
    pub const LIST_MODULES: &str = "kanari_listModules";
    #[open_rpc_method(
        summary = "Verify module bytecode",
        description = "Deserializes module bytes and returns basic verification outcome.",
        params = [(
            "request",
            "Module byte payload.",
            true,
            object_schema(&[("module_bytes", schema_array(schema_integer()))])
        )],
        result = ("verification", "Verification result.", schema_object()),
        tags = ["module"]
    )]
    pub const VERIFY_MODULE: &str = "kanari_verifyModule";

    // Function calls
    #[open_rpc_method(
        summary = "Call function",
        description = "Submits or executes a Move entry function transaction.",
        params = [("call", "Function call payload.", true, schema_object())],
        result = ("call_result", "Execution or pending result.", schema_object()),
        tags = ["function"]
    )]
    pub const CALL_FUNCTION: &str = "kanari_callFunction";
    #[open_rpc_method(
        summary = "View function",
        description = "Executes a read-only Move function without submitting a transaction.",
        params = [(
            "request",
            "Array containing one view-function payload.",
            true,
            schema_array(schema_object())
        )],
        result = ("view_result", "Read-only execution result.", schema_object()),
        tags = ["function"]
    )]
    pub const VIEW_FUNCTION: &str = "kanari_viewFunction";

    // Object queries
    #[open_rpc_method(
        summary = "Get object",
        description = "Returns one on-chain object by id.",
        params = [(
            "request",
            "Object id payload.",
            true,
            object_schema(&[("object_id", schema_string())])
        )],
        result = ("object", "Object info payload.", schema_object()),
        tags = ["object"]
    )]
    pub const GET_OBJECT: &str = "kanari_getObject";
    #[open_rpc_method(
        summary = "Get owned objects",
        description = "Returns owned objects for one address, optionally filtered by object type.",
        params = [(
            "request",
            "Owner and optional type filter payload.",
            true,
            object_schema(&[
                ("owner", schema_string()),
                ("object_type", optional_schema(schema_string()))
            ])
        )],
        result = ("objects", "Owned object list.", schema_object()),
        tags = ["object"]
    )]
    pub const GET_OWNED_OBJECTS: &str = "kanari_getOwnedObjects";

    // NFT queries
    #[open_rpc_method(
        summary = "Get owned NFTs",
        description = "Returns NFT-like objects owned by one address.",
        params = [("address", "Target owner address.", true, schema_string())],
        result = ("nfts", "Owned NFT list.", schema_array(schema_object())),
        tags = ["nft"]
    )]
    pub const GET_OWNED_NFTS: &str = "kanari_getOwnedNfts";

    // NFT collections
    #[open_rpc_method(
        summary = "List NFT collections",
        description = "Returns indexed NFT collections.",
        params = [],
        result = ("collections", "Collection list.", schema_array(schema_object())),
        tags = ["nft"]
    )]
    pub const LIST_COLLECTIONS: &str = "kanari_listCollections";
    #[open_rpc_method(
        summary = "Get NFTs by collection",
        description = "Returns NFTs that belong to one collection id.",
        params = [("id", "Collection id.", true, schema_string())],
        result = ("nfts", "Collection NFT list.", schema_array(schema_object())),
        tags = ["nft"]
    )]
    pub const GET_NFTS_BY_COLLECTION: &str = "kanari_getNftsByCollection";
}
