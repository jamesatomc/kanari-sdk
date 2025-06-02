// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use std::process::exit;

use colored::Colorize;

pub mod generate_certs;
pub mod init_server;
pub mod start_server;

struct CommandInfo {
    name: &'static str,
    description: &'static str,
}

const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "start",
        description: "Start the Kari server",
    },
    CommandInfo {
        name: "init",
        description: "Initialize the Kari server",
    },
    CommandInfo {
        name: "generate-certs",
        description: "Generate SSL certificates",
    },
];

fn display_help(show_error: bool) {
    if show_error {
        println!("\n{}", "ERROR: Invalid command".red().bold());
    }

    // Usage section
    println!("{}", "USAGE:".bright_yellow().bold());
    println!("kari server <command> [options]\n");

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

// Handle server commands
pub async fn handle_server_command() -> Option<String> {
    // Collect command line arguments
    let args: Vec<String> = std::env::args().collect();

    // Check if any arguments were provided
    if args.len() <= 2 {
        // No subcommand provided
        println!("{}", "Server commands:".bright_green().bold());
        // Usage section
        println!("{}", "USAGE:".bright_yellow().bold());
        println!("kari server <command> [options]\n");

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

        // Instead of exiting, just return None to go back to main
        return None;
    }

    // Collect command line arguments
    let command = &args[2];
    // Use string comparison in the match statement
    match command.as_str() {
        "start" => {
            // Extract peer and port arguments
            let mut peers = Vec::new();
            let mut port = None;
            let mut localhost_only = false;
            let mut selected_wallet = None;
            let mut use_tls = true; // Default to TLS enabled

            let mut i = 3; // Start at index 3 to skip program name, "server", and "start" command
            while i < args.len() {
                match args.get(i).map(|s| s.as_str()) {
                    Some("--peer") => {
                        if let Some(peer_addr) = args.get(i + 1) {
                            peers.push(peer_addr.to_string());
                            i += 2;
                        } else {
                            eprintln!(
                                "{}",
                                "Error: --peer requires an address argument".red().bold()
                            );
                            exit(1);
                        }
                    }
                    Some("--port") => {
                        if let Some(port_str) = args.get(i + 1) {
                            match port_str.parse::<u16>() {
                                Ok(p) => {
                                    port = Some(p);
                                    i += 2;
                                }
                                Err(_) => {
                                    eprintln!("{}", "Error: Invalid port number".red().bold());
                                    exit(1);
                                }
                            }
                        } else {
                            eprintln!(
                                "{}",
                                "Error: --port requires a number argument".red().bold()
                            );
                            exit(1);
                        }
                    }
                    Some("--localhost") => {
                        if let Some(value) = args.get(i + 1) {
                            // Better handling of boolean values with typo tolerance
                            localhost_only = match value.to_lowercase().as_str() {
                                "true" | "t" | "yes" | "y" | "1" | "ture" => true,
                                "false" | "f" | "no" | "n" | "0" => false,
                                _ => {
                                    println!("{}", format!("Warning: Invalid value for --localhost: '{}', defaulting to false", value).yellow().bold());
                                    false
                                }
                            };
                            i += 2;
                        } else {
                            // If no value provided, assume true (flag presence implies true)
                            localhost_only = true;
                            i += 1;
                        }
                    }
                    Some("--no-tls") => {
                        use_tls = false;
                        i += 1;
                    }
                    Some("--wallet") => {
                        if let Some(wallet_addr) = args.get(i + 1) {
                            selected_wallet = Some(wallet_addr.to_string());
                            i += 2;
                        } else {
                            eprintln!(
                                "{}",
                                "Error: --wallet requires an address argument".red().bold()
                            );
                            exit(1);
                        }
                    }
                    Some(unknown_arg) => {
                        eprintln!(
                            "{}",
                            format!("Unknown argument: {}", unknown_arg).red().bold()
                        );
                        display_help(true);
                        exit(1);
                    }
                    None => {
                        i += 1; // Skip any potential null arguments
                    }
                }
            }

            println!("{}", "Preparing to start server...".bright_green());
            if !peers.is_empty() {
                println!("Connecting to peers: {}", peers.join(", ").bright_white());
            }
            if let Some(p) = port {
                println!("Using custom port: {}", p.to_string().bright_white());
            }

            if let Err(err) =
                start_server::start_server(peers, port, localhost_only, selected_wallet, use_tls)
                    .await
            {
                eprintln!("{}", format!("Server startup failed: {}", err).red().bold());
                return None;
            }
            Some("Server started successfully".to_string())
        }

        "generate-certs" => match generate_certs::generate_ssl_certificates() {
            Ok(_) => Some("SSL certificates generated successfully".to_string()),
            Err(e) => {
                eprintln!(
                    "{}",
                    format!("Error generating certificates: {}", e).red().bold()
                );
                None
            }
        },

        "init" => match init_server::init_server_config() {
            Ok(_) => Some(
                "Kari server initialized successfully. Default kanari.yaml created.".to_string(),
            ),
            Err(e) => {
                eprintln!(
                    "{}",
                    format!("Error initializing server: {}", e).red().bold()
                );
                None
            }
        },

        _ => {
            display_help(true);
            None
        }
    }
}
