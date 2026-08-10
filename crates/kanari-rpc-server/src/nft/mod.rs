// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::{
    RpcServerState, invalid_params_response, parse_labeled_params, respond_with_serialize,
};
use kanari_rpc_api::{RpcRequest, RpcResponse};
use kanari_types::address::Address;
use serde::Deserialize;
use serde_json::json;

#[allow(dead_code)]
#[derive(Deserialize)]
struct MoveString {
    bytes: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct MoveUrl {
    inner: MoveString,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct UniversalStandardNft {
    _id: [u8; 32],
    name: MoveString,
    image_url: MoveUrl,
    description: MoveString,
    attribute_keys: Vec<MoveString>,
    attribute_values: Vec<MoveString>,
    number: MoveString,
    collection_id: [u8; 32],
    creator: [u8; 32],
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ParsedCollection {
    _id: [u8; 32],
    name: MoveString,
    description: MoveString,
    banner_url: MoveUrl,
    website_url: MoveUrl,
    creator: [u8; 32],
    max_supply: u64,
}

fn parse_any_nft(_type_tag: &str, data: &[u8]) -> Option<serde_json::Value> {
    if let Ok(parsed) = bcs::from_bytes::<UniversalStandardNft>(data) {
        return Some(json!({
            "name": move_str_to_string(&parsed.name),
            "image_url": move_str_to_string(&parsed.image_url.inner),
            "description": move_str_to_string(&parsed.description),
            "number": move_str_to_string(&parsed.number),
            "creator": format!("0x{}", hex::encode(parsed.creator)),
            "attributes": {
                "keys": move_str_vec_to_strings(&parsed.attribute_keys),
                "values": move_str_vec_to_strings(&parsed.attribute_values),
            }
        }));
    }
    None
}

fn move_str_to_string(m: &MoveString) -> String {
    String::from_utf8_lossy(&m.bytes).to_string()
}

fn move_str_vec_to_strings(vec: &[MoveString]) -> Vec<String> {
    vec.iter().map(move_str_to_string).collect()
}

fn parse_collection_fields(
    obj: &kanari_move_runtime_v1::changeset::CreatedObject,
) -> (String, String, String, String, u64) {
    let mut name = "Unknown Collection".to_string();
    let mut description = String::new();
    let mut banner_url = String::new();
    let mut website_url = String::new();
    let mut max_supply = 0u64;

    if obj.type_.contains("::collection::Collection")
        && let Ok(parsed) = bcs::from_bytes::<ParsedCollection>(&obj.data)
    {
        name = move_str_to_string(&parsed.name);
        description = move_str_to_string(&parsed.description);
        banner_url = move_str_to_string(&parsed.banner_url.inner);
        website_url = move_str_to_string(&parsed.website_url.inner);
        max_supply = parsed.max_supply;
    }

    (name, description, banner_url, website_url, max_supply)
}

pub async fn handle_get_owned_nfts(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let owner: String = match parse_labeled_params(request.id, &request.params, "owner") {
        Ok(owner) => owner,
        Err(response) => return *response,
    };

    let addr = match Address::parse_to_account_address(&owner) {
        Ok(a) => a,
        Err(_) => return invalid_params_response(request.id, "Invalid owner address format"),
    };

    let state_guard = state.engine.state_read();
    let owned_ids = match state_guard.get_owned_objects(&addr) {
        Ok(ids) => ids,
        Err(_) => return respond_with_serialize(request.id, Vec::<serde_json::Value>::new()),
    };

    let mut nfts = Vec::new();

    for id in owned_ids {
        if let Ok(Some(obj)) = state_guard.get_object(&id)
            && (obj.type_.contains("::nft::") || obj.type_.to_uppercase().contains("NFT"))
        {
            nfts.push(json!({
                "object_id": id,
                "type": obj.type_,
                "owner": owner,
                "version": obj.version,
                "parsed": parse_any_nft(&obj.type_, &obj.data),
                "data": obj.data,
            }));
        }
    }

    respond_with_serialize(request.id, nfts)
}

pub async fn handle_list_collections(state: &RpcServerState, request: &RpcRequest) -> RpcResponse {
    let state_guard = state.engine.state_read();
    let ids = state_guard.get_all_collection_ids();
    let mut collections = Vec::new();

    for id in ids {
        if let Ok(Some(obj)) = state_guard.get_object(&id) {
            let (name, description, banner_url, website_url, max_supply) =
                parse_collection_fields(&obj);

            collections.push(serde_json::json!({
                "id": id,
                "type": obj.type_,
                "name": name,
                "description": description,
                "banner_url": banner_url,
                "website_url": website_url,
                "max_supply": max_supply,
                "owner": format!("{:#x}", obj.owner),
                "data": obj.data,
            }));
        }
    }
    respond_with_serialize(request.id, collections)
}

pub async fn handle_get_nfts_by_collection(
    state: &RpcServerState,
    request: &RpcRequest,
) -> RpcResponse {
    let coll_id: String = match parse_labeled_params(request.id, &request.params, "id") {
        Ok(id) => id,
        Err(response) => return *response,
    };

    let state_guard = state.engine.state_read();
    let nft_ids = state_guard.get_collection_nft_ids(&coll_id);
    let mut nfts = Vec::new();

    for id in nft_ids {
        if let Ok(Some(obj)) = state_guard.get_object(&id) {
            nfts.push(serde_json::json!({
                "object_id": id,
                "type": obj.type_,
                "parsed": parse_any_nft(&obj.type_, &obj.data),
                "data": obj.data,
            }));
        }
    }
    respond_with_serialize(request.id, nfts)
}
