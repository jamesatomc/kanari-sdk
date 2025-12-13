use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_crypto::signatures::{sign_message, verify_signature_with_keypair};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let keypair = generate_keypair(CurveType::Dilithium5)?;
    let message = "Dilithium5 test message".as_bytes();

    let signature = sign_message(&keypair.private_key, message, keypair.curve_type)?;
    println!("Dilithium5 signature length: {} bytes", signature.len());

    let ok = verify_signature_with_keypair(&keypair, message, &signature)?;
    println!("Dilithium5 verified: {}", ok);

    Ok(())
}
