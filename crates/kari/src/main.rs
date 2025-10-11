use colored::Colorize;
use command::env_cli::handle_env_command;
use command::keytool_cli::handle_keytool_command;

use command::server_cli::handle_server_command;

use std::process::exit;

use command::public_cli::handle_public_command;
use tracing_subscriber::fmt::time::UtcTime;
use tracing_subscriber::prelude::*;

use std::process::Command;

static VERSION: &str = env!("CARGO_PKG_VERSION");

struct CommandInfo {
    name: &'static str,
    alias: Option<&'static str>,
    description: &'static str,
}

const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "server",
        alias: None,
        description: "Start the Kari server",
    },
    CommandInfo {
        name: "public",
        alias: None,
        description: "Manage Web3 public files and IPFS storage",
    },
    CommandInfo {
        name: "move",
        alias: None,
        description: "Execute and manage Move VM smart contracts",
    },
    CommandInfo {
        name: "keytool",
        alias: None,
        description: "Manage Kari accounts and cryptographic keys",
    },
    CommandInfo {
        name: "env",
        alias: Some("--e"),
        description: "Manage network configurations",
    },
    CommandInfo {
        name: "version",
        alias: Some("--V"),
        description: "Display CLI version information",
    },
    CommandInfo {
        name: "help",
        alias: Some("--h"),
        description: "Display this help message",
    },
    CommandInfo {
        name: "info",
        alias: Some("--i"),
        description: "Display information about the Kari node",
    },
];

fn display_help(show_error: bool) {
    if show_error {
        println!("\n{}", "ERROR: Invalid command".red().bold());
    }

    // Usage
    println!("{}", "USAGE:".bright_yellow().bold());
    println!("kari <command> [options]\n");

    // Commands
    println!("{}", "COMMANDS:".bright_yellow().bold());

    let max_name_len = COMMANDS
        .iter()
        .map(|cmd| cmd.name.len() + cmd.alias.map_or(0, |a| a.len() + 2))
        .max()
        .unwrap_or(0);

    for cmd in COMMANDS {
        let name = match cmd.alias {
            Some(alias) => format!("{}, {}", cmd.name, alias),
            None => cmd.name.to_string(),
        };

        println!(
            "  {}{}  {}",
            name.green().bold(),
            " ".repeat(max_name_len - name.len() + 2),
            cmd.description.bright_white()
        );
    }
    println!();

    exit(1);
}

#[tokio::main]
async fn main() {
    // Initialize global tracing/log formatting to match desired output
    // Format: [RFC3339_TIMESTAMP LEVEL target] message
    let fmt_layer = tracing_subscriber::fmt()
        .with_timer(UtcTime::rfc_3339())
        .with_target(true)
        .with_ansi(true)
        .finish();

    // Initialize the subscriber globally. try_init returns a Result which we ignore here.
    let _ = fmt_layer.try_init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() <= 1 {
        display_help(false);
    }

    match args.get(1).map(|s| s.as_str()) {
        Some("server") => {
            if let Some(message) = handle_server_command().await {
                println!("{}", message.bright_green().bold());
            }
        }
        Some("public") => {
            let _ = handle_public_command();
        }
        Some("keytool") => {
            let _ = handle_keytool_command();
        }
        Some("env") => {
            let _ = handle_env_command();
        }
        Some("version") | Some("--V") => println!("CLI Version: {}", VERSION),
        Some("help") | Some("--h") => display_help(false),
        Some("info") | Some("--i") => {
            println!("{}", "Opening Kari documentation...".bright_yellow());
            #[cfg(target_os = "windows")]
            Command::new("cmd")
                .args(["/C", "start", "https://docs.kanari.site"])
                .spawn()
                .expect("Failed to open documentation");

            #[cfg(target_os = "linux")]
            Command::new("xdg-open")
                .arg("https://docs.kanari.site")
                .spawn()
                .expect("Failed to open documentation");

            #[cfg(target_os = "macos")]
            Command::new("open")
                .arg("https://docs.kanari.site")
                .spawn()
                .expect("Failed to open documentation");
        }
        _ => display_help(true),
    }
}
