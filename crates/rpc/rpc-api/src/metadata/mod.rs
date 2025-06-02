// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use std::fs;
use jsonrpc_core::{Params, Result as JsonRpcResult, Error as RpcError};
use serde_json::{json, Value as JsonValue};
use mona_storage::file_storage::{FileStorage, StorageError2};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::Deserialize;

// File upload parameters
#[derive(Deserialize)]
pub struct UploadParams {
    pub filename: String,
    pub data: String, // base64 encoded file content
}

pub fn upload_file(params: Params) -> JsonRpcResult<JsonValue> {
    let upload_params: UploadParams = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid parameters: {}", e)))?;

    // Initialize storage
    FileStorage::init_storage()
        .map_err(|_| RpcError::internal_error())?;

    // Create temporary file from base64 data
    let file_data = BASE64.decode(upload_params.data)
        .map_err(|e| RpcError::invalid_params(format!("Invalid base64 data: {}", e)))?;
    
    let temp_dir = std::env::temp_dir();
    let temp_path = temp_dir.join(&upload_params.filename);
    fs::write(&temp_path, &file_data)
        .map_err(|_| RpcError::internal_error())?;

    // Use FileStorage::upload like CLI
    match FileStorage::upload(&temp_path, upload_params.filename) {
        Ok(storage) => {
            // Clean up temp file
            let _ = fs::remove_file(temp_path);
            
            let response = json!({
                "id": storage.id.to_string(),
                "filename": storage.metadata.filename,
                "location": storage.path.to_string_lossy(),
                "size": storage.metadata.size,
                "content_type": storage.metadata.content_type
            });
            Ok(response)
        },
        Err(_e) => {
            let _ = fs::remove_file(temp_path);
            Err(RpcError::internal_error())
        }
    }
}

pub fn get_file(params: Params) -> JsonRpcResult<JsonValue> {
    let file_id: String = params.parse()
        .map_err(|e| RpcError::invalid_params(format!("Invalid file ID: {}", e)))?;

    // Initialize storage
    FileStorage::init_storage()
        .map_err(|_| RpcError::internal_error())?;

    match FileStorage::get_by_id(&file_id) {
        Ok(storage) => {
            let file_data = fs::read(&storage.path)
                .map_err(|_| RpcError::internal_error())?;
            
            let response = json!({
                "id": storage.id.to_string(),
                "filename": storage.metadata.filename,
                "size": storage.metadata.size,
                "content_type": storage.metadata.content_type,
                "data": BASE64.encode(file_data),
                "location": storage.path.to_string_lossy()
            });
            
            Ok(response)
        },
        Err(StorageError2::NotFound) => Err(RpcError::invalid_params("File not found")),
        Err(_) => Err(RpcError::internal_error())
    }
}