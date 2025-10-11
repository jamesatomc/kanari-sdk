// Copyright (c) RoochNetwork
// SPDX-License-Identifier: Apache-2.0

use std::env::current_dir;

fn main() {
    if std::env::var("SKIP_STDLIB_BUILD").is_err() {
        unsafe {
            std::env::set_var("RUST_LOG", "WARN");
        }
        let _ = tracing_subscriber::fmt::try_init();
        let current_dir = current_dir().expect("Should be able to get current dir");
        // Get the project root directory
        let mut root_dir = current_dir;
        root_dir.pop();
        root_dir.pop();

        println!(
            "cargo:rerun-if-changed={}",
            root_dir
                .join("frameworks/kanari-library")
                .join("Move.toml")
                .display()
        );

        println!(
            "cargo:rerun-if-changed={}",
            root_dir
                .join("frameworks/kanari-library")
                .join("sources")
                .display()
        );

        match framework_builder::releaser::release_latest() {
            Ok(msgs) => {
                for msg in msgs {
                    println!("cargo::warning=\"{}\"", msg);
                }
            }
            Err(e) => {
                println!(
                    "cargo::warning=\"Failed to release latest framework: {:?}\"",
                    e
                );
            }
        }
    }
}
