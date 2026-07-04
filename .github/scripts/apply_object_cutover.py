from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    if old not in text:
        if new in text:
            return text
        raise RuntimeError(f"missing cutover marker: {label}")
    return text.replace(old, new, 1)


def remove_runtime_auto_merge() -> None:
    path = Path("move-execution/v1/kanari-move-runtime-v1/src/move_runtime/mod.rs")
    text = path.read_text()

    struct_start = "#[derive(serde::Serialize)]\nstruct AutoMergeReceiptData"
    if struct_start in text:
        start = text.index(struct_start)
        end = text.index("\ntype LoadedMutableObject", start)
        text = text[:start] + text[end + 1 :]

    for declaration in (
        "        let mut auto_merged_coin_ids = Vec::new();\n",
        "        let mut merged_coin_types = std::collections::HashSet::new();\n",
        "        let mut total_merge_reads: u64 = 0;\n",
        "        let mut synthetic_events = Vec::new();\n",
    ):
        text = text.replace(declaration, "")

    start_marker = '                        let is_coin = struct_tag.module.as_str() == "coin"\n'
    end_marker = "\n                        final_args[i] = stored_obj.data.clone();"
    if start_marker in text:
        start = text.index(start_marker)
        end = text.index(end_marker, start)
        text = text[:start] + text[end:]

    text = text.replace(
        "                        auto_merged_coin_ids.retain(|merged_id| merged_id != id);\n",
        "",
    )
    merged_loop = "                for merged_id in auto_merged_coin_ids {"
    dynamic_fields = "\n\n                for op in dynamic_fields_ops {"
    if merged_loop in text:
        start = text.index(merged_loop)
        end = text.index(dynamic_fields, start)
        text = text[:start] + text[end:]

    text = text.replace(
        "                    let complexity = 1 + (total_merge_reads as u32 / 10);\n",
        "                    let complexity = 1;\n",
    )
    text = text.replace(
        "                    let penalty_complexity = 5 + (total_merge_reads as u32);\n",
        "                    let penalty_complexity = 5;\n",
    )

    forbidden = (
        "AutoMergeReceiptData",
        "auto_merged_coin_ids",
        "merged_coin_types",
        "total_merge_reads",
        "synthetic_events",
    )
    for token in forbidden:
        if token in text:
            raise RuntimeError(f"legacy runtime auto merge remains: {token}")
    path.write_text(text)


def disable_legacy_rpc_routes() -> None:
    path = Path("crates/kanari-rpc-server/src/lib.rs")
    text = path.read_text()
    text = text.replace(
        "    transaction::{\n        handle_call_function, handle_get_transaction, handle_publish_module,\n        handle_submit_transaction, handle_view_function,\n    },\n",
        "    transaction::{handle_get_transaction, handle_view_function},\n",
    )
    text = text.replace(
        "pub mod nft;\npub mod transaction;\n",
        "pub mod nft;\npub mod object_transaction;\npub mod transaction;\n",
    )
    marker = "    let response = match request.method.as_str() {\n"
    routes = marker + "        object_transaction::SUBMIT_OBJECT_TRANSACTION => object_transaction::submit(&state, &request).await,\n        object_transaction::EXECUTE_OBJECT_TRANSACTION => object_transaction::execute(&state, &request).await,\n        object_transaction::GET_PENDING_OBJECT_TRANSACTIONS => object_transaction::pending(&state, &request).await,\n"
    if "object_transaction::SUBMIT_OBJECT_TRANSACTION" not in text:
        text = replace_once(text, marker, routes, "RPC dispatch")
    for route in (
        "        methods::SUBMIT_TRANSACTION => handle_submit_transaction(&state, &request).await,\n",
        "        methods::PUBLISH_MODULE => handle_publish_module(&state, &request).await,\n",
        "        methods::CALL_FUNCTION => handle_call_function(&state, &request).await,\n",
    ):
        text = text.replace(route, "")
    path.write_text(text)


def switch_rpc_broadcaster() -> None:
    path = Path("crates/kanari-rpc-server/src/lib.rs")
    text = path.read_text()
    text = text.replace(
        "use kanari_types::transaction::SignedTransaction;\n",
        "use kanari_types::signed_object_transaction::SignedObjectTransaction;\n",
    )
    text = text.replace(
        "Fn(SignedTransaction) -> Result<()>",
        "Fn(SignedObjectTransaction) -> Result<()>",
    )
    text = text.replace(
        "signed_tx: SignedTransaction",
        "signed_tx: SignedObjectTransaction",
    )
    text = text.replace(
        "broadcaster: impl Fn(SignedTransaction) -> Result<()>",
        "broadcaster: impl Fn(SignedObjectTransaction) -> Result<()>",
    )
    path.write_text(text)


def register_object_bootstrap() -> None:
    path = Path("move-execution/v1/kanari-move-runtime-v1/src/lib.rs")
    text = path.read_text()
    if "mod state_object_bootstrap;" not in text:
        text = replace_once(
            text,
            "mod state_object_apply_helpers;\n",
            "mod state_object_apply_helpers;\nmod state_object_bootstrap;\n",
            "object bootstrap module",
        )
    path.write_text(text)


remove_runtime_auto_merge()
disable_legacy_rpc_routes()
switch_rpc_broadcaster()
register_object_bootstrap()
