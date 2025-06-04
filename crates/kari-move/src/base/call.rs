use anyhow::Result;
use clap::Parser;
use move_core_types::account_address::AccountAddress;
use std::str::FromStr;
use std::time::Instant;

use mona_vm::{VM_STATE, VMTransaction, execute_vm_transaction};
use mona_crypto::load_wallet;
use sha3::{Digest, Sha3_256};
use common::*;


#[derive(Parser)]
#[clap(about = "Call functions in deployed Move modules on the blockchain")]
pub struct Call {
    #[clap(long, help = "Module ID to call (format: <address>::<module_name>)")]
    pub module_id: String,

    #[clap(long, help = "Function name to call")]
    pub function: String,

    #[clap(long, use_value_delimiter = true, value_delimiter = ',', help = "Comma-separated list of typed arguments (format: <type>:<value>)")]
    pub args: Vec<String>,

    #[clap(long, default_value = "1000000", help = "Amount of gas units allocated for function call")]
    pub gas_budget: u64,

    #[clap(long, help = "Blockchain address to call from (format: 0x...)")]
    pub address: Option<AccountAddress>,
    
    #[clap(long, help = "Password for wallet to sign transaction")]
    pub password: Option<String>,
}

impl Call {
    pub fn execute(self) -> Result<()> {
        // Validate inputs
        if self.module_id.is_empty() || self.function.is_empty() {
            return Err(anyhow::anyhow!("Module ID and function name are required"));
        }
        
        // Parse and validate module ID
        let (address, full_module_id) = self.parse_module_id()?;
        let sender = self.get_sender_address()?;
        let parsed_args = self.parse_arguments()?;

        self.display_call_info(&full_module_id, &sender);
        
        // Sign transaction if wallet available
        let (signature, wallet_address) = self.sign_transaction(&sender, &full_module_id, &parsed_args)?;
        
        // Execute VM transaction
        self.execute_vm_call(sender, full_module_id, parsed_args, signature, wallet_address)
    }
    
    fn parse_module_id(&self) -> Result<(AccountAddress, String)> {
        let parts: Vec<&str> = self.module_id.split("::").collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!("Invalid module ID format. Expected <address>::<module_name>"));
        }

        let address_str = parts[0].trim();
        let address = if address_str.starts_with("0x") {
            AccountAddress::from_hex_literal(address_str)
        } else {
            AccountAddress::from_hex(address_str)
        }.map_err(|_| anyhow::anyhow!("Invalid address in module ID: {}", address_str))?;
        
        let full_module_id = format!("0x{}::{}", address.to_hex(), parts[1].trim());
        Ok((address, full_module_id))
    }
    
    fn get_sender_address(&self) -> Result<AccountAddress> {
        Ok(self.address.unwrap_or_else(|| {
            get_main_wallet()
                .and_then(|wallet| {
                    let wallet_addr = if wallet.starts_with("0x") { wallet } else { format!("0x{}", wallet) };
                    AccountAddress::from_hex_literal(&wallet_addr).ok()
                })
                .unwrap_or_else(|| AccountAddress::from_hex_literal("0x1").unwrap())
        }))
    }
    
    fn display_call_info(&self, full_module_id: &str, sender: &AccountAddress) {
        println!("Calling function on blockchain...");
        println!("📦 Module ID: {}", full_module_id);
        println!("🔧 Function: {}", self.function);
        println!("👤 Sender: 0x{}", sender.to_hex());
        println!("⛽ Gas budget: {}", self.gas_budget);
        
        if !self.args.is_empty() {
            println!("📝 Arguments: {}", self.args.join(", "));
        }
    }
    
    fn sign_transaction(&self, sender: &AccountAddress, full_module_id: &str, parsed_args: &[Vec<u8>]) -> Result<(Option<Vec<u8>>, Option<String>)> {
        let Some(wallet_addr) = get_main_wallet() else {
            println!("ℹ️ No wallet configured. Proceeding with unsigned transaction.");
            return Ok((None, None));
        };
        
        // Create payload for signing
        let mut hasher = Sha3_256::new();
        hasher.update(sender.to_hex().as_bytes());
        hasher.update(self.function.as_bytes());
        hasher.update(full_module_id.as_bytes());
        hasher.update(self.gas_budget.to_le_bytes());
        hasher.update(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().to_le_bytes());
        for arg in parsed_args {
            hasher.update(arg);
        }
        let payload_to_sign = hasher.finalize();
        
        // Get password and sign
        let password = self.password.clone().unwrap_or_else(|| {
            println!("Enter password for wallet {}: ", wallet_addr);
            rpassword::read_password().unwrap_or_default()
        });
        
        match load_wallet(&wallet_addr, &password) {
            Ok(wallet) => match wallet.sign(&payload_to_sign, &password) {
                Ok(sig) => {
                    println!("✅ Transaction signed successfully");
                    Ok((Some(sig), Some(wallet_addr)))
                },
                Err(e) => {
                    println!("⚠️ Failed to sign: {}. Proceeding unsigned.", e);
                    Ok((None, None))
                }
            },
            Err(e) => {
                println!("⚠️ Wallet error: {}. Proceeding unsigned.", e);
                Ok((None, None))
            }
        }
    }
    
    fn execute_vm_call(&self, sender: AccountAddress, full_module_id: String, parsed_args: Vec<Vec<u8>>, signature: Option<Vec<u8>>, wallet_address: Option<String>) -> Result<()> {
        let start_time = Instant::now();
        
        let mut vm_tx = VMTransaction::new(
            format!("0x{}", sender.to_hex()),
            full_module_id.clone(),
            self.function.clone(),
            parsed_args,
            self.gas_budget
        );
        
        if let (Some(sig), Some(addr)) = (signature, wallet_address) {
            vm_tx = vm_tx.with_signature(sig, addr);
        }
        
        println!("\n⏳ Executing function call...");
        
        // Execute with simple retry
        for attempt in 1..=3 {
            match execute_vm_transaction(&vm_tx) {
                Ok(result) => {
                    let duration = start_time.elapsed();
                    println!("\n✅ Function call successful!");
                    println!("⏱️ Execution time: {:.2?}", duration);
                    println!("🧾 Transaction ID: {}", result["tx_id"].as_str().unwrap_or("unknown"));
                    println!("⛽ Gas used: {}", result["gas_display"].as_str().unwrap_or("unknown"));
                    
                    if let Some(return_value) = result.get("return_value") {
                        println!("\n📊 Return value: {}", serde_json::to_string_pretty(return_value)?);
                    }
                    
                    println!("\nExecution Result: {}", serde_json::to_string_pretty(&result)?);
                    return Ok(());
                },
                Err(e) => {
                    if attempt < 3 {
                        println!("⚠️ Attempt {}/3 failed: {}. Retrying...", attempt, e);
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        continue;
                    }
                    
                    println!("\n❌ Function call failed after {} attempts: {}", attempt, e);
                    
                    // Provide helpful suggestions
                    if e.contains("Module not found") {
                        self.suggest_modules(&full_module_id)?;
                    } else if e.contains("Function") && e.contains("not found") {
                        self.suggest_functions(&full_module_id)?;
                    }
                    
                    return Err(anyhow::anyhow!("Function call failed: {}", e));
                }
            }
        }
        
        unreachable!()
    }
    
    fn suggest_modules(&self, full_module_id: &str) -> Result<()> {
        if let Ok(vm_state) = VM_STATE.read() {
            let module_name = full_module_id.split("::").nth(1).unwrap_or("");
            let similar: Vec<&String> = vm_state.modules.keys()
                .filter(|key| key.contains(module_name))
                .take(5)
                .collect();
                
            if !similar.is_empty() {
                println!("\n🔍 Similar modules found:");
                for (idx, module) in similar.iter().enumerate() {
                    println!("   {}. {}", idx+1, module);
                }
            }
        }
        Ok(())
    }
    
    fn suggest_functions(&self, full_module_id: &str) -> Result<()> {
        if let Ok(vm_state) = VM_STATE.read() {
            if let Some(module) = vm_state.modules.get(full_module_id) {
                println!("\n🔍 Available functions:");
                for (idx, func) in module.public_functions.iter().enumerate() {
                    println!("   {}. {}", idx+1, func);
                }
            }
        }
        Ok(())
    }
    
    fn parse_arguments(&self) -> Result<Vec<Vec<u8>>> {
        self.args.iter().map(|arg| {
            let parts: Vec<&str> = arg.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(anyhow::anyhow!("Invalid argument format: '{}'. Expected '<type>:<value>'", arg));
            }
            
            let (arg_type, arg_value) = (parts[0].trim(), parts[1].trim());
            
            match arg_type {
                "address" => {
                    let addr = if arg_value.starts_with("0x") {
                        AccountAddress::from_hex_literal(arg_value)
                    } else {
                        AccountAddress::from_hex(arg_value)
                    }.map_err(|_| anyhow::anyhow!("Invalid address: {}", arg_value))?;
                    Ok(addr.to_vec())
                },
                "u8" => Ok(vec![u8::from_str(arg_value)?]),
                "u64" => Ok(u64::from_str(arg_value)?.to_le_bytes().to_vec()),
                "u128" => Ok(u128::from_str(arg_value)?.to_le_bytes().to_vec()),
                "bool" => Ok(vec![if bool::from_str(arg_value)? { 1 } else { 0 }]),
                "string" => Ok(arg_value.as_bytes().to_vec()),
                _ => Err(anyhow::anyhow!("Unsupported argument type: '{}'. Supported: address, u8, u64, u128, bool, string", arg_type))
            }
        }).collect()
    }
}