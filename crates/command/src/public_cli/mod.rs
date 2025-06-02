// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;
use std::process::exit;

use colored::Colorize;
use mona_storage::file_storage::FileStorage;

struct CommandInfo {
    name: &'static str,
    description: &'static str,
}

const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "upload",
        description: "Upload a file to storage",
    },
    CommandInfo {
        name: "get <id>",
        description: "Get a file from storage by ID Image/File",
    },
];

fn display_help(show_error: bool) {
    if show_error {
        println!("\n{}", "ERROR: Invalid command".red().bold());
    }

    // Usage section
    println!("{}", "USAGE:".bright_yellow().bold());
    println!("kari public <command> [options]\n");

    // Commands section
    println!("{}", "COMMANDS:".bright_yellow().bold());

    let max_name_len = COMMANDS.iter().map(|cmd| cmd.name.len()).max().unwrap_or(0);

    for cmd in COMMANDS {
        println!(
            "  {}{}  {}",
            cmd.name.green().bold(),
            " ".repeat(max_name_len - cmd.name.len() + 2),
            cmd.description.bright_white()
        );
    }
    println!();

    exit(1);
}

// Handle public commands
pub fn handle_public_command() -> Option<String> {
    // Collect command line arguments
    let args: Vec<String> = std::env::args().collect();

    // Check if any arguments were provided
    if args.len() > 2 {
        // Collect command line arguments
        let command = &args[2];
        // Use string comparison in the match statement
        match command.as_str() {
            "upload" => {
                if args.len() != 4 {
                    // Changed from 3 to 4 since command is at index 2
                    return Some("Usage: kari public upload <file_path>".to_string());
                }

                // Initialize storage first
                if let Err(e) = FileStorage::init_storage() {
                    return Some(format!("Failed to initialize storage: {}", e));
                }

                // Get file path from correct argument index (3 instead of 2)
                let file_path = Path::new(&args[3]);

                if !file_path.exists() {
                    return Some(format!("Error: File '{}' not found", file_path.display()));
                }

                let filename = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unnamed")
                    .to_string();

                match FileStorage::upload(file_path, filename) {
                    Ok(storage) => {
                        println!(
                            "\n{}\n\nFile ID: {}\nLocation: {}\nSize: {} bytes\nType: {}\n\n{}\n    kari public get {}\n",
                            "✓ File uploaded successfully!".green().bold(),
                            storage.id.to_string().yellow().bold(),
                            storage.path.display(),
                            storage.metadata.size,
                            storage.metadata.content_type,
                            "To download this file, use:".bright_blue(),
                            storage.id
                        );
                        None
                    }
                    Err(e) => {
                        println!("Debug: Upload error = {:?}", e);
                        Some(format!("{}: {}", "Upload failed".red().bold(), e))
                    }
                }
            }

            "get" => {
                if args.len() != 4 {
                    return Some("Usage: kari public get <file_id>".to_string());
                }

                let file_id = &args[3];

                if let Err(e) = FileStorage::init_storage() {
                    return Some(format!("Failed to initialize storage: {}", e));
                }

                match FileStorage::get_by_id(file_id) {
                    Ok(storage) => {
                        // Get current directory for saving the file
                        let current_dir =
                            std::env::current_dir().expect("Failed to get current directory");
                        let target_path = current_dir.join(&storage.metadata.filename);

                        // Copy file to current directory
                        match std::fs::copy(&storage.path, &target_path) {
                            Ok(_) => Some(format!(
                                "File downloaded successfully!\nID: {}\nSaved as: {}\nSize: {} bytes\nType: {}",
                                storage.id,
                                target_path.display(),
                                storage.metadata.size,
                                storage.metadata.content_type
                            )),
                            Err(e) => Some(format!("Failed to save file: {}", e)),
                        }
                    }
                    Err(e) => Some(format!("Failed to get file: {}", e)),
                }
            }

            _ => {
                display_help(true);
                None
            }
        }
    } else {
        display_help(false);
        return None;
    }
}
