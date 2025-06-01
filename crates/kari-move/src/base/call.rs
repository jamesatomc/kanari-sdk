use anyhow::Result;
use clap::Parser;
use move_core_types::account_address::AccountAddress;
use serde_json::json;
use std::str::FromStr;
use std::time::Instant;

use mona_vm::*;
use common::*;
// Add mona_crypto imports
use mona_crypto::{load_wallet, WalletError};
use sha3::{Digest, Sha3_256};

#[derive(Parser)]
#[clap(
    about = "Call functions in deployed Move modules on the blockchain",
    after_help = "EXAMPLES:
    # Call a function that returns information (no arguments needed)
    kari move call --module-id 0x123::token --function check_info

    # Mint tokens to an address
    kari move call --module-id 0x123::token --function mint --args address:0x456,u64:100

    # Burn tokens
    kari move call --module-id 0x123::token --function burn --args address:0x456

    # Block an address
    kari move call --module-id 0x123::token --function deny_list_add_admin --args address:0x456

    # Specify gas budget
    kari move call --module-id 0x123::token --function transfer --gas-budget=5000000

    # Call from a specific address
    kari move call --module-id 0x123::token --function transfer --address=0x789

    # Sign transaction with wallet password
    kari move call --module-id 0x123::token --function transfer --password=your_password"
)]
pub struct Call {
    /// Module ID in format <address>::<module_name>
    #[clap(long, help = "Module ID to call (format: <address>::<module_name>)")]
    pub module_id: String,

    /// Function name to call
    #[clap(long, help = "Function name to call")]
    pub function: String,

    /// List of typed arguments in format <type>:<value>
    /// Supported types: address, u8, u16, u32, u64, u128, u256, bool, string
    #[clap(long, use_value_delimiter = true, value_delimiter = ',', help = "Comma-separated list of typed arguments (format: <type>:<value>)")]
    pub args: Vec<String>,

    /// Gas budget for the call
    /// Default is 1_000_000 - 10_000_000 for local testing
    #[clap(long, default_value = "1_000_000", help = "Amount of gas units allocated for function call")]
    pub gas_budget: u64,

    /// Address to call from (sender)
    #[clap(long, help = "Blockchain address to call from (format: 0x...)")]
    pub address: Option<AccountAddress>,
    
    /// Password for wallet to sign transaction
    #[clap(long, help = "Password for wallet to sign transaction")]
    pub password: Option<String>,
}

impl Call {
    pub fn execute(self) -> Result<()> {
        // Check if module_id and function are provided
        if self.module_id.is_empty() {
            return Err(anyhow::anyhow!("Module ID is required. Use --module-id <address>::<module_name>"));
        }
        
        if self.function.is_empty() {
            return Err(anyhow::anyhow!("Function name is required. Use --function <name>"));
        }
        
        // Parse module_id into address and module name
        let parts: Vec<&str> = self.module_id.split("::").collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!("Invalid module ID format. Expected <address>::<module_name>"));
        }

        // Parse address from the first part
        let address_str = parts[0].trim();
        let module_name = parts[1].trim();
        
        let address = if address_str.starts_with("0x") {
            AccountAddress::from_hex_literal(address_str)
        } else {
            AccountAddress::from_hex(address_str)
        }.map_err(|_| anyhow::anyhow!("Invalid address in module ID: {}", address_str))?;
        
        // Set sender address (from address)
        let sender = self.address.unwrap_or_else(|| {
            if let Some(wallet) = get_main_wallet() {
                // Fix: Check if wallet already has 0x prefix before adding it
                let wallet_with_prefix = if wallet.starts_with("0x") {
                    wallet.clone()
                } else {
                    format!("0x{}", wallet)
                };
                
                AccountAddress::from_hex_literal(&wallet_with_prefix)
                    .or_else(|_| AccountAddress::from_hex(&wallet))
                    .unwrap_or_else(|_| AccountAddress::from_hex_literal("0x1").unwrap())
            } else {
                AccountAddress::from_hex_literal("0x1").unwrap()
            }
        });

        // Generate complete module ID
        let full_module_id = format!("0x{}::{}", address.to_hex(), module_name);
        
        // Parse arguments
        let parsed_args = self.parse_arguments()?;

        // Display call information
        println!("Calling function on blockchain...");
        println!("📦 Module ID: {}", full_module_id);
        println!("🔧 Function: {}", self.function);
        println!("👤 Sender: 0x{}", sender.to_hex());
        println!("⛽ Gas budget: {}", self.gas_budget);
        
        if !self.args.is_empty() {
            println!("📝 Arguments:");
            for (i, arg) in self.args.iter().enumerate() {
                println!("  {}. {}", i+1, arg);
            }
        } else {
            println!("📝 No arguments provided");
        }
        
        // Create transaction payload for signing
        let mut hasher = Sha3_256::new();
        hasher.update(sender.to_hex().as_bytes());
        hasher.update(self.function.as_bytes());
        hasher.update(full_module_id.as_bytes());
        hasher.update(self.gas_budget.to_le_bytes());
        
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        hasher.update(timestamp.to_le_bytes());
        
        // Include arguments hash in the payload
        for arg in &parsed_args {
            hasher.update(arg);
        }
        
        let payload_hash = hasher.finalize();
        let payload_to_sign = payload_hash.as_slice();
        
        // Sign with wallet if available
        let (signature, wallet_address) = match get_main_wallet() {
            Some(wallet_addr) => {
                // Request password
                let password = match &self.password {
                    Some(pwd) => pwd.clone(),
                    None => {
                        let display_addr = if wallet_addr.starts_with("0x") {
                            wallet_addr.clone()
                        } else {
                            format!("0x{}", wallet_addr)
                        };
                        
                        println!("Enter password for wallet {}: ", display_addr);
                        rpassword::read_password().unwrap_or_default()
                    }
                };
                
                // Load wallet and sign payload
                match load_wallet(&wallet_addr, &password) {
                    Ok(wallet) => {
                        match wallet.sign(payload_to_sign, &password) {
                            Ok(sig) => {
                                let display_addr = if wallet_addr.starts_with("0x") {
                                    wallet_addr.clone()
                                } else {
                                    format!("0x{}", wallet_addr)
                                };
                                println!("✅ Transaction signed successfully with wallet {}", display_addr);
                                (Some(sig), Some(wallet_addr.clone()))
                            },
                            Err(err) => {
                                println!("❌ ERROR: Failed to sign transaction: {}", err);
                                println!("Proceeding with unsigned transaction.");
                                (None, None)
                            }
                        }
                    },
                    Err(WalletError::InvalidPassword) => {
                        let display_addr = if wallet_addr.starts_with("0x") {
                            wallet_addr.clone()
                        } else {
                            format!("0x{}", wallet_addr)
                        };
                        println!("❌ ERROR: Invalid password for wallet {}", display_addr);
                        println!("Proceeding with unsigned transaction.");
                        (None, None)
                    },
                    Err(err) => {
                        println!("❌ ERROR: Failed to load wallet: {}", err);
                        println!("Proceeding with unsigned transaction.");
                        (None, None)
                    }
                }
            },
            None => {
                println!("ℹ️ No wallet configured. Proceeding with unsigned transaction.");
                (None, None)
            }
        };
        
        // Store whether we have a signature before moving the value
        let has_signature = signature.is_some();
        
        // Create timer to measure call duration
        let start_time = Instant::now();
        
        // Create VM transaction
        let mut vm_tx = VMTransaction::new(
            format!("0x{}", sender.to_hex()),
            full_module_id.clone(),
            self.function.clone(),
            parsed_args,
            self.gas_budget
        );
        
        // Add signature to transaction if available
        if let (Some(sig), Some(addr)) = (signature, wallet_address) {
            vm_tx = vm_tx.with_signature(sig, addr);
        }
        
        // Execute the transaction on the VM with retry logic
        let mut attempts = 0;
        let max_attempts = 3;
        let mut last_error = None;
        
        println!("\n⏳ Executing function call on blockchain...");
        
        while attempts < max_attempts {
            match execute_vm_transaction(&vm_tx) {
                Ok(result) => {
                    let duration = start_time.elapsed();
                    
                    // Print success result
                    println!("\n✅ Function call successful!");
                    println!("⏱️ Execution time: {:.2?}", duration);
                    println!("🧾 Transaction ID: {}", result["tx_id"].as_str().unwrap_or("unknown"));
                    println!("⛽ Gas used: {}", result["gas_display"].as_str().unwrap_or("unknown"));
                    
                    // If there's a return value, display it
                    if let Some(return_value) = result.get("return_value") {
                        println!("\n📊 Return value: {}", serde_json::to_string_pretty(return_value)?);
                    }
                    
                    // Create a structured result
                    println!("\nExecution Result: {}", serde_json::to_string_pretty(&result)?);
                    return Ok(());
                },
                Err(e) => {
                    attempts += 1;
                    last_error = Some(e.clone());
                    
                    if attempts < max_attempts {
                        println!("⚠️ Attempt {}/{} failed: {}", attempts, max_attempts, e);
                        
                        if e.contains("Module not found") {
                            println!("  💡 Check if the module ID is correct. Are you using the right address?");
                        } else if e.contains("Function not found") {
                            println!("  💡 Check if the function name is correct and public/entry.");
                        } else if e.contains("Argument") {
                            println!("  💡 Check if you provided the correct number and types of arguments.");
                        }
                        
                        // Wait briefly before retrying
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        continue;
                    }
                    
                    let duration = start_time.elapsed();
                    
                    // Create a structured error result
                    let error_result = json!({
                        "status": "error",
                        "type": "function_call",
                        "message": e,
                        "details": {
                            "module_id": full_module_id,
                            "function": self.function,
                            "sender": format!("0x{}", sender.to_hex()),
                            "gas_budget": self.gas_budget,
                            "elapsed_time_ms": duration.as_millis(),
                            "attempts": attempts,
                            "signed": has_signature
                        }
                    });
                    
                    // Print the error
                    println!("\n❌ Function call failed after {:.2?} ({} attempts)", duration, attempts);
                    println!("Error: {}\n", e);
                    
                    // Add helpful suggestions based on error
                    if e.contains("Module not found") {
                        println!("💡 Suggestion: Check if your module ID is correct. The module should be deployed at {}.", full_module_id);
                        println!("💡 Try using shorter address format or check capitalization.");
                        
                        // Try to suggest available modules
                        if let Ok(vm_state) = VM_STATE.read() {
                            let similar_modules: Vec<&String> = vm_state.modules.keys()
                                .filter(|key| key.contains(module_name))
                                .collect();
                                
                            if !similar_modules.is_empty() {
                                println!("\n🔍 Found similar modules that might match what you're looking for:");
                                for (idx, module) in similar_modules.iter().enumerate().take(5) {
                                    println!("   {}. {}", idx+1, module);
                                }
                                
                                // Suggest a command with one of the found modules
                                if !similar_modules.is_empty() {
                                    println!("\n💡 Try:");
                                    println!("   kari move call --module-id {} --function {}", 
                                            similar_modules[0], self.function);
                                }
                            }
                        }
                    }
                    else if e.contains("Function") && e.contains("not found") {
                        println!("💡 Suggestion: Check if the function '{}' exists and is public/entry.", self.function);
                        
                        // Try to suggest available functions
                        if let Ok(vm_state) = VM_STATE.read() {
                            if let Some(module) = vm_state.modules.get(&full_module_id) {
                                println!("\n🔍 Available functions in this module:");
                                for (idx, func) in module.public_functions.iter().enumerate() {
                                    println!("   {}. {}", idx+1, func);
                                }
                                
                                // Suggest a command with one of the available functions
                                if !module.public_functions.is_empty() {
                                    println!("\n💡 Try:");
                                    println!("   kari move call --module-id {} --function {}", 
                                            full_module_id, module.public_functions[0]);
                                }
                            }
                        }
                    }
                    else if e.contains("argument") || e.contains("parameter") {
                        println!("💡 Suggestion: Check the number and types of arguments required for this function.");
                        println!("💡 Arguments should be in the format: <type>:<value> e.g., address:0x123,u64:100");
                    }
                    else if e.contains("permission") || e.contains("unauthorized") {
                        println!("💡 Suggestion: You may need proper permissions to call this function.");
                        println!("💡 Try signing the transaction with a wallet that has the required capabilities.");
                    }
                    
                    println!("\nError Details: {}", serde_json::to_string_pretty(&error_result)?);
                    
                    break;
                }
            }
        }
        
        // Return the last error if we had one
        if let Some(e) = last_error {
            Err(anyhow::anyhow!("Function call failed after {} attempts: {}", attempts, e))
        } else {
            Err(anyhow::anyhow!("Function call failed with no specific error"))
        }
    }
    
    fn parse_arguments(&self) -> Result<Vec<Vec<u8>>> {
        let mut parsed_args = Vec::new();
        
        for arg in &self.args {
            let parts: Vec<&str> = arg.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(anyhow::anyhow!(
                    "Invalid argument format: '{}'. Expected '<type>:<value>'", arg
                ));
            }
            
            let arg_type = parts[0].trim();
            let arg_value = parts[1].trim();
            
            match arg_type {
                "address" => {
                    let addr = if arg_value.starts_with("0x") {
                        AccountAddress::from_hex_literal(arg_value)
                    } else {
                        AccountAddress::from_hex(arg_value)
                    }.map_err(|_| anyhow::anyhow!("Invalid address: {}", arg_value))?;
                    
                    parsed_args.push(addr.to_vec());
                },
                "u8" => {
                    let val = u8::from_str(arg_value)
                        .map_err(|_| anyhow::anyhow!("Invalid u8: {}", arg_value))?;
                    parsed_args.push(vec![val]);
                },
                "u64" => {
                    let val = u64::from_str(arg_value)
                        .map_err(|_| anyhow::anyhow!("Invalid u64: {}", arg_value))?;
                    parsed_args.push(val.to_le_bytes().to_vec());
                },
                "u128" => {
                    let val = u128::from_str(arg_value)
                        .map_err(|_| anyhow::anyhow!("Invalid u128: {}", arg_value))?;
                    parsed_args.push(val.to_le_bytes().to_vec());
                },
                "bool" => {
                    let val = bool::from_str(arg_value)
                        .map_err(|_| anyhow::anyhow!("Invalid bool: {}", arg_value))?;
                    parsed_args.push(vec![if val { 1 } else { 0 }]);
                },
                "string" => {
                    parsed_args.push(arg_value.as_bytes().to_vec());
                },
                _ => {
                    return Err(anyhow::anyhow!(
                        "Unsupported argument type: '{}'. Supported types: address, u8, u64, u128, bool, string", 
                        arg_type
                    ));
                }
            }
        }
        
        Ok(parsed_args)
    }
}