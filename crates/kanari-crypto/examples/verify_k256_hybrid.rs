use kanari_crypto::keys::{CurveType, generate_keypair};
use kanari_crypto::signatures::{sign_message, verify_signature_with_keypair};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let keypair = generate_keypair(CurveType::K256Dilithium3)?;
    let message = "K256 hybrid test message".as_bytes();

    let signature = sign_message(&keypair.private_key, message, keypair.curve_type)?;
    println!("K256 hybrid signature length: {} bytes", signature.len());

    let ok = verify_signature_with_keypair(&keypair, message, &signature)?;
    println!("K256 hybrid verified: {}", ok);

    Ok(())
}
