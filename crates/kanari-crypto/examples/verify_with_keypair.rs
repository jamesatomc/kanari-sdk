use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_crypto::signatures::{sign_message, verify_signature_with_keypair};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Generate a hybrid Ed25519+Dilithium3 keypair
    let keypair = generate_keypair(CurveType::Ed25519Dilithium3)?;

    // Use a UTF-8 string and convert to bytes (avoid non-ASCII byte string literal)
    let message = "Ed25519 Hybrid test message".as_bytes();

    // Sign using the combined private key format stored in KeyPair
    let signature = sign_message(&keypair.private_key, message, keypair.curve_type)?;

    println!("Ed25519 Hybrid Signature length: {} bytes", signature.len());

    // Verify using the KeyPair-aware verifier (uses pqc_public_key when present)
    let ok = verify_signature_with_keypair(&keypair, message, &signature)?;
    println!("Ed25519 Hybrid Signature verified: {}", ok);

    Ok(())
}
