// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use kanari_crypto::wallet::load_wallet;

#[derive(Parser, Debug)]
pub struct WalletInfo {
    /// Wallet address
    #[arg(short, long)]
    pub address: String,
    /// Password to decrypt wallet
    #[arg(short, long)]
    pub password: String,
    /// Show private key and seed phrase (dangerous!)
    #[arg(long, default_value = "true")]
    pub show_secrets: bool,
}

impl WalletInfo {
    pub fn execute(&self) -> Result<()> {
        let wallet = load_wallet(&self.address, &self.password).context("Failed to load wallet")?;

        eprintln!("\n╔════════════════════════════════════════════════════════════════╗");
        eprintln!("║              KANARI WALLET INFORMATION                         ║");
        eprintln!("╚════════════════════════════════════════════════════════════════╝\n");

        eprintln!("Address:");
        eprintln!("   0x{}\n", hex::encode(wallet.address.to_vec()));

        eprintln!("Cryptography:");
        eprintln!("   Algorithm: {}", wallet.curve_type);
        eprintln!(
            "   Security Level: {}/5",
            wallet.curve_type.security_level()
        );

        if wallet.curve_type.is_post_quantum() {
            if wallet.curve_type.is_hybrid() {
                eprintln!("   Type: Hybrid (Classical + Post-Quantum)");
                eprintln!("   Protection: Quantum-Safe + Classical Compatible");
            } else {
                eprintln!("   Type: Pure Post-Quantum Cryptography");
                eprintln!("   Protection: Quantum Computer Resistant");
            }
        } else {
            eprintln!("   Type: Classical Elliptic Curve Cryptography");
            eprintln!("   Protection: Vulnerable to future Quantum Computers");
        }

        if self.show_secrets {
            eprintln!("\nSENSITIVE INFORMATION (Keep Secret!):");
            eprintln!("─────────────────────────────────────────────────────────────────");
            eprintln!("Private Key:");
            eprintln!("   {}\n", wallet.private_key.as_str());

            if !wallet.seed_phrase.is_empty() {
                eprintln!("Seed Phrase (BIP39 Mnemonic):");
                eprintln!("   {}\n", wallet.seed_phrase.as_str());
            } else {
                eprintln!("Seed Phrase:");
                eprintln!("   Not stored for this wallet (created with direct random generation).");
                eprintln!("   All curves, including post-quantum ones, support BIP39 mnemonics.\n");
            }

            eprintln!("CRITICAL WARNING:");
            eprintln!("   NEVER share your private key or seed phrase with anyone!");
            eprintln!("   Anyone with this information can steal ALL your funds");
            eprintln!("   No legitimate service will ever ask for this information");
        } else {
            eprintln!("\nTip: Use --show-secrets to view private key and seed phrase");
            eprintln!("   Warning: Only use this in a secure, private environment");
        }

        eprintln!("\n════════════════════════════════════════════════════════════════\n");

        Ok(())
    }
}
