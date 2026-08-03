use ml_dsa::{
    EncodedVerifyingKey, Generate, Keypair as _, MlDsa44, MlDsa65, MlDsa87, Signature, Signer,
    SigningKey, Verifier, VerifyingKey,
};
use zeroize::Zeroizing;

use crate::SignatureError;

pub const ML_DSA_44_PUBLIC_KEY_BYTES: usize = 1_312;
pub const ML_DSA_65_PUBLIC_KEY_BYTES: usize = 1_952;
pub const ML_DSA_87_PUBLIC_KEY_BYTES: usize = 2_592;
pub const ML_DSA_44_SIGNATURE_BYTES: usize = 2_420;
pub const ML_DSA_65_SIGNATURE_BYTES: usize = 3_309;
pub const ML_DSA_87_SIGNATURE_BYTES: usize = 4_627;

pub fn generate_mldsa44_keypair_bytes() -> (Vec<u8>, Zeroizing<Vec<u8>>) {
    let signing_key = SigningKey::<MlDsa44>::generate();
    let public_key = signing_key.verifying_key().encode().as_slice().to_vec();
    let seed = Zeroizing::new(signing_key.to_seed().as_slice().to_vec());
    (public_key, seed)
}

pub fn generate_mldsa65_keypair_bytes() -> (Vec<u8>, Zeroizing<Vec<u8>>) {
    let signing_key = SigningKey::<MlDsa65>::generate();
    let public_key = signing_key.verifying_key().encode().as_slice().to_vec();
    let seed = Zeroizing::new(signing_key.to_seed().as_slice().to_vec());
    (public_key, seed)
}

pub fn generate_mldsa87_keypair_bytes() -> (Vec<u8>, Zeroizing<Vec<u8>>) {
    let signing_key = SigningKey::<MlDsa87>::generate();
    let public_key = signing_key.verifying_key().encode().as_slice().to_vec();
    let seed = Zeroizing::new(signing_key.to_seed().as_slice().to_vec());
    (public_key, seed)
}

pub fn sign_mldsa44(seed_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    sign::<MlDsa44>(seed_bytes, message, "ML-DSA-44")
}

pub fn sign_mldsa65(seed_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    sign::<MlDsa65>(seed_bytes, message, "ML-DSA-65")
}

pub fn sign_mldsa87(seed_bytes: &[u8], message: &[u8]) -> Result<Vec<u8>, SignatureError> {
    sign::<MlDsa87>(seed_bytes, message, "ML-DSA-87")
}

pub fn verify_mldsa44(
    public_key_bytes: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    verify::<MlDsa44>(public_key_bytes, message, signature, "ML-DSA-44")
}

pub fn verify_mldsa65(
    public_key_bytes: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    verify::<MlDsa65>(public_key_bytes, message, signature, "ML-DSA-65")
}

pub fn verify_mldsa87(
    public_key_bytes: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<bool, SignatureError> {
    verify::<MlDsa87>(public_key_bytes, message, signature, "ML-DSA-87")
}

fn sign<P: ml_dsa::MlDsaParams>(
    seed_bytes: &[u8],
    message: &[u8],
    label: &str,
) -> Result<Vec<u8>, SignatureError> {
    let seed = ml_dsa::Seed::try_from(seed_bytes)
        .map_err(|_| SignatureError::InvalidPrivateKey(format!("Invalid {label} seed length")))?;
    let signing_key = SigningKey::<P>::from_seed(&seed);
    let signature: Signature<P> = signing_key.sign(message);
    Ok(signature.encode().as_slice().to_vec())
}

fn verify<P: ml_dsa::MlDsaParams>(
    public_key_bytes: &[u8],
    message: &[u8],
    signature: &[u8],
    label: &str,
) -> Result<bool, SignatureError> {
    let encoded_public = EncodedVerifyingKey::<P>::try_from(public_key_bytes)
        .map_err(|_| SignatureError::InvalidPublicKey(format!("Invalid {label} public key")))?;
    let verifying_key = VerifyingKey::<P>::decode(&encoded_public);
    let signature = Signature::<P>::try_from(signature)
        .map_err(|_| SignatureError::InvalidFormat(format!("Invalid {label} signature")))?;
    Ok(verifying_key.verify(message, &signature).is_ok())
}
