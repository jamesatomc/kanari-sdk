// cargo run -p kanari-crypto --example hd_wallet_example

use kanari_crypto::keys::{CurveType, generate_mnemonic};
use kanari_crypto::wallet::{Wallet, create_hd_wallet, load_wallet, save_hd_wallet, save_mnemonic};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    println!("🔐 Kanari Crypto v2.0 - HD Wallet Example");
    println!("==========================================");
    println!("\nℹ️  Note: HD wallets currently support classical algorithms only.");
    println!("   Post-quantum algorithms will be added in future versions.\n");

    // 1) Generate a mnemonic (for demo only)
    let mnemonic = generate_mnemonic(12)?;
    println!("Generated mnemonic: {}", mnemonic);

    // Example addresses vector (empty initially)
    let addresses: Vec<String> = Vec::new();

    // 2) Save mnemonic into keystore with a strong password
    //    The library enforces a minimum-strength policy, so use mixed case,
    //    digits, and special characters.
    let password = "Str0ngP@ssw0rd!2025";
    save_mnemonic(&mnemonic, password, addresses)?;
    println!("Saved mnemonic into keystore (encrypted)");

    // 3) Derive an HD child wallet (example: Ethereum-like path)
    let path = "m/44'/60'/0'/0/0";
    let curve = CurveType::K256;

    let child_wallet: Wallet = create_hd_wallet(password, path, curve)?;
    println!(
        "Derived child wallet for path {} -> address {}",
        path, child_wallet.address
    );

    // 4) Persist the child wallet into keystore
    save_hd_wallet(&child_wallet, password)?;
    println!("Saved derived child wallet to keystore");

    // 5) Load the wallet back to verify
    let loaded = load_wallet(&child_wallet.address.to_string(), password)?;
    println!("Loaded wallet from keystore: {}", loaded.address);

    println!("\n✅ HD Wallet example completed successfully!");
    println!("\n💡 Note:");
    println!("   - Classical algorithms (Ed25519, K256, P256) support BIP39/BIP32 HD wallets");
    println!("   - Post-quantum algorithms don't yet support HD wallet derivation");
    println!("   - For PQC, use direct key generation: generate_keypair(CurveType::Dilithium3)");

    Ok(())
}
