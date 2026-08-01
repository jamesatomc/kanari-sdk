// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

// Blockchain data structures and operations
use crate::gas_coin::GasModule;
use anyhow::Result;
use kanari_crypto::keys::CurveType;
use kanari_crypto::verify_signature;
use kanari_crypto::{hash_data_blake3, signatures::sign_message};
use move_core_types::account_address::AccountAddress;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tracing::error;

/// Signed transaction wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedTransaction {
    pub transaction: Transaction,
    pub signature: Vec<u8>,
    #[serde(skip, default)]
    cached_transaction_hash: OnceLock<Vec<u8>>,
    #[serde(skip, default)]
    cached_verified_signature: OnceLock<(Vec<u8>, Vec<u8>)>,
}

/// Immutable transaction wrapper for internal paths that have already verified
/// the signature against the exact transaction bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedSignedTransaction {
    signed: SignedTransaction,
    tx_hash: Vec<u8>,
}

impl SignedTransaction {
    pub fn new(transaction: Transaction) -> Self {
        Self {
            transaction,
            signature: vec![],
            cached_transaction_hash: OnceLock::new(),
            cached_verified_signature: OnceLock::new(),
        }
    }

    pub fn sign(&mut self, private_key: &str, curve_type: CurveType) -> Result<()> {
        let tx_hash = self.transaction_hash().to_vec();
        let signature = sign_message(private_key, &tx_hash, curve_type)
            .map_err(|e| anyhow::anyhow!("Failed to sign transaction: {}", e))?;
        self.signature = signature;
        let _ = self
            .cached_verified_signature
            .set((tx_hash, self.signature.clone()));
        Ok(())
    }

    pub fn verify_signature(&self) -> Result<bool> {
        let tx_hash = self.transaction_hash();
        self.verify_signature_for_hash(tx_hash)
    }

    pub fn transaction_hash(&self) -> &[u8] {
        self.cached_transaction_hash
            .get_or_init(|| self.transaction.hash())
            .as_slice()
    }

    pub fn verify_signature_for_hash(&self, tx_hash: &[u8]) -> Result<bool> {
        if self.signature.is_empty() {
            anyhow::bail!("Transaction not signed");
        }

        let current_hash = self.transaction.hash();
        if tx_hash != current_hash.as_slice() {
            return Ok(false);
        }

        let signature = &self.signature;
        if self
            .cached_verified_signature
            .get()
            .map(|(cached_hash, cached_signature)| {
                cached_hash.as_slice() == current_hash.as_slice() && cached_signature == signature
            })
            .unwrap_or(false)
        {
            return Ok(true);
        }

        let sender = self.transaction.sender();

        let verified = verify_signature(sender, tx_hash, signature)
            .map_err(|e| anyhow::anyhow!("Signature verification failed: {}", e))?;
        if !verified {
            return Ok(false);
        }
        let _ = self
            .cached_verified_signature
            .set((current_hash, signature.clone()));
        Ok(true)
    }

    pub fn verified_transaction_hash(&self) -> Result<Vec<u8>> {
        if self.signature.is_empty() {
            anyhow::bail!("Transaction not signed");
        }

        let current_hash = self.transaction.hash();
        let signature = &self.signature;
        if self
            .cached_verified_signature
            .get()
            .map(|(cached_hash, cached_signature)| {
                cached_hash.as_slice() == current_hash.as_slice() && cached_signature == signature
            })
            .unwrap_or(false)
        {
            return Ok(current_hash);
        }

        let sender = self.transaction.sender();
        let verified = verify_signature(sender, &current_hash, signature)
            .map_err(|e| anyhow::anyhow!("Signature verification failed: {}", e))?;
        if !verified {
            anyhow::bail!("Invalid transaction signature");
        }
        let _ = self
            .cached_verified_signature
            .set((current_hash.clone(), signature.clone()));
        Ok(current_hash)
    }

    pub fn into_verified(self) -> Result<VerifiedSignedTransaction> {
        let tx_hash = self.verified_transaction_hash()?;
        Ok(VerifiedSignedTransaction {
            signed: self,
            tx_hash,
        })
    }

    pub fn hash(&self) -> Vec<u8> {
        let serialized = match bcs::to_bytes(self) {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to serialize SignedTransaction for hashing: {}", e);
                Vec::new()
            }
        };
        hash_data_blake3(&serialized)
    }
}

impl VerifiedSignedTransaction {
    pub fn hash(&self) -> &[u8] {
        &self.tx_hash
    }

    pub fn transaction(&self) -> &Transaction {
        &self.signed.transaction
    }

    pub fn into_signed_transaction(self) -> SignedTransaction {
        self.signed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCall {
    Transfer {
        coin_object_id: String,
        recipient: String,
        amount: u64,
    },
    Burn {
        amount: u64,
    },
}

impl NativeCall {
    pub fn tx_type_label(&self) -> &'static str {
        match self {
            Self::Transfer { .. } => "transfer",
            Self::Burn { .. } => "burn",
        }
    }

    pub fn required_native_amount(&self) -> u64 {
        match self {
            Self::Transfer { amount, .. } | Self::Burn { amount } => *amount,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectOwnerKind {
    AddressOwner(String),
    Shared,
    Immutable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub object_id: String,
    pub version: Option<u64>,
    pub digest: Option<String>,
}

impl ObjectRef {
    pub fn new(object_id: impl Into<String>, version: Option<u64>, digest: Option<String>) -> Self {
        Self {
            object_id: object_id.into(),
            version,
            digest,
        }
    }

    pub fn has_full_metadata(&self) -> bool {
        self.version.is_some() && self.digest.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectInput {
    pub object_ref: ObjectRef,
    pub owner: Option<ObjectOwnerKind>,
    pub mutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GasPayment {
    pub payment_objects: Vec<ObjectRef>,
    pub owner: String,
    pub budget: u64,
    pub price: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedModule {
    pub module_name: String,
    pub module_bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectChangeKind {
    Created,
    Mutated,
    Deleted,
    Transferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectGraphEdgeKind {
    InputCreate,
    InputMutate,
    InputDelete,
    InputTransfer,
    SharedInputCreate,
    SharedInputMutate,
    SharedInputDelete,
    SharedInputTransfer,
    ImmutableInputCreate,
    ImmutableInputMutate,
    ImmutableInputDelete,
    ImmutableInputTransfer,
    GasCreate,
    GasMutate,
    GasDelete,
    GasTransfer,
    CallContextCreate,
    VersionSuccessor,
    Delete,
    OwnershipTransfer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectGraphEdge {
    pub source_object_ref: ObjectRef,
    pub target_object_ref: ObjectRef,
    pub relation: ObjectGraphEdgeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectChange {
    pub change_type: ObjectChangeKind,
    pub object_ref: ObjectRef,
    pub previous_object_ref: Option<ObjectRef>,
    pub type_: Option<String>,
    pub owner: Option<ObjectOwnerKind>,
    pub previous_owner: Option<ObjectOwnerKind>,
    pub previous_version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionEffects {
    pub status: String,
    pub gas_used: u64,
    pub gas_payment: Option<GasPayment>,
    #[serde(default)]
    pub input_objects: Vec<ObjectRef>,
    #[serde(default)]
    pub shared_inputs: Vec<ObjectRef>,
    #[serde(default)]
    pub immutable_inputs: Vec<ObjectRef>,
    #[serde(default)]
    pub gas_object_refs: Vec<ObjectRef>,
    pub object_changes: Vec<ObjectChange>,
    #[serde(default)]
    pub created: Vec<ObjectChange>,
    #[serde(default)]
    pub mutated: Vec<ObjectChange>,
    #[serde(default)]
    pub deleted: Vec<ObjectChange>,
    #[serde(default)]
    pub transferred: Vec<ObjectChange>,
    #[serde(default)]
    pub causal_edges: Vec<ObjectGraphEdge>,
    pub error_message: Option<String>,
}

/// Transaction types in Kanari blockchain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transaction {
    /// Publish a Move module
    PublishModule {
        sender: String,
        module_bytes: Vec<u8>,
        module_name: String,
        gas_payment: Option<GasPayment>,
        gas_limit: u64,
        gas_price: u64,
        nonce: u64,
    },
    /// Publish a Move package as one atomic transaction.
    PublishPackage {
        sender: String,
        modules: Vec<PublishedModule>,
        gas_payment: Option<GasPayment>,
        gas_limit: u64,
        gas_price: u64,
        nonce: u64,
    },
    /// Upgrade an existing Move module.
    UpgradeModule {
        sender: String,
        module_bytes: Vec<u8>,
        module_name: String,
        gas_payment: Option<GasPayment>,
        gas_limit: u64,
        gas_price: u64,
        nonce: u64,
    },
    /// Upgrade an existing Move package as one atomic transaction.
    UpgradePackage {
        sender: String,
        modules: Vec<PublishedModule>,
        gas_payment: Option<GasPayment>,
        gas_limit: u64,
        gas_price: u64,
        nonce: u64,
    },
    /// Execute a Move function
    ExecuteFunction {
        sender: String,
        module: String,
        function: String,
        type_args: Vec<String>,
        args: Vec<Vec<u8>>,
        object_inputs: Vec<ObjectInput>,
        gas_payment: Option<GasPayment>,
        gas_limit: u64,
        gas_price: u64,
        nonce: u64,
    },
}

impl Transaction {
    pub fn hash(&self) -> Vec<u8> {
        let serialized = match bcs::to_bytes(self) {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to serialize Transaction for hashing: {}", e);
                Vec::new()
            }
        };
        hash_data_blake3(&serialized)
    }

    pub fn sender(&self) -> &str {
        match self {
            Transaction::PublishModule { sender, .. }
            | Transaction::PublishPackage { sender, .. }
            | Transaction::UpgradeModule { sender, .. }
            | Transaction::UpgradePackage { sender, .. } => sender,
            Transaction::ExecuteFunction { sender, .. } => sender,
        }
    }

    pub fn sender_address(&self) -> &str {
        self.sender()
    }

    pub fn nonce(&self) -> u64 {
        match self {
            Transaction::PublishModule { nonce, .. }
            | Transaction::PublishPackage { nonce, .. }
            | Transaction::UpgradeModule { nonce, .. }
            | Transaction::UpgradePackage { nonce, .. } => *nonce,
            Transaction::ExecuteFunction { nonce, .. } => *nonce,
        }
    }

    pub fn gas_limit(&self) -> u64 {
        match self {
            Transaction::PublishModule { gas_limit, .. }
            | Transaction::PublishPackage { gas_limit, .. }
            | Transaction::UpgradeModule { gas_limit, .. }
            | Transaction::UpgradePackage { gas_limit, .. } => *gas_limit,
            Transaction::ExecuteFunction { gas_limit, .. } => *gas_limit,
        }
    }

    pub fn gas_price(&self) -> u64 {
        match self {
            Transaction::PublishModule { gas_price, .. }
            | Transaction::PublishPackage { gas_price, .. }
            | Transaction::UpgradeModule { gas_price, .. }
            | Transaction::UpgradePackage { gas_price, .. } => *gas_price,
            Transaction::ExecuteFunction { gas_price, .. } => *gas_price,
        }
    }

    /// Get conflict keys for this transaction.
    /// Transactions with overlapping conflict keys must be executed sequentially.
    pub fn get_conflict_keys(&self) -> Vec<String> {
        let sender_norm = if let Ok(addr) = AccountAddress::from_hex_literal(self.sender()) {
            addr.to_hex_literal()
        } else {
            let s = self.sender();
            if !s.starts_with("0x") {
                format!("0x{}", s)
            } else {
                s.to_string()
            }
        };

        let mut keys = Vec::new();

        let object_access_keys = self.object_access_keys();
        if !object_access_keys.is_empty() {
            keys.extend(object_access_keys);
        } else {
            keys.push(format!("owner:{}", sender_norm));
        }

        match self {
            Transaction::ExecuteFunction { args, .. }
                if keys.len() == 1 && self.native_call().is_some() =>
            {
                for arg in args {
                    if arg.len() == 32
                        && let Ok(addr) = AccountAddress::from_bytes(arg)
                    {
                        keys.push(format!("object:{}", addr.to_hex_literal()));
                        continue;
                    }

                    // 2. Extract String from Argument (support both BCS String and Raw UTF-8)
                    let parsed_string = bcs::from_bytes::<String>(arg)
                        .or_else(|_| std::str::from_utf8(arg).map(|s| s.to_string()));

                    if let Ok(s) = parsed_string {
                        let s_trim = s.trim();

                        // Force add 0x prefix if missing
                        let hex_str = if !s_trim.starts_with("0x") {
                            format!("0x{}", s_trim)
                        } else {
                            s_trim.to_string()
                        };

                        // Use from_hex_literal to handle length and convert to Lowercase
                        if let Ok(addr) = AccountAddress::from_hex_literal(&hex_str) {
                            keys.push(format!("object:{}", addr.to_hex_literal()));
                        }
                    }
                }
            }
            Transaction::PublishModule { module_name, .. }
            | Transaction::UpgradeModule { module_name, .. } => {
                keys.push(format!("module:{}", module_name));
            }
            Transaction::PublishPackage { modules, .. }
            | Transaction::UpgradePackage { modules, .. } => {
                keys.extend(
                    modules
                        .iter()
                        .map(|module| format!("module:{}", module.module_name)),
                );
            }
            Transaction::ExecuteFunction { .. } => {}
        }
        keys.sort();
        keys.dedup();
        keys
    }

    pub fn object_inputs(&self) -> Vec<ObjectInput> {
        match self {
            Transaction::ExecuteFunction { object_inputs, .. } => object_inputs.clone(),
            Transaction::PublishModule { .. }
            | Transaction::PublishPackage { .. }
            | Transaction::UpgradeModule { .. }
            | Transaction::UpgradePackage { .. } => Vec::new(),
        }
    }

    pub fn gas_payment(&self) -> Option<GasPayment> {
        match self {
            Transaction::ExecuteFunction { gas_payment, .. } => gas_payment.clone(),
            Transaction::PublishModule { gas_payment, .. }
            | Transaction::PublishPackage { gas_payment, .. }
            | Transaction::UpgradeModule { gas_payment, .. }
            | Transaction::UpgradePackage { gas_payment, .. } => gas_payment.clone(),
        }
    }

    pub fn requires_strict_object_metadata(&self) -> bool {
        matches!(
            self,
            Transaction::ExecuteFunction { .. }
                | Transaction::PublishModule { .. }
                | Transaction::PublishPackage { .. }
                | Transaction::UpgradeModule { .. }
                | Transaction::UpgradePackage { .. }
        )
    }

    pub fn object_access_keys(&self) -> Vec<String> {
        let mut keys = self
            .object_inputs()
            .into_iter()
            // Conflict identity is the object itself. Access role must not create a
            // separate namespace, otherwise read/write and gas/input aliases look disjoint.
            .map(|input| format!("object:{}", input.object_ref.object_id))
            .collect::<Vec<_>>();

        if let Some(gas_payment) = self.gas_payment() {
            keys.extend(
                gas_payment
                    .payment_objects
                    .into_iter()
                    .map(|payment| format!("object:{}", payment.object_id)),
            );
        }

        keys.sort();
        keys.dedup();
        keys
    }

    pub fn primary_access_key(&self) -> String {
        if let Some(key) = self
            .object_inputs()
            .into_iter()
            .find(|input| input.mutable)
            .map(|input| format!("mut:object:{}", input.object_ref.object_id))
        {
            return key;
        }

        if let Some(key) = self.object_inputs().into_iter().next().map(|input| {
            let mutability = if input.mutable { "mut" } else { "ro" };
            format!("{mutability}:object:{}", input.object_ref.object_id)
        }) {
            return key;
        }

        if let Some(key) = self
            .gas_payment()
            .and_then(|gas_payment| gas_payment.payment_objects.into_iter().next())
            .map(|payment| format!("mut:gas:{}", payment.object_id))
        {
            return key;
        }

        format!("owner:{}", self.sender())
    }

    pub fn native_call(&self) -> Option<NativeCall> {
        let Transaction::ExecuteFunction {
            module,
            function,
            args,
            object_inputs,
            ..
        } = self
        else {
            return None;
        };

        let kanari_module = GasModule::module_path();
        if module != &kanari_module {
            return None;
        }

        match function.as_str() {
            function_name
                if function_name == GasModule::function_names().transfer && args.len() >= 3 =>
            {
                // The ABI carries the authoritative coin address in args[0]. Object
                // inputs are not ordered, so using the first input can select the
                // wrong coin when callers include additional inputs.
                let coin_object_id = AccountAddress::from_bytes(args.first()?.as_slice())
                    .ok()?
                    .to_hex_literal();
                // Keep the native fast path fail-closed: the ABI coin must be a
                // mutable object input. Other shapes go through full Move execution.
                if !object_inputs.iter().any(|input| {
                    input.mutable
                        && AccountAddress::from_hex_literal(&input.object_ref.object_id)
                            .map(|address| address.to_hex_literal() == coin_object_id)
                            .unwrap_or(false)
                }) {
                    return None;
                }
                let amount = bcs::from_bytes::<u64>(args.get(1)?).ok()?;
                let recipient = AccountAddress::from_bytes(args.get(2)?.as_slice())
                    .ok()?
                    .to_hex_literal();
                Some(NativeCall::Transfer {
                    coin_object_id,
                    recipient,
                    amount,
                })
            }
            function_name
                if function_name == GasModule::function_names().burn && !args.is_empty() =>
            {
                let amount = bcs::from_bytes::<u64>(args.last()?).ok()?;
                Some(NativeCall::Burn { amount })
            }
            _ => None,
        }
    }

    pub fn is_native_balance_call(&self) -> bool {
        self.native_call().is_some()
    }

    pub fn tx_type_label(&self) -> &'static str {
        if let Some(native_call) = self.native_call() {
            return native_call.tx_type_label();
        }

        match self {
            Transaction::PublishModule { .. } => "publish_module",
            Transaction::PublishPackage { .. } => "publish_package",
            Transaction::UpgradeModule { .. } => "upgrade_module",
            Transaction::UpgradePackage { .. } => "upgrade_package",
            Transaction::ExecuteFunction {
                module, function, ..
            } if module == &GasModule::module_path()
                && function == GasModule::function_names().transfer =>
            {
                "transfer"
            }
            Transaction::ExecuteFunction {
                module, function, ..
            } if module == &GasModule::module_path()
                && function == GasModule::function_names().burn =>
            {
                "burn"
            }
            Transaction::ExecuteFunction { .. } => "call",
        }
    }

    /// Create an object-input KANARI Move transfer transaction with default gas settings.
    pub fn new_transfer(
        from: String,
        coin_object_id: String,
        to: String,
        amount: u64,
        nonce: u64,
    ) -> Self {
        Self::new_transfer_with_gas(from, coin_object_id, to, amount, nonce, 100_000, 1000)
    }

    pub fn new_transfer_with_gas(
        from: String,
        coin_object_id: String,
        to: String,
        amount: u64,
        nonce: u64,
        gas_limit: u64,
        gas_price: u64,
    ) -> Self {
        Self::new_transfer_with_object_ref_and_gas(
            from,
            ObjectRef::new(coin_object_id, None, None),
            to,
            amount,
            nonce,
            gas_limit,
            gas_price,
        )
    }

    pub fn new_transfer_with_object_ref(
        from: String,
        coin_object_ref: ObjectRef,
        to: String,
        amount: u64,
        nonce: u64,
    ) -> Self {
        Self::new_transfer_with_object_ref_and_gas(
            from,
            coin_object_ref,
            to,
            amount,
            nonce,
            100_000,
            1000,
        )
    }

    pub fn new_transfer_with_object_ref_and_gas(
        from: String,
        coin_object_ref: ObjectRef,
        to: String,
        amount: u64,
        nonce: u64,
        gas_limit: u64,
        gas_price: u64,
    ) -> Self {
        let coin_object_addr = AccountAddress::from_hex_literal(&coin_object_ref.object_id)
            .unwrap_or(AccountAddress::ZERO);
        let recipient_addr = AccountAddress::from_hex_literal(&to).unwrap_or(AccountAddress::ZERO);

        Self::ExecuteFunction {
            sender: from.clone(),
            module: GasModule::module_path(),
            function: GasModule::function_names().transfer.to_string(),
            type_args: vec![],
            args: vec![
                coin_object_addr.to_vec(),
                bcs::to_bytes(&amount).unwrap_or_default(),
                recipient_addr.to_vec(),
            ],
            object_inputs: vec![ObjectInput {
                object_ref: coin_object_ref.clone(),
                owner: Some(ObjectOwnerKind::AddressOwner(from.clone())),
                mutable: true,
            }],
            gas_payment: Some(GasPayment {
                payment_objects: vec![coin_object_ref],
                owner: from,
                budget: gas_limit,
                price: gas_price,
            }),
            gas_limit,
            gas_price,
            nonce,
        }
    }

    /// Create a burn transaction with default gas settings
    pub fn new_burn(from: String, amount: u64, nonce: u64) -> Self {
        Self::new_burn_with_gas(from, amount, nonce, 100_000, 1000)
    }

    pub fn new_burn_with_gas(
        from: String,
        amount: u64,
        nonce: u64,
        gas_limit: u64,
        gas_price: u64,
    ) -> Self {
        Self::ExecuteFunction {
            sender: from.clone(),
            module: GasModule::module_path(),
            function: GasModule::function_names().burn.to_string(),
            type_args: vec![],
            args: vec![bcs::to_bytes(&amount).unwrap_or_default()],
            object_inputs: Vec::new(),
            gas_payment: Some(GasPayment {
                payment_objects: Vec::new(),
                owner: from,
                budget: gas_limit,
                price: gas_price,
            }),
            gas_limit,
            gas_price,
            nonce,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_helper_builds_native_execute_function() {
        let coin_object_id = format!("0x{:064x}", 0xaaaau64);
        let canonical_coin_object_id = AccountAddress::from_hex_literal(&coin_object_id)
            .unwrap()
            .to_hex_literal();
        let tx = Transaction::new_transfer_with_object_ref(
            "0x1".to_string(),
            ObjectRef::new(
                coin_object_id.clone(),
                Some(1),
                Some("0xtestdigest".to_string()),
            ),
            "0x2".to_string(),
            42,
            7,
        );

        match &tx {
            Transaction::ExecuteFunction {
                module,
                function,
                nonce,
                ..
            } => {
                assert_eq!(module, &GasModule::module_path());
                assert_eq!(function, GasModule::function_names().transfer);
                assert_eq!(*nonce, 7);
            }
            Transaction::PublishModule { .. }
            | Transaction::PublishPackage { .. }
            | Transaction::UpgradeModule { .. }
            | Transaction::UpgradePackage { .. } => {
                panic!("transfer helper must build a call")
            }
        }

        assert_eq!(
            tx.native_call(),
            Some(NativeCall::Transfer {
                coin_object_id: canonical_coin_object_id,
                recipient: AccountAddress::from_hex_literal("0x2")
                    .unwrap()
                    .to_hex_literal(),
                amount: 42,
            })
        );
        assert_eq!(tx.tx_type_label(), "transfer");
    }

    #[test]
    fn transfer_helper_builds_execute_function_with_coin_input() {
        let coin_object_id = format!("0x{:064x}", 0xaaaau64);
        let canonical_coin_object_id = AccountAddress::from_hex_literal(&coin_object_id)
            .unwrap()
            .to_hex_literal();
        let tx = Transaction::new_transfer_with_object_ref(
            "0x1".to_string(),
            ObjectRef::new(
                coin_object_id.clone(),
                Some(1),
                Some("0xtestdigest".to_string()),
            ),
            "0x2".to_string(),
            42,
            7,
        );

        match &tx {
            Transaction::ExecuteFunction {
                module,
                function,
                nonce,
                args,
                ..
            } => {
                assert_eq!(module, &GasModule::module_path());
                assert_eq!(function, GasModule::function_names().transfer);
                assert_eq!(*nonce, 7);
                assert_eq!(args.len(), 3);
            }
            Transaction::PublishModule { .. }
            | Transaction::PublishPackage { .. }
            | Transaction::UpgradeModule { .. }
            | Transaction::UpgradePackage { .. } => {
                panic!("object transfer helper must build a call")
            }
        }

        assert_eq!(
            tx.native_call(),
            Some(NativeCall::Transfer {
                coin_object_id: canonical_coin_object_id,
                recipient: AccountAddress::from_hex_literal("0x2")
                    .unwrap()
                    .to_hex_literal(),
                amount: 42,
            })
        );
    }

    #[test]
    fn native_transfer_uses_abi_coin_id_not_object_input_order() {
        let coin_object_id = format!("0x{:064x}", 0xaaaau64);
        let canonical_coin_object_id = AccountAddress::from_hex_literal(&coin_object_id)
            .unwrap()
            .to_hex_literal();
        let mut tx = Transaction::new_transfer_with_object_ref(
            "0x1".to_string(),
            ObjectRef::new(
                coin_object_id.clone(),
                Some(1),
                Some("0xtestdigest".to_string()),
            ),
            "0x2".to_string(),
            42,
            7,
        );
        let Transaction::ExecuteFunction { object_inputs, .. } = &mut tx else {
            panic!("transfer helper must build a call");
        };
        object_inputs.insert(
            0,
            ObjectInput {
                object_ref: ObjectRef::new(
                    format!("0x{:064x}", 0xbbbbu64),
                    Some(1),
                    Some("0xotherdigest".to_string()),
                ),
                owner: None,
                mutable: false,
            },
        );

        assert_eq!(
            tx.native_call(),
            Some(NativeCall::Transfer {
                coin_object_id: canonical_coin_object_id,
                recipient: AccountAddress::from_hex_literal("0x2")
                    .unwrap()
                    .to_hex_literal(),
                amount: 42,
            })
        );
    }

    #[test]
    fn transfer_helper_preserves_full_object_ref_metadata() {
        let object_ref =
            ObjectRef::new("0xaaaa".to_string(), Some(7), Some("0xdigest".to_string()));
        let tx = Transaction::new_transfer_with_object_ref(
            "0x1".to_string(),
            object_ref.clone(),
            "0x2".to_string(),
            42,
            7,
        );

        let object_inputs = tx.object_inputs();
        assert_eq!(object_inputs.len(), 1);
        assert_eq!(object_inputs[0].object_ref, object_ref);

        let gas_payment = tx.gas_payment().expect("transfer should carry gas payment");
        assert_eq!(gas_payment.payment_objects, vec![object_ref]);
    }

    #[test]
    fn non_native_execute_function_does_not_infer_conflicts_from_args_only() {
        let tx = Transaction::ExecuteFunction {
            sender: "0x1".to_string(),
            module: "0x99::demo".to_string(),
            function: "touch".to_string(),
            type_args: vec![],
            args: vec![AccountAddress::from_hex_literal("0xaaaa").unwrap().to_vec()],
            object_inputs: Vec::new(),
            gas_payment: None,
            gas_limit: 100_000,
            gas_price: 1,
            nonce: 0,
        };

        let keys = tx.get_conflict_keys();
        assert_eq!(keys, vec!["owner:0x1".to_string()]);
    }

    #[test]
    fn burn_helper_builds_native_execute_function() {
        let tx = Transaction::new_burn("0x1".to_string(), 9, 3);

        assert_eq!(tx.native_call(), Some(NativeCall::Burn { amount: 9 }));
        assert_eq!(tx.tx_type_label(), "burn");
    }

    #[test]
    fn upgrade_transactions_have_upgrade_type_labels() {
        let gas_payment = GasPayment {
            payment_objects: vec![ObjectRef::new(
                "0xgas",
                Some(7),
                Some("0xdigest".to_string()),
            )],
            owner: "0x1".to_string(),
            budget: 100_000,
            price: 1,
        };
        let module_tx = Transaction::UpgradeModule {
            sender: "0x1".to_string(),
            module_bytes: vec![1, 2, 3],
            module_name: "demo".to_string(),
            gas_payment: Some(gas_payment.clone()),
            gas_limit: 100_000,
            gas_price: 1,
            nonce: 0,
        };
        let package_tx = Transaction::UpgradePackage {
            sender: "0x1".to_string(),
            modules: vec![PublishedModule {
                module_name: "demo".to_string(),
                module_bytes: vec![1, 2, 3],
            }],
            gas_payment: Some(gas_payment.clone()),
            gas_limit: 100_000,
            gas_price: 1,
            nonce: 1,
        };

        assert_eq!(module_tx.tx_type_label(), "upgrade_module");
        assert_eq!(package_tx.tx_type_label(), "upgrade_package");
        assert!(module_tx.requires_strict_object_metadata());
        assert!(package_tx.requires_strict_object_metadata());
        assert!(module_tx.object_inputs().is_empty());
        assert!(package_tx.object_inputs().is_empty());
        assert_eq!(module_tx.gas_payment(), Some(gas_payment.clone()));
        assert_eq!(package_tx.gas_payment(), Some(gas_payment));
        assert_eq!(
            module_tx.get_conflict_keys(),
            vec!["module:demo".to_string(), "object:0xgas".to_string()]
        );
        assert_eq!(
            package_tx.get_conflict_keys(),
            vec!["module:demo".to_string(), "object:0xgas".to_string()]
        );
    }

    #[test]
    fn upgrade_package_conflict_keys_are_deterministic_and_deduplicated() {
        let tx = Transaction::UpgradePackage {
            sender: "0x1".to_string(),
            modules: vec![
                PublishedModule {
                    module_name: "zeta".to_string(),
                    module_bytes: vec![],
                },
                PublishedModule {
                    module_name: "alpha".to_string(),
                    module_bytes: vec![],
                },
                PublishedModule {
                    module_name: "alpha".to_string(),
                    module_bytes: vec![],
                },
            ],
            gas_payment: None,
            gas_limit: 100_000,
            gas_price: 1,
            nonce: 1,
        };

        assert_eq!(
            tx.get_conflict_keys(),
            vec![
                "module:alpha".to_string(),
                "module:zeta".to_string(),
                "owner:0x1".to_string()
            ]
        );
    }

    #[test]
    fn upgrade_transactions_without_gas_fall_back_to_owner_conflict_key() {
        let tx = Transaction::UpgradeModule {
            sender: "0x1".to_string(),
            module_bytes: vec![1, 2, 3],
            module_name: "demo".to_string(),
            gas_payment: None,
            gas_limit: 100_000,
            gas_price: 1,
            nonce: 0,
        };

        assert_eq!(
            tx.get_conflict_keys(),
            vec!["module:demo".to_string(), "owner:0x1".to_string()]
        );
    }

    #[test]
    fn tampering_transaction_after_signing_invalidates_signature_cache() {
        let keypair = kanari_crypto::keys::generate_keypair(CurveType::Ed25519).unwrap();
        let mut signed_tx = SignedTransaction::new(Transaction::new_burn_with_gas(
            keypair.tagged_address(),
            0,
            0,
            100_000,
            0,
        ));
        signed_tx
            .sign(&keypair.private_key, keypair.curve_type)
            .unwrap();

        assert!(signed_tx.verify_signature().unwrap());

        match &mut signed_tx.transaction {
            Transaction::ExecuteFunction { nonce, .. } => *nonce += 1,
            Transaction::PublishModule { .. }
            | Transaction::PublishPackage { .. }
            | Transaction::UpgradeModule { .. }
            | Transaction::UpgradePackage { .. } => {
                unreachable!()
            }
        }

        assert!(!signed_tx.verify_signature().unwrap());
    }

    #[test]
    fn deserialized_transaction_verifies_without_in_memory_signature_cache() {
        let keypair = kanari_crypto::keys::generate_keypair(CurveType::Ed25519).unwrap();
        let mut signed_tx = SignedTransaction::new(Transaction::new_burn_with_gas(
            keypair.tagged_address(),
            0,
            0,
            100_000,
            0,
        ));
        signed_tx
            .sign(&keypair.private_key, keypair.curve_type)
            .unwrap();

        let bytes = bcs::to_bytes(&signed_tx).unwrap();
        let decoded: SignedTransaction = bcs::from_bytes(&bytes).unwrap();

        assert!(decoded.verify_signature().unwrap());
    }

    #[test]
    fn verified_transaction_rejects_tampered_signed_payload() {
        let keypair = kanari_crypto::keys::generate_keypair(CurveType::Ed25519).unwrap();
        let mut signed_tx = SignedTransaction::new(Transaction::new_burn_with_gas(
            keypair.tagged_address(),
            0,
            0,
            100_000,
            0,
        ));
        signed_tx
            .sign(&keypair.private_key, keypair.curve_type)
            .unwrap();

        let verified = signed_tx.clone().into_verified().unwrap();
        assert_eq!(verified.hash(), signed_tx.transaction_hash());

        match &mut signed_tx.transaction {
            Transaction::ExecuteFunction { nonce, .. } => *nonce += 1,
            Transaction::PublishModule { .. }
            | Transaction::PublishPackage { .. }
            | Transaction::UpgradeModule { .. }
            | Transaction::UpgradePackage { .. } => {
                unreachable!()
            }
        }

        assert!(signed_tx.into_verified().is_err());
    }

    #[test]
    fn object_access_identity_unifies_read_write_and_gas_roles() {
        let object_ref = ObjectRef::new("0x42", Some(1), Some("digest".to_string()));
        let tx = Transaction::ExecuteFunction {
            sender: "0x1".to_string(),
            module: "example".to_string(),
            function: "call".to_string(),
            type_args: Vec::new(),
            args: Vec::new(),
            object_inputs: vec![ObjectInput {
                object_ref: object_ref.clone(),
                owner: Some(ObjectOwnerKind::AddressOwner("0x1".to_string())),
                mutable: false,
            }],
            gas_payment: Some(GasPayment {
                payment_objects: vec![object_ref],
                owner: "0x1".to_string(),
                budget: 100,
                price: 1,
            }),
            gas_limit: 100,
            gas_price: 1,
            nonce: 0,
        };

        assert_eq!(tx.object_access_keys(), vec!["object:0x42".to_string()]);
    }

    #[test]
    fn bcs_round_trips_optional_object_metadata() {
        let object_ref = ObjectRef::new("0x42", None, None);
        let bytes = bcs::to_bytes(&object_ref).unwrap();
        assert_eq!(bcs::from_bytes::<ObjectRef>(&bytes).unwrap(), object_ref);
    }

    #[test]
    fn bcs_round_trips_sparse_transaction_effects() {
        let effects = TransactionEffects {
            status: "success".to_string(),
            gas_used: 0,
            gas_payment: None,
            input_objects: Vec::new(),
            shared_inputs: Vec::new(),
            immutable_inputs: Vec::new(),
            gas_object_refs: Vec::new(),
            object_changes: Vec::new(),
            created: Vec::new(),
            mutated: Vec::new(),
            deleted: Vec::new(),
            transferred: Vec::new(),
            causal_edges: Vec::new(),
            error_message: None,
        };
        let bytes = bcs::to_bytes(&effects).unwrap();
        assert_eq!(
            bcs::from_bytes::<TransactionEffects>(&bytes).unwrap(),
            effects
        );
    }
}
