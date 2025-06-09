//! Enhanced Move CLI for Kanari blockchain with Kari gas support

use std::path::PathBuf;
use colored::Colorize;
use clap::{Args, Parser, Subcommand};
use log::{info, error};

use mona_types::address::Address;
use mona_vm::{MonaVM, VMConfig, GasParameters, VMStorage};
use mona_storage::{SmartContractStorage, SmartContractAddress, RocksDBStorage};
use mona_blockchain::block::{Transaction, TransactionType, ContractTransaction};

#[derive(Parser, Debug)]
#[command(name = "kari-move")]
#[command(about = "Kanari Move contract tools with Kari gas support")]
pub struct KariMoveCli {
    #[command(subcommand)]
    pub command: KariMoveCommand,
    
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
    
    #[arg(long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum KariMoveCommand {
    /// Deploy a Move smart contract
    Deploy(DeployArgs),
    /// Call a function on a deployed contract
    Call(CallArgs),
    /// Get contract information
    Info(InfoArgs),
    /// List deployed contracts
    List(ListArgs),
    /// Estimate gas cost for a contract operation
    EstimateGas(EstimateGasArgs),
    /// Get contract execution history
    History(HistoryArgs),
    /// Check Kari balance for gas payments
    Balance(BalanceArgs),
}

#[derive(Args, Debug)]
pub struct DeployArgs {
    /// Path to the Move source file
    #[arg(short, long)]
    pub source: PathBuf,
    
    /// Deployer address
    #[arg(short, long)]
    pub deployer: String,
    
    /// Gas limit for deployment
    #[arg(long, default_value = "1000000")]
    pub gas_limit: u64,
    
    /// Maximum Kari to spend on gas
    #[arg(long, default_value = "1000")]
    pub max_kari: u64,
    
    /// Module dependencies (comma-separated)
    #[arg(long)]
    pub dependencies: Option<String>,
    
    /// Private key for signing (optional, will prompt if not provided)
    #[arg(long)]
    pub private_key: Option<String>,
}

#[derive(Args, Debug)]
pub struct CallArgs {
    /// Contract address
    #[arg(short, long)]
    pub contract: String,
    
    /// Function name to call
    #[arg(short, long)]
    pub function: String,
    
    /// Function arguments (JSON format)
    #[arg(short, long)]
    pub args: Option<String>,
    
    /// Caller address
    #[arg(long)]
    pub caller: String,
    
    /// Gas limit for execution
    #[arg(long, default_value = "100000")]
    pub gas_limit: u64,
    
    /// Maximum Kari to spend on gas
    #[arg(long, default_value = "100")]
    pub max_kari: u64,
    
    /// Private key for signing
    #[arg(long)]
    pub private_key: Option<String>,
}

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// Contract address
    #[arg(short, long)]
    pub contract: String,
    
    /// Show detailed information including gas usage
    #[arg(long)]
    pub detailed: bool,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Show only contracts deployed by this address
    #[arg(long)]
    pub deployer: Option<String>,
    
    /// Limit number of results
    #[arg(long, default_value = "20")]
    pub limit: usize,
}

#[derive(Args, Debug)]
pub struct EstimateGasArgs {
    /// Contract address (for function calls)
    #[arg(short, long)]
    pub contract: Option<String>,
    
    /// Function name (for function calls)
    #[arg(short, long)]
    pub function: Option<String>,
    
    /// Function arguments (JSON format)
    #[arg(short, long)]
    pub args: Option<String>,
    
    /// Source file path (for deployment estimation)
    #[arg(long)]
    pub source: Option<PathBuf>,
    
    /// Caller address
    #[arg(long)]
    pub caller: String,
}

#[derive(Args, Debug)]
pub struct HistoryArgs {
    /// Contract address
    #[arg(short, long)]
    pub contract: String,
    
    /// Number of recent executions to show
    #[arg(long, default_value = "10")]
    pub limit: usize,
    
    /// Show only successful executions
    #[arg(long)]
    pub success_only: bool,
}

#[derive(Args, Debug)]
pub struct BalanceArgs {
    /// Address to check
    #[arg(short, long)]
    pub address: String,
}

/// Main CLI handler
pub async fn run_kari_move_cli() -> Result<(), Box<dyn std::error::Error>> {
    let cli = KariMoveCli::parse();
    
    if cli.verbose {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Debug)
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Info)
            .init();
    }

    // Initialize storage
    let data_dir = cli.data_dir.unwrap_or_else(|| {
        dirs::home_dir().unwrap_or_default().join(".kanari")
    });
    
    let db_path = data_dir.join("contracts_db");
    let storage = std::sync::Arc::new(RocksDBStorage::new(db_path)?);
    let smart_contract_storage = std::sync::Arc::new(
        SmartContractStorage::new(storage, 10 * 1024 * 1024) // 10MB cache
    );

    // Initialize VM
    let gas_params = std::sync::Arc::new(GasParameters::default());
    let vm_storage = std::sync::Arc::new(VMStorage::new(smart_contract_storage.clone()));
    let vm_config = VMConfig {
        max_gas_per_txn: 10_000_000,
        max_memory_per_txn: 1024 * 1024,
        max_stack_depth: 1000,
        enable_tracing: true,
        cache_size: 10 * 1024 * 1024,
    };
    let vm = std::sync::Arc::new(MonaVM::new(vm_config, gas_params, vm_storage)?);

    match cli.command {
        KariMoveCommand::Deploy(args) => {
            handle_deploy(args, vm, smart_contract_storage).await?;
        },
        KariMoveCommand::Call(args) => {
            handle_call(args, vm, smart_contract_storage).await?;
        },
        KariMoveCommand::Info(args) => {
            handle_info(args, smart_contract_storage).await?;
        },
        KariMoveCommand::List(args) => {
            handle_list(args, smart_contract_storage).await?;
        },
        KariMoveCommand::EstimateGas(args) => {
            handle_estimate_gas(args, vm, smart_contract_storage).await?;
        },
        KariMoveCommand::History(args) => {
            handle_history(args, smart_contract_storage).await?;
        },
        KariMoveCommand::Balance(args) => {
            handle_balance(args).await?;
        },
    }

    Ok(())
}

async fn handle_deploy(
    args: DeployArgs,
    vm: std::sync::Arc<MonaVM>,
    storage: std::sync::Arc<SmartContractStorage>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Deploying Move contract...".bright_cyan().bold());
    
    // Read source code
    let source_code = std::fs::read_to_string(&args.source)?;
    
    // Parse deployer address
    let deployer = Address::from_str(&args.deployer)?;
    
    // Parse dependencies
    let dependencies = args.dependencies
        .map(|d| d.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    // Deploy contract
    let deployment_info = vm.deploy_contract(
        source_code,
        dependencies,
        deployer,
        args.gas_limit,
    )?;

    println!("✅ {}", "Contract deployed successfully!".bright_green().bold());
    println!("📍 Contract Address: {}", deployment_info.contract_address.to_hex_literal().bright_yellow());
    println!("⛽ Gas Used: {} (≈ {} Kari)", deployment_info.gas_used, deployment_info.gas_used);
    println!("📝 Module: {}", deployment_info.module_name.bright_white());
    println!("🕒 Deployed At: {}", chrono::DateTime::from_timestamp(deployment_info.timestamp as i64, 0)
        .unwrap_or_default().format("%Y-%m-%d %H:%M:%S UTC"));

    Ok(())
}

async fn handle_call(
    args: CallArgs,
    vm: std::sync::Arc<MonaVM>,
    storage: std::sync::Arc<SmartContractStorage>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "Calling contract function...".bright_cyan().bold());
    
    // Parse addresses
    let contract_address = Address::from_str(&args.contract)?;
    let caller = Address::from_str(&args.caller)?;
    
    // Parse arguments
    let arguments = if let Some(args_json) = args.args {
        // Parse JSON arguments (simplified for now)
        serde_json::from_str::<Vec<Vec<u8>>>(&args_json)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // Execute function call
    let result = vm.call_function(
        contract_address,
        args.function.clone(),
        arguments,
        caller,
        args.gas_limit,
    )?;

    if result.success {
        println!("✅ {}", "Function call successful!".bright_green().bold());
        println!("⛽ Gas Used: {} (≈ {} Kari)", result.gas_used, result.gas_used);
        println!("📊 Return Values: {} items", result.return_values.len());
        
        if !result.events.is_empty() {
            println!("🎭 Events Emitted: {}", result.events.len());
            for (i, event) in result.events.iter().enumerate() {
                println!("  {}. {} ({}B data)", i + 1, event.event_type, event.data.len());
            }
        }
    } else {
        println!("❌ {}", "Function call failed!".bright_red().bold());
        if let Some(error) = result.error_message {
            println!("💥 Error: {}", error.bright_red());
        }
    }

    Ok(())
}

async fn handle_info(
    args: InfoArgs,
    storage: std::sync::Arc<SmartContractStorage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let contract_address = SmartContractAddress::from_hex(&args.contract)?;
    
    if let Some(metadata) = storage.load_contract_metadata(&contract_address)? {
        println!("📋 {}", "Contract Information".bright_cyan().bold());
        println!("📍 Address: {}", contract_address.to_hex_literal().bright_yellow());
        println!("📝 Name: {}", metadata.name.bright_white());
        println!("🔖 Version: {}", metadata.version);
        println!("👤 Deployer: {}", metadata.deployer.to_hex_literal());
        println!("🕒 Deployed: {}", chrono::DateTime::from_timestamp(metadata.deployed_at as i64, 0)
            .unwrap_or_default().format("%Y-%m-%d %H:%M:%S UTC"));
        println!("💾 Bytecode Size: {} bytes", metadata.bytecode_size);
        println!("🔧 Compiler: {}", metadata.compiler_version);
        
        if args.detailed {
            // Get gas usage statistics
            let current_time = chrono::Utc::now().timestamp() as u64;
            let one_day_ago = current_time - 86400;
            let kari_spent = storage.get_contract_kari_spent(&contract_address, one_day_ago, current_time)?;
            let storage_size = storage.get_contract_storage_size(&contract_address)?;
            
            println!("💰 Kari Spent (24h): {}", kari_spent);
            println!("🗄️ Storage Size: {} bytes", storage_size);
        }
    } else {
        println!("❌ Contract not found at address: {}", args.contract);
    }

    Ok(())
}

async fn handle_list(
    args: ListArgs,
    storage: std::sync::Arc<SmartContractStorage>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "📋 Deployed Contracts".bright_cyan().bold());
    
    // This is a simplified implementation
    // In a real system, you'd need an index of deployed contracts
    println!("ℹ️  Contract listing feature coming soon...");
    println!("💡 Use 'kari move info --contract <address>' to check specific contracts");
    
    Ok(())
}

async fn handle_estimate_gas(
    args: EstimateGasArgs,
    vm: std::sync::Arc<MonaVM>,
    storage: std::sync::Arc<SmartContractStorage>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "⛽ Estimating gas cost...".bright_cyan().bold());
    
    let caller = Address::from_str(&args.caller)?;
    
    if let (Some(contract), Some(function)) = (args.contract, args.function) {
        // Estimate for function call
        let contract_address = Address::from_str(&contract)?;
        let arguments = if let Some(args_json) = args.args {
            serde_json::from_str::<Vec<Vec<u8>>>(&args_json).unwrap_or_default()
        } else {
            Vec::new()
        };
        
        let estimated_gas = vm.estimate_gas(contract_address, function, arguments, caller)?;
        println!("⛽ Estimated Gas: {}", estimated_gas);
        println!("💰 Estimated Kari Cost: {}", estimated_gas);
    } else if let Some(source) = args.source {
        // Estimate for deployment
        println!("🚧 Deployment gas estimation coming soon...");
    } else {
        println!("❌ Please specify either contract call or source file for estimation");
    }
    
    Ok(())
}

async fn handle_history(
    args: HistoryArgs,
    storage: std::sync::Arc<SmartContractStorage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let contract_address = SmartContractAddress::from_hex(&args.contract)?;
    
    println!("{}", "📜 Contract Execution History".bright_cyan().bold());
    println!("ℹ️  Execution history feature coming soon...");
    
    Ok(())
}

async fn handle_balance(args: BalanceArgs) -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", "💰 Checking Kari balance...".bright_cyan().bold());
    println!("ℹ️  Balance checking feature coming soon...");
    println!("💡 Use existing Kanari wallet tools to check balance");
    
    Ok(())
}
