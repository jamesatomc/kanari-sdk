// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use crate::utils::{
    generate_object_id, get_module_dependencies, get_module_public_functions, reroot_path,
};
use framework::{Package, PackageSourceInfo, PackageType};
use framework::{get_framework_path, get_kanari_system_path, get_stdlib_path};
use move_core_types::account_address::AccountAddress;
use move_package::BuildConfig;
use move_package::compilation::compiled_package::CompiledUnitWithSource;
use serde_json::{Value as JsonValue, json};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Build {
    framework_packages: HashMap<PackageType, Option<Package>>,
}

impl Build {
    pub fn new() -> Self {
        Self {
            framework_packages: HashMap::new(),
        }
    }

    pub fn with_stdlib(mut self) -> Self {
        self.framework_packages
            .insert(PackageType::Stdlib, Package::new(PackageType::Stdlib).ok());
        self
    }

    pub fn with_system(mut self) -> Self {
        self.framework_packages
            .insert(PackageType::System, Package::new(PackageType::System).ok());
        self
    }

    pub fn with_framework(mut self) -> Self {
        self.framework_packages.insert(
            PackageType::Framework,
            Package::new(PackageType::Framework).ok(),
        );
        self
    }

    pub fn execute(mut self, path: Option<PathBuf>, config: BuildConfig) -> anyhow::Result<()> {
        let rerooted_path = reroot_path(path)?;

        let mut enhanced_config = config.clone();
        enhanced_config.additional_named_addresses.insert(
            "std".to_string(),
            AccountAddress::from_hex_literal("0x1").unwrap(),
        );

        enhanced_config.additional_named_addresses.insert(
            "kanari_framework".to_string(),
            AccountAddress::from_hex_literal("0x2").unwrap(),
        );

        let stdlib_path = get_stdlib_path();
        let kanari_system_path = get_kanari_system_path();
        let framework_path = get_framework_path();

        let mut framework_info = Vec::new();
        for (pkg_type, pkg_opt) in &mut self.framework_packages {
            if let Some(package) = pkg_opt {
                if let Err(_) = package.load_dependencies() {}

                framework_info.push(json!({
                    "type": format!("{:?}", pkg_type),
                    "path": match pkg_type {
                        PackageType::Stdlib => stdlib_path.to_string_lossy(),
                        PackageType::System => kanari_system_path.to_string_lossy(),
                        PackageType::Framework => framework_path.to_string_lossy(),
                    },
                    "dependencies": package.dependencies.len()
                }));

                Build::configure_for_framework(&mut enhanced_config, package)?;
            }
        }

        if config.fetch_deps_only {
            let mut config = enhanced_config.clone();
            if config.test_mode {
                config.dev_mode = true;
            }
            config.download_deps_for_package(&rerooted_path, &mut std::io::stdout())?;
            println!(
                "{}",
                json!({
                    "status": "success",
                    "type": "deps_only",
                    "path": rerooted_path.to_string_lossy(),
                    "framework_packages": framework_info
                })
            );
            return Ok(());
        }

        let package_source_info = self.analyze_package_sources(&rerooted_path);
        let compile_start = std::time::Instant::now();

        let compiled_package = enhanced_config.compile_package(&rerooted_path, &mut Vec::new())?;
        let compile_duration = compile_start.elapsed();

        let modules_info = self.extract_modules_info(&compiled_package.root_compiled_units);

        let result = json!({
            "status": "success",
            "type": "full_build",
            "compile_time_ms": compile_duration.as_millis(),
            "metadata": {
                "package": {
                    "name": compiled_package.compiled_package_info.package_name.to_string(),
                    "id": generate_object_id(),
                    "path": rerooted_path.to_string_lossy(),
                    "source_files": package_source_info,
                    "info": {
                        "source_digest": compiled_package.compiled_package_info.source_digest,
                        "addresses": compiled_package.compiled_package_info.address_alias_instantiation
                            .iter()
                            .map(|(name, addr)| (name.to_string(), json!(format!("0x{}", addr.to_hex()))))
                            .collect::<serde_json::Map<String, JsonValue>>(),
                    }
                },
                "framework": {
                    "packages": framework_info,
                    "stdlib_path": stdlib_path.to_string_lossy().to_string(),
                    "system_path": kanari_system_path.to_string_lossy().to_string(),
                    "framework_path": framework_path.to_string_lossy().to_string(),
                },
                "modules": modules_info
            }
        });

        println!("{}", serde_json::to_string_pretty(&result)?);
        Ok(())
    }

    fn configure_for_framework(config: &mut BuildConfig, package: &Package) -> anyhow::Result<()> {
        let _ = config;
        if let Ok(dep_paths) = package.resolve_dependencies() {
            for dep_path in dep_paths {
                if dep_path.exists() {}
            }
        }

        Ok(())
    }

    fn analyze_package_sources(&self, path: &PathBuf) -> JsonValue {
        let source_dir = path.join("sources");
        let mut source_files = Vec::new();

        if source_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(source_dir) {
                for entry in entries {
                    if let Ok(entry) = entry {
                        let path = entry.path();
                        if path.extension().map_or(false, |ext| ext == "move") {
                            if let Ok(info) = PackageSourceInfo::create(&path) {
                                source_files.push(json!({
                                    "path": path.to_string_lossy(),
                                    "name": info.module_name,
                                    "has_tests": info.has_tests,
                                    "dependencies": info.dependencies,
                                }));
                            } else {
                                source_files.push(json!({
                                    "path": path.to_string_lossy(),
                                    "name": path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown"),
                                }));
                            }
                        }
                    }
                }
            }
        }

        json!(source_files)
    }

    fn extract_modules_info(&self, compiled_units: &[CompiledUnitWithSource]) -> JsonValue {
        let modules = compiled_units
            .iter()
            .map(|unit| {
                json!({
                    "name": unit.unit.name().to_string(),
                    "source_path": unit.source_path.to_string_lossy(),
                    "dependencies": get_module_dependencies(&unit.unit),
                    "public_functions": get_module_public_functions(&unit.unit)
                })
            })
            .collect::<Vec<_>>();

        json!(modules)
    }
}
