use anyhow::Result;
use clap::Parser;
use move_core_types::account_address::AccountAddress;
use std::str::FromStr;
use std::time::Instant;
use std::time::SystemTime;

use mona_vm::*;
use common::*;
use mona_crypto::load_wallet;
use sha3::{Digest, Sha3_256};

#[derive(Parser)]
#[clap(about = "Call functions in deployed Move modules on the blockchain")]
pub struct Call {
    #[clap(long, help = "Module ID to call (format: <address>::<module_name>)")]
    pub module_id: String,
    #[clap(long, help = "Function name to call")]
    pub function: String,
    #[clap(long, use_value_delimiter = true, value_delimiter = ',', help = "Comma-separated list of typed arguments")]
    pub args: Vec<String>,
    #[clap(long, default_value = "1_000_000", help = "Gas units for function call")]
    pub gas_budget: u64,
    #[clap(long, help = "Address to call from")]
    pub address: Option<AccountAddress>,
    #[clap(long, help = "Wallet password")]
    pub password: Option<String>,
}

impl Call {
    pub fn execute(self) -> Result<()> {
        // Validate inputs
        if self.module_id.is_empty() || self.function.is_empty() {
            return Err(anyhow::anyhow!("Module ID and function name are required"));
        }
        
        // Parse module components
        let parts: Vec<&str> = self.module_id.split("::").collect();
        if parts.len() != 2 {
            return Err(anyhow::anyhow!("Invalid module ID format. Expected <address>::<module_name>"));
        }

        let address = parse_address(parts[0])?;
        let sender = self.address.unwrap_or_else(|| get_default_address());
        let full_module_id = format!("0x{}::{}", address.to_hex(), parts[1]);
        
        // Parse arguments
        let parsed_args = self.parse_arguments()?;

        // Display call info
        self.display_call_info(&full_module_id, &sender);
        
        // Create and sign transaction
        let payload = self.create_payload(&sender, &full_module_id, &parsed_args)?;
        let (signature, wallet_address) = self.sign_transaction(&payload)?;
        
        // Execute transaction
        let start_time = Instant::now();
        let vm_tx = self.create_vm_transaction(sender, full_module_id.clone(), parsed_args, signature, wallet_address);
        
        // Create blockchain transaction for VM call
        let blockchain_tx = self.create_blockchain_transaction(&vm_tx, &sender)?;
        
        // Submit to blockchain first
        match mona_blockchain::blockchain::submit_transaction(blockchain_tx) {
            Ok(()) => {
                println!("✅ Transaction submitted to blockchain");
                
                // Then execute VM transaction
                self.execute_with_retry(vm_tx, start_time)
            },
            Err(e) => {
                println!("❌ Failed to submit transaction to blockchain: {}", e);
                Err(anyhow::anyhow!("Blockchain submission failed: {}", e))
            }
        }
    }
    
    fn display_call_info(&self, module_id: &str, sender: &AccountAddress) {
        println!("Calling function on blockchain...");
        println!("📦 Module: {}", module_id);
        println!("🔧 Function: {}", self.function);
        println!("👤 Sender: 0x{}", sender.to_hex());
        println!("⛽ Gas: {}", self.gas_budget);
        if !self.args.is_empty() {
            println!("📝 Args: {}", self.args.join(", "));
        }
    }
    
    fn create_payload(&self, sender: &AccountAddress, module_id: &str, args: &[Vec<u8>]) -> Result<Vec<u8>> {
        let mut hasher = Sha3_256::new();
        hasher.update(sender.to_hex().as_bytes());
        hasher.update(self.function.as_bytes());
        hasher.update(module_id.as_bytes());
        hasher.update(self.gas_budget.to_le_bytes());
        hasher.update(SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs().to_le_bytes());
        
        for arg in args {
            hasher.update(arg);
        }
        
        Ok(hasher.finalize().to_vec())
    }
    
    fn sign_transaction(&self, payload: &[u8]) -> Result<(Option<Vec<u8>>, Option<String>)> {
        let wallet_addr = match get_main_wallet() {
            Some(addr) => addr,
            None => {
                println!("ℹ️ No wallet configured. Proceeding unsigned.");
                return Ok((None, None));
            }
        };
        
        let password = self.get_password(&wallet_addr)?;
        let wallet = load_wallet(&wallet_addr, &password)?;
        let signature = wallet.sign(payload, &password)?;
        
        println!("✅ Transaction signed with wallet {}", format_address(&wallet_addr));
        Ok((Some(signature), Some(wallet_addr)))
    }
    
    fn get_password(&self, wallet_addr: &str) -> Result<String> {
        match &self.password {
            Some(pwd) => Ok(pwd.clone()),
            None => {
                println!("Enter password for wallet {}: ", format_address(wallet_addr));
                Ok(rpassword::read_password()?)
            }
        }
    }
    
    fn create_vm_transaction(&self, sender: AccountAddress, module_id: String, args: Vec<Vec<u8>>, signature: Option<Vec<u8>>, wallet_addr: Option<String>) -> VMTransaction {
        let mut tx = VMTransaction::new(format!("0x{}", sender.to_hex()), module_id, self.function.clone(), args, self.gas_budget);
        if let (Some(sig), Some(addr)) = (signature, wallet_addr) {
            tx = tx.with_signature(sig, addr);
        }
        tx
    }
    
    fn create_blockchain_transaction(&self, vm_tx: &VMTransaction, sender: &AccountAddress) -> Result<mona_blockchain::block::Transaction> {
        let timestamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        
        let vm_data = format!("VM:{}:{}:{}", vm_tx.module_id, vm_tx.function, vm_tx.gas_budget);
        
        Ok(mona_blockchain::block::Transaction {
            transaction_id: vm_tx.tx_id.clone(),
            sender: mona_types::address::Address::from_hex_literal(&format!("0x{}", sender.to_hex()))
                .map_err(|_| anyhow::anyhow!("Invalid sender address"))?,
            receiver: mona_types::address::Address::from_hex_literal(&format!("0x{}", sender.to_hex()))
                .map_err(|_| anyhow::anyhow!("Invalid receiver address"))?,
            amount: 1, // Set minimal amount of 1 KA for VM function call
            gas_fee: vm_tx.gas_budget,
            timestamp,
            signature: vm_tx.signature.clone().unwrap_or_default(),
            data: Some(vm_data.into_bytes()),
        })
    }
    
    fn execute_with_retry(&self, vm_tx: VMTransaction, start_time: Instant) -> Result<()> {
        println!("\n⏳ Executing function call...");
        
        for attempt in 1..=3 {
            match execute_vm_transaction(&vm_tx) {
                Ok(result) => {
                    self.print_success(&result, start_time.elapsed());
                    return Ok(());
                },
                Err(e) => {
                    if attempt == 3 {
                        self.print_error(&e, start_time.elapsed(), &vm_tx);
                        return Err(anyhow::anyhow!("Call failed: {}", e));
                    }
                    println!("⚠️ Attempt {}/3 failed: {}", attempt, e);
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
            }
        }
        Ok(())
    }
    
    fn print_success(&self, result: &serde_json::Value, duration: std::time::Duration) {
        println!("\n✅ Function call successful!");
        println!("⏱️ Time: {:.2?}", duration);
        println!("🧾 TX ID: {}", result["tx_id"].as_str().unwrap_or("unknown"));
        println!("⛽ Gas: {}", result["gas_display"].as_str().unwrap_or("unknown"));
        
        if let Some(return_value) = result.get("return_value") {
            println!("📊 Return: {}", serde_json::to_string_pretty(return_value).unwrap_or_default());
        }
        println!("\nResult: {}", serde_json::to_string_pretty(result).unwrap_or_default());
    }
    
    fn print_error(&self, error: &str, duration: std::time::Duration, vm_tx: &VMTransaction) {
        println!("\n❌ Call failed after {:.2?}", duration);
        println!("Error: {}", error);
        
        // Provide helpful suggestions
        if error.contains("Module not found") {
            self.suggest_modules(&vm_tx.module_id);
        } else if error.contains("Function") && error.contains("not found") {
            self.suggest_functions(&vm_tx.module_id);
        }
    }
    
    fn suggest_modules(&self, module_id: &str) {
        println!("💡 Check if module ID is correct: {}", module_id);
        if let Ok(state) = VM_STATE.read() {
            let similar: Vec<_> = state.modules.keys().filter(|k| k.contains(&module_id.split("::").last().unwrap_or(""))).take(3).collect();
            if !similar.is_empty() {
                println!("🔍 Similar modules: {}", similar.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "));
            }
        }
    }
    
    fn suggest_functions(&self, module_id: &str) {
        if let Ok(state) = VM_STATE.read() {
            if let Some(module) = state.modules.get(module_id) {
                println!("🔍 Available functions: {}", module.public_functions.join(", "));
            }
        }
    }
    
    fn parse_arguments(&self) -> Result<Vec<Vec<u8>>> {
        self.args.iter().map(|arg| {
            let parts: Vec<&str> = arg.splitn(2, ':').collect();
            if parts.len() != 2 {
                return Err(anyhow::anyhow!("Invalid arg format: '{}'. Use <type>:<value>", arg));
            }
            
            let (arg_type, value) = (parts[0].trim(), parts[1].trim());
            match arg_type {
                "address" => parse_address(value).map(|a| a.to_vec()),
                "u8" => u8::from_str(value).map(|v| vec![v]).map_err(Into::into),
                "u64" => u64::from_str(value).map(|v| v.to_le_bytes().to_vec()).map_err(Into::into),
                "u128" => u128::from_str(value).map(|v| v.to_le_bytes().to_vec()).map_err(Into::into),
                "bool" => bool::from_str(value).map(|v| vec![if v { 1 } else { 0 }]).map_err(Into::into),
                "string" => Ok(value.as_bytes().to_vec()),
                _ => Err(anyhow::anyhow!("Unsupported type: {}", arg_type))
            }
        }).collect()
    }
}

// Helper functions
fn parse_address(addr_str: &str) -> Result<AccountAddress> {
    if addr_str.starts_with("0x") {
        AccountAddress::from_hex_literal(addr_str)
    } else {
        AccountAddress::from_hex(addr_str)
    }.map_err(|_| anyhow::anyhow!("Invalid address: {}", addr_str))
}

fn get_default_address() -> AccountAddress {
    get_main_wallet()
        .and_then(|w| parse_address(&w).ok())
        .unwrap_or_else(|| AccountAddress::from_hex_literal("0x1").unwrap())
}

fn format_address(addr: &str) -> String {
    if addr.starts_with("0x") { addr.to_string() } else { format!("0x{}", addr) }
}