use anyhow::Result;
use clap::Parser;
use move_core_types::account_address::AccountAddress;
use move_package::BuildConfig;
use serde_json::json;
use std::path::PathBuf;

use mona_vm::{VM_STATE, Publish as MonaVMPublish};
use common::*;
use mona_crypto::{load_wallet, WalletError};
use sha3::{Digest, Sha3_256};

#[derive(Parser)]
#[clap(about = "Publish Move modules to blockchain network")]
pub struct Publish {
    #[clap(long, help = "Directory path containing the Move package to publish")]
    pub module_path: PathBuf,

    #[clap(long, default_value = "3000000", help = "Amount of gas units allocated for deployment")]
    pub gas_budget: u64,

    #[clap(long, help = "Skip module verification (not recommended for production)")]
    pub skip_verify: bool,
    
    #[clap(long, help = "Blockchain address to deploy the module to (format: 0x...)")]
    pub address: Option<AccountAddress>,

    #[clap(long, help = "Password for wallet to sign deployment transaction")]
    pub password: Option<String>,
}

impl Publish {
    pub fn execute(self, path: Option<PathBuf>, config: BuildConfig) -> Result<()> {
        let package_path = path.unwrap_or_else(|| self.module_path.clone());
        
        // Validate package structure
        self.validate_package(&package_path)?;
        
        // Get deployment address
        let address = self.get_deployment_address()?;
        
        // Prepare build config
        let mut build_config = config;
        build_config.additional_named_addresses.insert("module_addr".to_string(), address);
        
        self.display_deployment_info(&package_path, &address);
        
        // Sign deployment transaction
        let (signature, wallet_address) = self.sign_deployment(&address, &package_path)?;
        
        // Execute deployment
        self.execute_deployment(package_path, address, build_config, signature, wallet_address)
    }
    
    fn validate_package(&self, package_path: &PathBuf) -> Result<()> {
        if !package_path.exists() {
            return Err(anyhow::anyhow!("Package path does not exist: {}", package_path.display()));
        }
        
        let sources_dir = package_path.join("sources");
        if !sources_dir.exists() {
            return Err(anyhow::anyhow!("No 'sources' directory found at: {}", package_path.display()));
        }
        
        // Check for .move files
        let has_move_files = std::fs::read_dir(&sources_dir)?
            .filter_map(Result::ok)
            .any(|entry| entry.path().extension().map_or(false, |ext| ext == "move"));
            
        if !has_move_files {
            return Err(anyhow::anyhow!("No .move files found in sources directory"));
        }
        
        println!("✅ Package validation successful");
        Ok(())
    }
    
    fn get_deployment_address(&self) -> Result<AccountAddress> {
        Ok(self.address.unwrap_or_else(|| {
            get_main_wallet()
                .and_then(|wallet| {
                    let wallet_addr = if wallet.starts_with("0x") { wallet } else { format!("0x{}", wallet) };
                    AccountAddress::from_hex_literal(&wallet_addr).ok()
                })
                .unwrap_or_else(|| AccountAddress::from_hex_literal("0x1").unwrap())
        }))
    }
    
    fn display_deployment_info(&self, package_path: &PathBuf, address: &AccountAddress) {
        println!("Publishing to blockchain network...");
        println!("📦 Package: {}", package_path.display());
        println!("🔑 Address: 0x{}", address.to_hex());
        println!("⛽ Gas budget: {}", self.gas_budget);
        if self.skip_verify {
            println!("⚠️ Verification: SKIPPED");
        }
    }
    
    fn sign_deployment(&self, address: &AccountAddress, package_path: &PathBuf) -> Result<(Option<Vec<u8>>, Option<String>)> {
        let Some(wallet_addr) = get_main_wallet() else {
            if self.address.is_some() {
                println!("ℹ️ No wallet configured. Publishing without signature.");
                return Ok((None, None));
            } else {
                return Err(anyhow::anyhow!("No wallet configured and no address specified. Run: kari wallet create"));
            }
        };
        
        // Create deployment payload
        let mut hasher = Sha3_256::new();
        hasher.update(address.to_hex().as_bytes());
        hasher.update(package_path.to_str().unwrap_or("").as_bytes());
        hasher.update(self.gas_budget.to_le_bytes());
        hasher.update(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs().to_le_bytes());
        let payload_to_sign = hasher.finalize();
        
        // Get password and sign
        let password = self.password.clone().unwrap_or_else(|| {
            println!("Enter password for wallet {}: ", wallet_addr);
            rpassword::read_password().unwrap_or_default()
        });
        
        match load_wallet(&wallet_addr, &password) {
            Ok(wallet) => match wallet.sign(&payload_to_sign, &password) {
                Ok(sig) => {
                    println!("✅ Deployment transaction signed successfully");
                    Ok((Some(sig), Some(wallet_addr)))
                },
                Err(e) => Err(anyhow::anyhow!("Transaction signing failed: {}", e))
            },
            Err(WalletError::InvalidPassword) => Err(anyhow::anyhow!("Invalid wallet password")),
            Err(e) => Err(anyhow::anyhow!("Wallet loading failed: {}", e))
        }
    }
    
    fn execute_deployment(&self, package_path: PathBuf, address: AccountAddress, build_config: BuildConfig, signature: Option<Vec<u8>>, wallet_address: Option<String>) -> Result<()> {
        let start_time = std::time::Instant::now();
        
        println!("\n⏳ Compiling and deploying modules...");
        
        let mona_vm_publish = MonaVMPublish { signature, signer_address: wallet_address };
        
        // Execute deployment with timeout handling
        let result = std::thread::scope(|s| {
            let handle = s.spawn(|| {
                mona_vm_publish.execute(
                    Some(package_path.clone()),
                    Some(address),
                    build_config,
                    Some(self.gas_budget),
                    self.skip_verify
                )
            });
            
            // Simple timeout check
            let timeout = std::time::Duration::from_secs(30);
            let mut elapsed = std::time::Duration::ZERO;
            
            while elapsed < timeout && !handle.is_finished() {
                std::thread::sleep(std::time::Duration::from_secs(2));
                elapsed += std::time::Duration::from_secs(2);
                if elapsed.as_secs() % 10 == 0 {
                    println!("⏳ Still deploying... ({:?} elapsed)", elapsed);
                }
            }
            
            if handle.is_finished() {
                handle.join().unwrap()
            } else {
                Err(anyhow::anyhow!("Deployment timed out after {:?}", timeout))
            }
        });
        
        match result {
            Ok(()) => {
                let duration = start_time.elapsed();
                println!("✅ Deployment successful in {:.2?}", duration);
                
                // Display deployed modules
                if let Ok(vm_state) = VM_STATE.read() {
                    let modules: Vec<_> = vm_state.modules.values()
                        .filter(|m| m.address == address)
                        .collect();
                    
                    if !modules.is_empty() {
                        println!("\n📦 Deployed {} modules:", modules.len());
                        for module in modules {
                            println!("  • {} ({})", module.name, module.module_id);
                            println!("    Functions: {}", module.public_functions.join(", "));
                        }
                    }
                }
                
                let result_json = json!({
                    "status": "success",
                    "address": format!("0x{}", address.to_hex()),
                    "deployment_time_ms": duration.as_millis(),
                    "gas_budget": self.gas_budget
                });
                
                println!("\nDeployment Result: {}", serde_json::to_string_pretty(&result_json)?);
                Ok(())
            },
            Err(e) => {
                let duration = start_time.elapsed();
                println!("\n❌ Deployment failed after {:.2?}: {}", duration, e);
                
                let error_json = json!({
                    "status": "error",
                    "message": e.to_string(),
                    "address": format!("0x{}", address.to_hex()),
                    "elapsed_time_ms": duration.as_millis()
                });
                
                println!("Error Details: {}", serde_json::to_string_pretty(&error_json)?);
                Err(e)
            }
        }
    }
}
