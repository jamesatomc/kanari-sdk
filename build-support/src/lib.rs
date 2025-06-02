// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;
use std::process::Command;

/// Checks if a git submodule exists and is initialized
pub fn is_submodule_initialized(workspace_root: &Path, submodule: &str) -> bool {
    let submodule_path = workspace_root.join(submodule);

    submodule_path.exists()
        && !submodule_path
            .read_dir()
            .map(|mut d| d.next().is_none())
            .unwrap_or(true)
}

/// Initializes a git submodule
pub fn initialize_submodule(workspace_root: &Path, submodule: &str) -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(workspace_root)
        .args(&["submodule", "update", "--init", "--recursive", submodule])
        .output()
        .map_err(|e| format!("Failed to execute git command: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("Git command failed: {}", stderr))
    }
}

/// Initializes all submodules in the workspace
pub fn initialize_all_submodules(workspace_root: &Path) -> Result<(), String> {
    // Default submodules to initialize
    let submodules = ["third_party/move"];

    for submodule in &submodules {
        if !is_submodule_initialized(workspace_root, submodule) {
            initialize_submodule(workspace_root, submodule)?;
        }
    }

    Ok(())
}
