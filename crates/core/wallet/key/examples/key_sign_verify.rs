use key::key_old;
use key_old::{
    generate_karix_address, sign_message_k256, sign_message_p256, sign_message_ed25519,
    verify_signature, verify_signature_p256, verify_signature_ed25519,
    CurveType, import_from_private_key,
};

fn main() {
    println!("Kanari Wallet - Signing and Verification Example");
    println!("================================================");
    
    // Example 1: K256 (secp256k1) signing and verification
    println!("\nExample 1: K256 Curve");
    println!("---------------------");
    
    // Generate a new K256 wallet
    let (private_key, public_address, seed_phrase) = generate_karix_address(12, CurveType::K256);
    
    println!("Generated new K256 wallet:");
    println!("  Address: {}", public_address);
    println!("  Private Key: {}", private_key);
    println!("  Seed Phrase: {}", seed_phrase);
    
    // Sign a message
    let message = b"Hello, Kanari!";
    let k256_signature = match sign_message_k256(&private_key, message) {
        Ok(sig) => {
            println!("\nSigned message successfully!");
            println!("  Message: \"{}\"", String::from_utf8_lossy(message));
            println!("  Signature (hex): {}", hex::encode(&sig));
            sig
        },
        Err(e) => {
            println!("Error signing message: {}", e);
            Vec::new()
        }
    };
    
    // Verify the signature
    if !k256_signature.is_empty() {
        match verify_signature(&public_address, message, &k256_signature) {
            Ok(true) => println!("✅ Signature verification successful!"),
            Ok(false) => println!("❌ Signature verification failed!"),
            Err(e) => println!("Error verifying signature: {}", e),
        }
        
        // Try with wrong message
        let wrong_message = b"Wrong message!";
        match verify_signature(&public_address, wrong_message, &k256_signature) {
            Ok(true) => println!("❌ Signature incorrectly verified with wrong message!"),
            Ok(false) => println!("✅ Signature correctly rejected for wrong message!"),
            Err(e) => println!("Error verifying signature: {}", e),
        }
    }
    
    // Example 2: P256 (secp256r1) signing and verification
    println!("\nExample 2: P256 Curve");
    println!("---------------------");
    
    // Generate a new P256 wallet
    let (private_key_p256, public_address_p256, seed_phrase_p256) = generate_karix_address(12, CurveType::P256);
    
    println!("Generated new P256 wallet:");
    println!("  Address: {}", public_address_p256);
    println!("  Private Key: {}", private_key_p256);
    println!("  Seed Phrase: {}", seed_phrase_p256);
    
    // Sign a message
    let message_p256 = b"Hello, Kanari with P256!";
    let p256_signature = match sign_message_p256(&private_key_p256, message_p256) {
        Ok(sig) => {
            println!("\nSigned message successfully!");
            println!("  Message: \"{}\"", String::from_utf8_lossy(message_p256));
            println!("  Signature (hex): {}", hex::encode(&sig));
            sig
        },
        Err(e) => {
            println!("Error signing message: {}", e);
            Vec::new()
        }
    };
    
    // Verify the signature
    if !p256_signature.is_empty() {
        // First try with the generic verify_signature function
        match verify_signature(&public_address_p256, message_p256, &p256_signature) {
            Ok(true) => println!("✅ Signature verification successful with generic function!"),
            Ok(false) => println!("❌ Signature verification failed with generic function!"),
            Err(e) => println!("Error verifying signature with generic function: {}", e),
        }
        
        // Then try with P256-specific verification function
        let address_hex = public_address_p256.trim_start_matches("0x");
        match verify_signature_p256(address_hex, message_p256, &p256_signature) {
            Ok(true) => println!("✅ Signature verification successful with P256-specific function!"),
            Ok(false) => println!("❌ Signature verification failed with P256-specific function!"),
            Err(e) => println!("Error verifying with P256-specific function: {}", e),
        }
    }
    
    // Example 3: Importing from private key and signing
    println!("\nExample 3: Importing from Private Key");
    println!("----------------------------------");
    
    // Import the wallet from K256 private key
    println!("\nImporting K256 wallet from private key:");
    match import_from_private_key(&private_key, CurveType::K256) {
        Ok((imported_private_key, _, imported_public_address)) => {
            println!("  Address: {}", imported_public_address);
            
            // Verify it's the same address
            if public_address == imported_public_address {
                println!("✅ Imported address matches original!");
            } else {
                println!("❌ Address mismatch: {} vs {}", public_address, imported_public_address);
            }
            
            // Sign the message with imported key
            match sign_message_k256(&imported_private_key, message) {
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
                    match verify_signature(&imported_public_address, message, &sig) {
                        Ok(true) => println!("✅ Signature verification successful!"),
                        Ok(false) => println!("❌ Signature verification failed!"),
                        Err(e) => println!("Error verifying signature: {}", e),
                    }
                },
                Err(e) => println!("Error signing with imported key: {}", e),
            }
        },
        Err(e) => println!("Error importing K256 wallet: {}", e),
    }
    
    // Import the wallet from P256 private key
    println!("\nImporting P256 wallet from private key:");
    match import_from_private_key(&private_key_p256, CurveType::P256) {
        Ok((imported_private_key, _, imported_public_address)) => {
            println!("  Address: {}", imported_public_address);
            
            // Verify it's the same address
            if public_address_p256 == imported_public_address {
                println!("✅ Imported address matches original!");
            } else {
                println!("❌ Address mismatch: {} vs {}", public_address_p256, imported_public_address);
            }
            
            // Sign the message with imported key
            match sign_message_p256(&imported_private_key, message_p256) {
                Ok(sig) => {
                    println!("\nSigned message with imported P256 key:");
                    println!("  Signature (hex): {}", hex::encode(&sig));
                    
                    // Verify this signature matches the original
                    if sig == p256_signature {
                        println!("✅ Signature from imported key matches original signature!");
                    } else {
                        println!("❌ Signature from imported key differs from original!");
                    }
                    
                    // Verify the signature works with both methods
                    match verify_signature(&imported_public_address, message_p256, &sig) {
                        Ok(true) => println!("✅ Signature verification successful with generic function!"),
                        Ok(false) => println!("❌ Signature verification failed with generic function!"),
                        Err(e) => println!("Error verifying signature with generic function: {}", e),
                    }
                    
                    // Direct P256 verification
                    let address_hex = imported_public_address.trim_start_matches("0x");
                    match verify_signature_p256(address_hex, message_p256, &sig) {
                        Ok(true) => println!("✅ Signature verification successful with P256-specific function!"),
                        Ok(false) => println!("❌ Signature verification failed with P256-specific function!"),
                        Err(e) => println!("Error verifying with P256-specific function: {}", e),
                    }
                },
                Err(e) => println!("Error signing with imported key: {}", e),
            }
        },
        Err(e) => println!("Error importing P256 wallet: {}", e),
    }
    
    // Example 4: Ed25519 (Edwards curve) signing and verification
    println!("\nExample 4: Ed25519 Curve");
    println!("------------------------");
    
    // Generate a new Ed25519 wallet
    let (private_key_ed25519, public_address_ed25519, seed_phrase_ed25519) = 
        generate_karix_address(12, CurveType::Ed25519);
    
    println!("Generated new Ed25519 wallet:");
    println!("  Address: {}", public_address_ed25519);
    println!("  Private Key: {}", private_key_ed25519);
    println!("  Seed Phrase: {}", seed_phrase_ed25519);
    
    // Sign a message
    let message_ed25519 = b"Hello, Kanari with Ed25519!";
    let ed25519_signature = match sign_message_ed25519(&private_key_ed25519, message_ed25519) {
        Ok(sig) => {
            println!("\nSigned message successfully!");
            println!("  Message: \"{}\"", String::from_utf8_lossy(message_ed25519));
            println!("  Signature (hex): {}", hex::encode(&sig));
            sig
        },
        Err(e) => {
            println!("Error signing message: {}", e);
            Vec::new()
        }
    };
    
    // Verify the signature
    if !ed25519_signature.is_empty() {
        // Try with the generic verify_signature function
        match verify_signature(&public_address_ed25519, message_ed25519, &ed25519_signature) {
            Ok(true) => println!("✅ Signature verification successful with generic function!"),
            Ok(false) => println!("❌ Signature verification failed with generic function!"),
            Err(e) => println!("Error verifying signature with generic function: {}", e),
        }
        
        // Try with Ed25519-specific verification function
        let address_hex = public_address_ed25519.trim_start_matches("0x");
        match verify_signature_ed25519(address_hex, message_ed25519, &ed25519_signature) {
            Ok(true) => println!("✅ Signature verification successful with Ed25519-specific function!"),
            Ok(false) => println!("❌ Signature verification failed with Ed25519-specific function!"),
            Err(e) => println!("Error verifying with Ed25519-specific function: {}", e),
        }
    }
    
    // Import the wallet from Ed25519 private key
    println!("\nImporting Ed25519 wallet from private key:");
    match import_from_private_key(&private_key_ed25519, CurveType::Ed25519) {
        Ok((imported_private_key, _, imported_public_address)) => {
            println!("  Address: {}", imported_public_address);
            
            // Verify it's the same address
            if public_address_ed25519 == imported_public_address {
                println!("✅ Imported address matches original!");
            } else {
                println!("❌ Address mismatch: {} vs {}", public_address_ed25519, imported_public_address);
            }
            
            // Sign the message with imported key
            match sign_message_ed25519(&imported_private_key, message_ed25519) {
                Ok(sig) => {
                    println!("\nSigned message with imported Ed25519 key:");
                    println!("  Signature (hex): {}", hex::encode(&sig));
                    
                    // Verify this signature matches the original
                    if sig == ed25519_signature {
                        println!("✅ Signature from imported key matches original signature!");
                    } else {
                        println!("❌ Signature from imported key differs from original!");
                    }
                    
                    // Verify the signature works with both methods
                    match verify_signature(&imported_public_address, message_ed25519, &sig) {
                        Ok(true) => println!("✅ Signature verification successful with generic function!"),
                        Ok(false) => println!("❌ Signature verification failed with generic function!"),
                        Err(e) => println!("Error verifying signature with generic function: {}", e),
                    }
                    
                    // Direct Ed25519 verification
                    let address_hex = imported_public_address.trim_start_matches("0x");
                    match verify_signature_ed25519(address_hex, message_ed25519, &sig) {
                        Ok(true) => println!("✅ Signature verification successful with Ed25519-specific function!"),
                        Ok(false) => println!("❌ Signature verification failed with Ed25519-specific function!"),
                        Err(e) => println!("Error verifying with Ed25519-specific function: {}", e),
                    }
                },
                Err(e) => println!("Error signing with imported key: {}", e),
            }
        },
        Err(e) => println!("Error importing Ed25519 wallet: {}", e),
    }
}
