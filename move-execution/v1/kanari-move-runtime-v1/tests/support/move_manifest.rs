use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub fn kanari_system_package_path() -> Result<String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let package = manifest_dir.join("../../../crates/kanari-frameworks/packages/kanari-system");
    let canonical = package.canonicalize().with_context(|| {
        format!(
            "resolve KanariSystem package path from {}",
            package.display()
        )
    })?;
    Ok(move_manifest_local_path(&canonical))
}

fn move_manifest_local_path(path: &Path) -> String {
    let normalized = path.display().to_string().replace('\\', "/");
    normalized
        .strip_prefix("//?/")
        .unwrap_or(&normalized)
        .to_string()
}
