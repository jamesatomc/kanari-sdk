use anyhow::Result;
use clap::Parser;
use move_core_types::account_address::AccountAddress;
use move_package::BuildConfig;
use serde_json::json;
use std::{path::PathBuf, time::SystemTime};

use mona_vm::*;
use common::*;
use mona_crypto::load_wallet;
use sha3::{Digest, Sha3_256};

#[derive(Parser)]
#[clap(about = "Publish Move modules to blockchain network")]
pub struct Publish {
    #[clap(long, help = "Directory path containing the Move package")]
    pub module_path: PathBuf,
    #[clap(long, default_value = "500_000", help = "Gas units for deployment")] // Reduced from 3_000_000
    pub gas_budget: u64,
    #[clap(long, help = "Skip module verification")]
    pub skip_verify: bool,
    #[clap(long, help = "Address to deploy to")]
    pub address: Option<AccountAddress>,
    #[clap(long, help = "Wallet password")]
    pub password: Option<String>,
}

impl Publish {
    pub fn execute(self, path: Option<PathBuf>, config: BuildConfig) -> Result<()> {
        let package_path = path.unwrap_or_else(|| self.module_path.clone());
        self.validate_package(&package_path)?;
        
        let address = self.get_target_address()?;
        let build_config = self.prepare_build_config(config, address);
        
        println!("Publishing to blockchain...");
        println!("📦 Package: {}", package_path.display());
        println!("🔑 Address: 0x{}", address.to_hex());
        println!("⛽ Gas: {}", self.gas_budget);
        
        let (signature, wallet_address) = self.sign_deployment(address, &package_path)?;
        let start_time = std::time::Instant::now();
        
        println!("\n⏳ Compiling and deploying...");
        
        // Create deployment transaction before VM execution
        let deployment_tx = self.create_deployment_transaction(address, &package_path, signature.clone(), wallet_address.clone())?;
        
        // Submit deployment transaction to blockchain
        match mona_blockchain::blockchain::submit_transaction(deployment_tx) {
            Ok(()) => {
                println!("✅ Deployment transaction submitted to blockchain");
                
                // Execute VM deployment
                let result = self.execute_deployment(package_path, address, build_config, signature, wallet_address);
                self.handle_result(result, start_time, address)
            },
            Err(e) => {
                println!("❌ Failed to submit deployment transaction: {}", e);
                Err(anyhow::anyhow!("Deployment transaction failed: {}", e))
            }
        }
    }
    
    fn validate_package(&self, path: &PathBuf) -> Result<()> {
        if !path.exists() {
            return Err(anyhow::anyhow!("Package path not found: {}", path.display()));
        }
        if !path.join("sources").exists() {
            return Err(anyhow::anyhow!("No sources directory found"));
        }
        Ok(())
    }
    
    fn get_target_address(&self) -> Result<AccountAddress> {
        Ok(self.address.unwrap_or_else(|| {
            get_main_wallet()
                .and_then(|w| parse_address(&w).ok())
                .unwrap_or_else(|| AccountAddress::from_hex_literal("0x1").unwrap())
        }))
    }
    
    fn prepare_build_config(&self, mut config: BuildConfig, address: AccountAddress) -> BuildConfig {
        config.additional_named_addresses.insert("module_addr".to_string(), address);
        config
    }
    
    fn sign_deployment(&self, address: AccountAddress, package_path: &PathBuf) -> Result<(Option<Vec<u8>>, Option<String>)> {
        let wallet_addr = match get_main_wallet() {
            Some(addr) => addr,
            None => {
                if self.address.is_some() {
                    println!("ℹ️ No wallet configured. Publishing unsigned.");
                    return Ok((None, None));
                }
                return Err(anyhow::anyhow!("No wallet configured and no address specified"));
            }
        };
        
        let password = self.get_password(&wallet_addr)?;
        let payload = self.create_deployment_payload(address, package_path)?;
        
        let wallet = load_wallet(&wallet_addr, &password)?;
        let signature = wallet.sign(&payload, &password)?;
        
        println!("✅ Deployment signed with wallet {}", format_address(&wallet_addr));
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
    
    fn create_deployment_payload(&self, address: AccountAddress, package_path: &PathBuf) -> Result<Vec<u8>> {
        let mut hasher = Sha3_256::new();
        hasher.update(address.to_hex().as_bytes());
        hasher.update(package_path.to_str().unwrap_or("").as_bytes());
        hasher.update(self.gas_budget.to_le_bytes());
        hasher.update(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs().to_le_bytes());
        Ok(hasher.finalize().to_vec())
    }
    
    fn create_deployment_transaction(&self, address: AccountAddress, package_path: &PathBuf, signature: Option<Vec<u8>>, wallet_address: Option<String>) -> Result<mona_blockchain::block::Transaction> {
        let _ = wallet_address;
        let timestamp = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        
        // Use minimal gas for deployment (10K KA = 0.00001 KARI)
        let deployment_gas = 10_000u64;
        
        let deployment_data = format!("VM_MODULE_DEPLOYMENT:{}:{}:{}", 
            address.to_hex(), 
            package_path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown"),
            deployment_gas
        );
        
        let tx_id = format!("deploy_{}_{}", address.to_hex(), timestamp);
        
        Ok(mona_blockchain::block::Transaction {
            transaction_id: tx_id,
            sender: mona_types::address::Address::from_hex_literal(&format!("0x{}", address.to_hex()))
                .map_err(|_| anyhow::anyhow!("Invalid address"))?,
            receiver: mona_types::address::Address::from_hex_literal(&format!("0x{}", address.to_hex()))
                .map_err(|_| anyhow::anyhow!("Invalid address"))?,
            amount: 0, // No token transfer for deployment
            gas_fee: deployment_gas,
            timestamp,
            signature: signature.unwrap_or_default(),
            data: Some(deployment_data.into_bytes()),
        })
    }
    
    fn execute_deployment(&self, package_path: PathBuf, address: AccountAddress, build_config: BuildConfig, signature: Option<Vec<u8>>, wallet_address: Option<String>) -> Result<()> {
        // Use minimal gas budget for actual VM deployment
        let reduced_gas = 10_000u64;
        let mona_vm_publish = mona_vm::Publish { signature, signer_address: wallet_address };
        mona_vm_publish.execute(Some(package_path), Some(address), build_config, Some(reduced_gas), self.skip_verify)
    }
    
    fn handle_result(&self, result: Result<()>, start_time: std::time::Instant, address: AccountAddress) -> Result<()> {
        let duration = start_time.elapsed();
        
        match result {
            Ok(()) => {
                println!("✅ Deployment successful in {:.2?}!", duration);
                self.display_deployed_modules(address);
                
                let result_json = json!({
                    "status": "success",
                    "address": format!("0x{}", address.to_hex()),
                    "deployment_time_ms": duration.as_millis(),
                    "gas_budget": self.gas_budget
                });
                println!("\nResult: {}", serde_json::to_string_pretty(&result_json)?);
                Ok(())
            },
            Err(e) => {
                println!("❌ Deployment failed after {:.2?}: {}", duration, e);
                Err(e)
            }
        }
    }
    
    fn display_deployed_modules(&self, address: AccountAddress) {
        if let Ok(vm_state) = VM_STATE.read() {
            let modules: Vec<_> = vm_state.modules.values().filter(|m| m.address == address).collect();
            if !modules.is_empty() {
                println!("\n✅ Deployed {} modules:", modules.len());
                for module in modules {
                    println!("  • {}: {} functions", module.name, module.public_functions.len());
                }
            }
        }
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

fn format_address(addr: &str) -> String {
    if addr.starts_with("0x") { addr.to_string() } else { format!("0x{}", addr) }
}
