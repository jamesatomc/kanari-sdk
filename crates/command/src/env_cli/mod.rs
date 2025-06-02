// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use colored::Colorize;
use common::{load_kanari_config, save_kanari_config};
use serde_yaml::{Mapping, Value};
use std::io::{self};

pub fn handle_env_command() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 {
        display_env_help();
        return Ok(());
    }

    match args[2].as_str() {
        "list" => list_envs()?,
        "switch" if args.len() >= 4 => switch_env(&args[3])?,
        "add" if args.len() >= 4 => add_env(
            &args[3],
            args.get(4)
                .map(|s| s.as_str())
                .unwrap_or("http://127.0.0.1:30030"),
        )?,
        "remove" if args.len() >= 4 => remove_env(&args[3])?,
        _ => display_env_help(),
    }

    Ok(())
}

fn display_env_help() {
    println!("\n{}", "KARI ENVIRONMENT MANAGEMENT".bright_yellow().bold());
    println!("{}", "Available commands:".bright_white());
    println!(
        "  {:<20} {}",
        "list".green(),
        "List all available environments"
    );
    println!(
        "  {:<20} {}",
        "switch <name>".green(),
        "Switch to the specified environment"
    );
    println!(
        "  {:<20} {}",
        "add <name> [url]".green(),
        "Add a new environment with optional URL"
    );
    println!(
        "  {:<20} {}",
        "remove <name>".green(),
        "Remove an environment configuration"
    );
    println!("\n{}", "Examples:".bright_yellow());
    println!("  kari env list");
    println!("  kari env switch dev");
    println!("  kari env add local_test http://127.0.0.1:30035");
    println!("  kari env remove local_test");
}

fn list_envs() -> io::Result<()> {
    let kanari_config = load_kanari_config()?;

    let active_env = kanari_config
        .get("active_env")
        .and_then(|v| v.as_str())
        .unwrap_or("local");

    println!("\n{}", "AVAILABLE ENVIRONMENTS".bright_yellow().bold());
    println!(
        "{:<15} {:<40} {}",
        "NAME".bright_white(),
        "RPC URL".bright_white(),
        "STATUS".bright_white()
    );

    if let Some(envs) = kanari_config.get("envs").and_then(|v| v.as_sequence()) {
        for env in envs {
            let alias = env
                .get("alias")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let rpc = env
                .get("rpc")
                .and_then(|v| v.as_str())
                .unwrap_or("not configured");

            if alias == active_env {
                println!(
                    "{:<15} {:<40} {}",
                    alias.green(),
                    rpc,
                    "ACTIVE".bright_green()
                );
            } else {
                println!("{:<15} {:<40}", alias, rpc);
            }
        }
    } else {
        println!("No networks configured. Use 'kari network add' to add one.");
    }

    println!("");
    Ok(())
}

fn switch_env(name: &str) -> io::Result<()> {
    let mut kanari_config = load_kanari_config()?;

    // Verify the environment exists
    let mut env_exists = false;
    if let Some(envs) = kanari_config.get("envs").and_then(|v| v.as_sequence()) {
        for env in envs {
            if let Some(alias) = env.get("alias").and_then(|v| v.as_str()) {
                if alias == name {
                    env_exists = true;
                    break;
                }
            }
        }
    }

    if !env_exists {
        println!("{}", format!("Environment '{}' not found.", name).red());
        return Ok(());
    }

    // Update active environment
    if let Some(mapping) = kanari_config.as_mapping_mut() {
        mapping.insert(
            Value::String("active_env".to_string()),
            Value::String(name.to_string()),
        );
    } else {
        println!("Failed to update configuration.");
        return Ok(());
    }

    // Save updated config
    save_kanari_config(&kanari_config)?;

    println!("{}", format!("Switched to network: {}", name).green());
    println!("Use 'kari start' to connect to this network.");

    Ok(())
}

fn add_env(name: &str, rpc_url: &str) -> io::Result<()> {
    let mut kanari_config = load_kanari_config()?;

    // Check if environment already exists
    let mut env_exists = false;
    if let Some(envs) = kanari_config.get("envs").and_then(|v| v.as_sequence()) {
        for env in envs {
            if let Some(alias) = env.get("alias").and_then(|v| v.as_str()) {
                if alias == name {
                    env_exists = true;
                    break;
                }
            }
        }
    }

    if env_exists {
        println!(
            "{}",
            format!(
                "Environment '{}' already exists. Use a different name.",
                name
            )
            .red()
        );
        return Ok(());
    }

    // Create new environment entry
    let mut new_env = Mapping::new();
    new_env.insert(
        Value::String("alias".to_string()),
        Value::String(name.to_string()),
    );
    new_env.insert(
        Value::String("rpc".to_string()),
        Value::String(rpc_url.to_string()),
    );
    new_env.insert(Value::String("ws".to_string()), Value::Null);

    // Add to environments list
    if let Some(mapping) = kanari_config.as_mapping_mut() {
        if let Some(envs) = mapping.get_mut("envs").and_then(|v| v.as_sequence_mut()) {
            envs.push(Value::Mapping(new_env));
        } else {
            // No environments yet, create the list
            mapping.insert(
                Value::String("envs".to_string()),
                Value::Sequence(vec![Value::Mapping(new_env)]),
            );
        }
    } else {
        // Empty config, create everything
        let mut new_config = Mapping::new();
        new_config.insert(
            Value::String("envs".to_string()),
            Value::Sequence(vec![Value::Mapping(new_env)]),
        );
        kanari_config = Value::Mapping(new_config);
    }

    // Save updated config
    save_kanari_config(&kanari_config)?;

    println!(
        "{}",
        format!("Added network: {} ({})", name, rpc_url).green()
    );
    println!("To use this network, run: kari network switch {}", name);

    Ok(())
}

fn remove_env(name: &str) -> io::Result<()> {
    // Don't allow removing primary environments
    if name == "local" || name == "dev" || name == "test" || name == "main" {
        println!(
            "{}",
            format!("Cannot remove built-in environment '{}'.", name).red()
        );
        return Ok(());
    }

    let mut kanari_config = load_kanari_config()?;

    let active_env = kanari_config
        .get("active_env")
        .and_then(|v| v.as_str())
        .unwrap_or("local");

    // Check if trying to remove active environment
    if active_env == name {
        println!("{}", "Cannot remove currently active environment.".red());
        println!("Switch to another environment first with: kari env switch <name>");
        return Ok(());
    }

    // Remove the environment
    let mut env_found = false;
    if let Some(mapping) = kanari_config.as_mapping_mut() {
        if let Some(envs) = mapping.get_mut("envs").and_then(|v| v.as_sequence_mut()) {
            let mut index_to_remove = None;
            for (i, env) in envs.iter().enumerate() {
                if let Some(alias) = env.get("alias").and_then(|v| v.as_str()) {
                    if alias == name {
                        index_to_remove = Some(i);
                        env_found = true;
                        break;
                    }
                }
            }

            if let Some(index) = index_to_remove {
                envs.remove(index);
            }
        }
    }

    if !env_found {
        println!("{}", format!("Environment '{}' not found.", name).red());
        return Ok(());
    }

    // Save updated config
    save_kanari_config(&kanari_config)?;

    println!("{}", format!("Removed environment: {}", name).green());

    Ok(())
}
