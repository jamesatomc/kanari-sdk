use crate::respond_with_serialize;

use super::{RpcError, RpcRequest, RpcResponse, RpcServerState};
use kanari_move_runtime::changeset::ChangeSet;
use kanari_rpc_api::{
    CallFunctionRequest, PublishModuleRequest, SignedTransactionData, TransactionDetails,
};
use kanari_types::address::Address;
use kanari_types::transaction::{SignedTransaction, Transaction};
use move_binary_format::CompiledModule;
use tracing::{error, info};

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

// Normalize address strings for comparison (converts to raw hex address)
fn normalize_addr(s: &str) -> String {
    use std::str::FromStr;
    Address::from_str(s)
        .map(|addr| addr.to_hex())
        .unwrap_or_else(|_| s.trim_start_matches("0x").to_lowercase())
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
    state_root_hex: Option<String>,
) -> TransactionDetails {
    let hash = format!("0x{}", tx_hash_hex);
    let status_str = status.to_string();

    match tx {
        Transaction::PublishModule {
            sender,
            module_bytes,
            module_name,
            sequence_number,
            gas_limit,
            gas_price,
            ..
        } => {
            let prefix = state_root_hex
                .map(|sr| format!("0x{}", sr))
                .unwrap_or_else(|| status_str.clone());
            let module_funcs = extract_functions_from_bytes(module_bytes).map(|fns| {
                fns.into_iter()
                    .map(|f| format!("{}::{}::{}", prefix, module_name, f))
                    .collect()
            });

            TransactionDetails {
                hash,
                status: status_str,
                block_height,
                gas_used: None,
                tx_type: "publish_module".to_string(),
                sender: sender.clone(),
                sequence_number: *sequence_number,
                gas_limit: *gas_limit,
                gas_price: *gas_price,
                module: Some(module_name.clone()),
                function: None,
                module_functions: module_funcs,
            }
        }
        Transaction::ExecuteFunction {
            sender,
            module,
            function,
            sequence_number,
            gas_limit,
            gas_price,
            ..
        } => TransactionDetails {
            hash,
            status: status_str,
            block_height,
            gas_used: None,
            tx_type: "call".to_string(),
            sender: sender.clone(),
            sequence_number: *sequence_number,
            gas_limit: *gas_limit,
            gas_price: *gas_price,
            module: Some(module.clone()),
            function: Some(function.clone()),
            module_functions: lookup_module_functions(state, module),
        },
        Transaction::Transfer {
            from,
            to,
            sequence_number,
            gas_limit,
            gas_price,
            ..
        } => TransactionDetails {
            hash,
            status: status_str,
            block_height,
            gas_used: None,
            tx_type: "transfer".to_string(),
            sender: from.clone(),
            sequence_number: *sequence_number,
            gas_limit: *gas_limit,
            gas_price: *gas_price,
            module: Some(format!("To: {}", to)),
            function: None,
            module_functions: None,
        },
        Transaction::Burn {
            from,
            sequence_number,
            gas_limit,
            gas_price,
            ..
        } => TransactionDetails {
            hash,
            status: status_str,
            block_height,
            gas_used: None,
            tx_type: "burn".to_string(),
            sender: from.clone(),
            sequence_number: *sequence_number,
            gas_limit: *gas_limit,
            gas_price: *gas_price,
            module: None,
            function: None,
            module_functions: None,
        },
    }
}

// =========================================================================
// HELPER: Format ChangeSet JSON for API responses
// =========================================================================
fn format_changeset_json(state: &RpcServerState, changeset: &ChangeSet) -> serde_json::Value {
    let mut cs_value = serde_json::to_value(changeset).unwrap_or(serde_json::json!(null));

    let state_guard = state.engine.state.read().unwrap_or_else(|p| {
        error!("state lock poisoned while formatting changeset; recovering");
        p.into_inner()
    });

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
    cs_value
}

// =========================================================================
// HANDLERS
// =========================================================================

/// Handle submit transaction request
pub async fn handle_submit_transaction(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let tx_data: SignedTransactionData = match serde_json::from_value(request.params.clone()) {
        Ok(data) => data,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::invalid_params(format!(
                    "Invalid transaction data: {}",
                    e
                ))),
                id: request.id,
            };
        }
    };

    let sender = match Address::from_hex_literal(&tx_data.sender) {
        Ok(addr) => addr,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::invalid_params(format!(
                    "Invalid sender address: {}",
                    e
                ))),
                id: request.id,
            };
        }
    };

    let transaction =
        if let (Some(recipient_str), Some(amount)) = (&tx_data.recipient, tx_data.amount) {
            let recipient = match Address::from_hex_literal(recipient_str) {
                Ok(addr) => addr,
                Err(e) => {
                    return RpcResponse {
                        jsonrpc: "2.0".into(),
                        result: None,
                        error: Some(RpcError::invalid_params(format!(
                            "Invalid recipient: {}",
                            e
                        ))),
                        id: request.id,
                    };
                }
            };

            Transaction::Transfer {
                from: tx_data.sender.clone(),
                to: recipient.to_string(),
                amount,
                gas_limit: tx_data.gas_limit,
                gas_price: tx_data.gas_price,
                sequence_number: tx_data.sequence_number,
            }
        } else if let (None, Some(amount)) = (&tx_data.recipient, tx_data.amount) {
            let system_addr =
                Address::from_hex_literal(Address::KANARI_SYSTEM_ADDRESS).unwrap_or(Address::ZERO);
            let dev_addr = Address::from_hex_literal(Address::DEV_ADDRESS).unwrap_or(Address::ZERO);

            if sender != system_addr && sender != dev_addr {
                error!("Unauthorized burn attempt from {}", sender.to_hex_literal());
                return RpcResponse {
                    jsonrpc: "2.0".into(),
                    result: None,
                    error: Some(RpcError::invalid_params(
                        "Burn transactions are restricted to system administrators",
                    )),
                    id: request.id,
                };
            }

            Transaction::Burn {
                from: tx_data.sender.clone(),
                amount,
                gas_limit: tx_data.gas_limit,
                gas_price: tx_data.gas_price,
                sequence_number: tx_data.sequence_number,
            }
        } else {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::invalid_params(
                    "Only transfer or burn transactions are supported",
                )),
                id: request.id,
            };
        };

    let sig = match tx_data.signature {
        Some(s) => s,
        None => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::invalid_params("Missing signature")),
                id: request.id,
            };
        }
    };

    let mut signed_tx = SignedTransaction::new(transaction);
    signed_tx.signature = sig;

    match state.engine.submit_transaction(signed_tx) {
        Ok(tx_hash) => {
            let tx_hash_hex = hex::encode(&tx_hash);
            info!("Transaction submitted successfully: {}", tx_hash_hex);
            respond_with_serialize(
                request.id,
                serde_json::json!({ "hash": tx_hash_hex, "status": "pending" }),
            )
        }
        Err(e) => RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcError::internal_error(format!(
                "Submission failed: {}",
                e
            ))),
            id: request.id,
        },
    }
}

/// Handle get transaction by hash request
pub async fn handle_get_transaction(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let hash_param: String = match serde_json::from_value(request.params.clone()) {
        Ok(h) => h,
        Err(_) => match serde_json::from_value::<serde_json::Value>(request.params.clone()) {
            Ok(v) => v
                .get("hash")
                .and_then(|hv| hv.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            Err(_) => "".to_string(),
        },
    };

    if hash_param.is_empty() {
        return RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcError::invalid_params(
                "Invalid or missing 'hash' parameter",
            )),
            id: request.id,
        };
    }

    let normalized = hash_param.trim_start_matches("0x").to_lowercase();

    let chain = state.engine.blockchain.read().unwrap_or_else(|p| {
        error!("blockchain lock poisoned; recovering");
        p.into_inner()
    });

    for block in chain.blocks.iter().rev() {
        for tx in block.transactions.iter().rev() {
            let tx_hash = hex::encode(tx.hash().0.0);
            if tx_hash.to_lowercase() == normalized {
                // 🚀 ค้นหาสถานะจริงจาก SQLite Indexer
                let mut final_status = "committed".to_string();
                if let Ok(Some(db_status)) = state.indexer.get_transaction_status(&normalized) {
                    final_status = db_status;
                }

                let details = map_transaction_to_details(
                    state,
                    &tx.transaction,
                    &tx_hash,
                    &final_status,
                    Some(block.header.height),
                    Some(hex::encode(&block.header.state_root)),
                );
                return respond_with_serialize(request.id, details);
            }
        }
    }

    let pending = state.engine.pending_txs.read().unwrap_or_else(|p| {
        error!("pending_txs lock poisoned; recovering");
        p.into_inner()
    });

    for tx in pending.iter() {
        let tx_hash = hex::encode(tx.hash().0.0);
        if tx_hash.to_lowercase() == normalized {
            let details =
                map_transaction_to_details(state, &tx.transaction, &tx_hash, "pending", None, None);
            return respond_with_serialize(request.id, details);
        }
    }

    RpcResponse {
        jsonrpc: "2.0".into(),
        result: None,
        error: Some(RpcError::internal_error("Transaction not found")),
        id: request.id,
    }
}

/// Handle request to list all transactions (committed + pending)
pub async fn handle_get_all_transactions(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let limit = request
        .params
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(50) as usize;

    let account_norm = request
        .params
        .as_str()
        .or_else(|| {
            request
                .params
                .as_object()
                .and_then(|obj| obj.get("account").and_then(|v| v.as_str()))
        })
        .map(|a| a.trim_start_matches("0x").to_lowercase());

    let mut results: Vec<TransactionDetails> = Vec::new();

    let pending = state.engine.pending_txs.read().unwrap_or_else(|p| {
        error!("pending_txs lock poisoned while listing transactions; recovering");
        p.into_inner()
    });

    for tx in pending.iter().rev() {
        if results.len() >= limit {
            break;
        }

        if let Some(ref acct) = account_norm {
            let matches = match &tx.transaction {
                Transaction::PublishModule { sender, .. }
                | Transaction::ExecuteFunction { sender, .. }
                | Transaction::Burn { from: sender, .. } => normalize_addr(sender) == *acct,
                Transaction::Transfer { from, to, .. } => {
                    normalize_addr(from) == *acct || normalize_addr(to) == *acct
                }
            };
            if !matches {
                continue;
            }
        }

        results.push(map_transaction_to_details(
            state,
            &tx.transaction,
            &hex::encode(tx.hash().0.0),
            "pending",
            None,
            None,
        ));
    }

    if results.len() < limit {
        let chain = state.engine.blockchain.read().unwrap_or_else(|p| {
            error!("blockchain lock poisoned while listing transactions; recovering");
            p.into_inner()
        });

        for block in chain.blocks.iter().rev() {
            if results.len() >= limit {
                break;
            }

            for tx in block.transactions.iter().rev() {
                if results.len() >= limit {
                    break;
                }

                if let Some(ref acct) = account_norm {
                    let matches = match &tx.transaction {
                        Transaction::PublishModule { sender, .. }
                        | Transaction::ExecuteFunction { sender, .. }
                        | Transaction::Burn { from: sender, .. } => normalize_addr(sender) == *acct,
                        Transaction::Transfer { from, to, .. } => {
                            normalize_addr(from) == *acct || normalize_addr(to) == *acct
                        }
                    };
                    if !matches {
                        continue;
                    }
                }

                let tx_hash_hex = hex::encode(tx.hash().0.0);

                // 🚀 ค้นหาสถานะจริงจาก SQLite Indexer
                let mut final_status = "committed".to_string();
                if let Ok(Some(db_status)) = state.indexer.get_transaction_status(&tx_hash_hex) {
                    final_status = db_status;
                }

                results.push(map_transaction_to_details(
                    state,
                    &tx.transaction,
                    &tx_hash_hex,
                    &final_status,
                    Some(block.header.height),
                    Some(hex::encode(&block.header.state_root)),
                ));
            }
        }
    }

    respond_with_serialize(request.id, results)
}

/// Handle publish module request
pub async fn handle_publish_module(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let module_data: PublishModuleRequest = match serde_json::from_value(request.params.clone()) {
        Ok(data) => data,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::invalid_params(format!(
                    "Invalid module data: {}",
                    e
                ))),
                id: request.id,
            };
        }
    };

    if let Err(e) = Address::from_hex_literal(&module_data.sender) {
        return RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcError::invalid_params(format!(
                "Invalid sender address: {}",
                e
            ))),
            id: request.id,
        };
    }

    let mut signed_tx = SignedTransaction::new(Transaction::PublishModule {
        sender: module_data.sender.clone(),
        module_bytes: module_data.module_bytes,
        module_name: module_data.module_name,
        gas_limit: module_data.gas_limit,
        gas_price: module_data.gas_price,
        sequence_number: module_data.sequence_number,
    });

    if let Some(sig) = module_data.signature {
        signed_tx.signature = sig;
    }

    if module_data.execute_immediate.unwrap_or(true) {
        let exec_tx = signed_tx.clone();
        match state.engine.execute_transaction_immediate(exec_tx) {
            Ok((tx_hash, changeset)) => {
                let tx_hash_hex = hex::encode(&tx_hash);
                let cs_value = format_changeset_json(state, &changeset);

                if !changeset.success {
                    return respond_with_serialize(
                        request.id,
                        serde_json::json!({ "hash": tx_hash_hex, "status": "failed", "action": "publish", "changeset": cs_value }),
                    );
                }

                if let Err(e) = state.engine.submit_transaction(signed_tx) {
                    error!("Failed to submit executed transaction: {}", e);
                    return RpcResponse {
                        jsonrpc: "2.0".into(),
                        result: None,
                        error: Some(RpcError::transaction_error(format!(
                            "Simulation successful, but mempool submission failed: {}",
                            e
                        ))),
                        id: request.id,
                    };
                }

                info!(
                    "Module publish executed immediately & submitted: {}",
                    tx_hash_hex
                );
                respond_with_serialize(
                    request.id,
                    serde_json::json!({ "hash": tx_hash_hex, "status": "executed", "action": "publish", "changeset": cs_value }),
                )
            }
            Err(e) => RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::internal_error(format!(
                    "Immediate execution failed: {}",
                    e
                ))),
                id: request.id,
            },
        }
    } else {
        match state.engine.submit_transaction(signed_tx) {
            Ok(tx_hash) => respond_with_serialize(
                request.id,
                serde_json::json!({ "hash": hex::encode(&tx_hash), "status": "pending", "action": "publish" }),
            ),
            Err(e) => RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::internal_error(format!(
                    "Module publication failed: {}",
                    e
                ))),
                id: request.id,
            },
        }
    }
}

/// Handle call function request
pub async fn handle_call_function(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let call_data: CallFunctionRequest = match serde_json::from_value(request.params.clone()) {
        Ok(data) => data,
        Err(e) => {
            return RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::invalid_params(format!(
                    "Invalid call data: {}",
                    e
                ))),
                id: request.id,
            };
        }
    };

    if let Err(e) = Address::from_hex_literal(&call_data.sender) {
        return RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcError::invalid_params(format!(
                "Invalid sender address: {}",
                e
            ))),
            id: request.id,
        };
    }
    if let Err(e) = Address::from_hex_literal(&call_data.package) {
        return RpcResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(RpcError::invalid_params(format!(
                "Invalid package address: {}",
                e
            ))),
            id: request.id,
        };
    }

    let mut signed_tx = SignedTransaction::new(Transaction::ExecuteFunction {
        sender: call_data.sender.clone(),
        module: format!("{}::{}", call_data.package, call_data.module),
        function: call_data.function,
        type_args: call_data.type_args,
        args: call_data.args,
        gas_limit: call_data.gas_limit,
        gas_price: call_data.gas_price,
        sequence_number: call_data.sequence_number,
    });

    if let Some(sig) = call_data.signature {
        signed_tx.signature = sig;
    }

    if call_data.execute_immediate.unwrap_or(true) {
        let exec_tx = signed_tx.clone();
        match state.engine.execute_transaction_immediate(exec_tx) {
            Ok((tx_hash, changeset)) => {
                let tx_hash_hex = hex::encode(&tx_hash);
                let mut cs_value = format_changeset_json(state, &changeset);

                if let Some(obj_arr) = cs_value.get("created_objects").and_then(|v| v.as_array())
                    && obj_arr.is_empty()
                    && let Ok(a) = Address::parse_to_account_address(&call_data.sender)
                {
                    let state_guard = state.engine.state.read().unwrap_or_else(|p| p.into_inner());
                    if let Ok(ids) = state_guard.get_owned_objects(&a) {
                        let objs: Vec<_> = ids
                            .iter()
                            .rev()
                            .take(10)
                            .filter_map(|uid| {
                                state_guard.get_object(uid).ok().flatten().map(|co| {
                                    serde_json::json!({ "id": uid, "type": co.type_, "owner": format!("0x{}", hex::encode(co.owner.as_ref())) })
                                })
                            })
                            .collect();

                        if !objs.is_empty()
                            && let Some(map) = cs_value.as_object_mut()
                        {
                            map.insert(
                                "created_objects".to_string(),
                                serde_json::Value::Array(objs),
                            );
                        }
                    }
                }

                if !changeset.success {
                    return respond_with_serialize(
                        request.id,
                        serde_json::json!({ "hash": tx_hash_hex, "status": "failed", "action": "call", "changeset": cs_value }),
                    );
                }

                if let Err(e) = state.engine.submit_transaction(signed_tx) {
                    error!("Failed to submit executed transaction: {}", e);
                    return RpcResponse {
                        jsonrpc: "2.0".into(),
                        result: None,
                        error: Some(RpcError::transaction_error(format!(
                            "Simulation successful, but mempool submission failed: {}",
                            e
                        ))),
                        id: request.id,
                    };
                }

                info!("Function executed immediately & submitted: {}", tx_hash_hex);
                respond_with_serialize(
                    request.id,
                    serde_json::json!({ "hash": tx_hash_hex, "status": "executed", "action": "call", "changeset": cs_value }),
                )
            }
            Err(e) => RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::internal_error(format!(
                    "Immediate execution failed: {}",
                    e
                ))),
                id: request.id,
            },
        }
    } else {
        match state.engine.submit_transaction(signed_tx) {
            Ok(tx_hash) => respond_with_serialize(
                request.id,
                serde_json::json!({ "hash": hex::encode(&tx_hash), "status": "pending", "action": "call" }),
            ),
            Err(e) => RpcResponse {
                jsonrpc: "2.0".into(),
                result: None,
                error: Some(RpcError::transaction_error(format!(
                    "Call submission failed: {}",
                    e
                ))),
                id: request.id,
            },
        }
    }
}
