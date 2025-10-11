// Copyright (c) RoochNetwork
// SPDX-License-Identifier: Apache-2.0

use std::fs;

use crate::{Stdlib, StdlibBuildConfig, path_in_crate, release_dir, stdlib_version::StdlibVersion};
use anyhow::Result;
use move_package::BuildConfig;
use once_cell::sync::Lazy;

static STDLIB_BUILD_CONFIGS: Lazy<Vec<StdlibBuildConfig>> = Lazy::new(|| {
    let kanari_library_path = path_in_crate("../kanari-library")
        .canonicalize()
        .expect("canonicalize path failed");

    let latest_version_dir = release_dir().join(StdlibVersion::Latest.to_string());
    fs::create_dir_all(&latest_version_dir).expect("create latest failed");
    vec![StdlibBuildConfig {
        path: kanari_library_path.clone(),
        error_prefix: "Error".to_string(),
        error_code_map_output_file: latest_version_dir
            .join("kanari_library_error_description.errmap"),
        document_template: kanari_library_path.join("doc_template/README.md"),
        document_output_directory: kanari_library_path.join("doc"),
        build_config: BuildConfig::default(),
        stable: true,
    }]
});

pub fn build_stdlib(stable: bool) -> Result<Stdlib> {
    let configs = if stable {
        STDLIB_BUILD_CONFIGS
            .iter()
            .filter(|config| config.stable)
            .cloned()
            .collect()
    } else {
        STDLIB_BUILD_CONFIGS.clone()
    };
    Stdlib::build(configs)
}
