use anyhow::Result;
use kanari_move_runtime_v1::move_runtime::MoveRuntime;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::ModuleId;
use move_package::BuildConfig;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

pub fn coin_object_bytes(object_addr: AccountAddress, balance: u64) -> Vec<u8> {
    let mut bytes = object_addr.to_vec();
    bytes.extend_from_slice(&balance.to_le_bytes());
    bytes
}

pub fn build_temp_module_bytes(
    package_name: &str,
    named_address: &str,
    named_value: &str,
    source: &str,
) -> Result<(ModuleId, Vec<u8>)> {
    let dir = tempdir()?;
    let package_dir = dir.path();
    let framework_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../crates/kanari-frameworks/packages/kanari-system");
    let framework_dir = framework_dir.to_string_lossy().replace('\\', "/");
    let install_dir = tempdir()?.keep();

    fs::create_dir_all(package_dir.join("sources"))?;
    fs::write(
        package_dir.join("Move.toml"),
        format!(
            r#"[package]
name = "{package_name}"

[addresses]
{named_address} = "{named_value}"

[dependencies]
KanariSystem = {{ local = "{}" }}
"#,
            framework_dir
        ),
    )?;
    fs::write(package_dir.join("sources/module.move"), source)?;

    let compiled = BuildConfig {
        install_dir: Some(install_dir),
        ..Default::default()
    }
    .compile_package(package_dir, &mut std::io::sink())?;

    let module = compiled
        .all_modules()
        .next()
        .ok_or_else(|| anyhow::anyhow!("expected compiled module"))?;
    let module_id = module.unit.module.self_id();
    let mut bytes = Vec::new();
    module.unit.module.serialize(&mut bytes)?;
    Ok((module_id, bytes))
}

pub fn publish_temp_module(
    runtime: &MoveRuntime,
    package_name: &str,
    named_address: &str,
    publisher: AccountAddress,
    source: &str,
) -> Result<ModuleId> {
    let (module_id, module_bytes) = build_temp_module_bytes(
        package_name,
        named_address,
        &publisher.to_hex_literal(),
        source,
    )?;
    runtime.publish_module(module_bytes, publisher, None, None)?;
    Ok(module_id)
}
