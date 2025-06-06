use colored::Colorize;
use mona_crypto::{
    list_wallet_files,
    load_wallet, save_wallet, set_selected_wallet,
    save_mnemonic, load_mnemonic, check_mnemonic_exists, remove_mnemonic, get_mnemonic_addresses,
    save_session_key, load_session_key, remove_session_key, clear_session_keys,
};
use std::io::{self, Write};

use mona_types::storage::{get_balance, load_blockchain_with_retry};
use rpassword::read_password;
use std::process::exit;
use serde_json::json;
use std::process::Command;
// Add the required import for CurveType and key-related functions
use key::keys::{import_from_private_key, import_from_seed_phrase, generate_karix_address};
use key::keys::CurveType;
use common::{load_config, load_kanari_config};

struct CommandInfo {
    name: &'static str,
    description: &'static str,
}

const COMMANDS: &[CommandInfo] = &[
    CommandInfo {
        name: "generate",
        description: "Generate new address",
    },
    CommandInfo {
        name: "balance",
        description: "Check balance",
    },
    CommandInfo {
        name: "transfer",
        description: "Transfer coins to another address",
    },
    CommandInfo {
        name: "select",
        description: "Select wallet",
    },
    CommandInfo {
        name: "wallet",
        description: "Load existing wallet",
    },
    CommandInfo {
        name: "list",
        description: "List wallet files",
    },
    CommandInfo {
        name: "import",
        description: "Import from seed phrase",
    },
    CommandInfo {
        name: "privatekey",
        description: "Import from private key",
    },
    CommandInfo {
        name: "mnemonic",
        description: "Manage BIP39 mnemonic phrases",
    },
    CommandInfo {
        name: "session",
        description: "Manage session keys",
    },
];

fn display_help(show_error: bool) {
    if show_error {
        println!("\n{}", "ERROR: Invalid command".red().bold());
    }

    // Usage section
    println!("{}", "USAGE:".bright_yellow().bold());
    println!("  kari keytool <command> [options]\n");

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

pub fn handle_keytool_command() -> Option<String> {
    // Collect command line arguments
    let args: Vec<String> = std::env::args().collect();

    // Check if any arguments were provided
    if args.len() > 2 {
        // Collect command line arguments
        let command = &args[2];
        // Use string comparison in the match statement
        match command.as_str() {
            "generate" => {
                println!("Enter mnemonic length (12 or 24):");
                let mut mnemonic_length_str = String::new();
                match io::stdin().read_line(&mut mnemonic_length_str) {
                    Ok(_) => {
                        match mnemonic_length_str.trim().parse::<usize>() {
                            Ok(mnemonic_length) => {
                                if mnemonic_length != 12 && mnemonic_length != 24 {
                                    println!(
                                        "{}",
                                        "Invalid mnemonic length. Must be 12 or 24.".red()
                                    );
                                    return None;
                                }
                                
                                // Add curve type selection with Ed25519 option
                                println!("Select curve type:");
                                println!("1. K-256 (secp256k1)");
                                println!("2. P-256 (secp256r1)");
                                println!("3. Ed25519");
                                
                                let mut curve_choice = String::new();
                                match io::stdin().read_line(&mut curve_choice) {
                                    Ok(_) => {
                                        let curve_type = match curve_choice.trim() {
                                            "1" => CurveType::K256,
                                            "2" => CurveType::P256,
                                            "3" => CurveType::Ed25519,
                                            _ => {
                                                println!("{}", "Invalid choice, using secp256k1 as default.".yellow());
                                                CurveType::K256
                                            }
                                        };
                                        
                                        let (private_key, public_address, seed_phrase) =
                                            generate_karix_address(mnemonic_length, curve_type);
                                            
                                        println!("New address generated:");
                                        println!("Private Key: {}", private_key.green());
                                        println!("Public Address: {}", public_address.green());
                                        println!("Seed Phrase: {}", seed_phrase.green());
                                        println!("Curve Type: {}", format!("{:?}", curve_type).green());
            
                                        let password = prompt_password(true);
                                        // Convert public_address to Address type
                                        match public_address.parse() {
                                            Ok(address) => {
                                                match save_wallet(
                                                    &address,
                                                    &private_key,
                                                    &seed_phrase,
                                                    &password,
                                                    curve_type,
                                                ) {
                                                    Ok(_) => {
                                                        println!("Wallet saved successfully!");
                                                        return Some(public_address);
                                                    }
                                                    Err(e) => {
                                                        println!(
                                                            "{}",
                                                            format!("Failed to save wallet: {}", e).red()
                                                        );
                                                        return None;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                println!(
                                                    "{}",
                                                    format!("Failed to parse public address: {}", e).red()
                                                );
                                                return None;
                                            }
                                        }
                                    },
                                    Err(e) => {
                                        println!("{}", format!("Failed to read input: {}", e).red());
                                        return None;
                                    }
                                }
                            }
                            Err(_) => {
                                println!("{}", "Invalid input - please enter 12 or 24".red());
                                return None;
                            }
                        }
                    }
                    Err(e) => {
                        println!("{}", format!("Failed to read input: {}", e).red());
                        return None;
                    }
                }
            }

            "balance" => {
                println!("Enter public address:");
                let mut public_address = String::new();
                match io::stdin().read_line(&mut public_address) {
                    Ok(_) => {
                        let public_address = public_address.trim().to_string();
                        
                        // Use a more reliable approach
                        match load_blockchain_with_retry() {
                            Ok(_) => match get_balance(&public_address) {
                                Ok(balance) => {
                                    // Format the balance in a human-readable way
                                    let formatted_balance = format_balance(balance);
                                    println!(
                                        "Balance for {}: {} Kari",
                                        public_address.green(),
                                        formatted_balance.green()
                                    );
                                }
                                Err(e) => {
                                    println!("{}: {}", "Error getting balance".red(), e);
                                }
                            },
                            Err(e) => {
                                println!("{}: {}", "Error loading blockchain".red(), e);
                            }
                        }
                    }
                    Err(e) => {
                        println!("{}: {}", "Error reading input".red(), e);
                    }
                }
                return None;
            }

            "transfer" => {
                // Load blockchain first
                match load_blockchain_with_retry() {
                    Ok(_) => {
                        // Get RPC endpoint from configuration
                        let rpc_endpoint = get_rpc_endpoint();
                        
                        // Get sender address (current wallet)
                        let wallets = match list_wallet_files() {
                            Ok(w) => w,
                            Err(e) => {
                                println!("{}", format!("Error listing wallets: {}", e).red());
                                return None;
                            }
                        };
                        
                        // Find selected wallet
                        let selected_wallet = wallets.iter()
                            .find(|(_, is_selected)| *is_selected)
                            .map(|(name, _)| name.trim_end_matches(".enc").to_string());
                        
                        let sender_address = match selected_wallet {
                            Some(addr) => addr,
                            None => {
                                // Try to select wallet interactively if none is selected
                                println!("{}", "No wallet selected. Please select a wallet first.".yellow());
                                
                                if wallets.is_empty() {
                                    println!("{}", "No wallets found!".red());
                                    return None;
                                }
                                
                                println!("\nAvailable wallets:");
                                for (i, (wallet, _)) in wallets.iter().enumerate() {
                                    let wallet_name = wallet.trim_end_matches(".enc");
                                    println!("{}. {}", i + 1, wallet_name);
                                }
                                
                                println!("\nEnter wallet number to use (or press Enter to cancel):");
                                let mut input = String::new();
                                match io::stdin().read_line(&mut input) {
                                    Ok(_) => {
                                        if input.trim().is_empty() {
                                            println!("Transfer cancelled.");
                                            return None;
                                        }
                                        
                                        match input.trim().parse::<usize>() {
                                            Ok(index) if index > 0 && index <= wallets.len() => {
                                                let selected = wallets[index - 1].0.trim_end_matches(".enc");
                                                match set_selected_wallet(selected) {
                                                    Ok(_) => {
                                                        println!("Using wallet: {}", selected.green());
                                                        selected.to_string()
                                                    }
                                                    Err(e) => {
                                                        println!("{}", format!("Error setting wallet: {}", e).red());
                                                        return None;
                                                    }
                                                }
                                            }
                                            _ => {
                                                println!("{}", format!("Invalid selection. Please enter a number between 1 and {}", wallets.len()).red());
                                                return None;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        println!("{}", format!("Error reading input: {}", e).red());
                                        return None;
                                    }
                                }
                            }
                        };
                        
                        // Check sender balance
                        let balance = match get_balance(&sender_address) {
                            Ok(b) => b,
                            Err(e) => {
                                println!("{}", format!("Error checking balance: {}", e).red());
                                return None;
                            }
                        };
                        
                        let formatted_balance = format_balance(balance);
                        println!("Your balance: {} KARI", formatted_balance.green());
                        
                        // Get recipient address
                        println!("Enter recipient address:");
                        let mut recipient = String::new();
                        match io::stdin().read_line(&mut recipient) {
                            Ok(_) => {},
                            Err(e) => {
                                println!("{}", format!("Error reading input: {}", e).red());
                                return None;
                            }
                        };
                        let recipient = recipient.trim();
                        
                        // Get amount to send
                        println!("Enter amount to send (in KARI):");
                        let mut amount_str = String::new();
                        match io::stdin().read_line(&mut amount_str) {
                            Ok(_) => {},
                            Err(e) => {
                                println!("{}", format!("Error reading input: {}", e).red());
                                return None;
                            }
                        };
                        
                        // Parse amount with validation
                        let amount_kari = match amount_str.trim().parse::<f64>() {
                            Ok(a) if a > 0.0 => a,
                            Ok(_) => {
                                println!("{}", "Amount must be greater than 0".red());
                                return None;
                            }
                            Err(e) => {
                                println!("{}", format!("Invalid amount: {}", e).red());
                                return None;
                            }
                        };
                        
                        // Confirm the transfer
                        println!("\nTransaction details:");
                        println!("  From: {}", sender_address.green());
                        println!("  To:   {}", recipient.green());
                        println!("  Amount: {} KARI", amount_str.trim());
                        
                        println!("\nConfirm transfer? (y/n)");
                        let mut confirm = String::new();
                        match io::stdin().read_line(&mut confirm) {
                            Ok(_) => {},
                            Err(e) => {
                                println!("{}", format!("Error reading input: {}", e).red());
                                return None;
                            }
                        };
                        
                        if !confirm.trim().eq_ignore_ascii_case("y") {
                            println!("Transfer cancelled.");
                            return None;
                        }
                        
                        // Get password
                        println!("Enter wallet password:");
                        let password = prompt_password(false);
                        
                        println!("Sending transaction...");
                        
                        // Create the JSON payload
                        let json_payload = json!({
                            "jsonrpc": "2.0",
                            "method": "transfer",
                            "params": {
                                "from": sender_address,
                                "to": recipient,
                                "amount": amount_kari,
                                "password": password
                            },
                            "id": 1
                        }).to_string();
                        
                        // Use curl if available, otherwise try to implement a simple TCP client
                        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
                        {
                            // Determine the curl command based on the platform
                            let curl_cmd = if cfg!(target_os = "windows") {
                                "curl.exe"
                            } else {
                                "curl"
                            };
                            
                            // Execute curl command to send the request with dynamic endpoint
                            let output = Command::new(curl_cmd)
                                .arg("-s")
                                .arg("-X")
                                .arg("POST")
                                .arg(&rpc_endpoint)  // Use dynamic endpoint instead of hardcoded
                                .arg("-H")
                                .arg("Content-Type: application/json")
                                .arg("-d")
                                .arg(&json_payload)
                                .output();
                                
                            match output {
                                Ok(output) => {
                                    if output.status.success() {
                                        // Parse the response JSON
                                        match serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                                            Ok(json_response) => {
                                                // Check for errors in the response
                                                if let Some(error) = json_response.get("error") {
                                                    println!("{}", format!("Transfer failed: {}", error).red());
                                                    return None;
                                                }
                                                
                                                // Success!
                                                println!("{}", "Transfer initiated successfully!".green());
                                                println!("Transaction will be included in the next block.");
                                                
                                                // Show transaction ID if available
                                                if let Some(result) = json_response.get("result") {
                                                    if let Some(tx_id) = result.get("transaction_id") {
                                                        println!("Transaction ID: {}", tx_id.as_str().unwrap_or("unknown").green());
                                                    }
                                                }
                                                
                                                return Some(sender_address);
                                            },
                                            Err(e) => {
                                                println!("{}", format!("Failed to parse API response: {}", e).red());
                                                println!("Raw response: {}", String::from_utf8_lossy(&output.stdout));
                                                return None;
                                            }
                                        }
                                    } else {
                                        println!("{}", format!("HTTP request failed to {}", rpc_endpoint).red());
                                        println!("Error: {}", String::from_utf8_lossy(&output.stderr));
                                        return None;
                                    }
                                },
                                Err(e) => {
                                    println!("{}", format!("Failed to execute HTTP request to {}: {}", rpc_endpoint, e).red());
                                    println!("Make sure curl is installed and the node is running at {}", rpc_endpoint);
                                    return None;
                                }
                            }
                        }
                        
                        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
                        {
                            println!("{}", "Transfer not supported on this platform".red());
                            return None;
                        }
                    },
                    Err(e) => {
                        println!("{}", format!("Failed to load blockchain: {}", e).red());
                        return None;
                    }
                }
            },

            "select" => match list_wallet_files() {
                Ok(wallets) => {
                    if wallets.is_empty() {
                        println!("{}", "No wallets found!".red());
                        return None;
                    }

                    println!("\n{}", "Available wallets:".bright_yellow().bold());
                    println!("{}", "─".repeat(60));
                    println!("{:<5} {:<42} {}", "No.", "Address", "Status");
                    println!("{}", "─".repeat(60));
                    
                    for (i, (wallet, is_selected)) in wallets.iter().enumerate() {
                        let wallet_name = wallet.trim_end_matches(".enc");
                        let display_name = if wallet_name.len() > 38 {
                            format!("{}...", &wallet_name[..35])
                        } else {
                            wallet_name.to_string()
                        };
                        
                        if *is_selected {
                            println!("{:<5} {:<42} {}", 
                                (i + 1).to_string().green().bold(),
                                display_name.green(),
                                "ACTIVE".green().bold()
                            );
                        } else {
                            println!("{:<5} {:<42}", (i + 1), display_name);
                        }
                    }
                    println!("{}", "─".repeat(60));

                    println!("\nEnter wallet number to select (or press Enter to cancel):");
                    let mut input = String::new();
                    match io::stdin().read_line(&mut input) {
                        Ok(_) => {
                            if input.trim().is_empty() {
                                return None;
                            }

                            match input.trim().parse::<usize>() {
                                Ok(index) if index > 0 && index <= wallets.len() => {
                                    let selected = wallets[index - 1].0.trim_end_matches(".enc");
                                    match set_selected_wallet(selected) {
                                        Ok(_) => {
                                            println!("{} {}", 
                                                "✓".green().bold(), 
                                                format!("Wallet selected: {}", selected).green()
                                            );
                                            Some(selected.to_string())
                                        }
                                        Err(e) => {
                                            println!(
                                                "{}",
                                                format!("Error setting wallet: {}", e).red()
                                            );
                                            None
                                        }
                                    }
                                }
                                _ => {
                                    println!("{}", format!("Invalid selection. Please enter a number between 1 and {}", wallets.len()).red());
                                    None
                                }
                            }
                        }
                        Err(e) => {
                            println!("{}", format!("Error reading input: {}", e).red());
                            None
                        }
                    }
                }
                Err(e) => {
                    println!("{}", format!("Error listing wallets: {}", e).red());
                    None
                }
            },

            "wallet" => {
                println!("Enter public address to load:");
                let mut public_address = String::new();
                match io::stdin().read_line(&mut public_address) {
                    Ok(_) => {
                        let public_address = public_address.trim().to_string();
                        let password = prompt_password(false);

                        match load_wallet(&public_address, &password) {
                            Ok(wallet_data) => {
                                println!("\n{}", "Wallet Information:".bright_yellow().bold());
                                println!("{}", "─".repeat(60));
                                println!("{:<15} {}", "Address:".bold(), wallet_data.address.to_string().green());
                                println!("{:<15} {}", "Curve Type:".bold(), format!("{:?}", wallet_data.curve_type).green());
                                
                                // Show full private key instead of truncated version
                                println!("{:<15} {}", "Private Key:".bold(), wallet_data.private_key.green());
                                
                                if !wallet_data.seed_phrase.is_empty() {
                                    // Show full seed phrase instead of just word count
                                    println!("{:<15} {}", "Seed Phrase:".bold(), wallet_data.seed_phrase.green());
                                } else {
                                    println!("{:<15} {}", "Seed Phrase:".bold(), "None".yellow());
                                }
                                println!("{}", "─".repeat(60));
                                
                                println!("\n{} {}", 
                                    "✓".green().bold(), 
                                    "Wallet loaded successfully".green()
                                );
                                return Some(public_address);
                            }
                            Err(e) => {
                                println!("{}", format!("Failed to load wallet: {}", e).red());
                                return None;
                            }
                        }
                    }
                    Err(e) => {
                        println!("{}", format!("Failed to read input: {}", e).red());
                        return None;
                    }
                }
            },

            "list" => {
                match list_wallet_files() {
                    Ok(wallets) => {
                        if wallets.is_empty() {
                            println!("{}", "\nNo wallets found!".yellow());
                            return None;
                        }
                        
                        // Store the length before consuming the collection
                        let wallet_count = wallets.len();
                        
                        println!("\n{}", "Available Wallets:".bright_yellow().bold());
                        println!("{}", "─".repeat(70));
                        
                        for (wallet_name, is_selected) in wallets {
                            let wallet_display = wallet_name.trim_end_matches(".enc");
                            if is_selected {
                                println!("{} {} {}", 
                                    "➤".green().bold(),
                                    wallet_display.green().bold(),
                                    "[ACTIVE]".green().bold()
                                );
                            } else {
                                println!("  {}", wallet_display);
                            }
                        }
                        println!("{}", "─".repeat(70));
                        println!("Total wallets: {}", wallet_count);
                    }
                    Err(e) => {
                        println!(
                            "{}Failed to list wallet files: {}",
                            "ERROR: ".red().bold(),
                            e
                        );
                    }
                }
                return None;
            }

            "import" => {
                println!("Enter seed phrase:");
                let mut phrase = String::new();
                io::stdin()
                    .read_line(&mut phrase)
                    .expect("Failed to read line");
            
                // Add curve type selection with Ed25519 option
                println!("Select curve type:");
                println!("1. K-256 (secp256k1)");
                println!("2. P-256 (secp256r1)");
                println!("3. Ed25519");
                
                let mut curve_choice = String::new();
                match io::stdin().read_line(&mut curve_choice) {
                    Ok(_) => {
                        let curve_type = match curve_choice.trim() {
                            "1" => CurveType::K256,
                            "2" => CurveType::P256,
                            "3" => CurveType::Ed25519,
                            _ => {
                                println!("{}", "Invalid choice, using Secp256k1 as default.".yellow());
                                CurveType::K256
                            }
                        };
                                
                        match import_from_seed_phrase(phrase.trim(), curve_type) {
                            Ok((private_key, _, public_address)) => {
                                let password = prompt_password(true);
            
                                match public_address.parse() {
                                    Ok(address) => {
                                        match save_wallet(&address, &private_key, phrase.trim(), &password, curve_type) {
                                            Ok(_) => match set_selected_wallet(&public_address) {
                                                Ok(_) => {
                                                    println!(
                                                        "Imported wallet with address: {}",
                                                        public_address
                                                    );
                                                    return Some(public_address);
                                                }
                                                Err(e) => {
                                                    println!(
                                                        "{}",
                                                        format!("Failed to set selected wallet: {}", e)
                                                            .red()
                                                    );
                                                    return None;
                                                }
                                            },
                                            Err(e) => {
                                                println!(
                                                    "{}",
                                                    format!("Failed to save wallet: {}", e).red()
                                                );
                                                return None;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        println!(
                                            "{}",
                                            format!("Failed to parse public address: {}", e).red()
                                        );
                                        return None;
                                    }
                                }
                            }
                            Err(e) => {
                                println!("{}", format!("Failed to import seed phrase: {}", e).red());
                                return None;
                            }
                        }
                    },
                    Err(e) => {
                        println!("{}", format!("Failed to read curve choice: {}", e).red());
                        return None;
                    }
                }
            }

            "privatekey" => {
                println!("Enter private key:");
                let mut private_key = String::new();
                io::stdin()
                    .read_line(&mut private_key)
                    .expect("Failed to read line");
            
                // Add curve type selection with Ed25519 option
                println!("Select curve type:");
                println!("1. K-256 (secp256k1)");
                println!("2. P-256 (secp256r1)");
                println!("3. Ed25519");
                
                let mut curve_choice = String::new();
                match io::stdin().read_line(&mut curve_choice) {
                    Ok(_) => {
                        let curve_type = match curve_choice.trim() {
                            "1" => CurveType::K256,
                            "2" => CurveType::P256,
                            "3" => CurveType::Ed25519,
                            _ => {
                                println!("{}", "Invalid choice, using Secp256k1 as default.".yellow());
                                CurveType::K256
                            }
                        };
            
                        match import_from_private_key(private_key.trim(), curve_type) {
                            Ok((private_key, _, public_address)) => {
                                let password = prompt_password(true);
                                // Convert public_address to Address type
                                match public_address.parse() {
                                    Ok(address) => {
                                        match save_wallet(&address, &private_key, "", &password, curve_type) {
                                            Ok(_) => match set_selected_wallet(&public_address) {
                                                Ok(_) => {
                                                    println!(
                                                        "Imported wallet with address: {}",
                                                        public_address
                                                    );
                                                    return Some(public_address);
                                                }
                                                Err(e) => {
                                                    println!(
                                                        "{}",
                                                        format!("Failed to set selected wallet: {}", e)
                                                            .red()
                                                    );
                                                    return None;
                                                }
                                            },
                                            Err(e) => {
                                                println!(
                                                    "{}",
                                                    format!("Failed to save wallet: {}", e).red()
                                                );
                                                return None;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        println!(
                                            "{}",
                                            format!("Failed to parse public address: {}", e).red()
                                        );
                                        return None;
                                    }
                                }
                            }
                            Err(e) => {
                                println!("{}", format!("Failed to import private key: {}", e).red());
                                return None;
                            }
                        }
                    },
                    Err(e) => {
                        println!("{}", format!("Failed to read curve choice: {}", e).red());
                        return None;
                    }
                }
            }

            "mnemonic" => {
                // Handle mnemonic subcommands
                if args.len() > 3 {
                    let subcommand = &args[3];
                    match subcommand.as_str() {
                        "save" => {
                            println!("Enter BIP39 mnemonic phrase:");
                            let mut mnemonic = String::new();
                            match io::stdin().read_line(&mut mnemonic) {
                                Ok(_) => {
                                    let mnemonic = mnemonic.trim();
                                    
                                    // Validate mnemonic (basic check for word count)
                                    let word_count = mnemonic.split_whitespace().count();
                                    if word_count != 12 && word_count != 24 {
                                        println!("{}", "Invalid mnemonic: must be 12 or 24 words".red());
                                        return None;
                                    }
                                    
                                    println!("Enter associated addresses (comma-separated, or press Enter for none):");
                                    let mut addresses_input = String::new();
                                    match io::stdin().read_line(&mut addresses_input) {
                                        Ok(_) => {
                                            let addresses: Vec<String> = if addresses_input.trim().is_empty() {
                                                Vec::new()
                                            } else {
                                                addresses_input
                                                    .trim()
                                                    .split(',')
                                                    .map(|s| s.trim().to_string())
                                                    .filter(|s| !s.is_empty())
                                                    .collect()
                                            };
                                            
                                            let password = prompt_password(true);
                                            
                                            match save_mnemonic(mnemonic, &password, addresses) {
                                                Ok(_) => {
                                                    println!("{}", "✓ Mnemonic saved successfully!".green().bold());
                                                    return Some("mnemonic_saved".to_string());
                                                }
                                                Err(e) => {
                                                    println!("{}", format!("Failed to save mnemonic: {}", e).red());
                                                    return None;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            println!("{}", format!("Error reading addresses: {}", e).red());
                                            return None;
                                        }
                                    }
                                }
                                Err(e) => {
                                    println!("{}", format!("Error reading mnemonic: {}", e).red());
                                    return None;
                                }
                            }
                        }
                        
                        "load" => {
                            if !check_mnemonic_exists() {
                                println!("{}", "No mnemonic found in keystore".yellow());
                                return None;
                            }
                            
                            let password = prompt_password(false);
                            
                            match load_mnemonic(&password) {
                                Ok(mnemonic) => {
                                    println!("\n{}", "Mnemonic Information:".bright_yellow().bold());
                                    println!("{}", "─".repeat(60));
                                    println!("{:<15} {}", "Mnemonic:".bold(), mnemonic.green());
                                    
                                    match get_mnemonic_addresses() {
                                        Ok(addresses) => {
                                            if !addresses.is_empty() {
                                                println!("{:<15}", "Addresses:".bold());
                                                for addr in addresses {
                                                    println!("                {}", addr.green());
                                                }
                                            } else {
                                                println!("{:<15} {}", "Addresses:".bold(), "None".yellow());
                                            }
                                        }
                                        Err(e) => {
                                            println!("{}", format!("Error getting addresses: {}", e).red());
                                        }
                                    }
                                    println!("{}", "─".repeat(60));
                                    
                                    return Some("mnemonic_loaded".to_string());
                                }
                                Err(e) => {
                                    println!("{}", format!("Failed to load mnemonic: {}", e).red());
                                    return None;
                                }
                            }
                        }
                        
                        "remove" => {
                            if !check_mnemonic_exists() {
                                println!("{}", "No mnemonic found in keystore".yellow());
                                return None;
                            }
                            
                            println!("{}", "⚠️  WARNING: This will permanently delete your mnemonic phrase!".red().bold());
                            println!("Type 'CONFIRM' to proceed:");
                            
                            let mut confirmation = String::new();
                            match io::stdin().read_line(&mut confirmation) {
                                Ok(_) => {
                                    if confirmation.trim() == "CONFIRM" {
                                        match remove_mnemonic() {
                                            Ok(_) => {
                                                println!("{}", "✓ Mnemonic removed successfully".green().bold());
                                                return Some("mnemonic_removed".to_string());
                                            }
                                            Err(e) => {
                                                println!("{}", format!("Failed to remove mnemonic: {}", e).red());
                                                return None;
                                            }
                                        }
                                    } else {
                                        println!("Operation cancelled.");
                                        return None;
                                    }
                                }
                                Err(e) => {
                                    println!("{}", format!("Error reading confirmation: {}", e).red());
                                    return None;
                                }
                            }
                        }
                        
                        "status" => {
                            if check_mnemonic_exists() {
                                println!("{}", "✓ Mnemonic exists in keystore".green().bold());
                                match get_mnemonic_addresses() {
                                    Ok(addresses) => {
                                        println!("Associated addresses: {}", addresses.len());
                                        for addr in addresses {
                                            println!("  - {}", addr.green());
                                        }
                                    }
                                    Err(e) => {
                                        println!("{}", format!("Error getting addresses: {}", e).red());
                                    }
                                }
                            } else {
                                println!("{}", "No mnemonic found in keystore".yellow());
                            }
                            return None;
                        }
                        
                        _ => {
                            println!("{}", "Mnemonic subcommands:".bright_yellow().bold());
                            println!("  save    - Save a BIP39 mnemonic phrase");
                            println!("  load    - Load and display mnemonic phrase");
                            println!("  remove  - Remove mnemonic from keystore");
                            println!("  status  - Check mnemonic status");
                            return None;
                        }
                    }
                } else {
                    println!("{}", "Mnemonic subcommands:".bright_yellow().bold());
                    println!("  save    - Save a BIP39 mnemonic phrase");
                    println!("  load    - Load and display mnemonic phrase");
                    println!("  remove  - Remove mnemonic from keystore");
                    println!("  status  - Check mnemonic status");
                    return None;
                }
            }
            
            "session" => {
                // Handle session key subcommands
                if args.len() > 3 {
                    let subcommand = &args[3];
                    match subcommand.as_str() {
                        "set" => {
                            if args.len() > 5 {
                                let key = &args[4];
                                let value = &args[5];
                                
                                match save_session_key(key, value) {
                                    Ok(_) => {
                                        println!("{}", format!("✓ Session key '{}' saved", key).green().bold());
                                        return Some(format!("session_set_{}", key));
                                    }
                                    Err(e) => {
                                        println!("{}", format!("Failed to save session key: {}", e).red());
                                        return None;
                                    }
                                }
                            } else {
                                println!("Usage: kari keytool session set <key> <value>");
                                return None;
                            }
                        }
                        
                        "get" => {
                            if args.len() > 4 {
                                let key = &args[4];
                                
                                match load_session_key(key) {
                                    Ok(Some(value)) => {
                                        println!("{}: {}", key.green().bold(), value.green());
                                        return Some(format!("session_get_{}", key));
                                    }
                                    Ok(None) => {
                                        println!("{}", format!("Session key '{}' not found", key).yellow());
                                        return None;
                                    }
                                    Err(e) => {
                                        println!("{}", format!("Failed to load session key: {}", e).red());
                                        return None;
                                    }
                                }
                            } else {
                                println!("Usage: kari keytool session get <key>");
                                return None;
                            }
                        }
                        
                        "remove" => {
                            if args.len() > 4 {
                                let key = &args[4];
                                
                                match remove_session_key(key) {
                                    Ok(_) => {
                                        println!("{}", format!("✓ Session key '{}' removed", key).green().bold());
                                        return Some(format!("session_removed_{}", key));
                                    }
                                    Err(e) => {
                                        println!("{}", format!("Failed to remove session key: {}", e).red());
                                        return None;
                                    }
                                }
                            } else {
                                println!("Usage: kari keytool session remove <key>");
                                return None;
                            }
                        }
                        
                        "clear" => {
                            println!("{}", "⚠️  This will remove ALL session keys. Continue? (y/n)".yellow().bold());
                            
                            let mut confirmation = String::new();
                            match io::stdin().read_line(&mut confirmation) {
                                Ok(_) => {
                                    if confirmation.trim().eq_ignore_ascii_case("y") {
                                        match clear_session_keys() {
                                            Ok(_) => {
                                                println!("{}", "✓ All session keys cleared".green().bold());
                                                return Some("session_cleared".to_string());
                                            }
                                            Err(e) => {
                                                println!("{}", format!("Failed to clear session keys: {}", e).red());
                                                return None;
                                            }
                                        }
                                    } else {
                                        println!("Operation cancelled.");
                                        return None;
                                    }
                                }
                                Err(e) => {
                                    println!("{}", format!("Error reading confirmation: {}", e).red());
                                    return None;
                                }
                            }
                        }
                        
                        _ => {
                            println!("{}", "Session key subcommands:".bright_yellow().bold());
                            println!("  set <key> <value>  - Save a session key");
                            println!("  get <key>          - Get a session key value");
                            println!("  remove <key>       - Remove a session key");
                            println!("  clear              - Remove all session keys");
                            return None;
                        }
                    }
                } else {
                    println!("{}", "Session key subcommands:".bright_yellow().bold());
                    println!("  set <key> <value>  - Save a session key");
                    println!("  get <key>          - Get a session key value");
                    println!("  remove <key>       - Remove a session key");
                    println!("  clear              - Remove all session keys");
                    return None;
                }
            }

            _ => {
                display_help(true);
                return None;
            }
        }
    } else {
        display_help(false);
        return None;
    }
}

// Add this function to format KA balance into human-readable Kari format
fn format_balance(ka_balance: u64) -> String {
    // KA is 10^-9 of a Kari token
    const KA_PER_KARI: u64 = 1_000_000_000;
    
    // Separate whole Kari and fractional part (KA)
    let whole_kari = ka_balance / KA_PER_KARI;
    let fractional_ka = ka_balance % KA_PER_KARI;
    
    // Format whole part with thousands separators
    let whole_part_formatted = format!("{}", whole_kari)
        .chars()
        .rev()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(",")
        .chars()
        .rev()
        .collect::<String>();
    
    // Format fractional part with leading zeros if needed
    let fractional_part_formatted = format!("{:09}", fractional_ka);
    
    format!("{}.{}", whole_part_formatted, fractional_part_formatted)
}

fn prompt_password(confirm: bool) -> String {
    print!("Enter password for wallet: ");
    io::stdout().flush().unwrap();

    let password = match read_password() {
        Ok(pwd) => pwd,
        Err(e) => {
            eprintln!("Error reading password: {}. Falling back to standard input.", e);
            // Fallback to standard input when secure input fails
            let mut input = String::new();
            io::stdin().read_line(&mut input).expect("Failed to read input");
            input.trim().to_string()
        }
    };

    if confirm {
        print!("Confirm password: ");
        io::stdout().flush().unwrap();
        
        let confirm = match read_password() {
            Ok(pwd) => pwd,
            Err(e) => {
                eprintln!("Error reading password: {}. Falling back to standard input.", e);
                let mut input = String::new();
                io::stdin().read_line(&mut input).expect("Failed to read input");
                input.trim().to_string()
            }
        };

        if password != confirm {
            println!("{}", "Passwords do not match!".red());
            return prompt_password(true);
        }
    }
    password
}

// Add helper function to get RPC endpoint from configuration
fn get_rpc_endpoint() -> String {
    // Try to get RPC endpoint from kanari config first
    if let Ok(kanari_config) = load_kanari_config() {
        if let Some(active_env) = kanari_config.get("active_env").and_then(|v| v.as_str()) {
            if let Some(envs) = kanari_config.get("envs").and_then(|v| v.as_sequence()) {
                for env in envs {
                    if let Some(alias) = env.get("alias").and_then(|v| v.as_str()) {
                        if alias == active_env {
                            if let Some(rpc_url) = env.get("rpc").and_then(|v| v.as_str()) {
                                println!("Using RPC endpoint from kanari config: {}", rpc_url.green());
                                return rpc_url.to_string();
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Fallback to load_config (legacy support)
    if let Ok(config) = load_config() {
        if let Some(port) = config.get("rpc_port").and_then(|v| v.as_u64()) {
            let endpoint = format!("http://127.0.0.1:{}", port);
            println!("Using RPC endpoint from config: {}", endpoint.green());
            return endpoint;
        }
    }
    
    // Final fallback to default
    let default_endpoint = "http://127.0.0.1:30030".to_string();
    println!("{}", format!("Warning: Could not determine RPC endpoint from config, using default: {}", default_endpoint).yellow());
    println!("Make sure your node is running on the default port or check your configuration.");
    default_endpoint
}