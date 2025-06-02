// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use common::get_kari_dir;
use mona_storage::{BlockchainStorage, RocksDBStorage};
use move_core_types::account_address::AccountAddress;
use std::time::{SystemTime, UNIX_EPOCH};

// Structure to represent a Move VM Module
#[derive(Clone)]
pub struct VMModule {
    pub module_id: String,
    pub address: AccountAddress,
    pub name: String,
    pub bytecode: Vec<u8>,
    pub public_functions: Vec<String>,
    pub deploy_block_height: u32,
}

impl VMModule {
    pub fn new(
        address: AccountAddress,
        name: String,
        bytecode: Vec<u8>,
        public_functions: Vec<String>,
        deploy_block_height: u32,
    ) -> Self {
        let module_id = format!("0x{}::{}", address.to_hex(), name);

        Self {
            module_id,
            address,
            name,
            bytecode,
            public_functions,
            deploy_block_height,
        }
    }
}

// Load module from secure storage
pub fn load_module_from_mvsm(
    module_id: &str,
    _mvsm_path: Option<&str>,
) -> Result<VMModule, String> {
    let kari_dir = get_kari_dir();
    let db_path = kari_dir.join("storage").join("mvsm_db");
    let storage = RocksDBStorage::new(db_path)
        .map_err(|e| format!("Failed to initialize MVSM storage: {}", e))?;

    let parts: Vec<&str> = module_id.split("::").collect();
    if parts.len() != 2 {
        return Err(format!("Invalid module ID format: {}", module_id));
    }

    let address = parts[0].trim_start_matches("0x");
    let module_name = parts[1];

    let storage_keys = vec![
        format!("{}_{}", address, module_name),
        format!("0x{}_{}", address, module_name),
        module_id.to_string(),
    ];

    log::info!("Loading MVSM module from secure storage: {}", module_id);

    let mut module_data = None;
    for key in storage_keys {
        if let Ok(Some(data)) = storage.load_data(key.as_bytes()) {
            module_data = Some(data);
            log::debug!("Found module data with key: {}", key);
            break;
        }
    }

    let file_content =
        module_data.ok_or_else(|| format!("MVSM module not found in storage: {}", module_id))?;

    let content_str = String::from_utf8_lossy(&file_content);
    let parts: Vec<&str> = content_str.split("\n===BYTECODE===\n").collect();

    if parts.len() != 2 {
        return Err("Invalid MVSM data format in storage".to_string());
    }

    let metadata: serde_json::Value = serde_json::from_str(parts[0])
        .map_err(|e| format!("Failed to parse MVSM metadata: {}", e))?;

    let address_str = metadata["address"]
        .as_str()
        .ok_or_else(|| "Missing address in MVSM metadata".to_string())?;
    let address = AccountAddress::from_hex_literal(address_str)
        .or_else(|_| AccountAddress::from_hex(address_str.trim_start_matches("0x")))
        .map_err(|e| format!("Invalid address format: {}", e))?;

    let name = metadata["name"]
        .as_str()
        .ok_or_else(|| "Missing module name in MVSM metadata".to_string())?
        .to_string();

    let public_functions = metadata["public_functions"]
        .as_array()
        .map(|funcs| {
            funcs
                .iter()
                .filter_map(|f| f.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let bytecode = parts[1].as_bytes().to_vec();
    let deploy_block_height = metadata["deploy_block_height"].as_u64().unwrap_or(0) as u32;

    let vm_module = VMModule::new(
        address,
        name,
        bytecode,
        public_functions,
        deploy_block_height,
    );

    log::info!(
        "Successfully loaded module {} from secure storage",
        vm_module.module_id
    );
    Ok(vm_module)
}

// Serialize module to secure storage
pub fn serialize_module_to_mvsm(
    module: &VMModule,
    package_name: &str,
    _output_path: Option<&std::path::PathBuf>,
) -> anyhow::Result<String> {
    let kari_dir = get_kari_dir();
    let db_path = kari_dir.join("storage").join("mvsm_db");
    let storage = RocksDBStorage::new(db_path)?;

    let module_data = serde_json::json!({
        "module_id": module.module_id.clone(),
        "address": format!("0x{}", module.address.to_hex()),
        "name": module.name.clone(),
        "package": package_name,
        "deploy_block_height": module.deploy_block_height,
        "public_functions": module.public_functions.clone(),
        "bytecode_size": module.bytecode.len(),
        "version": "1.0",
        "timestamp": SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    });

    let mut file_content = module_data.to_string().into_bytes();
    file_content.extend_from_slice(b"\n===BYTECODE===\n");
    file_content.extend_from_slice(&module.bytecode);

    let storage_key = format!("{}_{}", module.address.to_hex(), module.name);
    storage.save_data(storage_key.as_bytes(), &file_content)?;
    storage.flush()?;

    let storage_location = format!("secure_storage://{}", storage_key);
    log::info!(
        "Stored .mvsm module in secure storage: {}",
        storage_location
    );

    Ok(storage_location)
}
