use kanari_crypto::keys::{
    CurveType, generate_keypair, generate_mnemonic, keypair_from_mnemonic, keypair_from_private_key,
};
use kanari_crypto::{sign_message, verify_signature};

fn main() {
    println!("🔐 Kanari Crypto v2.0 - Signing and Verification Example");
    println!("========================================================");
    println!("\nℹ️  This example demonstrates both classical and post-quantum signatures");

    // Example 1: K256 (secp256k1) signing and verification
    println!("\n📝 Example 1: K256 Curve (Classical - NOT Quantum-Safe)");
    println!("-------------------------------------------------------");

    // Generate a new K256 wallet
    let keypair = match generate_keypair(CurveType::K256) {
        Ok(kp) => kp,
        Err(e) => {
            eprintln!("Failed to generate K256 keypair: {}", e);
            return;
        }
    };

    println!("Generated new K256 wallet:");
    println!("  Address: {}", keypair.address);
    println!("  Private Key: {}", keypair.private_key.to_string());
    println!("  Public Key: {}", keypair.public_key);

    // Sign a message
    let message = b"Hello, Mona!";
    let k256_signature = match sign_message(&keypair.private_key, message, CurveType::K256) {
        Ok(sig) => {
            println!("\nSigned message successfully!");
            println!("  Message: \"{}\"", String::from_utf8_lossy(message));
            println!("  Signature (hex): {}", hex::encode(&sig));
            sig
        }
        Err(e) => {
            println!("Error signing message: {}", e);
            Vec::new()
        }
    };

    // Verify the signature
    if !k256_signature.is_empty() {
        match verify_signature(&keypair.address, message, &k256_signature) {
            Ok(true) => println!("✅ Signature verification successful!"),
            Ok(false) => println!("❌ Signature verification failed!"),
            Err(e) => println!("Error verifying signature: {}", e),
        }

        // Try with wrong message
        let wrong_message = b"Wrong message!";
        match verify_signature(&keypair.address, wrong_message, &k256_signature) {
            Ok(true) => println!("❌ Signature incorrectly verified with wrong message!"),
            Ok(false) => println!("✅ Signature correctly rejected for wrong message!"),
            Err(e) => println!("Error verifying signature: {}", e),
        }
    }

    // Example 2: P256 (secp256r1) signing and verification
    println!("\nExample 2: P256 Curve");
    println!("---------------------");

    // Generate a new P256 wallet
    let p256_keypair = match generate_keypair(CurveType::P256) {
        Ok(kp) => kp,
        Err(e) => {
            eprintln!("Failed to generate P256 keypair: {}", e);
            return;
        }
    };

    println!("Generated new P256 wallet:");
    println!("  Address: {}", p256_keypair.address);
    println!("  Private Key: {}", p256_keypair.private_key.to_string());
    println!("  Public Key: {}", p256_keypair.public_key);

    // Sign a message
    let message_p256 = b"Hello, Mona with P256!";
    let p256_signature =
        match sign_message(&p256_keypair.private_key, message_p256, CurveType::P256) {
            Ok(sig) => {
                println!("\nSigned message successfully!");
                println!("  Message: \"{}\"", String::from_utf8_lossy(message_p256));
                println!("  Signature (hex): {}", hex::encode(&sig));
                sig
            }
            Err(e) => {
                println!("Error signing message: {}", e);
                Vec::new()
            }
        };

    // Verify the signature
    if !p256_signature.is_empty() {
        // Try with the generic verify_signature function
        match verify_signature(&p256_keypair.address, message_p256, &p256_signature) {
            Ok(true) => println!("✅ Signature verification successful with generic function!"),
            Ok(false) => println!("❌ Signature verification failed with generic function!"),
            Err(e) => println!("Error verifying signature with generic function: {}", e),
        }

        // Try with verification using curve type
        match verify_signature(&p256_keypair.address, message_p256, &p256_signature) {
            Ok(true) => println!("✅ Signature verification successful!"),
            Ok(false) => println!("❌ Signature verification failed!"),
            Err(e) => println!("Error verifying signature: {}", e),
        }
    }

    // Example 3: Importing from private key
    println!("\nExample 3: Importing from Private Key");
    println!("----------------------------------");

    // Import the wallet from K256 private key
    println!("\nImporting K256 wallet from private key:");
    match keypair_from_private_key(&keypair.private_key, CurveType::K256) {
        Ok(imported_keypair) => {
            println!("  Address: {}", imported_keypair.address);

            // Verify it's the same address
            if keypair.address == imported_keypair.address {
                println!("✅ Imported address matches original!");
            } else {
                println!(
                    "❌ Address mismatch: {} vs {}",
                    keypair.address, imported_keypair.address
                );
            }

            // Sign the message with imported key
            match sign_message(&imported_keypair.private_key, message, CurveType::K256) {
                Ok(sig) => {
                    println!("\nSigned message with imported K256 key:");
                    println!("  Signature (hex): {}", hex::encode(&sig));

                    // Verify this signature matches the original
                    if sig == k256_signature {
                        println!("✅ Signature from imported key matches original signature!");
                    } else {
                        println!("❌ Signature from imported key differs from original!");
                    }

                    // Verify the signature works
                    match verify_signature(&imported_keypair.address, message, &sig) {
                        Ok(true) => println!("✅ Signature verification successful!"),
                        Ok(false) => println!("❌ Signature verification failed!"),
                        Err(e) => println!("Error verifying signature: {}", e),
                    }
                }
                Err(e) => println!("Error signing with imported key: {}", e),
            }
        }
        Err(e) => println!("Error importing K256 wallet: {}", e),
    }

    // Import the wallet from P256 private key
    println!("\nImporting P256 wallet from private key:");
    match keypair_from_private_key(&p256_keypair.private_key, CurveType::P256) {
        Ok(imported_keypair) => {
            println!("  Address: {}", imported_keypair.address);

            // Verify it's the same address
            if p256_keypair.address == imported_keypair.address {
                println!("✅ Imported address matches original!");
            } else {
                println!(
                    "❌ Address mismatch: {} vs {}",
                    p256_keypair.address, imported_keypair.address
                );
            }

            // Sign the message with imported key
            match sign_message(&imported_keypair.private_key, message_p256, CurveType::P256) {
                Ok(sig) => {
                    println!("\nSigned message with imported P256 key:");
                    println!("  Signature (hex): {}", hex::encode(&sig));

                    // Verify this signature matches the original
                    if sig == p256_signature {
                        println!("✅ Signature from imported key matches original signature!");
                    } else {
                        println!("❌ Signature from imported key differs from original!");
                    }

                    // Verify the signature works
                    match verify_signature(&imported_keypair.address, message_p256, &sig) {
                        Ok(true) => println!("✅ Signature verification successful!"),
                        Ok(false) => println!("❌ Signature verification failed!"),
                        Err(e) => println!("Error verifying signature: {}", e),
                    }
                }
                Err(e) => println!("Error signing with imported key: {}", e),
            }
        }
        Err(e) => println!("Error importing P256 wallet: {}", e),
    }

    // Example 4: Ed25519 (Edwards curve) signing and verification
    println!("\nExample 4: Ed25519 Curve");
    println!("------------------------");

    // Generate a new Ed25519 wallet
    let ed25519_keypair = match generate_keypair(CurveType::Ed25519) {
        Ok(kp) => kp,
        Err(e) => {
            eprintln!("Failed to generate Ed25519 keypair: {}", e);
            return;
        }
    };

    println!("Generated new Ed25519 wallet:");
    println!("  Address: {}", ed25519_keypair.address);
    println!("  Private Key: {}", ed25519_keypair.private_key.to_string());
    println!("  Public Key: {}", ed25519_keypair.public_key);

    // Sign a message
    let message_ed25519 = b"Hello, Mona with Ed25519!";
    let ed25519_signature = match sign_message(
        &ed25519_keypair.private_key,
        message_ed25519,
        CurveType::Ed25519,
    ) {
        Ok(sig) => {
            println!("\nSigned message successfully!");
            println!(
                "  Message: \"{}\"",
                String::from_utf8_lossy(message_ed25519)
            );
            println!("  Signature (hex): {}", hex::encode(&sig));
            sig
        }
        Err(e) => {
            println!("Error signing message: {}", e);
            Vec::new()
        }
    };

    // Verify the signature
    if !ed25519_signature.is_empty() {
        // Try with the generic verify_signature function
        match verify_signature(
            &ed25519_keypair.address,
            message_ed25519,
            &ed25519_signature,
        ) {
            Ok(true) => println!("✅ Signature verification successful with generic function!"),
            Ok(false) => println!("❌ Signature verification failed with generic function!"),
            Err(e) => println!("Error verifying signature with generic function: {}", e),
        }
    }

    // Import the wallet from Ed25519 private key
    println!("\nImporting Ed25519 wallet from private key:");
    match keypair_from_private_key(&ed25519_keypair.private_key, CurveType::Ed25519) {
        Ok(imported_keypair) => {
            println!("  Address: {}", imported_keypair.address);

            // Verify it's the same address
            if ed25519_keypair.address == imported_keypair.address {
                println!("✅ Imported address matches original!");
            } else {
                println!(
                    "❌ Address mismatch: {} vs {}",
                    ed25519_keypair.address, imported_keypair.address
                );
            }

            // Sign the message with imported key
            match sign_message(
                &imported_keypair.private_key,
                message_ed25519,
                CurveType::Ed25519,
            ) {
                Ok(sig) => {
                    println!("\nSigned message with imported Ed25519 key:");
                    println!("  Signature (hex): {}", hex::encode(&sig));

                    // Verify this signature matches the original
                    if sig == ed25519_signature {
                        println!("✅ Signature from imported key matches original signature!");
                    } else {
                        println!("❌ Signature from imported key differs from original!");
                    }

                    // Verify the signature works
                    match verify_signature(&imported_keypair.address, message_ed25519, &sig) {
                        Ok(true) => println!("✅ Signature verification successful!"),
                        Ok(false) => println!("❌ Signature verification failed!"),
                        Err(e) => println!("Error verifying signature: {}", e),
                    }
                }
                Err(e) => println!("Error signing with imported key: {}", e),
            }
        }
        Err(e) => println!("Error importing Ed25519 wallet: {}", e),
    }

    // Example 5: Generate and import from mnemonic
    println!("\nExample 5: Mnemonic Phrase Generation and Import");
    println!("----------------------------------------------");

    // Generate a mnemonic for K256
    let mnemonic = match generate_mnemonic(12) {
        Ok(phrase) => {
            println!("Generated mnemonic: {}", phrase);
            phrase
        }
        Err(e) => {
            println!("Error generating mnemonic: {}", e);
            return;
        }
    };

    // Import from mnemonic
    println!("\nImporting wallet from mnemonic:");
    match keypair_from_mnemonic(&mnemonic, CurveType::K256, "") {
        // Added empty password as 3rd parameter
        Ok(mnemonic_keypair) => {
            println!("  Address: {}", mnemonic_keypair.address);
            println!(
                "  Private Key: {}",
                mnemonic_keypair.private_key.to_string()
            );

            // Sign a message with the mnemonic-derived key
            let message_mnemonic = b"Hello from mnemonic!";
            match sign_message(
                &mnemonic_keypair.private_key,
                message_mnemonic,
                CurveType::K256,
            ) {
                Ok(sig) => {
                    println!("\nSigned message with mnemonic-derived key:");
                    println!(
                        "  Message: \"{}\"",
                        String::from_utf8_lossy(message_mnemonic)
                    );
                    println!("  Signature (hex): {}", hex::encode(&sig));

                    // Verify the signature
                    match verify_signature(&mnemonic_keypair.address, message_mnemonic, &sig) {
                        Ok(true) => println!("✅ Signature verification successful!"),
                        Ok(false) => println!("❌ Signature verification failed!"),
                        Err(e) => println!("Error verifying signature: {}", e),
                    }
                }
                Err(e) => println!("Error signing with mnemonic-derived key: {}", e),
            }
        }
        Err(e) => println!("Error importing from mnemonic: {}", e),
    }
}
