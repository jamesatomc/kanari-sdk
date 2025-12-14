// rust/src/api.rs

use crate::keys::{CurveType, generate_keypair, keypair_from_mnemonic, keypair_from_private_key};
use crate::signatures::{sign_message, verify_signature_with_curve};
use serde::{Deserialize, Serialize};

/// แสดงชื่อ curve อย่างปลอดภัยสำหรับ Dart
#[derive(Serialize, Deserialize, Debug)]
pub struct CurveInfo {
    pub name: String,
    pub is_post_quantum: bool,
    pub is_hybrid: bool,
    pub security_level: u8,
}

/// โครงสร้างข้อมูล keypair ที่ปลอดภัยสำหรับส่งผ่าน FFI
#[derive(Serialize, Deserialize, Debug)]
pub struct KeyPairData {
    pub private_key: String, // รูปแบบ "kanari...", "kanapqc...", "kanahybrid..."
    pub public_key: String,  // hex
    pub address: String,     // 0x...
    pub raw_public_key: Vec<u8>, // raw bytes of public key (for PQC verification)
    pub curve_type: String,  // เช่น "K256", "Ed25519Dilithium3"
}

/// สร้าง keypair ตามประเภท curve
pub fn generate_keypair_api(curve_name: String) -> Result<KeyPairData, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    let kp = generate_keypair(curve).map_err(|e| format!("Key generation failed: {}", e))?;

    // Clone the pieces we need to avoid moving out of `kp` more than once
    let public_key_clone = kp.public_key.clone();
    let address_clone = kp.address.clone();
    let pqc_clone = kp.pqc_public_key.clone();
    let raw_public_key =
        hex::decode(pqc_clone.unwrap_or_else(|| public_key_clone.clone())).unwrap_or_default();

    Ok(KeyPairData {
        private_key: kp.private_key.to_string(),
        public_key: public_key_clone,
        address: address_clone,
        raw_public_key,
        curve_type: format!("{:?}", curve),
    })
}

/// สร้าง keypair จาก mnemonic (BIP39)
pub fn derive_keypair_from_mnemonic(
    mnemonic: String,
    curve_name: String,
    password: String,
) -> Result<KeyPairData, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    if curve.is_post_quantum() {
        return Err("Post-quantum curves do not support BIP39 derivation".to_string());
    }

    let kp = keypair_from_mnemonic(&mnemonic, curve, &password)
        .map_err(|e| format!("Mnemonic derivation failed: {}", e))?;

    let public_key_clone = kp.public_key.clone();
    let address_clone = kp.address.clone();
    let pqc_clone = kp.pqc_public_key.clone();
    let raw_public_key =
        hex::decode(pqc_clone.unwrap_or_else(|| public_key_clone.clone())).unwrap_or_default();

    Ok(KeyPairData {
        private_key: kp.private_key.to_string(),
        public_key: public_key_clone,
        address: address_clone,
        raw_public_key,
        curve_type: format!("{:?}", curve),
    })
}

/// สร้าง keypair จาก private key ที่ให้มา
pub fn import_keypair_from_private_key(
    private_key: String,
    curve_name: String,
) -> Result<KeyPairData, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    let kp = keypair_from_private_key(&private_key, curve)
        .map_err(|e| format!("Private key import failed: {}", e))?;

    let public_key_clone = kp.public_key.clone();
    let address_clone = kp.address.clone();
    let pqc_clone = kp.pqc_public_key.clone();
    let raw_public_key =
        hex::decode(pqc_clone.unwrap_or_else(|| public_key_clone.clone())).unwrap_or_default();

    Ok(KeyPairData {
        private_key: kp.private_key.to_string(),
        public_key: public_key_clone,
        address: address_clone,
        raw_public_key,
        curve_type: format!("{:?}", curve),
    })
}

/// ลงนามข้อความ
pub fn sign_message_api(
    private_key: String,
    message: Vec<u8>,
    curve_name: String,
) -> Result<Vec<u8>, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    sign_message(&private_key, &message, curve).map_err(|e| format!("Signing failed: {}", e))
}

/// ตรวจสอบลายเซ็น
pub fn verify_signature_api(
    address: String,
    message: Vec<u8>,
    signature: Vec<u8>,
    curve_name: String,
) -> Result<bool, String> {
    let curve = parse_curve_type(&curve_name)
        .ok_or_else(|| format!("Unsupported curve type: {}", curve_name))?;

    verify_signature_with_curve(&address, &message, &signature, curve)
        .map_err(|e| format!("Verification failed: {}", e))
}

/// สร้าง mnemonic แบบสุ่ม
pub fn generate_mnemonic_api(word_count: usize) -> Result<String, String> {
    if word_count != 12 && word_count != 24 {
        return Err("Only 12 or 24-word mnemonics are supported".to_string());
    }

    crate::keys::generate_mnemonic(word_count)
        .map_err(|e| format!("Mnemonic generation failed: {}", e))
}

/// ดึงข้อมูล curve ทั้งหมดที่รองรับ
pub fn list_supported_curves() -> Vec<CurveInfo> {
    use CurveType::*;
    let curves = [
        K256,
        P256,
        Ed25519,
        Dilithium2,
        Dilithium3,
        Dilithium5,
        SphincsPlusSha256Robust,
        Ed25519Dilithium3,
        K256Dilithium3,
    ];

    curves
        .iter()
        .map(|&c| CurveInfo {
            name: format!("{:?}", c),
            is_post_quantum: c.is_post_quantum(),
            is_hybrid: c.is_hybrid(),
            security_level: c.security_level(),
        })
        .collect()
}

// --- Helper Internals ---

fn parse_curve_type(name: &str) -> Option<CurveType> {
    match name {
        "K256" => Some(CurveType::K256),
        "P256" => Some(CurveType::P256),
        "Ed25519" => Some(CurveType::Ed25519),
        "Dilithium2" => Some(CurveType::Dilithium2),
        "Dilithium3" => Some(CurveType::Dilithium3),
        "Dilithium5" => Some(CurveType::Dilithium5),
        "SphincsPlusSha256Robust" => Some(CurveType::SphincsPlusSha256Robust),
        "Ed25519Dilithium3" => Some(CurveType::Ed25519Dilithium3),
        "K256Dilithium3" => Some(CurveType::K256Dilithium3),
        _ => None,
    }
}
