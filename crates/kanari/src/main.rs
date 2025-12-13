use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use kanari_crypto::{
    keys::{CurveType, generate_keypair, generate_mnemonic, keypair_from_mnemonic},
    wallet::{Wallet, list_wallet_files, load_wallet, save_wallet, set_selected_wallet},
};
use kanari_move_runtime::SignedTransaction;
use kanari_rpc_client::RpcClient;
use move_core_types::account_address::AccountAddress;
use std::str::FromStr;

pub mod command;
use command::move_cli;

/// Kanari - A Move-based money transfer system
#[derive(Parser)]
#[command(name = "kanari")]
#[command(about = "Money transfer system using Move language", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new wallet with kanari-crypto
    CreateWallet {
        /// Password for wallet encryption
        #[arg(short, long)]
        password: String,
        /// Curve type (ed25519, k256, p256, dilithium2, dilithium3, dilithium5, sphincs+, ed25519+dilithium3, k256+dilithium3)
        #[arg(short, long, default_value = "ed25519")]
        curve: String,
        /// Number of seed words (12 or 24)
        #[arg(short, long, default_value = "12")]
        words: usize,
    },
    /// Load an existing wallet
    LoadWallet {
        /// Wallet address to load
        #[arg(short, long)]
        address: String,
        /// Password to decrypt wallet
        #[arg(short, long)]
        password: String,
    },
    /// List all wallets with balances
    ListWallets,
    /// Show detailed wallet information
    WalletInfo {
        /// Wallet address
        #[arg(short, long)]
        address: String,
        /// Password to decrypt wallet
        #[arg(short, long)]
        password: String,
        /// Show private key and seed phrase (dangerous!)
        #[arg(long, default_value = "false")]
        show_secrets: bool,
    },
    /// Transfer Kanari tokens to another address
    Transfer {
        /// Sender wallet address (optional). If omitted, uses selected wallet in config.
        #[arg(short, long)]
        from: Option<String>,
        /// Recipient address
        #[arg(short, long)]
        to: String,
        /// Amount in Kanari (will be converted to Mist)
        #[arg(short, long)]
        amount: f64,
        /// Wallet password
        #[arg(short, long)]
        password: String,
    },
    /// Burn Kanari tokens from a wallet (remove from total supply)
    Burn {
        /// Wallet address to burn from (optional). If omitted, uses selected wallet in config.
        #[arg(short, long)]
        from: Option<String>,
        /// Amount in Kanari to burn
        #[arg(short, long)]
        amount: f64,
        /// Wallet password
        #[arg(short, long)]
        password: String,
    },
    /// Check wallet balance
    Balance {
        /// Wallet address
        #[arg(short, long)]
        address: String,
    },
    /// Show blockchain statistics
    Stats,
    /// Show token balances for an address
    Balances(command::balances::Balances),
    /// Account operations (get account info)
    Account {
        #[command(subcommand)]
        command: command::account::AccountCommand,
    },
    /// Manage Move packages and tools
    Move {
        #[command(subcommand)]
        command: move_cli::MoveCommand,
    },
    /// Import an existing wallet from private key or seed phrase
    AddWallet {
        /// Import using a raw private key (hex with or without kanari prefix)
        #[arg(long)]
        private_key: Option<String>,
        /// Import using a BIP39 seed phrase
        #[arg(long)]
        seed: Option<String>,
        /// Password for wallet encryption
        #[arg(short, long)]
        password: String,
        /// Curve type (supports classical and PQC private-key imports: ed25519, k256, p256, dilithium2, dilithium3, dilithium5, sphincs+)
        #[arg(short, long, default_value = "ed25519")]
        curve: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Use tokio runtime for async RPC calls
    let runtime = tokio::runtime::Runtime::new()?;

    match cli.command {
        Commands::CreateWallet {
            password,
            curve,
            words,
        } => {
            let curve_type = match curve.to_lowercase().as_str() {
                "ed25519" => CurveType::Ed25519,
                "k256" | "secp256k1" => CurveType::K256,
                "p256" | "secp256r1" => CurveType::P256,
                "dilithium2" => CurveType::Dilithium2,
                "dilithium3" => CurveType::Dilithium3,
                "dilithium5" => CurveType::Dilithium5,
                "sphincs+" | "sphincsplus" => CurveType::SphincsPlusSha256Robust,
                "ed25519+dilithium3" | "ed25519_dilithium3" => CurveType::Ed25519Dilithium3,
                "k256+dilithium3" | "k256_dilithium3" => CurveType::K256Dilithium3,
                other => {
                    println!("Unknown curve '{}', falling back to Ed25519", other);
                    CurveType::Ed25519
                }
            };

            // For classical curves we can derive from a mnemonic; for PQC/hybrid generate directly
            let (private_key, address_str, seed_phrase) = if curve_type.is_post_quantum()
                || curve_type.is_hybrid()
            {
                let kp = generate_keypair(curve_type).context("Failed to generate keypair")?;
                let zk = kp.export_private_key_secure();
                (zk.to_string(), kp.get_address().to_string(), String::new())
            } else {
                let mnemonic = generate_mnemonic(words).context("Failed to generate mnemonic")?;
                let kp = keypair_from_mnemonic(&mnemonic, curve_type, "")
                    .context("Failed to derive keypair from mnemonic")?;
                let zk = kp.export_private_key_secure();
                (zk.to_string(), kp.get_address().to_string(), mnemonic)
            };

            let address =
                AccountAddress::from_str(&address_str).context("Generated invalid address")?;

            // Save wallet
            save_wallet(
                &address,
                &private_key,
                &seed_phrase,
                None,
                &password,
                curve_type,
            )
            .context("Failed to save wallet")?;

            println!("Created wallet: {}", address_str);
            if !seed_phrase.is_empty() {
                println!("Seed phrase: {}", seed_phrase);
            }

            Ok(())
        }

        Commands::AddWallet {
            private_key,
            seed,
            password,
            curve,
        } => {
            let curve_type = match curve.to_lowercase().as_str() {
                "ed25519" => CurveType::Ed25519,
                "k256" | "secp256k1" => CurveType::K256,
                "p256" | "secp256r1" => CurveType::P256,
                "dilithium2" => CurveType::Dilithium2,
                "dilithium3" => CurveType::Dilithium3,
                "dilithium5" => CurveType::Dilithium5,
                "sphincs+" | "sphincsplus" => CurveType::SphincsPlusSha256Robust,
                "ed25519+dilithium3" | "ed25519_dilithium3" => CurveType::Ed25519Dilithium3,
                "k256+dilithium3" | "k256_dilithium3" => CurveType::K256Dilithium3,
                other => {
                    println!("Unknown curve '{}', falling back to Ed25519", other);
                    CurveType::Ed25519
                }
            };

            if private_key.is_none() && seed.is_none() {
                return Err(anyhow::anyhow!(
                    "Please provide either --private-key or --seed to import a wallet"
                ));
            }

            if let Some(pk) = private_key {
                let (privk, _pubk, address_str) =
                    kanari_crypto::keys::import_from_private_key(&pk, curve_type)
                        .map_err(|e| anyhow::anyhow!("Import from private key failed: {}", e))?;

                let address =
                    AccountAddress::from_str(&address_str).context("Generated invalid address")?;

                save_wallet(&address, &privk, "", None, &password, curve_type)
                    .context("Failed to save imported private-key wallet")?;

                println!("Imported wallet from private key: {}", address_str);
            } else if let Some(seed_phrase) = seed {
                // Importing from BIP39 seed phrases only works for classical curves.
                if curve_type.is_post_quantum() || curve_type.is_hybrid() {
                    return Err(anyhow::anyhow!(
                        "Import from seed phrase is not supported for post-quantum or hybrid curves; use CreateWallet to generate such keys"
                    ));
                }
                let (privk, _pubk, address_str) =
                    kanari_crypto::keys::import_from_seed_phrase(&seed_phrase, curve_type)
                        .map_err(|e| anyhow::anyhow!("Import from seed phrase failed: {}", e))?;

                let address =
                    AccountAddress::from_str(&address_str).context("Generated invalid address")?;

                save_wallet(&address, &privk, &seed_phrase, None, &password, curve_type)
                    .context("Failed to save imported seed wallet")?;

                println!("Imported wallet from seed phrase: {}", address_str);
            }

            Ok(())
        }

        Commands::LoadWallet { address, password } => {
            let wallet: Wallet =
                load_wallet(&address, &password).context("Failed to load wallet")?;
            println!("Wallet loaded: {} (curve: {})", address, wallet.curve_type);

            // Mark this wallet as selected in the kanari config so `list-wallets`
            // shows the expected selected address.
            set_selected_wallet(&address).context("Failed to set selected wallet")?;
            println!("Selected wallet: {}", address);

            Ok(())
        }

        Commands::ListWallets => {
            let wallets = list_wallet_files().context("Failed to list wallets")?;
            println!("Found {} wallets", wallets.len());
            if wallets.is_empty() {
                println!("No wallets found.");
            } else {
                for (addr, selected) in wallets {
                    if selected {
                        println!("- {}  (selected)", addr);
                    } else {
                        println!("- {}", addr);
                    }
                }
            }
            Ok(())
        }

        Commands::WalletInfo {
            address,
            password,
            show_secrets,
        } => {
            let wallet = load_wallet(&address, &password).context("Failed to load wallet")?;

            println!("\n╔════════════════════════════════════════════════════════════════╗");
            println!("║              KANARI WALLET INFORMATION                         ║");
            println!("╚════════════════════════════════════════════════════════════════╝\n");

            println!("Address:");
            println!("   0x{}\n", hex::encode(wallet.address.to_vec()));

            println!("Cryptography:");
            println!("   Algorithm: {}", wallet.curve_type);
            println!(
                "   Security Level: {}/5",
                wallet.curve_type.security_level()
            );

            if wallet.curve_type.is_post_quantum() {
                if wallet.curve_type.is_hybrid() {
                    println!("   Type: Hybrid (Classical + Post-Quantum)");
                    println!("   Protection: Quantum-Safe + Classical Compatible");
                } else {
                    println!("   Type: Pure Post-Quantum Cryptography");
                    println!("   Protection: Quantum Computer Resistant");
                }
            } else {
                println!("   Type: Classical Elliptic Curve Cryptography");
                println!("   Protection: Vulnerable to future Quantum Computers");
            }

            if show_secrets {
                println!("\nSENSITIVE INFORMATION (Keep Secret!):");
                println!("─────────────────────────────────────────────────────────────────");
                println!("Private Key:");
                println!("   {}\n", wallet.private_key.as_str());

                if !wallet.seed_phrase.is_empty() {
                    println!("Seed Phrase (BIP39 Mnemonic):");
                    println!("   {}\n", wallet.seed_phrase.as_str());
                } else {
                    println!("Seed Phrase:");
                    println!("   Not available - Post-Quantum keys use direct generation");
                    println!("   PQC algorithms don't support BIP39/BIP32 derivation\n");
                }

                println!("CRITICAL WARNING:");
                println!("   NEVER share your private key or seed phrase with anyone!");
                println!("   Anyone with this information can steal ALL your funds");
                println!("   No legitimate service will ever ask for this information");
            } else {
                println!("\nTip: Use --show-secrets to view private key and seed phrase");
                println!("   Warning: Only use this in a secure, private environment");
            }

            println!("\n════════════════════════════════════════════════════════════════\n");

            Ok(())
        }

        Commands::Transfer {
            from,
            to,
            amount,
            password,
        } => {
            runtime.block_on(async {
                // Load sender wallet to verify ownership
                        // Determine sender: prefer explicit `--from`, otherwise use selected wallet
                        let from_addr = if let Some(f) = from.clone() { f } else {
                            kanari_crypto::wallet::get_selected_wallet()
                                .ok_or_else(|| anyhow::anyhow!("No sender provided and no selected wallet set. Use --from or run `kanari load-wallet` to select one."))?
                        };

                        let wallet =
                            load_wallet(&from_addr, &password).context("Failed to load sender wallet")?;

                println!("Transferring Kanari tokens...");
                println!("  From: {}", from_addr);
                println!("  To: {}", to);
                println!("  Amount: {} KANARI", amount);

                // Convert Kanari to Mist (1 KANARI = 10^9 Mist)
                // Use rounding to avoid floating-point truncation artifacts
                const MIST_PER_KANARI: f64 = 1_000_000_000.0;
                let amount_mist_f = amount * MIST_PER_KANARI;
                let amount_mist = amount_mist_f.round() as u64;
                println!("  Amount (Mist): {}", amount_mist);

                // Connect to RPC server instead of creating engine
                let client = RpcClient::new("http://127.0.0.1:3000");

                // Get current block height to verify connection
                match client.get_block_height().await {
                    Ok(height) => println!("  Connected to node (height: {})", height),
                    Err(_) => {
                        eprintln!("  Cannot connect to RPC server at http://127.0.0.1:3000");
                        eprintln!("  Please start the node first: cargo run --bin kanari-node");
                        return Err(anyhow::anyhow!("RPC server not available"));
                    }
                }

                // Get account to get sequence number before creating the transaction
                let account = client
                    .get_account(&from_addr)
                    .await
                    .context("Failed to get sender account")?;

                // Create and sign transaction (include sequence number so signature matches server verification)
                let tx = kanari_move_runtime::Transaction::Transfer {
                    from: from_addr.clone(),
                    to: to.clone(),
                    amount: amount_mist,
                    gas_limit: 100_000,
                    gas_price: 1000,
                    sequence_number: account.sequence_number,
                };

                println!("  Gas Limit: {}", tx.gas_limit());
                println!("  Gas Price: {} Mist/gas", tx.gas_price());

                // Sign transaction with wallet private key
                let mut signed_tx = SignedTransaction::new(tx);
                signed_tx
                    .sign(&wallet.private_key, wallet.curve_type)
                    .context("Failed to sign transaction")?;
                println!("  Transaction signed");

                println!("  Submitting transaction to node...");

                // Convert SignedTransaction to RPC format
                use kanari_rpc_api::SignedTransactionData;
                let tx_data = SignedTransactionData {
                    sender: from_addr.clone(),
                    recipient: Some(to.clone()),
                    amount: Some(amount_mist),
                    gas_limit: signed_tx.transaction.gas_limit(),
                    gas_price: signed_tx.transaction.gas_price(),
                    sequence_number: account.sequence_number,
                    signature: signed_tx.signature.clone(),
                };

                // Submit transaction via RPC
                match client.submit_transaction(tx_data).await {
                    Ok(status) => {
                        println!("  Transaction submitted successfully");
                        println!("  Transaction hash: {}", status.hash);
                        println!("  Status: {}", status.status);
                        println!("  Waiting for block confirmation...");
                        println!(
                            "  Check balance with: cargo run --bin kanari balance --address {}",
                            to
                        );
                    }
                    Err(e) => {
                        eprintln!("  Failed to submit transaction: {}", e);
                        return Err(e);
                    }
                }

                Ok::<(), anyhow::Error>(())
            })?;

            Ok(())
        }

        Commands::Burn {
            from,
            amount,
            password,
        } => {
            runtime.block_on(async {
                // Determine sender: prefer explicit `--from`, otherwise use selected wallet
                let from_addr = if let Some(f) = from.clone() { f } else {
                    kanari_crypto::wallet::get_selected_wallet()
                        .ok_or_else(|| anyhow::anyhow!("No sender provided and no selected wallet set. Use --from or run `kanari load-wallet` to select one."))?
                };

                let wallet = load_wallet(&from_addr, &password).context("Failed to load sender wallet")?;

                println!("Burning Kanari tokens...");
                println!("  From: {}", from_addr);
                println!("  Amount: {} KANARI", amount);

                // Convert Kanari to Mist (1 KANARI = 10^9 Mist)
                const MIST_PER_KANARI: f64 = 1_000_000_000.0;
                let amount_mist_f = amount * MIST_PER_KANARI;
                let amount_mist = amount_mist_f.round() as u64;
                println!("  Amount (Mist): {}", amount_mist);

                // Connect to RPC server
                let client = RpcClient::new("http://127.0.0.1:3000");

                match client.get_block_height().await {
                    Ok(height) => println!("  Connected to node (height: {})", height),
                    Err(_) => {
                        eprintln!("  Cannot connect to RPC server at http://127.0.0.1:3000");
                        eprintln!("  Please start the node first: cargo run --bin kanari-node");
                        return Err(anyhow::anyhow!("RPC server not available"));
                    }
                }

                // Get account to get sequence number
                let account = client
                    .get_account(&from_addr)
                    .await
                    .context("Failed to get sender account")?;

                // Create burn transaction
                let tx = kanari_move_runtime::Transaction::Burn {
                    from: from_addr.clone(),
                    amount: amount_mist,
                    gas_limit: 100_000,
                    gas_price: 1000,
                    sequence_number: account.sequence_number,
                };

                println!("  Gas Limit: {}", tx.gas_limit());
                println!("  Gas Price: {} Mist/gas", tx.gas_price());

                // Sign transaction
                let mut signed_tx = SignedTransaction::new(tx);
                signed_tx
                    .sign(&wallet.private_key, wallet.curve_type)
                    .context("Failed to sign transaction")?;
                println!("  Transaction signed");

                println!("  Submitting burn transaction to node...");

                use kanari_rpc_api::SignedTransactionData;
                let tx_data = SignedTransactionData {
                    sender: from_addr.clone(),
                    recipient: None,
                    amount: Some(amount_mist),
                    gas_limit: signed_tx.transaction.gas_limit(),
                    gas_price: signed_tx.transaction.gas_price(),
                    sequence_number: account.sequence_number,
                    signature: signed_tx.signature.clone(),
                };

                match client.submit_transaction(tx_data).await {
                    Ok(status) => {
                        println!("  Burn transaction submitted successfully");
                        println!("  Transaction hash: {}", status.hash);
                        println!("  Status: {}", status.status);
                        println!("  Waiting for block confirmation...");
                    }
                    Err(e) => {
                        eprintln!("  Failed to submit burn transaction: {}", e);
                        return Err(e);
                    }
                }

                Ok::<(), anyhow::Error>(())
            })?;

            Ok(())
        }

        Commands::Balance { address } => {
            runtime.block_on(async {
                let client = RpcClient::new("http://127.0.0.1:3000");

                match client.get_account(&address).await {
                    Ok(account) => {
                        const MIST_PER_KANARI: f64 = 1_000_000_000.0;
                        let balance_kanari = account.balance as f64 / MIST_PER_KANARI;

                        println!("Balance for {}", address);
                        println!("  Kanari: {:.9} KANARI", balance_kanari);
                        println!("  Mist: {} Mist", account.balance);
                        println!("  Sequence: {}", account.sequence_number);
                        if !account.modules.is_empty() {
                            println!("  Modules deployed: {}", account.modules.len());
                        }
                    }
                    Err(e) => {
                        if e.to_string().contains("Account not found") {
                            println!("Account not found: {}", address);
                            println!("   This address has no transactions yet.");
                        } else {
                            eprintln!("  Cannot connect to RPC server");
                            eprintln!("  Please start the node first: cargo run --bin kanari-node");
                            return Err(e);
                        }
                    }
                }

                Ok::<(), anyhow::Error>(())
            })?;

            Ok(())
        }

        Commands::Stats => {
            runtime.block_on(async {
                let client = RpcClient::new("http://127.0.0.1:3000");

                match client.get_stats().await {
                    Ok(stats) => {
                        const MIST_PER_KANARI: f64 = 1_000_000_000.0;
                        let total_supply_kanari = stats.total_supply as f64 / MIST_PER_KANARI;

                        println!("Kanari Blockchain Statistics");
                        println!("------------------------------");
                        println!("  Block Height: {}", stats.height);
                        println!("  Total Blocks: {}", stats.total_blocks);
                        println!("  Total Transactions: {}", stats.total_transactions);
                        println!("  Pending Transactions: {}", stats.pending_transactions);
                        println!("  Total Accounts: {}", stats.total_accounts);
                        println!("  Total Supply: {:.0} KANARI", total_supply_kanari);
                        println!("─────────────────────────────────");
                    }
                    Err(_) => {
                        eprintln!("  Cannot connect to RPC server at http://127.0.0.1:3000");
                        eprintln!("  Please start the node first: cargo run --bin kanari-node");
                        return Err(anyhow::anyhow!("RPC server not available"));
                    }
                }

                Ok::<(), anyhow::Error>(())
            })?;

            Ok(())
        }

        Commands::Balances(balances) => {
            balances.execute().context("Failed to query balances")?;
            Ok(())
        }

        Commands::Account { command } => {
            command.execute().context("Failed to query account")?;
            Ok(())
        }

        Commands::Move { command } => {
            // Dispatch into the move CLI helper
            command
                .execute()
                .context("Failed to execute move subcommand")?;

            Ok(())
        }
    }
}
