// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use colored::Colorize;
use common::{get_kanari_config_path, init_default_config};
use std::io;

pub fn init_server_config() -> io::Result<()> {
    let config_path = get_kanari_config_path();

    if config_path.exists() {
        println!(
            "{}",
            "kanari.yaml already exists. Initialization skipped.".yellow()
        );
        return Ok(());
    }

    // Use the centralized initialization function
    init_default_config()?;

    println!(
        "{} {}",
        "Default kanari.yaml created at:".green(),
        config_path.display().to_string().bright_white()
    );
    Ok(())
}
