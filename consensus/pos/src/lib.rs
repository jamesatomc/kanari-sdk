use serde::{Deserialize, Serialize};
use blake3::Hasher;
use hex::encode;


#[derive(Serialize, Deserialize, Clone)]
pub struct Blake3Algorithm;

// Add constructor implementation
impl Blake3Algorithm {
    pub fn new() -> Self {
        Blake3Algorithm
    }
    
    /// Static method to hash data and return Vec<u8> for compatibility
    pub fn hash(input: &[u8]) -> Vec<u8> {
        let mut hasher = Hasher::new();
        hasher.update(input);
        let result = hasher.finalize();
        result.as_bytes().to_vec()
    }
}

pub trait HashAlgorithm {
    fn hash(&self, input: &[u8]) -> String;
    fn log_input(&self, input: &[u8]);
}

impl HashAlgorithm for Blake3Algorithm {
    fn hash(&self, input: &[u8]) -> String {
        let mut hasher = Hasher::new();
        hasher.update(input);
        let result = hasher.finalize();
        encode(result.as_bytes())
    }

    fn log_input(&self, input: &[u8]) {
        println!("Hashing input: {:?}", input);
    }
}




