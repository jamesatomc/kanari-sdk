// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use clap::Parser;
use kanari_crypto::hd_wallet::derive_keypair_from_path;
use kanari_crypto::keys::{CurveType, ImportedWallet};
use kanari_crypto::wallet::save_wallet;
use kanari_types::error::KanariUnwrapExt;
use move_core_types::account_address::AccountAddress;
use std::str::FromStr;

#[derive(Parser, Debug)]
pub struct AddWallet {
    /// Import using a raw private key (hex with or without kanari prefix)
    #[arg(long)]
    pub private_key: Option<String>,
    /// Import using a BIP39 seed phrase
    #[arg(long)]
    pub seed: Option<String>,
    /// Password for wallet encryption
    #[arg(short, long)]
    pub password: String,
    /// Curve type (ed25519, k256, p256, ed25519+dilithium3, k256+dilithium3)
    #[arg(short, long, default_value = "ed25519")]
    pub curve: String,
    /// BIP32 derivation path (default: m/44'/637'/0'/0/0)
    #[arg(long, default_value = "m/44'/637'/0'/0/0")]
    pub path: String,
}

impl AddWallet {
    pub fn execute(&self) -> Result<()> {
        let curve_type = match self.curve.to_lowercase().as_str() {
            "ed25519" => CurveType::Ed25519,
            "k256" | "secp256k1" => CurveType::K256,
            "p256" | "secp256r1" => CurveType::P256,
            "ed25519+dilithium3" | "ed25519_dilithium3" => CurveType::Ed25519Dilithium3,
            "k256+dilithium3" | "k256_dilithium3" => CurveType::K256Dilithium3,
            other => {
                eprintln!("Unknown curve '{}', falling back to Ed25519", other);
                CurveType::Ed25519
            }
        };

        if self.private_key.is_none() && self.seed.is_none() {
            return Err(anyhow::anyhow!(
                "Please provide either --private-key or --seed to import a wallet"
            ));
        }

        if let Some(pk) = &self.private_key {
            let imported: ImportedWallet =
                kanari_crypto::keys::import_from_private_key(pk, curve_type)
                    .require("Import from private key failed")?;

            let address =
                AccountAddress::from_str(&imported.address).context("Generated invalid address")?;

            save_wallet(
                &address,
                &imported.private_key,
                "",
                None,
                &self.password,
                curve_type,
            )
            .context("Failed to save imported private-key wallet")?;

            eprintln!("Imported wallet from private key: {}", imported.address);
            eprintln!("Private Key: {}", *imported.private_key);
            eprintln!("Curve Type: {:?}", curve_type);
        } else if let Some(seed_phrase) = &self.seed {
            // Importing from BIP39 seed phrases only works for classical curves.
            if curve_type.is_post_quantum() || curve_type.is_hybrid() {
                return Err(anyhow::anyhow!(
                    "Import from seed phrase is not supported for post-quantum or hybrid curves; use CreateWallet to generate such keys"
                ));
            }
            let kp = derive_keypair_from_path(seed_phrase, "", &self.path, curve_type)
                .require("Import from seed phrase failed")?;

            let address_str = kp.address.clone();
            let zk = kp.export_private_key_secure();
            let privk = zk.to_string();

            let address =
                AccountAddress::from_str(&address_str).context("Generated invalid address")?;

            save_wallet(
                &address,
                &privk,
                seed_phrase,
                Some(&self.path),
                &self.password,
                curve_type,
            )
            .context("Failed to save imported seed wallet")?;

            eprintln!("Imported wallet from seed phrase: {}", address_str);
            eprintln!("Private Key: {}", privk);
            eprintln!("Derivation Path: {}", self.path);
            eprintln!("Curve Type: {:?}", curve_type);
            eprintln!("Seed Phrase: {}", seed_phrase);
        }

        Ok(())
    }
}
