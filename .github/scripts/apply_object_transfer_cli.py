from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old in text:
        return text.replace(old, new, 1)
    if new in text:
        return text
    raise RuntimeError(f"missing transfer cutover marker: {label}")


def replace_cli_transfer() -> None:
    Path("crates/kanari/src/command/client_cli/transfer.rs").write_text(r'''// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::command::common::{
    check_node_connection, get_rpc_endpoint, get_sender_for_tx, load_wallet_for, normalize_addr,
    resolve_sender, resolve_transaction_gas,
};
use anyhow::{Context, Result};
use clap::Parser;
use kanari_rpc_client::RpcClient;
use kanari_types::kanari::KANARI_TOKEN_TYPE;
use kanari_types::object::{ObjectDigest, ObjectID, ObjectRef};
use kanari_types::object_transaction::{
    GasData, ObjectTransactionData, ObjectTransactionKind, TransactionExpiration,
};
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use move_core_types::account_address::AccountAddress;

fn read_coin_balance(data: &[u8]) -> Option<u64> {
    let bytes: [u8; 8] = data.get(32..40)?.try_into().ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn object_ref(id: &str, version: u64, digest: &str) -> Result<ObjectRef> {
    let digest = hex::decode(digest.trim_start_matches("0x"))
        .context("Invalid object digest returned by RPC")?;
    Ok(ObjectRef::new(
        ObjectID::from_hex_literal(id).context("Invalid coin object ID")?,
        version,
        ObjectDigest::from_bytes(&digest)?,
    ))
}

#[derive(Parser, Debug)]
pub struct Transfer {
    #[arg(short, long)]
    pub from: Option<String>,
    #[arg(short, long)]
    pub to: String,
    #[arg(short, long)]
    pub amount: f64,
    #[arg(short, long)]
    pub password: String,
    #[clap(long = "rpc")]
    pub rpc_endpoint: Option<String>,
}

impl Transfer {
    pub async fn execute(&self) -> Result<()> {
        let rpc = get_rpc_endpoint(self.rpc_endpoint.clone());
        let from_addr = resolve_sender(self.from.clone())?;
        let to_addr = normalize_addr(&self.to)?;
        let wallet = load_wallet_for(&from_addr, Some(self.password.clone()))?;
        let (gas_budget, gas_price) = resolve_transaction_gas(None, None);

        eprintln!("Transferring Kanari tokens...");
        eprintln!("  From: {}", from_addr);
        eprintln!("  To: {}", to_addr);
        eprintln!("  Amount: {} KANARI", self.amount);

        const MIST_PER_KANARI: f64 = 1_000_000_000.0;
        let amount_mist = (self.amount * MIST_PER_KANARI).round() as u64;
        if amount_mist == 0 {
            anyhow::bail!("Transfer amount must be greater than zero");
        }
        eprintln!("  Amount (Mist): {}", amount_mist);

        let client = RpcClient::new(&rpc);
        check_node_connection(&client, &rpc).await?;

        let coin_type = format!("0x2::coin::Coin<{}>", KANARI_TOKEN_TYPE);
        let mut coins = client
            .get_owned_object_refs(&from_addr, Some(coin_type.clone()))
            .await
            .context("Failed to get exact sender coin objects")?;
        coins.sort_by(|left, right| left.id.cmp(&right.id));

        let maximum_gas_fee = gas_budget
            .checked_mul(gas_price)
            .context("Maximum gas fee overflow")?;
        let required = amount_mist
            .checked_add(maximum_gas_fee)
            .and_then(|value| value.checked_add(1))
            .context("Payment plus maximum gas fee overflow")?;

        let mut selected_refs = Vec::new();
        let mut selected_balance = 0u64;
        let mut total_balance = 0u64;
        for coin in coins {
            if coin.type_ != coin_type {
                continue;
            }
            let Some(balance) = read_coin_balance(&coin.data) else {
                continue;
            };
            total_balance = total_balance
                .checked_add(balance)
                .context("Coin balance overflow")?;
            if selected_balance < required {
                selected_refs.push(object_ref(&coin.id, coin.version, &coin.digest)?);
                selected_balance = selected_balance
                    .checked_add(balance)
                    .context("Selected coin balance overflow")?;
            }
        }

        if selected_balance < required {
            anyhow::bail!(
                "Insufficient Coin<{}> balance. requested={} Mist, max_gas={} Mist, spendable={} Mist",
                KANARI_TOKEN_TYPE,
                amount_mist,
                maximum_gas_fee,
                total_balance
            );
        }

        eprintln!("  Selected coin objects: {}", selected_refs.len());
        for reference in &selected_refs {
            eprintln!("    - {}", reference.object_id);
        }
        eprintln!("  Selected Coin Balance (Mist): {}", selected_balance);
        eprintln!("  Total Spendable Coin Balance (Mist): {}", total_balance);
        eprintln!("  Gas Budget: {}", gas_budget);
        eprintln!("  Gas Price: {} Mist/gas", gas_price);

        let sender = AccountAddress::from_hex_literal(&from_addr)
            .context("Invalid sender account address")?;
        let recipient = AccountAddress::from_hex_literal(&to_addr)
            .context("Invalid recipient account address")?;
        let data = ObjectTransactionData::new(
            sender,
            ObjectTransactionKind::Pay {
                coins: selected_refs.clone(),
                recipient,
                amount: amount_mist,
            },
            GasData {
                payment: selected_refs,
                owner: sender,
                price: gas_price,
                budget: gas_budget,
            },
            TransactionExpiration::None,
        )?;
        let mut transaction =
            SignedObjectTransaction::new(data, get_sender_for_tx(&wallet, &from_addr)?)?;
        transaction
            .sign_sender(&wallet.private_key, wallet.curve_type)
            .context("Failed to sign object transaction")?;

        eprintln!("  Object transaction signed");
        eprintln!("  Submitting transaction for consensus...");
        let status = client
            .submit_object_transaction(transaction)
            .await
            .context("Failed to submit object transaction")?;
        eprintln!("  Transaction submitted successfully");
        eprintln!("  Transaction digest: {}", status.hash);
        eprintln!("  Status: {}", status.status);
        Ok(())
    }
}
''')


def update_rpc_client() -> None:
    cargo = Path("crates/kanari-rpc-client/Cargo.toml")
    cargo_text = cargo.read_text()
    if "serde = { workspace = true" not in cargo_text:
        cargo_text = replace_once(
            cargo_text,
            "serde_json = { workspace = true }\n",
            "serde = { workspace = true, features = [\"derive\"] }\nserde_json = { workspace = true }\n",
            "rpc client serde dependency",
        )
    cargo.write_text(cargo_text)

    path = Path("crates/kanari-rpc-client/src/lib.rs")
    text = path.read_text()
    text = text.replace(
        "use kanari_types::error::KanariUnwrapExt;\n",
        "use kanari_types::error::KanariUnwrapExt;\nuse kanari_types::signed_object_transaction::SignedObjectTransaction;\nuse serde::Deserialize;\n",
    )
    struct_marker = "/// RPC client\npub struct RpcClient"
    object_struct = '''#[derive(Debug, Clone, Deserialize)]
pub struct ObjectRefInfo {
    pub id: String,
    pub owner: String,
    pub type_: String,
    pub data: Vec<u8>,
    pub version: u64,
    pub digest: String,
}

/// RPC client
pub struct RpcClient'''
    text = replace_once(text, struct_marker, object_struct, "rpc object ref response")

    method_marker = "    /// Submit signed transaction\n"
    methods = '''    /// Get exact, unaggregated object references owned by an address.
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
'''
    text = replace_once(text, method_marker, methods, "rpc object methods")
    path.write_text(text)


def update_object_rpc_server() -> None:
    path = Path("crates/kanari-rpc-server/src/module/mod.rs")
    text = path.read_text()
    old_push = "            objects.push(build_object_info(uid, obj));\n"
    new_push = '''            let object_id = match kanari_types::object::ObjectID::from_hex_literal(&uid) {
                Ok(object_id) => object_id,
                Err(error) => {
                    return internal_error_response(request.id, error.to_string());
                }
            };
            let reference = match state_guard.get_object_ref_exact(object_id) {
                Ok(Some(reference)) => reference,
                Ok(None) => continue,
                Err(error) => {
                    return internal_error_response(request.id, error.to_string());
                }
            };
            objects.push(serde_json::json!({
                "id": uid,
                "owner": format!("{:#x}", obj.owner),
                "type_": obj.type_,
                "data": obj.data,
                "version": obj.version,
                "digest": reference.digest.to_hex(),
            }));
'''
    text = replace_once(text, old_push, new_push, "exact owned object response")
    text = text.replace("    let objects = aggregate_owned_objects(objects);\n\n", "")
    path.write_text(text)

    path = Path("crates/kanari-rpc-server/src/object_transaction.rs")
    path.write_text(r'''// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use super::{RpcRequest, RpcResponse, RpcServerState};
use crate::{internal_error_response, invalid_params_response, respond_with_serialize};
use kanari_types::signed_object_transaction::SignedObjectTransaction;
use serde::Serialize;

pub const SUBMIT_OBJECT_TRANSACTION: &str = "kanari_submitObjectTransaction";
pub const GET_PENDING_OBJECT_TRANSACTIONS: &str = "kanari_getPendingObjectTransactions";

#[derive(Debug, Serialize)]
struct SubmitResponse {
    digest: String,
    status: &'static str,
}

pub async fn submit(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let transaction: SignedObjectTransaction = match serde_json::from_value(request.params.clone()) {
        Ok(transaction) => transaction,
        Err(error) => return invalid_params_response(request.id, error.to_string()),
    };
    let broadcast = transaction.clone();
    match state.engine.submit_protocol_transaction(transaction) {
        Ok(digest) => {
            state.broadcast_submitted_transaction(broadcast);
            respond_with_serialize(
                request.id,
                SubmitResponse {
                    digest: format!("0x{}", hex::encode(digest)),
                    status: "pending_consensus",
                },
            )
        }
        Err(error) => invalid_params_response(request.id, error.to_string()),
    }
}

pub async fn pending(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    match state.engine.pending_object_transactions() {
        Ok(transactions) => respond_with_serialize(request.id, transactions),
        Err(error) => internal_error_response(request.id, error.to_string()),
    }
}
''')

    path = Path("crates/kanari-rpc-server/src/lib.rs")
    text = path.read_text()
    text = text.replace(
        "use kanari_types::transaction::SignedTransaction;",
        "use kanari_types::signed_object_transaction::SignedObjectTransaction;",
    )
    text = text.replace("Fn(SignedTransaction) -> Result<()>", "Fn(SignedObjectTransaction) -> Result<()>")
    text = text.replace("signed_tx: SignedTransaction", "signed_tx: SignedObjectTransaction")
    execute_route = '''        object_transaction::EXECUTE_OBJECT_TRANSACTION => {
            object_transaction::execute(&state, &request).await
        }
'''
    text = text.replace(execute_route, "")
    path.write_text(text)

    path = Path("crates/kanari-rpc-server/src/transaction/mod.rs")
    text = path.read_text().replace(
        "state.broadcast_submitted_transaction(tx_for_broadcast);",
        "let _ = tx_for_broadcast;",
    )
    path.write_text(text)


def update_node_gossip() -> None:
    path = Path("crates/kanari-node/src/sync.rs")
    text = path.read_text()
    text = text.replace(
        "use kanari_types::transaction::SignedTransaction;",
        "use kanari_types::signed_object_transaction::SignedObjectTransaction;",
    )
    text = text.replace(
        'Self::parse_message::<SignedTransaction>(&tx_data, "transaction")',
        'Self::parse_message::<SignedObjectTransaction>(&tx_data, "object transaction")',
    )
    old = '''            match self
                .engine
                .submit_transactions_batch(vec![signed_tx.clone()])
            {
                Ok(tx_hashes) => {
                    info!(
                        "Received transaction from network: 0x{}",
                        hex::encode(&tx_hashes[0])
                    );
                }
                Err(e) => {
                    warn!("Failed to submit transaction from network: {}", e);
                }
            }
'''
    new = '''            match self.engine.submit_protocol_transaction(signed_tx) {
                Ok(tx_hash) => {
                    info!(
                        "Received object transaction from network: 0x{}",
                        hex::encode(tx_hash)
                    );
                }
                Err(e) => {
                    warn!("Failed to submit object transaction from network: {}", e);
                }
            }
'''
    text = replace_once(text, old, new, "node object gossip")
    path.write_text(text)


replace_cli_transfer()
update_rpc_client()
update_object_rpc_server()
update_node_gossip()
