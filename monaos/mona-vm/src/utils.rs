// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use move_compiler::compiled_unit::CompiledUnit;
use move_package::source_package::layout::SourcePackageLayout;
use rand::{Rng, thread_rng};
use serde_json::{Value as JsonValue, json};
use sha3::{Digest, Sha3_256};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn reroot_path(path: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let path = path.unwrap_or_else(|| PathBuf::from("."));
    let rooted_path = SourcePackageLayout::try_find_root(&path.canonicalize()?)?;
    std::env::set_current_dir(rooted_path).unwrap();

    Ok(PathBuf::from("."))
}

pub fn generate_object_id() -> String {
    let mut hasher = Sha3_256::new();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    hasher.update(timestamp.to_le_bytes());
    hasher.update(counter.to_le_bytes());

    let hash = hasher.finalize();
    format!("0x{:0>64}", hex::encode(hash))
}

pub fn generate_random_hex(length: usize) -> String {
    let mut rng = thread_rng();
    const CHARSET: &[u8] = b"0123456789abcdef";

    (0..length)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect()
}

pub fn get_module_dependencies(module: &CompiledUnit) -> Vec<String> {
    let compiled_module = &module.module;

    compiled_module
        .module_handles()
        .iter()
        .filter_map(|handle| {
            if handle.name == compiled_module.self_handle().name {
                return None;
            }

            let module_name = compiled_module.identifier_at(handle.name);
            let address = compiled_module.address_identifier_at(handle.address);

            Some(format!("{}::{}", address, module_name))
        })
        .collect()
}

pub fn get_module_public_functions(module: &CompiledUnit) -> Vec<JsonValue> {
    let compiled_module = &module.module;
    let module_address = compiled_module.address().to_string();
    let module_name = compiled_module.name().to_string();
    let full_module_id = format!("0x{}", module_address);

    compiled_module
        .function_defs()
        .iter()
        .filter_map(|func_def| {
            if func_def.visibility == move_binary_format::file_format::Visibility::Public {
                let func_handle = compiled_module.function_handle_at(func_def.function);
                let func_name = compiled_module.identifier_at(func_handle.name).to_string();
                let complete_func_path = format!("{}::{}", full_module_id, module_name);
                let signature = compiled_module.signature_at(func_handle.parameters);
                let param_count = signature.0.len();

                Some(json!({
                    "name": func_name,
                    "full_name": format!("{}::{}", complete_func_path, func_name),
                    "module": complete_func_path,
                    "visibility": "public",
                    "parameters": param_count,
                }))
            } else {
                None
            }
        })
        .collect()
}
