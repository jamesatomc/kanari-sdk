// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Kanari RPC API Definitions
//!
//! Defines request/response types and RPC methods for Kanari blockchain
use kanari_types::event::Event;
use kanari_types::transaction::{
    GasPayment, ObjectChange, ObjectGraphEdge, ObjectInput, ObjectOwnerKind, ObjectRef,
    SignedTransaction, TransactionEffects,
};
use serde::{Deserialize, Serialize};
use std::fmt;

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

/// Structured transaction error reasons that clients can rely on instead of parsing text.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransactionErrorReason {
    InvalidGasPaymentType,
    GasPaymentObjectOverlap,
    GasPaymentObjectNotFound,
    GasPaymentOwnerMismatch,
    GasPaymentVersionMismatch,
    GasPaymentDigestMismatch,
    InsufficientNativeCoinObjects,
    NativeCoinConsolidationBlocked,
    InsufficientTransferCoinBalance,
    InsufficientGasCoinBalance,
    NativeTransferPolicyNotSatisfied,
}

impl TransactionErrorReason {
    pub fn uses_native_transfer_policy(self) -> bool {
        matches!(
            self,
            Self::InsufficientNativeCoinObjects
                | Self::NativeCoinConsolidationBlocked
                | Self::InsufficientTransferCoinBalance
                | Self::InsufficientGasCoinBalance
                | Self::NativeTransferPolicyNotSatisfied
        )
    }
}

impl fmt::Display for TransactionErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::InvalidGasPaymentType => "invalid_gas_payment_type",
            Self::GasPaymentObjectOverlap => "gas_payment_object_overlap",
            Self::GasPaymentObjectNotFound => "gas_payment_object_not_found",
            Self::GasPaymentOwnerMismatch => "gas_payment_owner_mismatch",
            Self::GasPaymentVersionMismatch => "gas_payment_version_mismatch",
            Self::GasPaymentDigestMismatch => "gas_payment_digest_mismatch",
            Self::InsufficientNativeCoinObjects => "insufficient_native_coin_objects",
            Self::NativeCoinConsolidationBlocked => "native_coin_consolidation_blocked",
            Self::InsufficientTransferCoinBalance => "insufficient_transfer_coin_balance",
            Self::InsufficientGasCoinBalance => "insufficient_gas_coin_balance",
            Self::NativeTransferPolicyNotSatisfied => "native_transfer_policy_not_satisfied",
        };
        f.write_str(label)
    }
}

/// Canonical native-transfer policy shared across RPC server, CLI, and faucet tooling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeTransferPolicyContract {
    /// The backend selects canonical object refs and gas payment objects for clients.
    pub api_selects_objects: bool,
    /// Prepared objects must include canonical version and digest metadata.
    pub canonical_object_refs_required: bool,
    /// Gas payment objects must be native `Coin<0x2::kanari::KANARI>` objects.
    pub gas_payment_must_be_native_kanari: bool,
    /// A single native coin cannot be reused for both transfer and gas.
    pub allows_single_coin_for_transfer_and_gas: bool,
    /// Native transfer requires distinct transfer/gas objects.
    pub allows_distinct_transfer_and_gas_objects: bool,
}

impl NativeTransferPolicyContract {
    pub fn kanari_native() -> Self {
        Self {
            api_selects_objects: true,
            canonical_object_refs_required: true,
            gas_payment_must_be_native_kanari: true,
            allows_single_coin_for_transfer_and_gas: false,
            allows_distinct_transfer_and_gas_objects: true,
        }
    }

    pub fn summary(&self) -> &'static str {
        "two distinct Coin<0x2::kanari::KANARI> objects: one mutable transfer input and one separate gas payment object"
    }
}

/// Structured transaction error payload carried in `RpcError.data`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionErrorData {
    pub reason: TransactionErrorReason,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_transfer_policy: Option<NativeTransferPolicyContract>,
}

impl TransactionErrorData {
    pub fn new(reason: TransactionErrorReason) -> Self {
        Self {
            reason,
            native_transfer_policy: None,
        }
    }

    pub fn with_native_transfer_policy(reason: TransactionErrorReason) -> Self {
        Self {
            reason,
            native_transfer_policy: Some(NativeTransferPolicyContract::kanari_native()),
        }
    }
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

    pub fn transaction_error_with_data(msg: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            code: -32002,
            message: format!("Transaction error: {}", msg.into()),
            data: Some(data),
        }
    }

    pub fn transaction_error_structured(
        msg: impl Into<String>,
        data: TransactionErrorData,
    ) -> Self {
        Self::transaction_error_with_data(
            msg,
            serde_json::to_value(data).unwrap_or_else(|_| serde_json::json!({})),
        )
    }

    pub fn transaction_error_details(&self) -> Option<TransactionErrorData> {
        self.data
            .clone()
            .and_then(|data| serde_json::from_value(data).ok())
    }

    pub fn transaction_error_reason(&self) -> Option<TransactionErrorReason> {
        self.transaction_error_details().map(|data| data.reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_transfer_policy_contract_is_backend_canonical() {
        let policy = NativeTransferPolicyContract::kanari_native();

        assert!(policy.api_selects_objects);
        assert!(policy.canonical_object_refs_required);
        assert!(policy.gas_payment_must_be_native_kanari);
        assert!(!policy.allows_single_coin_for_transfer_and_gas);
        assert!(policy.allows_distinct_transfer_and_gas_objects);
        assert!(policy.summary().contains("Coin<0x2::kanari::KANARI>"));
    }

    #[test]
    fn structured_transaction_errors_mark_policy_reasons_only() {
        assert!(
            TransactionErrorReason::InsufficientNativeCoinObjects.uses_native_transfer_policy()
        );
        assert!(
            TransactionErrorReason::NativeCoinConsolidationBlocked.uses_native_transfer_policy()
        );
        assert!(
            TransactionErrorReason::InsufficientTransferCoinBalance.uses_native_transfer_policy()
        );
        assert!(TransactionErrorReason::InsufficientGasCoinBalance.uses_native_transfer_policy());
        assert!(
            TransactionErrorReason::NativeTransferPolicyNotSatisfied.uses_native_transfer_policy()
        );

        assert!(!TransactionErrorReason::GasPaymentObjectOverlap.uses_native_transfer_policy());
        assert!(!TransactionErrorReason::GasPaymentDigestMismatch.uses_native_transfer_policy());
    }

    #[test]
    fn rpc_error_round_trips_structured_transaction_reason() {
        let error = RpcError::transaction_error_structured(
            "native transfer policy not satisfied",
            TransactionErrorData::with_native_transfer_policy(
                TransactionErrorReason::NativeTransferPolicyNotSatisfied,
            ),
        );

        assert_eq!(
            error.transaction_error_reason(),
            Some(TransactionErrorReason::NativeTransferPolicyNotSatisfied)
        );
        assert!(
            error
                .transaction_error_details()
                .and_then(|details| details.native_transfer_policy)
                .is_some()
        );
    }
}

/// Owner info response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerInfo {
    pub owner: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub nonce: Option<u64>,
    pub modules: Vec<String>,
    pub balances: std::collections::BTreeMap<String, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owned_object_count: Option<usize>,
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
    pub total_owners: usize,
    pub total_supply: u64,
    pub state_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetSmtStatusRequest {
    /// Run a full persisted-leaf and canonical-root audit. This can be
    /// expensive on a large database and defaults to false.
    #[serde(default)]
    pub audit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmtStatusResponse {
    pub height: u64,
    pub checkpoint_state_root: String,
    pub enabled: bool,
    pub persisted_root: Option<String>,
    pub effective_root: String,
    pub overlay_entries: usize,
    pub overlay_updates: usize,
    pub overlay_deletes: usize,
    pub canonical_membership_changed: bool,
    pub runtime_schema_version: Option<u32>,
    pub expected_runtime_schema_version: u32,
    pub wallet_supply_index_version: Option<u32>,
    pub expected_wallet_supply_index_version: u32,
    pub audit_requested: bool,
    pub audit_performed: bool,
    pub persisted_leaf_count: Option<usize>,
    pub consistent: Option<bool>,
    pub consistency_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalStateEntry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GetCanonicalStateSnapshotRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalStateSnapshotResponse {
    pub height: u64,
    pub state_root: String,
    pub entry_count: usize,
    pub entries: Vec<CanonicalStateEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompareCanonicalStateSnapshotRequest {
    pub entries: Vec<CanonicalStateEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalStateDiffResponse {
    pub height: u64,
    pub state_root: String,
    pub local_entry_count: usize,
    pub remote_entry_count: usize,
    pub first_divergence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectInfo {
    pub id: String,
    pub owner: String,
    pub owner_kind: ObjectOwnerKind,
    pub type_: String,
    pub data: Vec<u8>,
    pub version: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RpcObjectOwnerKindFilter {
    Address,
    Shared,
    Immutable,
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
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub transaction_effects: Vec<TransactionEffects>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub object_changes: Vec<ObjectChange>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub object_graph_edges: Vec<ObjectGraphEdge>,
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
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub transaction_effects: Vec<TransactionEffects>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub object_changes: Vec<ObjectChange>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub object_graph_edges: Vec<ObjectGraphEdge>,
    pub vertices: Vec<String>,
}

/// Request for state root (optional height). If `height` is None, latest root is returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetStateRootRequest {
    pub height: Option<u64>,
}

/// Owner state proof returned to light clients
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerProof {
    pub state_root: String,
    pub is_member: bool,
    pub leaf_hash: Vec<u8>,
    pub siblings: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOwnerProofRequest {
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
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub previewed: bool,
    #[serde(default)]
    pub submitted: bool,
    #[serde(default)]
    pub committed: bool,
}

/// Detailed transaction information returned by `getTransaction` and `getAllTransactions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionDetails {
    pub hash: String,
    pub status: String,
    pub block_height: Option<u64>,
    pub gas_used: Option<u64>,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub previewed: bool,
    #[serde(default)]
    pub submitted: bool,
    #[serde(default)]
    pub committed: bool,
    pub tx_type: String,
    pub sender: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    pub gas_limit: u64,
    pub gas_price: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_inputs: Option<Vec<ObjectInput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_payment: Option<GasPayment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<TransactionEffects>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_used: Option<u64>,
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub previewed: bool,
    #[serde(default)]
    pub submitted: bool,
    #[serde(default)]
    pub committed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effects: Option<TransactionEffects>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Submit object transfer request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitObjectTransferRequest {
    pub transaction: ObjectTransferData,
}

/// Build native transfer request.
///
/// Native KANARI transfer follows the shared `NativeTransferPolicyContract`:
/// the backend chooses canonical object refs and requires distinct native
/// transfer/gas coin objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildNativeTransferRequest {
    pub sender: String,
    pub recipient: String,
    pub amount: u64,
    pub gas_limit: u64,
    pub gas_price: u64,
    /// Object ids the caller already has in-flight locally and does not want
    /// the backend selector to reuse for this build. This is advisory and is
    /// combined with the node's pending access-key snapshot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_object_ids: Vec<String>,
    /// Preferred replay nonce field. If omitted, RPC generates one from OS randomness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

/// Build one native coin consolidation step that joins another native coin into a primary coin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildNativeCoinConsolidationRequest {
    pub sender: String,
    pub required_amount: u64,
    pub gas_limit: u64,
    pub gas_price: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

/// Object transfer submission data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectTransferData {
    pub sender: String,
    pub coin_object_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coin_object_ref: Option<ObjectRef>,
    pub recipient: String,
    pub amount: u64,
    pub gas_limit: u64,
    pub gas_price: u64,
    /// Canonical replay nonce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_payment: Option<GasPayment>,
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
    /// Canonical replay nonce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_payment: Option<GasPayment>,
    pub signature: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishPackageModule {
    pub module_name: String,
    pub module_bytes: Vec<u8>,
}

/// Publish package request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishPackageRequest {
    pub sender: String,
    pub modules: Vec<PublishPackageModule>,
    pub gas_limit: u64,
    pub gas_price: u64,
    /// Canonical replay nonce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_payment: Option<GasPayment>,
    pub signature: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

/// Build publish module request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildPublishModuleRequest {
    pub sender: String,
    pub module_bytes: Vec<u8>,
    pub module_name: String,
    pub gas_limit: u64,
    pub gas_price: u64,
    /// Preferred replay nonce field. If omitted, RPC generates one from OS randomness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

/// Build publish package request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildPublishPackageRequest {
    pub sender: String,
    pub modules: Vec<PublishPackageModule>,
    pub gas_limit: u64,
    pub gas_price: u64,
    /// Preferred replay nonce field. If omitted, RPC generates one from OS randomness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_inputs: Option<Vec<ObjectInput>>,
    pub gas_limit: u64,
    pub gas_price: u64,
    /// Canonical replay nonce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_payment: Option<GasPayment>,
    pub signature: Option<Vec<u8>>,
    pub execute_immediate: Option<bool>,
}

fn canonical_request_nonce(nonce: Option<u64>) -> Result<u64, String> {
    if let Some(nonce) = nonce {
        if nonce == 0 {
            return Err("Prepared request nonce must be non-zero".to_string());
        }
        return Ok(nonce);
    }

    Err("Prepared request is missing canonical nonce".to_string())
}

impl ObjectTransferData {
    pub fn canonical_nonce(&self) -> Result<u64, String> {
        canonical_request_nonce(self.nonce)
    }

    pub fn require_nonce(&mut self) -> Result<u64, String> {
        let nonce = self.canonical_nonce()?;
        self.nonce = Some(nonce);
        Ok(nonce)
    }
}

impl PublishModuleRequest {
    pub fn canonical_nonce(&self) -> Result<u64, String> {
        canonical_request_nonce(self.nonce)
    }

    pub fn require_nonce(&mut self) -> Result<u64, String> {
        let nonce = self.canonical_nonce()?;
        self.nonce = Some(nonce);
        Ok(nonce)
    }
}

impl PublishPackageRequest {
    pub fn canonical_nonce(&self) -> Result<u64, String> {
        canonical_request_nonce(self.nonce)
    }

    pub fn require_nonce(&mut self) -> Result<u64, String> {
        let nonce = self.canonical_nonce()?;
        self.nonce = Some(nonce);
        Ok(nonce)
    }
}

impl CallFunctionRequest {
    pub fn canonical_nonce(&self) -> Result<u64, String> {
        canonical_request_nonce(self.nonce)
    }

    pub fn require_nonce(&mut self) -> Result<u64, String> {
        let nonce = self.canonical_nonce()?;
        self.nonce = Some(nonce);
        Ok(nonce)
    }
}

/// Build call function request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildCallFunctionRequest {
    pub sender: String,
    pub package: String,
    pub module: String,
    pub function: String,
    pub type_args: Vec<String>,
    pub args: Vec<Vec<u8>>,
    pub gas_limit: u64,
    pub gas_price: u64,
    /// Preferred replay nonce field. If omitted, RPC generates one from OS randomness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

/// Build token transfer request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildTokenTransferRequest {
    pub sender: String,
    pub recipient: String,
    pub token_type: String,
    pub amount: u64,
    pub gas_limit: u64,
    pub gas_price: u64,
    /// Preferred replay nonce field. If omitted, RPC generates one from OS randomness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute_immediate: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnedObjectsResponse {
    pub objects: Vec<ObjectInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<OwnerObjectSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerObjectSummary {
    pub owner: String,
    pub total_objects: usize,
    pub object_changes_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectChangeSet {
    pub changes: Vec<ObjectChange>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_inputs: Option<Vec<ObjectInput>>,
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
    #[serde(default)]
    pub functions: Vec<String>,
    pub function_count: usize,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetObjectsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_kind: Option<RpcObjectOwnerKindFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_version: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetObjectByRefRequest {
    pub object_ref: ObjectRef,
}

/// Get token balance request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTokenBalanceRequest {
    pub owner: String,
    pub token_type: String,
}

/// Get all balances for an owner request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetOwnerBalancesRequest {
    pub owner: String,
}

/// Get fungible asset summary request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetFungibleAssetRequest {
    pub token_type: String,
}

/// Get fungible asset holder list request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetFungibleAssetHoldersRequest {
    pub token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

/// Get transactions that involve a fungible asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetFungibleAssetTransactionsRequest {
    pub token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FungibleAssetInfo {
    pub token_type: String,
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub total_supply: u64,
    pub wallet_visible_supply: u64,
    pub circulating_supply: u64,
    pub object_locked_supply: u64,
    pub accounted_supply: u64,
    pub untracked_supply: u64,
    pub holders_count: usize,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FungibleAssetHolder {
    pub owner: String,
    pub balance: u64,
    pub coin_object_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FungibleAssetHoldersResponse {
    pub token_type: String,
    pub holders: Vec<FungibleAssetHolder>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FungibleAssetTransactionsResponse {
    pub token_type: String,
    pub transactions: Vec<TransactionDetails>,
}

/// RPC Methods
#[kanari_open_rpc::open_rpc]
pub mod methods {
    use kanari_open_rpc::{
        object_schema, optional_schema, schema_array, schema_boolean, schema_integer,
        schema_object, schema_string,
    };

    // Owner & Balance
    #[open_rpc_method(
        summary = "Get owner info",
        description = "Returns owner data including nonce, balances, modules, and owned objects.",
        params = [("owner", "Target owner address.", true, schema_string())],
        result = ("owner", "Owner information object.", schema_object()),
        tags = ["owner", "balance"]
    )]
    pub const GET_OWNER: &str = "kanari_getOwner";
    #[open_rpc_method(
        summary = "Get one token balance",
        description = "Returns the balance of a specific token type for one owner.",
        params = [(
            "request",
            "Owner and token type payload.",
            true,
            object_schema(&[("owner", schema_string()), ("token_type", schema_string())])
        )],
        result = ("balance", "Token balance payload.", schema_object()),
        tags = ["owner", "balance"]
    )]
    pub const GET_TOKEN_BALANCE: &str = "kanari_getTokenBalance";
    #[open_rpc_method(
        summary = "Get all balances",
        description = "Returns all visible balances for one owner.",
        params = [(
            "request",
            "Owner payload.",
            true,
            object_schema(&[("owner", schema_string())])
        )],
        result = ("balances", "All owner balances.", schema_object()),
        tags = ["owner", "balance"]
    )]
    pub const GET_OWNER_BALANCES: &str = "kanari_getOwnerBalances";
    #[open_rpc_method(
        summary = "List tokens",
        description = "Returns visible token metadata and supply summaries.",
        params = [],
        result = ("tokens", "List of tracked token summaries.", schema_array(schema_object())),
        tags = ["balance"]
    )]
    pub const LIST_TOKENS: &str = "kanari_listTokens";
    #[open_rpc_method(
        summary = "Get fungible asset",
        description = "Returns metadata, supply, and holder count for one fungible asset token type.",
        params = [(
            "request",
            "Fungible asset token type payload.",
            true,
            object_schema(&[("token_type", schema_string())])
        )],
        result = ("asset", "Fungible asset summary.", schema_object()),
        tags = ["asset", "balance"]
    )]
    pub const GET_FUNGIBLE_ASSET: &str = "kanari_getFungibleAsset";
    #[open_rpc_method(
        summary = "Get fungible asset holders",
        description = "Returns wallets that currently hold a positive balance of one fungible asset.",
        params = [(
            "request",
            "Fungible asset holder query.",
            true,
            object_schema(&[
                ("token_type", schema_string()),
                ("limit", optional_schema(schema_integer()))
            ])
        )],
        result = ("holders", "Fungible asset holders.", schema_object()),
        tags = ["asset", "balance"]
    )]
    pub const GET_FUNGIBLE_ASSET_HOLDERS: &str = "kanari_getFungibleAssetHolders";
    #[open_rpc_method(
        summary = "Get fungible asset transactions",
        description = "Returns recent committed and pending transactions that involve one fungible asset.",
        params = [(
            "request",
            "Fungible asset transaction query.",
            true,
            object_schema(&[
                ("token_type", schema_string()),
                ("owner", optional_schema(schema_string())),
                ("limit", optional_schema(schema_integer()))
            ])
        )],
        result = ("transactions", "Fungible asset transaction list.", schema_object()),
        tags = ["asset", "transaction"]
    )]
    pub const GET_FUNGIBLE_ASSET_TRANSACTIONS: &str = "kanari_getFungibleAssetTransactions";

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
            "Optional list options such as limit or owner filter.",
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
    pub const SUBMIT_OBJECT_TRANSFER: &str = "kanari_submitObjectTransfer";
    #[open_rpc_method(
        summary = "Build native transfer transaction",
        description = "Applies the shared NativeTransferPolicyContract: the backend selects canonical native coin refs, gas payment, and nonce, then returns a canonical unsigned transfer payload. Native KANARI transfer is a Move object call and requires distinct transfer/gas coin objects.",
        params = [("transaction", "Unsigned native transfer payload.", true, schema_object())],
        result = ("transaction", "Prepared object-transfer request.", schema_object()),
        tags = ["transaction"]
    )]
    pub const BUILD_NATIVE_TRANSFER: &str = "kanari_buildNativeTransfer";
    #[open_rpc_method(
        summary = "Build native coin consolidation step",
        description = "Builds one backend-approved coin::join_entry step when NativeTransferPolicyContract cannot yet be satisfied by current native coin layout.",
        params = [("transaction", "Unsigned native coin consolidation payload.", true, schema_object())],
        result = ("call", "Prepared call-function request for one native coin join step.", schema_object()),
        tags = ["transaction"]
    )]
    pub const BUILD_NATIVE_COIN_CONSOLIDATION: &str = "kanari_buildNativeCoinConsolidation";

    // Stats & Info
    #[open_rpc_method(
        summary = "Get chain stats",
        description = "Returns aggregate blockchain statistics.",
        params = [],
        result = ("stats", "Blockchain statistics.", schema_object()),
        tags = ["system"]
    )]
    pub const GET_STATS: &str = "kanari_getStats";
    #[open_rpc_method(
        summary = "Get sparse Merkle tree status",
        description = "Returns read-only incremental SMT roots, overlay counters, and schema versions. Set audit=true to perform an expensive full canonical consistency audit; this endpoint never repairs state.",
        params = [(
            "request",
            "Optional SMT diagnostics options.",
            false,
            object_schema(&[("audit", schema_boolean())])
        )],
        result = ("status", "Sparse Merkle tree diagnostics.", schema_object()),
        tags = ["system", "debug"]
    )]
    pub const GET_SMT_STATUS: &str = "kanari_getSmtStatus";
    #[open_rpc_method(
        summary = "Get canonical state snapshot diagnostics",
        description = "Returns canonical state key/value entries for the current node state, with optional prefix and limit filters, for divergence debugging.",
        params = [(
            "request",
            "Optional canonical snapshot diagnostics filter.",
            false,
            object_schema(&[])
        )],
        result = ("snapshot", "Canonical state snapshot diagnostics.", schema_object()),
        tags = ["system", "debug"]
    )]
    pub const GET_CANONICAL_STATE_SNAPSHOT: &str = "kanari_getCanonicalStateSnapshot";
    #[open_rpc_method(
        summary = "Compare canonical state snapshot diagnostics",
        description = "Compares current node canonical state against a caller-provided snapshot and returns the first divergent key/value.",
        params = [(
            "request",
            "Canonical snapshot entries captured from another node.",
            true,
            object_schema(&[("entries", schema_array(schema_object()))])
        )],
        result = ("diff", "Canonical state divergence diagnostics.", schema_object()),
        tags = ["system", "debug"]
    )]
    pub const COMPARE_CANONICAL_STATE_SNAPSHOT: &str = "kanari_compareCanonicalStateSnapshot";
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
        summary = "Get active gas model",
        description = "Returns the active gas policy and both requested and effective default gas prices.",
        params = [],
        result = ("gas", "Active gas policy metadata.", schema_object()),
        tags = ["system"]
    )]
    pub const GET_GAS_INFO: &str = "kanari_getGasInfo";

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
        summary = "Publish or upgrade module",
        description = "Publishes Move module bytecode. If the module id already exists, this performs a compatibility-checked upgrade.",
        params = [("module", "Module publication payload.", true, schema_object())],
        result = ("publication", "Publish/upgrade execution or pending status.", schema_object()),
        tags = ["module"]
    )]
    pub const PUBLISH_MODULE: &str = "kanari_publishModule";
    #[open_rpc_method(
        summary = "Upgrade module",
        description = "Alias of kanari_publishModule for compatibility-checked module upgrades.",
        params = [("module", "Module upgrade payload.", true, schema_object())],
        result = ("upgrade", "Upgrade execution or pending status.", schema_object()),
        tags = ["module"]
    )]
    pub const UPGRADE_MODULE: &str = "kanari_upgradeModule";
    #[open_rpc_method(
        summary = "Build publish or upgrade module transaction",
        description = "Resolves nonce and gas payment for a module publication or compatibility-checked upgrade and returns a canonical unsigned request.",
        params = [("module", "Unsigned module publication/upgrade payload.", true, schema_object())],
        result = ("module", "Prepared publish/upgrade-module request.", schema_object()),
        tags = ["module"]
    )]
    pub const BUILD_PUBLISH_MODULE: &str = "kanari_buildPublishModule";
    #[open_rpc_method(
        summary = "Build upgrade module transaction",
        description = "Alias of kanari_buildPublishModule for compatibility-checked module upgrades.",
        params = [("module", "Unsigned module upgrade payload.", true, schema_object())],
        result = ("module", "Prepared upgrade-module request.", schema_object()),
        tags = ["module"]
    )]
    pub const BUILD_UPGRADE_MODULE: &str = "kanari_buildUpgradeModule";
    #[open_rpc_method(
        summary = "Publish or upgrade package",
        description = "Publishes multiple Move modules atomically in one package transaction. Existing module ids are compatibility-checked upgrades.",
        params = [("package", "Package publication payload.", true, schema_object())],
        result = ("publication", "Publish/upgrade execution or pending status.", schema_object()),
        tags = ["module"]
    )]
    pub const PUBLISH_PACKAGE: &str = "kanari_publishPackage";
    #[open_rpc_method(
        summary = "Upgrade package",
        description = "Alias of kanari_publishPackage for compatibility-checked atomic package upgrades.",
        params = [("package", "Package upgrade payload.", true, schema_object())],
        result = ("upgrade", "Upgrade execution or pending status.", schema_object()),
        tags = ["module"]
    )]
    pub const UPGRADE_PACKAGE: &str = "kanari_upgradePackage";
    #[open_rpc_method(
        summary = "Build publish or upgrade package transaction",
        description = "Resolves nonce and gas payment for an atomic package publication or compatibility-checked upgrade and returns a canonical unsigned request.",
        params = [("package", "Unsigned package publication/upgrade payload.", true, schema_object())],
        result = ("package", "Prepared publish/upgrade-package request.", schema_object()),
        tags = ["module"]
    )]
    pub const BUILD_PUBLISH_PACKAGE: &str = "kanari_buildPublishPackage";
    #[open_rpc_method(
        summary = "Build upgrade package transaction",
        description = "Alias of kanari_buildPublishPackage for compatibility-checked atomic package upgrades.",
        params = [("package", "Unsigned package upgrade payload.", true, schema_object())],
        result = ("package", "Prepared upgrade-package request.", schema_object()),
        tags = ["module"]
    )]
    pub const BUILD_UPGRADE_PACKAGE: &str = "kanari_buildUpgradePackage";
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
        summary = "Build function call transaction",
        description = "Resolves object inputs, nonce, and gas payment for a Move entry function call and returns a canonical unsigned request.",
        params = [("call", "Unsigned function call payload.", true, schema_object())],
        result = ("call", "Prepared call-function request.", schema_object()),
        tags = ["function"]
    )]
    pub const BUILD_CALL_FUNCTION: &str = "kanari_buildCallFunction";
    #[open_rpc_method(
        summary = "Build token transfer transaction",
        description = "Selects a token coin object, resolves gas and nonce, and returns a canonical unsigned Move call for token transfer.",
        params = [("call", "Unsigned token transfer payload.", true, schema_object())],
        result = ("call", "Prepared token-transfer call request.", schema_object()),
        tags = ["function"]
    )]
    pub const BUILD_TOKEN_TRANSFER: &str = "kanari_buildTokenTransfer";
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
    #[open_rpc_method(
        summary = "Get objects by type",
        description = "Returns all known objects that match one exact object type string.",
        params = [(
            "request",
            "Object type payload.",
            true,
            object_schema(&[("object_type", schema_string())])
        )],
        result = ("objects", "Object list.", schema_object()),
        tags = ["object"]
    )]
    pub const GET_OBJECTS_BY_TYPE: &str = "kanari_getObjectsByType";
    #[open_rpc_method(
        summary = "Query objects",
        description = "Returns objects filtered by owner, owner kind, object type, and/or version range.",
        params = [(
            "request",
            "Object query filter payload.",
            true,
            object_schema(&[])
        )],
        result = ("objects", "Filtered object list.", schema_object()),
        tags = ["object"]
    )]
    pub const GET_OBJECTS: &str = "kanari_getObjects";
    #[open_rpc_method(
        summary = "Get object by ref",
        description = "Returns one object only if the full object ref (id, version, digest) matches the current canonical object.",
        params = [(
            "request",
            "Full object ref payload.",
            true,
            object_schema(&[("object_ref", schema_object())])
        )],
        result = ("object", "Exact object ref match payload.", schema_object()),
        tags = ["object"]
    )]
    pub const GET_OBJECT_BY_REF: &str = "kanari_getObjectByRef";

    // NFT queries
    #[open_rpc_method(
        summary = "Get owned NFTs",
        description = "Returns NFT-like objects owned by one owner.",
        params = [("owner", "Target owner address.", true, schema_string())],
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
