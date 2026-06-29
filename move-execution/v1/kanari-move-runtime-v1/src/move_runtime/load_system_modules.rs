// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Result, anyhow};
use kanari_crypto::hash_data_blake3;
use kanari_types::address::Address as KanariAddress;
use log::warn;
use move_binary_format::compatibility::Compatibility;
use move_binary_format::file_format::CompiledModule;
use move_binary_format::normalized;
use move_bytecode_verifier::dependencies;
use move_bytecode_verifier::verifier::verify_module_unmetered;
use move_core_types::account_address::AccountAddress;
use move_core_types::language_storage::ModuleId;
use move_package::BuildConfig;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

pub(crate) const MOVE_STDLIB_BYTECODE_SEGMENTS: &[&str] = &[
    "crates",
    "kanari-frameworks",
    "packages",
    "move-stdlib",
    "build",
    "MoveStdlib",
    "bytecode_modules",
];

pub(crate) const KANARI_SYSTEM_BYTECODE_SEGMENTS: &[&str] = &[
    "crates",
    "kanari-frameworks",
    "packages",
    "kanari-system",
    "build",
    "KanariSystem",
    "bytecode_modules",
];

fn mv_filename(name: &str) -> String {
    if name.ends_with(".mv") {
        name.to_owned()
    } else {
        format!("{name}.mv")
    }
}

fn is_test_module_artifact(file_name: &str, module_name: &str) -> bool {
    let lower_file = file_name.to_ascii_lowercase();
    let lower_module = module_name.to_ascii_lowercase();
    lower_file.ends_with("_tests.mv")
        || lower_file.ends_with("_test.mv")
        || lower_module.ends_with("_tests")
        || lower_module.ends_with("_test")
}

fn build_info_path(modules_dir: &Path) -> Option<PathBuf> {
    Some(modules_dir.parent()?.join("BuildInfo.yaml"))
}

fn package_root_from_modules_dir(modules_dir: &Path) -> Option<PathBuf> {
    Some(modules_dir.parent()?.parent()?.parent()?.to_path_buf())
}

fn ensure_production_build_artifacts(modules_dir: &Path) -> Result<()> {
    let Some(build_info) = build_info_path(modules_dir) else {
        return Ok(());
    };
    let Ok(contents) = std::fs::read_to_string(&build_info) else {
        return Ok(());
    };

    if !contents.contains("test_mode: true") {
        return Ok(());
    }

    let package_root = package_root_from_modules_dir(modules_dir).ok_or_else(|| {
        anyhow!(
            "Could not infer package root from contaminated build dir: {}",
            modules_dir.display()
        )
    })?;

    tracing::warn!(
        "Detected test-mode Move build artifacts at {}; rebuilding package in production mode",
        modules_dir.display()
    );

    let mut sink = std::io::sink();
    BuildConfig::default()
        .compile_package(&package_root, &mut sink)
        .map_err(|e| {
            anyhow!(
                "Failed to rebuild production Move package at {}: {}",
                package_root.display(),
                e
            )
        })?;

    let refreshed = std::fs::read_to_string(&build_info).map_err(|e| {
        anyhow!(
            "Failed to re-read BuildInfo after rebuilding {}: {}",
            build_info.display(),
            e
        )
    })?;
    if refreshed.contains("test_mode: true") {
        return Err(anyhow!(
            "Move package at {} still reports test_mode: true after rebuild",
            package_root.display()
        ));
    }

    Ok(())
}

#[derive(Clone)]
pub(crate) struct DiscoveredModule {
    pub module_id: ModuleId,
    pub module_name: String,
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub compiled: CompiledModule,
    pub deps: Vec<ModuleId>,
}

fn discover_modules_in_dir(
    modules_dir: &Path,
    expected_addr: AccountAddress,
) -> Vec<DiscoveredModule> {
    let Ok(dir) = std::fs::read_dir(modules_dir) else {
        return vec![];
    };

    let mut modules = Vec::new();
    for entry in dir.flatten() {
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mv") {
            continue;
        }

        let file_name = entry.file_name().to_string_lossy().to_string();
        let Ok(bytes) = std::fs::read(&path) else {
            warn!("Warning: Failed to read module file: {:?}", path);
            continue;
        };

        let compiled = match CompiledModule::deserialize_with_defaults(&bytes) {
            Ok(m) => m,
            Err(e) => {
                warn!("Warning: Failed to decode {}: {}", file_name, e);
                continue;
            }
        };

        let module_id = compiled.self_id();
        let module_name = module_id.name().to_string();

        if is_test_module_artifact(&file_name, &module_name) {
            continue;
        }

        if *module_id.address() != expected_addr {
            continue;
        }

        if let Err(e) = verify_module_unmetered(&compiled) {
            warn!("Warning: Bytecode verify failed for {}: {:?}", file_name, e);
            continue;
        }

        let deps = compiled.immediate_dependencies();
        modules.push(DiscoveredModule {
            module_id,
            module_name,
            file_name,
            bytes,
            compiled,
            deps,
        });
    }

    modules.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    modules
}

fn topo_sort_modules(
    modules: Vec<DiscoveredModule>,
) -> Result<Vec<DiscoveredModule>, anyhow::Error> {
    if modules.len() <= 1 {
        return Ok(modules);
    }

    let mut index_by_id: HashMap<ModuleId, usize> = HashMap::with_capacity(modules.len());
    for (idx, m) in modules.iter().enumerate() {
        index_by_id.insert(m.module_id.clone(), idx);
    }

    let mut indegree = vec![0usize; modules.len()];
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); modules.len()];

    for (idx, m) in modules.iter().enumerate() {
        for dep in &m.deps {
            if let Some(&dep_idx) = index_by_id.get(dep) {
                outgoing[dep_idx].push(idx);
                indegree[idx] += 1;
            }
        }
    }

    let mut ready: BTreeSet<(String, usize)> = BTreeSet::new();
    for idx in 0..modules.len() {
        if indegree[idx] == 0 {
            ready.insert((modules[idx].module_name.clone(), idx));
        }
    }

    let mut ordered_idxs = Vec::with_capacity(modules.len());
    let mut emitted = vec![false; modules.len()];

    while let Some((module_name, idx)) = ready.iter().next().cloned() {
        ready.remove(&(module_name, idx));
        ordered_idxs.push(idx);
        emitted[idx] = true;

        for &next in &outgoing[idx] {
            indegree[next] = indegree[next].saturating_sub(1);
            if indegree[next] == 0 {
                ready.insert((modules[next].module_name.clone(), next));
            }
        }
    }

    if ordered_idxs.len() != modules.len() {
        // Cycles or missing intra-dir deps. In Move, circular dependencies are prohibited,
        // and modules must be published in a strict dependency order. An error should be returned
        // to alert the operator of an invalid framework state.
        let remaining: Vec<(String, usize)> = (0..modules.len())
            .filter(|i| !emitted[*i])
            .map(|i| (modules[i].module_name.clone(), i))
            .collect();

        return Err(anyhow!(
            "Could not fully resolve module dependency order ({} of {}). \
             Found {} modules with cyclic or missing dependencies: {:?}",
            ordered_idxs.len(),
            modules.len(),
            remaining.len(),
            remaining.iter().map(|(name, _)| name).collect::<Vec<_>>()
        ));
    }

    let mut slots: Vec<Option<DiscoveredModule>> = modules.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(slots.len());
    for idx in ordered_idxs {
        if let Some(m) = slots.get_mut(idx).and_then(|s| s.take()) {
            out.push(m);
        }
    }
    Ok(out)
}

fn compute_framework_manifest_and_hash(
    modules: &[DiscoveredModule],
) -> (Vec<(String, String)>, String) {
    let mut manifest: Vec<(String, String)> = modules
        .iter()
        .map(|m| {
            let id = format!(
                "{}::{}",
                m.module_id.address().to_hex_literal(),
                m.module_id.name()
            );
            let h = hex::encode(hash_data_blake3(&m.bytes));
            (id, h)
        })
        .collect();
    manifest.sort_by(|a, b| a.0.cmp(&b.0));

    let mut blob = Vec::new();
    for (id, h) in &manifest {
        blob.extend_from_slice(id.as_bytes());
        blob.push(0);
        blob.extend_from_slice(h.as_bytes());
        blob.push(0);
    }
    let root_hash = hex::encode(hash_data_blake3(&blob));
    (manifest, root_hash)
}

fn verify_framework_hash(
    state: &crate::storage::move_vm_state::MoveVMState,
    framework_id: &str,
    modules: &[DiscoveredModule],
    expected_env_var: &str,
    changed_warning: &str,
) -> Result<()> {
    let (manifest, hash_hex) = compute_framework_manifest_and_hash(modules);
    log::info!("{} framework hash (disk): {}", framework_id, hash_hex);
    if let Some(prev) = state.get_framework_hash(framework_id)
        && prev != hash_hex
    {
        warn!(
            "{}",
            changed_warning
                .replace("{prev}", &prev)
                .replace("{hash}", &hash_hex)
        );
    }
    state.save_framework_manifest(framework_id, &manifest, &hash_hex)?;
    if let Ok(expected) = std::env::var(expected_env_var)
        && expected != hash_hex
    {
        return Err(anyhow!(
            "{} framework hash mismatch: expected {}, got {}",
            framework_id,
            expected,
            hash_hex
        ));
    }
    Ok(())
}

fn prune_framework_modules(
    runtime: &super::MoveRuntime,
    modules: &[DiscoveredModule],
    framework_addr: AccountAddress,
) {
    #[cfg(feature = "framework-pruning")]
    {
        let keep: BTreeSet<ModuleId> = modules.iter().map(|m| m.module_id.clone()).collect();
        for id in runtime.state.get_all_module_ids().unwrap_or_default() {
            if *id.address() == framework_addr && !keep.contains(&id) {
                if let Err(e) = runtime.state.delete_module(&id) {
                    warn!("Warning: Failed to prune module {}: {}", id, e);
                } else if let Ok(mut mods) = runtime.published_modules.write() {
                    mods.remove(&id);
                }
            }
        }
    }

    #[cfg(not(feature = "framework-pruning"))]
    let _ = (runtime, modules, framework_addr);
}

fn save_framework_modules(
    runtime: &super::MoveRuntime,
    modules: Vec<DiscoveredModule>,
    incompatible_error_prefix: &str,
) -> Result<usize> {
    let mut count = 0;
    for m in modules {
        let module_file = mv_filename(&m.file_name);
        if let Some(old_bytes) = runtime.state.get_module(&m.module_id)
            && old_bytes != m.bytes
        {
            let old_compiled = CompiledModule::deserialize_with_defaults(&old_bytes)?;
            let old_norm = normalized::Module::new(&old_compiled);
            let new_norm = normalized::Module::new(&m.compiled);
            if let Err(e) = Compatibility::full_check().check(&old_norm, &new_norm) {
                if std::env::var("KANARI_FRAMEWORK_ALLOW_INCOMPATIBLE")
                    .ok()
                    .as_deref()
                    != Some("1")
                {
                    return Err(anyhow!(
                        "Incompatible {} upgrade for {} (set KANARI_FRAMEWORK_ALLOW_INCOMPATIBLE=1 to override): {:?}",
                        incompatible_error_prefix,
                        m.module_id,
                        e
                    ));
                }
                warn!(
                    "Warning: Allowing incompatible {} upgrade for {}: {:?}",
                    incompatible_error_prefix, m.module_id, e
                );
            }
        }
        if let Err(e) = runtime.state.save_module(&m.module_id, &m.bytes) {
            tracing::warn!("Failed to save {}: {}", module_file, e);
            continue;
        }
        if let Ok(mut mods) = runtime.published_modules.write() {
            mods.insert(m.module_id);
        }
        count += 1;
    }
    Ok(count)
}

fn find_modules_dir(env_var: &str, segments: &[&str]) -> PathBuf {
    if let Ok(path_str) = std::env::var(env_var) {
        return PathBuf::from(path_str);
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut dir = Some(cwd.as_path());
    while let Some(d) = dir {
        let mut p = PathBuf::from(d);
        for seg in segments.iter() {
            p.push(seg);
        }
        if p.exists() {
            return p;
        }
        dir = d.parent();
    }
    let mut p = cwd;
    for seg in segments.iter() {
        p.push(seg);
    }
    p
}

fn find_move_stdlib_modules_dir() -> PathBuf {
    find_modules_dir("MOVE_STDLIB_PATH", MOVE_STDLIB_BYTECODE_SEGMENTS)
}

pub(crate) fn find_kanari_system_modules_dir() -> PathBuf {
    find_modules_dir("KANARI_FRAMEWORK_PATH", KANARI_SYSTEM_BYTECODE_SEGMENTS)
}

fn verbose_startup_enabled() -> bool {
    std::env::var("KANARI_VERBOSE_STARTUP")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

/// Load move-stdlib and kanari-system modules as methods on `MoveRuntime`
impl super::MoveRuntime {
    /// Load move-stdlib modules (0x1::*)
    pub(crate) fn load_move_stdlib(&self) -> Result<()> {
        let modules_dir = find_move_stdlib_modules_dir();
        ensure_production_build_artifacts(&modules_dir)?;

        if verbose_startup_enabled() {
            tracing::info!("Looking for Move stdlib modules at {:?}", modules_dir);
        }

        if !modules_dir.exists() {
            warn!(
                "Warning: Move stdlib modules not found at {:?}",
                modules_dir
            );
            warn!("Standard library will not be pre-loaded.");
            return Ok(());
        }

        // Load stdlib modules in dependency order
        let std_addr = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)?;

        let mut count = 0;
        let modules = topo_sort_modules(discover_modules_in_dir(&modules_dir, std_addr))?;
        if modules.is_empty() {
            return Err(anyhow::anyhow!(
                "No Move stdlib modules (*.mv) found at {:?}",
                modules_dir
            ));
        }

        for m in &modules {
            dependencies::verify_module(&m.compiled, modules.iter().map(|d| &d.compiled)).map_err(
                |e| anyhow::anyhow!("Stdlib deps verify failed for {}: {:?}", m.file_name, e),
            )?;
        }

        verify_framework_hash(
            &self.state,
            "0x1",
            &modules,
            "KANARI_FRAMEWORK_EXPECTED_HASH_0X1",
            "Warning: stdlib framework hash changed (db: {prev}, disk: {hash}). Ensure all validators upgrade together.",
        )?;
        prune_framework_modules(self, &modules, std_addr);
        count += save_framework_modules(self, modules, "stdlib")?;

        tracing::info!(
            modules = count,
            address = "0x1",
            "Move stdlib modules loaded"
        );
        Ok(())
    }

    /// Load Kanari system modules (0x2::*)
    pub(crate) fn load_kanari_system(&self) -> Result<()> {
        let modules_dir = find_kanari_system_modules_dir();
        ensure_production_build_artifacts(&modules_dir)?;

        if verbose_startup_enabled() {
            tracing::info!("Looking for Kanari system modules at {:?}", modules_dir);
        }

        if !modules_dir.exists() {
            warn!(
                "Warning: Kanari system modules not found at {:?}",
                modules_dir
            );
            warn!("System modules will not be pre-loaded. You may need to publish them manually.");
            warn!("To fix this:");
            warn!("  cd crates/kanari-frameworks/packages/move-stdlib");
            warn!("  cd crates/kanari-frameworks/packages/kanari-system");
            warn!("  kanari move build");
            return Ok(());
        }

        let system_addr = AccountAddress::from_hex_literal(KanariAddress::KANARI_SYSTEM_ADDRESS)?;
        let mut count = 0;

        let modules = topo_sort_modules(discover_modules_in_dir(&modules_dir, system_addr))?;
        if modules.is_empty() {
            return Err(anyhow::anyhow!(
                "No kanari-system modules (*.mv) found at {:?}",
                modules_dir
            ));
        }

        verify_framework_hash(
            &self.state,
            "0x2",
            &modules,
            "KANARI_FRAMEWORK_EXPECTED_HASH_0X2",
            "Warning: system framework hash changed (db: {prev}, disk: {hash}). Ensure all validators upgrade together.",
        )?;

        let stdlib_dir = find_move_stdlib_modules_dir();
        let std_addr = AccountAddress::from_hex_literal(KanariAddress::STD_ADDRESS)?;
        let stdlib_modules = topo_sort_modules(discover_modules_in_dir(&stdlib_dir, std_addr))?;
        let all_deps: Vec<&CompiledModule> = stdlib_modules
            .iter()
            .map(|m| &m.compiled)
            .chain(modules.iter().map(|m| &m.compiled))
            .collect();
        for m in &modules {
            dependencies::verify_module(&m.compiled, all_deps.iter().copied()).map_err(|e| {
                anyhow::anyhow!("System deps verify failed for {}: {:?}", m.file_name, e)
            })?;
        }

        prune_framework_modules(self, &modules, system_addr);
        count += save_framework_modules(self, modules, "system")?;

        tracing::info!(
            modules = count,
            address = "0x2",
            "Kanari system modules loaded"
        );
        Ok(())
    }
}

/// Public API: Load and sort system modules from a directory
pub(crate) fn load_system_modules_from_dir(modules_dir: &Path) -> Result<Vec<DiscoveredModule>> {
    let system_addr = KanariAddress::kanari_system_account_address();
    let move_system_addr = AccountAddress::from_hex_literal(system_addr.to_hex_literal().as_str())?;

    // Discover all modules in the directory
    let discovered_modules = discover_modules_in_dir(modules_dir, move_system_addr);

    if discovered_modules.is_empty() {
        return Err(anyhow!(
            "No valid framework modules found in {}",
            modules_dir.display()
        ));
    }

    // Sort modules in topological order (dependencies first)
    let sorted_modules = topo_sort_modules(discovered_modules)?;

    Ok(sorted_modules)
}
