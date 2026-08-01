use crate::{
    first_array_param, internal_error_response, invalid_params_response, parse_labeled_params,
    respond_with_serialize,
};

use super::{RpcError, RpcRequest, RpcResponse, RpcServerState};
use kanari_core::engine::{PendingTransactionMetadata, PendingTransactionRecord};
use kanari_move_runtime_v1::changeset::ChangeSet;
use kanari_rpc_api::{
    BuildCallFunctionRequest, BuildNativeCoinConsolidationRequest, BuildNativeTransferRequest,
    BuildPublishModuleRequest, BuildPublishPackageRequest, BuildTokenTransferRequest,
    CallFunctionRequest, FungibleAssetTransactionsResponse, GetFungibleAssetTransactionsRequest,
    ObjectTransferData, PublishModuleRequest, PublishPackageRequest, TransactionDetails,
    TransactionErrorData, TransactionErrorReason, ViewFunctionRequest, methods,
};
use kanari_types::address::Address;
use kanari_types::coin::CoinModule;
use kanari_types::effective_gas_price;
use kanari_types::gas_coin::{GAS_COIN, GasModule};
use kanari_types::transaction::{
    GasPayment, NativeCall, ObjectInput, ObjectOwnerKind, ObjectRef, PublishedModule,
    SignedTransaction, Transaction,
};
use move_binary_format::{
    CompiledModule,
    file_format::{SignatureToken, StructHandleIndex},
};
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::TypeTag;
use rand::{TryRng, rngs::SysRng};
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, error, info};

// Extract function names from module bytecode (returns None on error)
fn extract_functions_from_bytes(bytes: &[u8]) -> Option<Vec<String>> {
    CompiledModule::deserialize_with_defaults(bytes)
        .ok()
        .map(|module| {
            module
                .function_defs()
                .iter()
                .map(|func_def| {
                    let fh = module.function_handle_at(func_def.function);
                    module.identifier_at(fh.name).as_str().to_string()
                })
                .collect()
        })
}

// Best-effort lookup of module functions using the engine registry
fn lookup_module_functions(state: &RpcServerState, module_str: &str) -> Option<Vec<String>> {
    // If given as "address::Name" try direct lookup
    if let Some(idx) = module_str.find("::") {
        let addr = &module_str[..idx];
        let name = &module_str[idx + 2..];
        if let Some(bytes) = state.engine.get_module_bytecode(addr, name) {
            return extract_functions_from_bytes(&bytes);
        }
    }

    // Search published modules by name or address
    let modules = state.engine.list_all_modules();
    let results: Vec<String> = modules
        .into_iter()
        .filter(|(addr, name)| name == module_str || addr == module_str)
        .filter_map(|(addr, name)| {
            state
                .engine
                .get_module_bytecode(&addr, &name)
                .and_then(|bytes| extract_functions_from_bytes(&bytes))
                .map(|fns| {
                    fns.into_iter()
                        .map(move |f| format!("{}::{}::{}", addr, name, f))
                })
        })
        .flatten()
        .collect();

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

fn is_tx_context_token(module: &CompiledModule, token: &SignatureToken) -> bool {
    let struct_idx = match token {
        SignatureToken::Struct(idx) => *idx,
        SignatureToken::StructInstantiation(instantiation) => instantiation.0,
        _ => return false,
    };
    let struct_handle = module.struct_handle_at(struct_idx);
    let module_handle = module.module_handle_at(struct_handle.module);
    let address = module.address_identifier_at(module_handle.address);
    let module_name = module.identifier_at(module_handle.name);
    let struct_name = module.identifier_at(struct_handle.name);
    *address == Address::kanari_system_account_address()
        && module_name.as_str() == "tx_context"
        && struct_name.as_str() == "TxContext"
}

fn struct_token_has_key_ability(module: &CompiledModule, struct_idx: StructHandleIndex) -> bool {
    module.struct_handle_at(struct_idx).abilities.has_key()
}

fn token_is_object_param(module: &CompiledModule, token: &SignatureToken) -> bool {
    fn visit(module: &CompiledModule, token: &SignatureToken, by_ref: bool) -> bool {
        match token {
            SignatureToken::Reference(inner) | SignatureToken::MutableReference(inner) => {
                visit(module, inner, true)
            }
            SignatureToken::Struct(struct_idx) => {
                !is_tx_context_token(module, token)
                    && struct_token_has_key_ability(module, *struct_idx)
            }
            SignatureToken::StructInstantiation(instantiation) => {
                // Generic structs passed by reference are object bindings even
                // when dependency loading does not expose key ability metadata.
                !is_tx_context_token(module, token)
                    && (by_ref || struct_token_has_key_ability(module, instantiation.0))
            }
            _ => false,
        }
    }

    visit(module, token, false)
}

fn function_object_param_indices(
    state: &RpcServerState,
    package: &str,
    module_name: &str,
    function_name: &str,
) -> Option<HashSet<usize>> {
    let bytes = state.engine.get_module_bytecode(package, module_name)?;
    let module = CompiledModule::deserialize_with_defaults(&bytes).ok()?;
    let func_def = module.function_defs().iter().find(|func_def| {
        let handle = module.function_handle_at(func_def.function);
        module.identifier_at(handle.name).as_str() == function_name
    })?;
    let handle = module.function_handle_at(func_def.function);
    let signature = module.signature_at(handle.parameters);
    Some(
        signature
            .0
            .iter()
            .enumerate()
            .filter_map(|(index, token)| token_is_object_param(&module, token).then_some(index))
            .collect(),
    )
}

// Normalize address strings for comparison (converts to raw hex address)
fn normalize_addr(s: &str) -> String {
    use std::str::FromStr;
    Address::from_str(s)
        .map(|addr| addr.to_hex())
        .unwrap_or_else(|_| s.trim_start_matches("0x").to_lowercase())
}

fn fresh_nonce(request_id: u64, nonce: Option<u64>) -> anyhow::Result<u64> {
    if let Some(nonce) = nonce {
        anyhow::ensure!(nonce != 0, "nonce must be non-zero");
        return Ok(nonce);
    }

    static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);
    let counter = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut random = [0u8; 32];
    let mut rng = SysRng;
    rng.try_fill_bytes(&mut random)
        .map_err(|e| anyhow::anyhow!("OS randomness unavailable for nonce: {}", e))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(counter);
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"kanari-rpc-nonce-v1");
    hasher.update(&random);
    hasher.update(&request_id.to_le_bytes());
    hasher.update(&counter.to_le_bytes());
    hasher.update(&nanos.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest.as_bytes()[..8]);
    Ok(u64::from_le_bytes(bytes).max(1))
}

fn coin_token_type_from_object_type(object_type: &str) -> Option<String> {
    if let Some(start) = object_type.find('<')
        && let Some(end) = object_type.rfind('>')
    {
        let outer = &object_type[..start];
        if outer.ends_with("::coin::Coin") || outer.ends_with("::coin::coin::Coin") {
            return Some(CoinModule::normalize_token_type(
                &object_type[start + 1..end],
            ));
        }
    }

    if let Ok(TypeTag::Struct(st)) = TypeTag::from_str(object_type)
        && st.module.as_str() == "coin"
        && st.name.as_str() == "Coin"
        && let Some(TypeTag::Struct(inner)) = st.type_params.first()
    {
        return Some(format!("{}", inner));
    }

    None
}

fn build_object_input(
    object: &kanari_rpc_api::ObjectInfo,
    sender: &str,
) -> anyhow::Result<ObjectInput> {
    let owner = match &object.owner_kind {
        ObjectOwnerKind::AddressOwner(address) => {
            if normalize_addr(address) != normalize_addr(sender) {
                anyhow::bail!(
                    "Object input {} is owned by {}, not sender {}",
                    object.id,
                    address,
                    sender
                );
            }
            Some(ObjectOwnerKind::AddressOwner(address.clone()))
        }
        ObjectOwnerKind::Shared => Some(ObjectOwnerKind::Shared),
        ObjectOwnerKind::Immutable => Some(ObjectOwnerKind::Immutable),
    };

    Ok(ObjectInput {
        object_ref: ObjectRef::new(
            object.id.clone(),
            Some(object.version),
            object.digest.clone(),
        ),
        owner,
        mutable: !matches!(object.owner_kind, ObjectOwnerKind::Immutable),
    })
}

fn infer_object_inputs(
    state: &RpcServerState,
    sender: &str,
    args: &[Vec<u8>],
    candidate_arg_indices: Option<&HashSet<usize>>,
) -> anyhow::Result<Vec<ObjectInput>> {
    let state_guard = state.engine.state_read();
    let mut seen = HashSet::new();
    let mut inputs = Vec::new();

    for (index, arg) in args.iter().enumerate() {
        if let Some(indices) = candidate_arg_indices
            && !indices.contains(&index)
        {
            continue;
        }
        if arg.len() != 32 {
            continue;
        }
        let object_id = format!("0x{}", hex::encode(arg));
        if !seen.insert(object_id.clone()) {
            continue;
        }
        let Some(stored) = state_guard.get_object(&object_id)? else {
            continue;
        };
        let digest = stored.digest();
        let object = kanari_rpc_api::ObjectInfo {
            id: object_id,
            owner: format!("{:#x}", stored.owner),
            owner_kind: stored.owner_kind,
            type_: stored.type_,
            data: stored.data,
            version: stored.version,
            digest: Some(digest),
        };
        inputs.push(build_object_input(&object, sender)?);
    }

    Ok(inputs)
}

fn select_native_gas_payment(
    owned_objects: &[kanari_rpc_api::ObjectInfo],
    sender: &str,
    required_amount: u64,
    gas_limit: u64,
    gas_price: u64,
    exclude_object_ids: &[String],
    pending_access_keys: &HashSet<String>,
) -> anyhow::Result<GasPayment> {
    let excluded = exclude_object_ids
        .iter()
        .map(|id| normalize_addr(id))
        .collect::<HashSet<_>>();
    let mut best: Option<(ObjectRef, u64)> = None;
    for object in owned_objects {
        if excluded.contains(&normalize_addr(&object.id)) {
            continue;
        }
        if pending_access_keys.contains(&format!("mut:gas:{}", object.id)) {
            continue;
        }
        if object.type_ != CoinModule::coin_type(GAS_COIN) {
            continue;
        }
        let Some(balance) = CoinModule::read_balance(&object.data) else {
            continue;
        };
        if balance < required_amount {
            continue;
        }
        match &best {
            Some((_, current)) if *current <= balance => {}
            _ => {
                best = Some((
                    ObjectRef::new(
                        object.id.clone(),
                        Some(object.version),
                        object.digest.clone(),
                    ),
                    balance,
                ))
            }
        }
    }

    let (payment_object, _) = best.ok_or_else(|| {
        anyhow::anyhow!(
            "No spendable native gas coin object found with required balance {}",
            required_amount
        )
    })?;

    Ok(GasPayment {
        payment_objects: vec![payment_object],
        owner: sender.to_string(),
        budget: gas_limit,
        price: gas_price,
    })
}

fn build_call_native_burn_amount(build_data: &BuildCallFunctionRequest) -> Option<u64> {
    if normalize_addr(&build_data.package) != normalize_addr(Address::KANARI_SYSTEM_ADDRESS) {
        return None;
    }
    if build_data.module != "kanari" || build_data.function != GasModule::function_names().burn {
        return None;
    }
    if build_data.args.len() != 1 {
        return None;
    }

    bcs::from_bytes::<u64>(&build_data.args[0]).ok()
}

fn select_native_transfer_and_gas_payment(
    owned_objects: &[kanari_rpc_api::ObjectInfo],
    sender: &str,
    transfer_amount: u64,
    gas_limit: u64,
    gas_price: u64,
    pending_access_keys: &HashSet<String>,
    excluded_object_ids: &HashSet<String>,
) -> anyhow::Result<(ObjectRef, GasPayment)> {
    let native_coin_type = CoinModule::coin_type(GAS_COIN);
    let required_gas = gas_limit.saturating_mul(effective_gas_price(gas_price));
    let mut native_coins: Vec<(ObjectRef, u64)> = owned_objects
        .iter()
        .filter(|object| object.type_ == native_coin_type)
        .filter(|object| !excluded_object_ids.contains(&object.id))
        .filter(|object| !pending_access_keys.contains(&format!("mut:object:{}", object.id)))
        .filter(|object| !pending_access_keys.contains(&format!("mut:gas:{}", object.id)))
        .filter_map(|object| {
            CoinModule::read_balance(&object.data).map(|balance| {
                (
                    ObjectRef::new(
                        object.id.clone(),
                        Some(object.version),
                        object.digest.clone(),
                    ),
                    balance,
                )
            })
        })
        .collect();

    native_coins.sort_by_key(|(_, balance)| *balance);

    let mut best_pair: Option<((ObjectRef, u64), (ObjectRef, u64))> = None;
    for (transfer_ref, transfer_balance) in native_coins
        .iter()
        .filter(|(_, balance)| *balance >= transfer_amount)
    {
        let Some((gas_ref, gas_balance)) = native_coins
            .iter()
            .filter(|(gas_ref, gas_balance)| {
                gas_ref.object_id != transfer_ref.object_id && *gas_balance >= required_gas
            })
            .min_by_key(|(_, gas_balance)| *gas_balance)
            .cloned()
        else {
            continue;
        };

        let candidate = (
            (transfer_ref.clone(), *transfer_balance),
            (gas_ref, gas_balance),
        );
        // Keep the smallest sufficient coin as the long-lived gas reserve.
        // Preferring the smallest transfer coin can consume it completely and
        // strand the wallet with one funded coin plus one zero-balance coin,
        // making every subsequent native transfer violate the distinct-gas
        // policy.
        match &best_pair {
            Some(((best_transfer_ref, best_transfer_balance), (_, best_gas_balance)))
                if (
                    *best_gas_balance,
                    *best_transfer_balance,
                    &best_transfer_ref.object_id,
                ) <= (candidate.1.1, candidate.0.1, &candidate.0.0.object_id) => {}
            _ => best_pair = Some(candidate),
        }
    }

    if let Some(((coin_object_ref, _), (gas_object_ref, _))) = best_pair {
        return Ok((
            coin_object_ref,
            GasPayment {
                payment_objects: vec![gas_object_ref],
                owner: sender.to_string(),
                budget: gas_limit,
                price: gas_price,
            },
        ));
    }

    let has_transfer_coin = native_coins
        .iter()
        .any(|(_, balance)| *balance >= transfer_amount);
    if !has_transfer_coin {
        anyhow::bail!(
            "No single Coin<{}> object can cover requested amount {}",
            GAS_COIN,
            transfer_amount
        );
    }

    let has_distinct_gas_coin = native_coins
        .iter()
        .any(|(_, balance)| *balance >= required_gas);
    if !has_distinct_gas_coin {
        anyhow::bail!(
            "No spendable native gas coin object found with at least {} Mist",
            required_gas
        );
    }

    anyhow::bail!(
        "Native transfer requires two distinct Coin<{}> objects: one mutable transfer input and one separate gas payment object",
        GAS_COIN
    )
}

fn select_native_coin_consolidation_step(
    owned_objects: &[kanari_rpc_api::ObjectInfo],
    sender: &str,
    required_amount: u64,
    gas_limit: u64,
    gas_price: u64,
) -> anyhow::Result<(
    kanari_rpc_api::ObjectInfo,
    kanari_rpc_api::ObjectInfo,
    GasPayment,
)> {
    let native_coin_type = CoinModule::coin_type(GAS_COIN);
    let required_gas = gas_limit.saturating_mul(effective_gas_price(gas_price));
    let native_coins: Vec<(kanari_rpc_api::ObjectInfo, u64)> = owned_objects
        .iter()
        .filter(|object| object.type_ == native_coin_type)
        .filter_map(|object| {
            CoinModule::read_balance(&object.data).map(|balance| (object.clone(), balance))
        })
        .filter(|(_, balance)| *balance > 0)
        .collect();

    if native_coins.len() < 2 {
        anyhow::bail!(
            "Native coin consolidation requires at least two spendable Coin<{}> objects; found {}",
            GAS_COIN,
            native_coins.len()
        );
    }

    let mut gas_candidates: Vec<(kanari_rpc_api::ObjectInfo, u64)> = native_coins
        .iter()
        .filter(|(_, balance)| *balance >= required_gas)
        .cloned()
        .collect();
    gas_candidates.sort_by_key(|(_, balance)| *balance);

    for (gas_object, _) in gas_candidates {
        let mut remaining: Vec<(kanari_rpc_api::ObjectInfo, u64)> = native_coins
            .iter()
            .filter(|(object, _)| object.id != gas_object.id)
            .cloned()
            .collect();

        let remaining_total = remaining
            .iter()
            .fold(0u64, |sum, (_, balance)| sum.saturating_add(*balance));
        if remaining_total < required_amount || remaining.len() < 2 {
            continue;
        }

        remaining.sort_by_key(|(_, balance)| std::cmp::Reverse(*balance));
        let (primary_object, primary_balance) = remaining[0].clone();
        if primary_balance >= required_amount {
            continue;
        }
        let (merge_object, _) = remaining[1].clone();

        let gas_payment = GasPayment {
            payment_objects: vec![ObjectRef::new(
                gas_object.id.clone(),
                Some(gas_object.version),
                gas_object.digest.clone(),
            )],
            owner: sender.to_string(),
            budget: gas_limit,
            price: gas_price,
        };

        return Ok((primary_object, merge_object, gas_payment));
    }

    anyhow::bail!(
        "Native coin consolidation could not reserve a separate gas coin while preserving enough non-gas balance to reach {} Mist",
        required_amount
    )
}

fn select_coin_object_for_token(
    owned_objects: &[kanari_rpc_api::ObjectInfo],
    token_type: &str,
    required_amount: u64,
) -> anyhow::Result<ObjectRef> {
    let wanted_token = CoinModule::normalize_token_type(token_type);
    let mut best: Option<(ObjectRef, u64)> = None;
    let mut largest: Option<(ObjectRef, u64)> = None;

    for object in owned_objects {
        let Some(obj_token) = coin_token_type_from_object_type(&object.type_) else {
            continue;
        };
        if obj_token != wanted_token {
            continue;
        }
        let Some(balance) = CoinModule::read_balance(&object.data) else {
            continue;
        };
        let object_ref = ObjectRef::new(
            object.id.clone(),
            Some(object.version),
            object.digest.clone(),
        );
        if balance >= required_amount {
            match &best {
                Some((_, current)) if *current <= balance => {}
                _ => best = Some((object_ref.clone(), balance)),
            }
        }
        match &largest {
            Some((_, current)) if *current >= balance => {}
            _ => largest = Some((object_ref, balance)),
        }
    }

    let (selected, selected_balance) = best
        .or(largest)
        .ok_or_else(|| anyhow::anyhow!("No spendable Coin<{}> object found", wanted_token))?;
    if selected_balance < required_amount {
        anyhow::bail!(
            "No single Coin<{}> object can cover requested amount {}",
            wanted_token,
            required_amount
        );
    }
    Ok(selected)
}

fn tx_matches_owner(tx: &Transaction, owner_norm: Option<&str>) -> bool {
    let Some(owner) = owner_norm else {
        return true;
    };

    if let Some(native_call) = tx.native_call() {
        match native_call {
            NativeCall::Transfer { recipient, .. } => {
                return normalize_addr(tx.sender()) == owner || normalize_addr(&recipient) == owner;
            }
            NativeCall::Burn { .. } => {}
        }
    }

    normalize_addr(tx.sender()) == owner
}

fn token_module_path(token_type: &str) -> Option<String> {
    let token_type = CoinModule::normalize_token_type(token_type);
    let mut parts = token_type.split("::");
    let address = parts.next()?;
    let module = parts.next()?;
    let _struct_name = parts.next()?;
    Some(format!("{address}::{module}"))
}

fn tx_mentions_token_type(tx: &Transaction, token_type: &str) -> bool {
    let token_type = CoinModule::normalize_token_type(token_type);
    let target_module = token_module_path(&token_type);

    match tx {
        Transaction::ExecuteFunction {
            module,
            type_args,
            object_inputs,
            ..
        } => {
            if token_type == GAS_COIN && module == &GasModule::module_path() {
                return true;
            }

            if let Some(target_module) = &target_module
                && module == target_module
            {
                return true;
            }

            if type_args
                .iter()
                .any(|arg| CoinModule::normalize_token_type(arg) == token_type)
            {
                return true;
            }

            object_inputs
                .iter()
                .any(|input| input.object_ref.object_id.contains(token_type.as_str()))
        }
        Transaction::PublishModule { .. }
        | Transaction::PublishPackage { .. }
        | Transaction::UpgradeModule { .. }
        | Transaction::UpgradePackage { .. } => false,
    }
}

fn push_unique_tx_details(
    results: &mut Vec<TransactionDetails>,
    seen_hashes: &mut HashSet<String>,
    limit: usize,
    details: TransactionDetails,
) -> bool {
    if results.len() >= limit {
        return false;
    }

    if !seen_hashes.insert(details.hash.to_lowercase()) {
        return true;
    }

    results.push(details);
    true
}

fn parse_hex_address(id: u64, raw: &str, field: &str) -> Result<Address, Box<RpcResponse>> {
    Address::from_hex_literal(raw).map_err(|e| {
        Box::new(invalid_params_response(
            id,
            format!("Invalid {}: {}", field, e),
        ))
    })
}

fn serialize_move_arg<T: serde::Serialize>(
    id: u64,
    value: &T,
    field: &str,
) -> Result<Vec<u8>, Box<RpcResponse>> {
    bcs::to_bytes(value).map_err(|error| {
        Box::new(internal_error_response(
            id,
            format!("Failed to serialize {field}: {error}"),
        ))
    })
}

fn extract_hash_param(params: &serde_json::Value) -> Option<String> {
    serde_json::from_value::<String>(params.clone())
        .ok()
        .or_else(|| {
            params
                .get("hash")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
}

fn build_publish_signed_tx(module_data: PublishModuleRequest, upgrade: bool) -> SignedTransaction {
    let nonce = module_data
        .canonical_nonce()
        .expect("publish request canonical nonce must be validated before signing");
    let transaction = if upgrade {
        Transaction::UpgradeModule {
            sender: module_data.sender,
            module_bytes: module_data.module_bytes,
            module_name: module_data.module_name,
            gas_payment: module_data.gas_payment,
            gas_limit: module_data.gas_limit,
            gas_price: module_data.gas_price,
            nonce,
        }
    } else {
        Transaction::PublishModule {
            sender: module_data.sender,
            module_bytes: module_data.module_bytes,
            module_name: module_data.module_name,
            gas_payment: module_data.gas_payment,
            gas_limit: module_data.gas_limit,
            gas_price: module_data.gas_price,
            nonce,
        }
    };
    let mut signed_tx = SignedTransaction::new(transaction);
    maybe_attach_signature(&mut signed_tx, module_data.signature);
    signed_tx
}

fn build_publish_package_signed_tx(
    package_data: PublishPackageRequest,
    upgrade: bool,
) -> SignedTransaction {
    let nonce = package_data
        .canonical_nonce()
        .expect("publish package request canonical nonce must be validated before signing");
    let modules = package_data
        .modules
        .into_iter()
        .map(|module| PublishedModule {
            module_name: module.module_name,
            module_bytes: module.module_bytes,
        })
        .collect();
    let transaction = if upgrade {
        Transaction::UpgradePackage {
            sender: package_data.sender,
            modules,
            gas_payment: package_data.gas_payment,
            gas_limit: package_data.gas_limit,
            gas_price: package_data.gas_price,
            nonce,
        }
    } else {
        Transaction::PublishPackage {
            sender: package_data.sender,
            modules,
            gas_payment: package_data.gas_payment,
            gas_limit: package_data.gas_limit,
            gas_price: package_data.gas_price,
            nonce,
        }
    };
    let mut signed_tx = SignedTransaction::new(transaction);
    maybe_attach_signature(&mut signed_tx, package_data.signature);
    signed_tx
}

fn build_call_signed_tx(call_data: CallFunctionRequest) -> SignedTransaction {
    let nonce = call_data
        .canonical_nonce()
        .expect("call request canonical nonce must be validated before signing");
    let mut signed_tx = SignedTransaction::new(Transaction::ExecuteFunction {
        sender: call_data.sender.clone(),
        module: format!("{}::{}", call_data.package, call_data.module),
        function: call_data.function,
        type_args: call_data.type_args,
        args: call_data.args,
        object_inputs: call_data.object_inputs.unwrap_or_default(),
        gas_payment: call_data.gas_payment,
        gas_limit: call_data.gas_limit,
        gas_price: call_data.gas_price,
        nonce,
    });
    maybe_attach_signature(&mut signed_tx, call_data.signature);
    signed_tx
}

fn base_transaction_details(
    hash: String,
    status: String,
    block_height: Option<u64>,
    tx_type: &str,
    sender: String,
    sender_address: String,
    nonce: u64,
    gas_limit: u64,
    gas_price: u64,
) -> TransactionDetails {
    let (success, previewed, submitted, committed) = derive_transaction_state_flags(&status);
    TransactionDetails {
        hash,
        status,
        block_height,
        gas_used: None,
        success,
        previewed,
        submitted,
        committed,
        tx_type: tx_type.to_string(),
        sender,
        sender_address: Some(sender_address),
        nonce: Some(nonce),
        gas_limit,
        gas_price,
        object_inputs: None,
        gas_payment: None,
        effects: None,
        module: None,
        function: None,
        module_functions: None,
    }
}

fn derive_transaction_state_flags(status: &str) -> (bool, bool, bool, bool) {
    match status {
        "failed" => (false, true, false, false),
        "simulated_pending" => (true, true, true, false),
        "pending" => (true, false, true, false),
        "committed" => (true, false, true, true),
        "executed" => (true, true, true, false),
        _ => (false, false, false, false),
    }
}

fn pending_status(record: &PendingTransactionRecord) -> &'static str {
    if record.metadata.previewed {
        "simulated_pending"
    } else {
        "pending"
    }
}

fn apply_pending_preview_metadata(
    record: &PendingTransactionRecord,
    details: &mut TransactionDetails,
) {
    if let Some(gas_used) = record.metadata.preview_gas_used {
        details.gas_used = Some(gas_used);
    }
    if let Some(effects) = &record.metadata.preview_effects {
        details.effects = Some(effects.clone());
    }
}

fn apply_committed_effect(
    details: &mut TransactionDetails,
    effect: Option<&kanari_types::transaction::TransactionEffects>,
) {
    let Some(effect) = effect else {
        return;
    };
    let execution_succeeded = effect.status == "success";
    details.status = if execution_succeeded {
        "committed".to_string()
    } else {
        "failed".to_string()
    };
    details.success = execution_succeeded;
    details.previewed = false;
    details.submitted = true;
    // A failed Move execution is still final once its Mysticeti sub-DAG commits.
    details.committed = true;
    details.gas_used = Some(effect.gas_used);
    details.effects = Some(effect.clone());
}

// =========================================================================
// HELPER: Map Transaction to API TransactionDetails
// =========================================================================
fn map_transaction_to_details(
    state: &RpcServerState,
    tx: &Transaction,
    tx_hash_hex: &str,
    status: &str,
    block_height: Option<u64>,
    _state_root_hex: Option<String>,
) -> TransactionDetails {
    let hash = format!("0x{}", tx_hash_hex);
    let status_str = status.to_string();
    let sender_address = Address::parse_to_account_address(tx.sender_address())
        .map(|addr| addr.to_hex_literal())
        .unwrap_or_else(|_| tx.sender_address().to_string());

    match tx {
        Transaction::PublishModule {
            sender,
            module_bytes,
            module_name,
            nonce,
            gas_payment,
            gas_limit,
            gas_price,
            ..
        }
        | Transaction::UpgradeModule {
            sender,
            module_bytes,
            module_name,
            nonce,
            gas_payment,
            gas_limit,
            gas_price,
            ..
        } => {
            let tx_type = tx.tx_type_label();
            let module_prefix = sender_address.clone();
            let module_funcs = extract_functions_from_bytes(module_bytes).map(|fns| {
                fns.into_iter()
                    .map(|f| format!("{}::{}::{}", module_prefix, module_name, f))
                    .collect()
            });

            let mut details = base_transaction_details(
                hash,
                status_str,
                block_height,
                tx_type,
                sender.clone(),
                sender_address.clone(),
                *nonce,
                *gas_limit,
                *gas_price,
            );
            details.module = Some(format!("{}::{}", sender_address, module_name));
            details.module_functions = module_funcs;
            details.gas_payment = gas_payment.clone();
            details
        }
        Transaction::PublishPackage {
            sender,
            modules,
            nonce,
            gas_payment,
            gas_limit,
            gas_price,
            ..
        }
        | Transaction::UpgradePackage {
            sender,
            modules,
            nonce,
            gas_payment,
            gas_limit,
            gas_price,
            ..
        } => {
            let tx_type = tx.tx_type_label();
            let published_modules = modules
                .iter()
                .map(|module| format!("{}::{}", sender_address, module.module_name))
                .collect::<Vec<_>>();
            let module_functions = modules
                .iter()
                .flat_map(|module| {
                    extract_functions_from_bytes(&module.module_bytes)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|function| {
                            format!("{}::{}::{}", sender_address, module.module_name, function)
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            let mut details = base_transaction_details(
                hash,
                status_str,
                block_height,
                tx_type,
                sender.clone(),
                sender_address.clone(),
                *nonce,
                *gas_limit,
                *gas_price,
            );
            details.module = Some(published_modules.join(", "));
            details.module_functions = Some(module_functions);
            details.gas_payment = gas_payment.clone();
            details
        }
        Transaction::ExecuteFunction {
            sender,
            module,
            function,
            nonce,
            gas_limit,
            gas_price,
            ..
        } => {
            let mut details = base_transaction_details(
                hash,
                status_str,
                block_height,
                tx.tx_type_label(),
                sender.clone(),
                sender_address.clone(),
                *nonce,
                *gas_limit,
                *gas_price,
            );
            details.module = Some(module.clone());
            details.function = Some(function.clone());
            details.module_functions = lookup_module_functions(state, module);
            let object_inputs = tx.object_inputs();
            if !object_inputs.is_empty() {
                details.object_inputs = Some(object_inputs);
            }
            details.gas_payment = tx.gas_payment();
            if let Some(native_call) = tx.native_call() {
                match native_call {
                    NativeCall::Transfer {
                        coin_object_id,
                        recipient,
                        ..
                    } => {
                        details.module = Some(format!("To: {} via {}", recipient, coin_object_id));
                    }
                    NativeCall::Burn { .. } => {}
                }
            }
            details
        }
    }
}

// =========================================================================
// HELPER: Format ChangeSet JSON for API responses
// =========================================================================
fn format_changeset_json(state: &RpcServerState, changeset: &ChangeSet) -> serde_json::Value {
    let mut cs_value = serde_json::to_value(changeset).unwrap_or(serde_json::json!(null));

    let state_guard = state.engine.state_read();

    if let Some(map) = cs_value.as_object_mut()
        && let Some(created_val) = map.get_mut("created_objects")
        && let Some(arr) = created_val.as_array_mut()
    {
        for entry in arr.iter_mut() {
            if let Some(obj) = entry.get_mut(1).and_then(|o| o.as_object_mut()) {
                // 1. Truncate Data payload for cleaner JSON output
                if let Some(data_val) = obj.get_mut("data")
                    && let Some(data_arr) = data_val.as_array_mut()
                {
                    let original_len = data_arr.len();
                    let max_len = 32usize;
                    let bytes: Vec<u8> = data_arr
                        .iter()
                        .take(max_len)
                        .filter_map(|v| v.as_u64().map(|n| n as u8))
                        .collect();

                    *data_val = serde_json::Value::String(format!("0x{}", hex::encode(bytes)));
                    if original_len > max_len {
                        obj.insert("data_truncated".to_string(), serde_json::Value::Bool(true));
                        obj.insert("data_len".to_string(), serde_json::json!(original_len));
                    }
                }

                // 2. Normalize noisy Move-VM Types to clean persisted types
                if let Some(type_val) = obj.get("type").and_then(|v| v.as_str())
                    && (type_val.contains("StructInstantiation")
                        || type_val.contains("CachedStructIndex"))
                    && let Some(id) = obj.get("id").and_then(|v| v.as_str())
                {
                    let id_norm = id.trim_start_matches("0x");
                    // Try direct or normalized ID
                    if let Ok(Some(stored)) = state_guard
                        .get_object(id)
                        .or_else(|_| state_guard.get_object(id_norm))
                    {
                        obj.insert(
                            "type".to_string(),
                            serde_json::Value::String(stored.type_.clone()),
                        );
                    }
                }
            }
        }
    }
    if let Some(map) = cs_value.as_object_mut() {
        map.insert(
            "effects".to_string(),
            serde_json::to_value(changeset.effects(None)).unwrap_or(serde_json::json!(null)),
        );
    }
    cs_value
}

fn validate_object_ref_completeness(
    id: u64,
    field: &str,
    object_ref: &ObjectRef,
) -> Result<(), Box<RpcResponse>> {
    if object_ref.version.is_some() ^ object_ref.digest.is_some() {
        return Err(Box::new(invalid_params_response(
            id,
            format!("{field} must include both version and digest when either is provided"),
        )));
    }
    Ok(())
}

fn canonical_rpc_object_id(
    id: u64,
    field: &str,
    object_id: &str,
) -> Result<String, Box<RpcResponse>> {
    AccountAddress::from_hex_literal(object_id)
        .map(|address| address.to_hex_literal())
        .map_err(|err| {
            Box::new(invalid_params_response(
                id,
                format!("{field} must be a valid object id: {err}"),
            ))
        })
}

fn validate_object_inputs_and_gas(
    id: u64,
    object_inputs: &[kanari_types::transaction::ObjectInput],
    gas_payment: Option<&kanari_types::transaction::GasPayment>,
) -> Result<(), Box<RpcResponse>> {
    let mut mutable_object_ids = HashSet::new();
    for (index, input) in object_inputs.iter().enumerate() {
        validate_object_ref_completeness(
            id,
            &format!("object_inputs[{index}].object_ref"),
            &input.object_ref,
        )?;
        let canonical_object_id = canonical_rpc_object_id(
            id,
            &format!("object_inputs[{index}].object_ref.object_id"),
            &input.object_ref.object_id,
        )?;
        if input.mutable && !mutable_object_ids.insert(canonical_object_id) {
            return Err(Box::new(invalid_params_response(
                id,
                format!(
                    "Duplicate mutable object input {} is not allowed",
                    input.object_ref.object_id
                ),
            )));
        }
    }

    if let Some(gas_payment) = gas_payment {
        let mut gas_object_ids = HashSet::new();
        for (index, payment) in gas_payment.payment_objects.iter().enumerate() {
            validate_object_ref_completeness(
                id,
                &format!("gas_payment.payment_objects[{index}]"),
                payment,
            )?;
            let canonical_payment_id = canonical_rpc_object_id(
                id,
                &format!("gas_payment.payment_objects[{index}].object_id"),
                &payment.object_id,
            )?;
            if !gas_object_ids.insert(canonical_payment_id.clone()) {
                return Err(Box::new(invalid_params_response(
                    id,
                    format!(
                        "Duplicate gas payment object {} is not allowed",
                        payment.object_id
                    ),
                )));
            }
            if mutable_object_ids.contains(&canonical_payment_id) {
                return Err(Box::new(invalid_params_response(
                    id,
                    format!(
                        "Gas payment object {} cannot overlap with a mutable object input",
                        payment.object_id
                    ),
                )));
            }
        }
    }

    Ok(())
}

fn validate_object_inputs_match_state(
    state: &RpcServerState,
    id: u64,
    object_inputs: &[kanari_types::transaction::ObjectInput],
) -> Result<(), Box<RpcResponse>> {
    for input in object_inputs {
        match state.engine.get_object_by_ref(&input.object_ref) {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(Box::new(invalid_params_response(
                    id,
                    format!(
                        "Object input {} does not match current state ref",
                        input.object_ref.object_id
                    ),
                )));
            }
            Err(err) => {
                return Err(Box::new(internal_error_response(
                    id,
                    format!(
                        "Failed to validate object input {}: {}",
                        input.object_ref.object_id, err
                    ),
                )));
            }
        }
    }

    Ok(())
}

fn maybe_attach_signature(signed_tx: &mut SignedTransaction, signature: Option<Vec<u8>>) {
    if let Some(sig) = signature {
        signed_tx.signature = sig;
    }
}

fn classify_transaction_error_data(message: &str) -> Option<TransactionErrorData> {
    let data = if message.contains("must be Coin<") {
        TransactionErrorData::new(TransactionErrorReason::InvalidGasPaymentType)
    } else if message.contains("cannot overlap with a mutable object input") {
        TransactionErrorData::new(TransactionErrorReason::GasPaymentObjectOverlap)
    } else if message.contains("Gas payment object") && message.contains("does not exist") {
        TransactionErrorData::new(TransactionErrorReason::GasPaymentObjectNotFound)
    } else if (message.contains("Gas payment object") && message.contains("is not owned by sender"))
        || message.contains("Gas payment owner must match sender")
    {
        TransactionErrorData::new(TransactionErrorReason::GasPaymentOwnerMismatch)
    } else if message.contains("Gas payment version mismatch") {
        TransactionErrorData::new(TransactionErrorReason::GasPaymentVersionMismatch)
    } else if message.contains("Gas payment digest mismatch") {
        TransactionErrorData::new(TransactionErrorReason::GasPaymentDigestMismatch)
    } else if message.contains("requires at least two spendable Coin<") {
        TransactionErrorData::with_native_transfer_policy(
            TransactionErrorReason::InsufficientNativeCoinObjects,
        )
    } else if message.contains("could not reserve a separate gas coin") {
        TransactionErrorData::with_native_transfer_policy(
            TransactionErrorReason::NativeCoinConsolidationBlocked,
        )
    } else if message.contains("No single Coin<") && message.contains("can cover requested amount")
    {
        TransactionErrorData::with_native_transfer_policy(
            TransactionErrorReason::InsufficientTransferCoinBalance,
        )
    } else if message.contains("No spendable native gas coin object found with at least") {
        TransactionErrorData::with_native_transfer_policy(
            TransactionErrorReason::InsufficientGasCoinBalance,
        )
    } else if message.contains("Native transfer requires two distinct Coin<") {
        TransactionErrorData::with_native_transfer_policy(
            TransactionErrorReason::NativeTransferPolicyNotSatisfied,
        )
    } else {
        return None;
    };

    Some(data)
}

fn transaction_error_with_reason(message: impl Into<String>) -> RpcError {
    let message = message.into();
    if let Some(data) = classify_transaction_error_data(&message) {
        RpcError::transaction_error_structured(message, data)
    } else {
        RpcError::transaction_error(message)
    }
}

fn submit_pending_response(
    state: &RpcServerState,
    request_id: u64,
    signed_tx: SignedTransaction,
    action: &str,
    submit_error: &str,
) -> RpcResponse {
    let tx_for_broadcast = signed_tx.clone();
    match state.engine.submit_transactions_batch(vec![signed_tx]) {
        Ok(tx_hashes) => {
            let tx_hash = hex::encode(&tx_hashes[0]);
            debug!("{} accepted into mempool: {}", action, tx_hash);
            state.broadcast_submitted_transaction(tx_for_broadcast);

            respond_with_serialize(
                request_id,
                serde_json::json!({
                    "hash": tx_hash,
                    "status": "pending",
                    "action": action,
                    "success": true,
                    "previewed": false,
                    "submitted": true,
                    "committed": false
                }),
            )
        }
        Err(e) => RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(transaction_error_with_reason(format!(
                "{}: {}",
                submit_error, e
            ))),
            id: request_id,
        },
    }
}

async fn submit_after_immediate_execution(
    state: &RpcServerState,
    signed_tx: SignedTransaction,
    metadata: PendingTransactionMetadata,
) -> anyhow::Result<()> {
    state
        .engine
        .submit_transactions_batch_with_metadata(vec![signed_tx], metadata)
        .map(|_| ())
}

async fn execute_or_submit_response(
    state: &RpcServerState,
    request_id: u64,
    signed_tx: SignedTransaction,
    execute_immediate: bool,
    action: &str,
    pending_submit_error: &str,
) -> RpcResponse {
    if signed_tx.signature.is_empty() {
        return invalid_params_response(request_id, "Missing or empty signature");
    }

    if !execute_immediate {
        return submit_pending_response(state, request_id, signed_tx, action, pending_submit_error);
    }

    let exec_tx = signed_tx.clone();
    match state.engine.execute_transaction_immediate(exec_tx) {
        Ok((tx_hash, changeset)) => {
            let tx_hash_hex = hex::encode(&tx_hash);
            let _cs_value = format_changeset_json(state, &changeset);

            if !changeset.success {
                return respond_with_serialize(
                    request_id,
                    kanari_rpc_api::TransactionResult {
                        hash: tx_hash_hex,
                        status: "failed".to_string(),
                        gas_used: Some(changeset.gas_used),
                        success: false,
                        previewed: true,
                        submitted: false,
                        committed: false,
                        effects: Some(changeset.effects(None)),
                        error_message: changeset.error_message.clone(),
                    },
                );
            }

            let preview_effects = changeset.effects(None);
            let tx_for_broadcast = signed_tx.clone();
            if let Err(e) = submit_after_immediate_execution(
                state,
                signed_tx,
                PendingTransactionMetadata {
                    previewed: true,
                    preview_gas_used: Some(changeset.gas_used),
                    preview_effects: Some(preview_effects.clone()),
                },
            )
            .await
            {
                error!("Failed to submit executed transaction: {}", e);
                return RpcResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(transaction_error_with_reason(format!(
                        "Simulation successful, but mempool submission failed: {}",
                        e
                    ))),
                    id: request_id,
                };
            }
            state.broadcast_submitted_transaction(tx_for_broadcast);

            info!(
                "{} previewed immediately & submitted pending: {}",
                action, tx_hash_hex
            );
            respond_with_serialize(
                request_id,
                kanari_rpc_api::TransactionResult {
                    hash: tx_hash_hex,
                    status: "simulated_pending".to_string(),
                    gas_used: Some(changeset.gas_used),
                    success: true,
                    previewed: true,
                    submitted: true,
                    committed: false,
                    effects: Some(preview_effects),
                    error_message: None,
                },
            )
        }
        Err(e) => RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(transaction_error_with_reason(format!(
                "Immediate execution failed: {}",
                e
            ))),
            id: request_id,
        },
    }
}

fn select_build_gas_and_nonce(
    state: &RpcServerState,
    request_id: u64,
    sender: &str,
    gas_limit: u64,
    gas_price: u64,
    nonce: Option<u64>,
) -> Result<(GasPayment, u64), Box<RpcResponse>> {
    parse_hex_address(request_id, sender, "sender address")?;

    let owner_info = state
        .engine
        .get_owner_info(sender)
        .ok_or_else(|| Box::new(internal_error_response(request_id, "Owner not found")))?;
    let owned_objects = owner_info.owned_objects.ok_or_else(|| {
        Box::new(internal_error_response(
            request_id,
            "Owner has no owned object list",
        ))
    })?;
    let pending_access_keys = state.engine.pending_access_keys_snapshot();
    let gas_payment = select_native_gas_payment(
        &owned_objects,
        sender,
        gas_limit.saturating_mul(effective_gas_price(gas_price)),
        gas_limit,
        gas_price,
        &[],
        &pending_access_keys,
    )
    .map_err(|error| {
        Box::new(RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(transaction_error_with_reason(error.to_string())),
            id: request_id,
        })
    })?;
    let nonce = fresh_nonce(request_id, nonce)
        .map_err(|error| Box::new(internal_error_response(request_id, error.to_string())))?;

    Ok((gas_payment, nonce))
}

// =========================================================================
// HANDLERS
// =========================================================================

pub async fn handle_build_publish_module(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let build_data: BuildPublishModuleRequest =
        match parse_labeled_params(request.id, &request.params, "build publish data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    let (gas_payment, nonce) = match select_build_gas_and_nonce(
        state,
        request.id,
        &build_data.sender,
        build_data.gas_limit,
        build_data.gas_price,
        build_data.nonce,
    ) {
        Ok(prepared) => prepared,
        Err(response) => return *response,
    };

    respond_with_serialize(
        request.id,
        PublishModuleRequest {
            sender: build_data.sender,
            module_bytes: build_data.module_bytes,
            module_name: build_data.module_name,
            gas_limit: build_data.gas_limit,
            gas_price: build_data.gas_price,
            nonce: Some(nonce),
            gas_payment: Some(gas_payment),
            signature: None,
            execute_immediate: build_data.execute_immediate,
        },
    )
}

pub async fn handle_build_publish_package(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let build_data: BuildPublishPackageRequest =
        match parse_labeled_params(request.id, &request.params, "build publish package data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    if build_data.modules.is_empty() {
        return invalid_params_response(request.id, "Package publish requires at least one module");
    }

    let (gas_payment, nonce) = match select_build_gas_and_nonce(
        state,
        request.id,
        &build_data.sender,
        build_data.gas_limit,
        build_data.gas_price,
        build_data.nonce,
    ) {
        Ok(prepared) => prepared,
        Err(response) => return *response,
    };

    respond_with_serialize(
        request.id,
        PublishPackageRequest {
            sender: build_data.sender,
            modules: build_data.modules,
            gas_limit: build_data.gas_limit,
            gas_price: build_data.gas_price,
            nonce: Some(nonce),
            gas_payment: Some(gas_payment),
            signature: None,
            execute_immediate: build_data.execute_immediate,
        },
    )
}

pub async fn handle_build_native_transfer(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let build_data: BuildNativeTransferRequest =
        match parse_labeled_params(request.id, &request.params, "build native transfer data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    if let Err(response) = parse_hex_address(request.id, &build_data.sender, "sender address") {
        return *response;
    }
    if let Err(response) = parse_hex_address(request.id, &build_data.recipient, "recipient") {
        return *response;
    }

    let Some(owner_info) = state.engine.get_owner_info(&build_data.sender) else {
        return internal_error_response(request.id, "Owner not found");
    };
    let Some(owned_objects) = owner_info.owned_objects else {
        return internal_error_response(request.id, "Owner has no owned object list");
    };

    let pending_access_keys = state.engine.pending_access_keys_snapshot();
    let (coin_object_ref, gas_payment) = match select_native_transfer_and_gas_payment(
        &owned_objects,
        &build_data.sender,
        build_data.amount,
        build_data.gas_limit,
        build_data.gas_price,
        &pending_access_keys,
        &build_data
            .excluded_object_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>(),
    ) {
        Ok(payment) => payment,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(transaction_error_with_reason(format!(
                    "{}. Native KANARI transfer is a Move object call, so the gas object must not overlap the mutable transfer input.",
                    e
                ))),
                id: request.id,
            };
        }
    };
    let nonce = match fresh_nonce(request.id, build_data.nonce) {
        Ok(nonce) => nonce,
        Err(e) => return internal_error_response(request.id, e.to_string()),
    };
    respond_with_serialize(
        request.id,
        ObjectTransferData {
            sender: build_data.sender,
            coin_object_id: coin_object_ref.object_id.clone(),
            coin_object_ref: Some(coin_object_ref),
            recipient: build_data.recipient,
            amount: build_data.amount,
            gas_limit: build_data.gas_limit,
            gas_price: build_data.gas_price,
            nonce: Some(nonce),
            gas_payment: Some(gas_payment),
            signature: None,
            execute_immediate: build_data.execute_immediate,
        },
    )
}

pub async fn handle_build_native_coin_consolidation(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let build_data: BuildNativeCoinConsolidationRequest = match parse_labeled_params(
        request.id,
        &request.params,
        "build native coin consolidation data",
    ) {
        Ok(data) => data,
        Err(response) => return *response,
    };

    if let Err(response) = parse_hex_address(request.id, &build_data.sender, "sender address") {
        return *response;
    }

    let Some(owner_info) = state.engine.get_owner_info(&build_data.sender) else {
        return internal_error_response(request.id, "Owner not found");
    };
    let Some(owned_objects) = owner_info.owned_objects else {
        return internal_error_response(request.id, "Owner has no owned object list");
    };

    let (primary_object, merge_object, gas_payment) = match select_native_coin_consolidation_step(
        &owned_objects,
        &build_data.sender,
        build_data.required_amount,
        build_data.gas_limit,
        build_data.gas_price,
    ) {
        Ok(selection) => selection,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(transaction_error_with_reason(e.to_string())),
                id: request.id,
            };
        }
    };

    let primary_input = match build_object_input(&primary_object, &build_data.sender) {
        Ok(input) => input,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(transaction_error_with_reason(e.to_string())),
                id: request.id,
            };
        }
    };
    let merge_input = match build_object_input(&merge_object, &build_data.sender) {
        Ok(input) => input,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(transaction_error_with_reason(e.to_string())),
                id: request.id,
            };
        }
    };
    let nonce = match fresh_nonce(request.id, build_data.nonce) {
        Ok(nonce) => nonce,
        Err(e) => return internal_error_response(request.id, e.to_string()),
    };
    let primary_id = match parse_hex_address(request.id, &primary_object.id, "primary object id") {
        Ok(address) => address.to_vec(),
        Err(response) => return *response,
    };
    let merge_id = match parse_hex_address(request.id, &merge_object.id, "merge object id") {
        Ok(address) => address.to_vec(),
        Err(response) => return *response,
    };

    respond_with_serialize(
        request.id,
        CallFunctionRequest {
            sender: build_data.sender,
            package: Address::KANARI_SYSTEM_ADDRESS.to_string(),
            module: CoinModule::COIN_MODULE.to_string(),
            function: CoinModule::function_names().join_entry.to_string(),
            type_args: vec![GAS_COIN.to_string()],
            args: vec![primary_id, merge_id],
            object_inputs: Some(vec![primary_input, merge_input]),
            gas_limit: build_data.gas_limit,
            gas_price: build_data.gas_price,
            nonce: Some(nonce),
            gas_payment: Some(gas_payment),
            signature: None,
            execute_immediate: build_data.execute_immediate,
        },
    )
}

pub async fn handle_build_call_function(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let build_data: BuildCallFunctionRequest =
        match parse_labeled_params(request.id, &request.params, "build call data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    if let Err(response) = parse_hex_address(request.id, &build_data.sender, "sender address") {
        return *response;
    }
    if let Err(response) = parse_hex_address(request.id, &build_data.package, "package address") {
        return *response;
    }

    let Some(owner_info) = state.engine.get_owner_info(&build_data.sender) else {
        return internal_error_response(request.id, "Owner not found");
    };
    let Some(owned_objects) = owner_info.owned_objects else {
        return internal_error_response(request.id, "Owner has no owned object list");
    };

    let object_ref_arg_indices = function_object_param_indices(
        state,
        &build_data.package,
        &build_data.module,
        &build_data.function,
    );
    let object_inputs = if let Some(inputs) = build_data.object_inputs.clone() {
        if let Err(response) = validate_object_inputs_and_gas(request.id, &inputs, None) {
            return *response;
        }
        if let Err(response) = validate_object_inputs_match_state(state, request.id, &inputs) {
            return *response;
        }
        inputs
    } else {
        match infer_object_inputs(
            state,
            &build_data.sender,
            &build_data.args,
            object_ref_arg_indices.as_ref(),
        ) {
            Ok(inputs) => inputs,
            Err(e) => {
                return RpcResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(transaction_error_with_reason(e.to_string())),
                    id: request.id,
                };
            }
        }
    };
    let exclude_ids = object_inputs
        .iter()
        .filter(|input| input.mutable)
        .map(|input| input.object_ref.object_id.clone())
        .collect::<Vec<_>>();
    let pending_access_keys = state.engine.pending_access_keys_snapshot();
    let burn_amount = build_call_native_burn_amount(&build_data);
    let required_gas_balance = build_data
        .gas_limit
        .saturating_mul(effective_gas_price(build_data.gas_price));
    let required_native_balance = burn_amount
        .and_then(|amount| amount.checked_add(required_gas_balance))
        .unwrap_or(required_gas_balance);
    let gas_payment = match select_native_gas_payment(
        &owned_objects,
        &build_data.sender,
        required_native_balance,
        build_data.gas_limit,
        build_data.gas_price,
        &exclude_ids,
        &pending_access_keys,
    ) {
        Ok(payment) => payment,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(transaction_error_with_reason(e.to_string())),
                id: request.id,
            };
        }
    };
    let nonce = match fresh_nonce(request.id, build_data.nonce) {
        Ok(nonce) => nonce,
        Err(e) => return internal_error_response(request.id, e.to_string()),
    };

    respond_with_serialize(
        request.id,
        CallFunctionRequest {
            sender: build_data.sender,
            package: build_data.package,
            module: build_data.module,
            function: build_data.function,
            type_args: build_data.type_args,
            args: build_data.args,
            object_inputs: if object_inputs.is_empty() {
                None
            } else {
                Some(object_inputs)
            },
            gas_limit: build_data.gas_limit,
            gas_price: build_data.gas_price,
            nonce: Some(nonce),
            gas_payment: Some(gas_payment),
            signature: None,
            execute_immediate: build_data.execute_immediate,
        },
    )
}

pub async fn handle_build_token_transfer(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let build_data: BuildTokenTransferRequest =
        match parse_labeled_params(request.id, &request.params, "build token transfer data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    if let Err(response) = parse_hex_address(request.id, &build_data.sender, "sender address") {
        return *response;
    }
    let recipient = match parse_hex_address(request.id, &build_data.recipient, "recipient") {
        Ok(address) => address,
        Err(response) => return *response,
    };

    let Some(owner_info) = state.engine.get_owner_info(&build_data.sender) else {
        return internal_error_response(request.id, "Owner not found");
    };
    let Some(owned_objects) = owner_info.owned_objects else {
        return internal_error_response(request.id, "Owner has no owned object list");
    };

    let coin_object_ref = match select_coin_object_for_token(
        &owned_objects,
        &build_data.token_type,
        build_data.amount,
    ) {
        Ok(selected) => selected,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(transaction_error_with_reason(e.to_string())),
                id: request.id,
            };
        }
    };
    let object = match state.engine.get_object_by_ref(&coin_object_ref) {
        Ok(Some(object)) => object,
        Ok(None) => return internal_error_response(request.id, "Selected token object not found"),
        Err(e) => return internal_error_response(request.id, e.to_string()),
    };
    let object_input = match build_object_input(&object, &build_data.sender) {
        Ok(input) => input,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(transaction_error_with_reason(e.to_string())),
                id: request.id,
            };
        }
    };
    let gas_payment = match select_native_gas_payment(
        &owned_objects,
        &build_data.sender,
        build_data
            .gas_limit
            .saturating_mul(effective_gas_price(build_data.gas_price)),
        build_data.gas_limit,
        build_data.gas_price,
        std::slice::from_ref(&coin_object_ref.object_id),
        &state.engine.pending_access_keys_snapshot(),
    ) {
        Ok(payment) => payment,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(transaction_error_with_reason(e.to_string())),
                id: request.id,
            };
        }
    };

    let module_parts: Vec<&str> = build_data.token_type.split("::").collect();
    if module_parts.len() < 3 {
        return invalid_params_response(
            request.id,
            "Invalid token type format. Expected address::module::struct",
        );
    }
    let nonce = match fresh_nonce(request.id, build_data.nonce) {
        Ok(nonce) => nonce,
        Err(e) => return internal_error_response(request.id, e.to_string()),
    };
    let coin_object_id =
        match parse_hex_address(request.id, &coin_object_ref.object_id, "coin object id") {
            Ok(address) => address.to_vec(),
            Err(response) => return *response,
        };
    let amount = match serialize_move_arg(request.id, &build_data.amount, "transfer amount") {
        Ok(amount) => amount,
        Err(response) => return *response,
    };
    let recipient = match serialize_move_arg(
        request.id,
        &recipient.to_account_address(),
        "recipient address",
    ) {
        Ok(recipient) => recipient,
        Err(response) => return *response,
    };

    respond_with_serialize(
        request.id,
        CallFunctionRequest {
            sender: build_data.sender,
            package: module_parts[0].to_string(),
            module: module_parts[1].to_string(),
            // Token modules expose the amount-based transfer entry point.
            // Keep the coin selection and gas policy in the RPC layer while
            // passing only the canonical Move call to the client.
            function: "transfer_amount".to_string(),
            type_args: vec![],
            args: vec![coin_object_id, amount, recipient],
            object_inputs: Some(vec![object_input]),
            gas_limit: build_data.gas_limit,
            gas_price: build_data.gas_price,
            nonce: Some(nonce),
            gas_payment: Some(gas_payment),
            signature: None,
            execute_immediate: build_data.execute_immediate,
        },
    )
}

/// Handle submit object transfer request
pub async fn handle_submit_object_transfer(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let mut tx_data: ObjectTransferData = match serde_json::from_value(request.params.clone()) {
        Ok(data) => data,
        Err(e) => {
            return invalid_params_response(request.id, format!("Invalid transaction data: {}", e));
        }
    };

    let _sender = match parse_hex_address(request.id, &tx_data.sender, "sender address") {
        Ok(addr) => addr,
        Err(response) => return *response,
    };

    if tx_data
        .signature
        .as_ref()
        .map(|signature| signature.is_empty())
        .unwrap_or(true)
    {
        return invalid_params_response(request.id, "Missing or empty signature");
    }

    let recipient = match parse_hex_address(request.id, &tx_data.recipient, "recipient") {
        Ok(addr) => addr,
        Err(response) => return *response,
    };

    let coin_object_ref = tx_data.coin_object_ref.clone().ok_or_else(|| {
        invalid_params_response(
            request.id,
            "coin_object_ref is required and must include (object_id, version, digest)",
        )
    });
    let coin_object_ref = match coin_object_ref {
        Ok(object_ref) => object_ref,
        Err(response) => return response,
    };
    if let Err(response) =
        validate_object_ref_completeness(request.id, "coin_object_ref", &coin_object_ref)
    {
        return *response;
    }
    if !(coin_object_ref.version.is_some() && coin_object_ref.digest.is_some()) {
        return invalid_params_response(
            request.id,
            "coin_object_ref must include (object_id, version, digest)",
        );
    }
    if let Err(response) =
        validate_object_inputs_and_gas(request.id, &[], tx_data.gas_payment.as_ref())
    {
        return *response;
    }
    let canonical_nonce = match tx_data.require_nonce() {
        Ok(nonce) => nonce,
        Err(message) => return invalid_params_response(request.id, &message),
    };

    let mut transaction = Transaction::new_transfer_with_object_ref_and_gas(
        tx_data.sender.clone(),
        coin_object_ref,
        recipient.to_hex_literal(),
        tx_data.amount,
        canonical_nonce,
        tx_data.gas_limit,
        tx_data.gas_price,
    );
    if let Transaction::ExecuteFunction { gas_payment, .. } = &mut transaction
        && tx_data.gas_payment.is_some()
    {
        *gas_payment = tx_data.gas_payment.clone();
    }

    let mut signed_tx = SignedTransaction::new(transaction);
    maybe_attach_signature(&mut signed_tx, tx_data.signature);

    execute_or_submit_response(
        state,
        request.id,
        signed_tx,
        tx_data.execute_immediate.unwrap_or(false),
        "submit",
        "Submission failed",
    )
    .await
}

/// Handle get transaction by hash request
pub async fn handle_get_transaction(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let hash_param = extract_hash_param(&request.params).unwrap_or_default();

    if hash_param.is_empty() {
        return invalid_params_response(request.id, "Invalid or missing 'hash' parameter");
    }

    let normalized = hash_param.trim_start_matches("0x").to_lowercase();
    let tx_hash_bytes = match hex::decode(&normalized) {
        Ok(bytes) => bytes,
        Err(_) => return invalid_params_response(request.id, "Invalid transaction hash hex"),
    };

    let chain = state.engine.blockchain.read().unwrap_or_else(|p| {
        error!("blockchain lock poisoned; recovering");
        p.into_inner()
    });

    if let Some((tx, height, state_root, effect)) =
        chain.get_transaction_location_with_effect(&tx_hash_bytes)
    {
        let mut details = map_transaction_to_details(
            state,
            &tx.transaction,
            &hex::encode(tx.transaction_hash()),
            "committed",
            Some(height),
            Some(hex::encode(state_root)),
        );
        apply_committed_effect(&mut details, effect);
        return respond_with_serialize(request.id, details);
    }
    drop(chain);

    if let Some((tx, height, state_root)) = state
        .engine
        .get_committed_transaction_from_history(&tx_hash_bytes)
    {
        let mut details = map_transaction_to_details(
            state,
            &tx.transaction,
            &hex::encode(tx.transaction_hash()),
            "committed",
            Some(height),
            Some(hex::encode(state_root)),
        );
        let effect = state
            .engine
            .get_committed_transaction_effect_from_history(&tx_hash_bytes);
        apply_committed_effect(&mut details, effect.as_ref());
        return respond_with_serialize(request.id, details);
    }

    if let Some(tx) = state.engine.find_pending_transaction(&tx_hash_bytes) {
        let tx_hash = hex::encode(tx.signed_tx.transaction_hash());
        let mut details = map_transaction_to_details(
            state,
            &tx.signed_tx.transaction,
            &tx_hash,
            pending_status(&tx),
            None,
            None,
        );
        apply_pending_preview_metadata(&tx, &mut details);
        return respond_with_serialize(request.id, details);
    }

    internal_error_response(request.id, "Transaction not found")
}

fn collect_transaction_details<F>(
    state: &RpcServerState,
    limit: usize,
    matches: F,
    lock_context: &str,
) -> Vec<TransactionDetails>
where
    F: Fn(&Transaction) -> bool,
{
    let mut results: Vec<TransactionDetails> = Vec::new();
    let mut seen_hashes = HashSet::new();
    let pending = state
        .engine
        .filter_pending_transaction_records(limit, |tx| matches(&tx.signed_tx.transaction));

    for tx in &pending {
        if !push_unique_tx_details(&mut results, &mut seen_hashes, limit, {
            let mut details = map_transaction_to_details(
                state,
                &tx.signed_tx.transaction,
                &hex::encode(tx.signed_tx.transaction_hash()),
                pending_status(tx),
                None,
                None,
            );
            apply_pending_preview_metadata(tx, &mut details);
            details
        }) {
            break;
        }
    }

    if results.len() < limit {
        let chain = state.engine.blockchain.read().unwrap_or_else(|p| {
            error!("blockchain lock poisoned while {lock_context}; recovering");
            p.into_inner()
        });

        for checkpoint in chain.dag_checkpoints.iter().rev() {
            if results.len() >= limit {
                break;
            }

            for (index, tx) in checkpoint.transactions.iter().enumerate().rev() {
                if !matches(&tx.transaction) {
                    continue;
                }

                let mut details = map_transaction_to_details(
                    state,
                    &tx.transaction,
                    &hex::encode(tx.transaction_hash()),
                    "committed",
                    Some(checkpoint.sequence),
                    Some(hex::encode(&checkpoint.state_root)),
                );
                apply_committed_effect(&mut details, checkpoint.transaction_effects.get(index));
                if !push_unique_tx_details(&mut results, &mut seen_hashes, limit, details) {
                    break;
                }
            }
        }
    }

    if results.len() < limit {
        for (tx, height, state_root) in state
            .engine
            .list_committed_transactions_from_history(limit, &matches)
        {
            let tx_hash = tx.transaction_hash().to_vec();
            let effect = state
                .engine
                .get_committed_transaction_effect_from_history(&tx_hash);
            let mut details = map_transaction_to_details(
                state,
                &tx.transaction,
                &hex::encode(&tx_hash),
                "committed",
                Some(height),
                Some(hex::encode(state_root)),
            );
            apply_committed_effect(&mut details, effect.as_ref());
            if !push_unique_tx_details(&mut results, &mut seen_hashes, limit, details) {
                break;
            }
        }
    }

    results
}

/// Handle request to list all transactions (committed + pending)
/// Optimized: Fetches latest transactions first and implements pagination (Limit)
pub async fn handle_get_all_transactions(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let limit = request
        .params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .min(500) as usize;

    let owner_norm = request
        .params
        .as_str()
        .or_else(|| {
            request
                .params
                .as_object()
                .and_then(|obj| obj.get("owner").and_then(|v| v.as_str()))
        })
        .map(|a| a.trim_start_matches("0x").to_lowercase());

    let results = collect_transaction_details(
        state,
        limit,
        |tx| tx_matches_owner(tx, owner_norm.as_deref()),
        "listing transactions",
    );
    respond_with_serialize(request.id, results)
}

pub async fn handle_get_fungible_asset_transactions(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let req_data: GetFungibleAssetTransactionsRequest = match parse_labeled_params(
        request.id,
        &request.params,
        "fungible asset transaction query",
    ) {
        Ok(data) => data,
        Err(response) => return *response,
    };

    let token_type = CoinModule::normalize_token_type(&req_data.token_type);
    let limit = req_data.limit.unwrap_or(50).min(500);
    let owner_norm = req_data
        .owner
        .as_deref()
        .map(|owner| owner.trim_start_matches("0x").to_lowercase());

    let results = collect_transaction_details(
        state,
        limit,
        |tx| tx_mentions_token_type(tx, &token_type) && tx_matches_owner(tx, owner_norm.as_deref()),
        "listing asset transactions",
    );

    respond_with_serialize(
        request.id,
        FungibleAssetTransactionsResponse {
            token_type,
            transactions: results,
        },
    )
}

/// Handle publish module request
pub async fn handle_publish_module(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let is_upgrade = request.method == methods::UPGRADE_MODULE;
    let action = if is_upgrade {
        "upgrade_module"
    } else {
        "publish"
    };
    let failure_message = if is_upgrade {
        "Module upgrade failed"
    } else {
        "Module publication failed"
    };
    let mut module_data: PublishModuleRequest =
        match parse_labeled_params(request.id, &request.params, "module data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    if let Err(response) = parse_hex_address(request.id, &module_data.sender, "sender address") {
        return *response;
    }
    if let Err(response) =
        validate_object_inputs_and_gas(request.id, &[], module_data.gas_payment.as_ref())
    {
        return *response;
    }
    if let Err(message) = module_data.require_nonce() {
        return invalid_params_response(request.id, &message);
    }

    let execute_immediate = module_data.execute_immediate.unwrap_or(false);
    let signed_tx = build_publish_signed_tx(module_data, is_upgrade);

    execute_or_submit_response(
        state,
        request.id,
        signed_tx,
        execute_immediate,
        action,
        failure_message,
    )
    .await
}

pub async fn handle_publish_package(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let is_upgrade = request.method == methods::UPGRADE_PACKAGE;
    let action = if is_upgrade {
        "upgrade_package"
    } else {
        "publish_package"
    };
    let empty_package_message = if is_upgrade {
        "Package upgrade requires at least one module"
    } else {
        "Package publish requires at least one module"
    };
    let failure_message = if is_upgrade {
        "Package upgrade failed"
    } else {
        "Package publication failed"
    };
    let mut package_data: PublishPackageRequest =
        match parse_labeled_params(request.id, &request.params, "package data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    if let Err(response) = parse_hex_address(request.id, &package_data.sender, "sender address") {
        return *response;
    }
    if package_data.modules.is_empty() {
        return invalid_params_response(request.id, empty_package_message);
    }
    if let Err(response) =
        validate_object_inputs_and_gas(request.id, &[], package_data.gas_payment.as_ref())
    {
        return *response;
    }
    if let Err(message) = package_data.require_nonce() {
        return invalid_params_response(request.id, &message);
    }

    let execute_immediate = package_data.execute_immediate.unwrap_or(false);
    let signed_tx = build_publish_package_signed_tx(package_data, is_upgrade);

    execute_or_submit_response(
        state,
        request.id,
        signed_tx,
        execute_immediate,
        action,
        failure_message,
    )
    .await
}

/// Handle call function request
pub async fn handle_call_function(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let mut call_data: CallFunctionRequest =
        match parse_labeled_params(request.id, &request.params, "call data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    if let Err(response) = parse_hex_address(request.id, &call_data.sender, "sender address") {
        return *response;
    }
    if let Err(response) = parse_hex_address(request.id, &call_data.package, "package address") {
        return *response;
    }
    if let Err(response) = validate_object_inputs_and_gas(
        request.id,
        call_data.object_inputs.as_deref().unwrap_or(&[]),
        call_data.gas_payment.as_ref(),
    ) {
        return *response;
    }
    if let Err(message) = call_data.require_nonce() {
        return invalid_params_response(request.id, &message);
    }

    let execute_immediate = call_data.execute_immediate.unwrap_or(false);
    let signed_tx = build_call_signed_tx(call_data);

    execute_or_submit_response(
        state,
        request.id,
        signed_tx,
        execute_immediate,
        "call",
        "Call submission failed",
    )
    .await
}

/// Handle view function request (read-only, no transaction submission)
pub async fn handle_view_function(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let first_param = match first_array_param(request.id, &request.params) {
        Ok(param) => param,
        Err(response) => return *response,
    };

    let view_data: ViewFunctionRequest =
        match parse_labeled_params(request.id, first_param, "view function data") {
            Ok(data) => data,
            Err(response) => return *response,
        };

    if let Err(response) = parse_hex_address(request.id, &view_data.package, "package address") {
        return *response;
    }
    if let Err(response) = validate_object_inputs_and_gas(
        request.id,
        view_data.object_inputs.as_deref().unwrap_or(&[]),
        None,
    ) {
        return *response;
    }
    if let Err(response) = validate_object_inputs_match_state(
        state,
        request.id,
        view_data.object_inputs.as_deref().unwrap_or(&[]),
    ) {
        return *response;
    }

    info!(
        "Executing view function: {}::{}::{}",
        view_data.package, view_data.module, view_data.function
    );

    match state.engine.execute_view_function(
        &view_data.package,
        &view_data.module,
        &view_data.function,
        &view_data.type_args,
        &view_data.args,
        view_data.object_inputs.as_deref().unwrap_or(&[]),
    ) {
        Ok(result) => {
            info!("View function executed successfully");
            respond_with_serialize(
                request.id,
                serde_json::json!({
                    "status": "success",
                    "action": "view",
                    "result": result
                }),
            )
        }
        Err(e) => {
            error!("View function execution failed: {}", e);
            internal_error_response(request.id, format!("View function execution failed: {}", e))
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/transaction_tests.rs"]
mod tests;
