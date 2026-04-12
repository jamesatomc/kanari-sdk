// crates/kanari-types/src/digest.rs
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Base Digest type wrapping a 32-byte array
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Digest(pub [u8; 32]);

impl Digest {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn into_inner(self) -> [u8; 32] {
        self.0
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 32 {
            return Err(anyhow!(
                "Invalid digest length: expected 32, got {}",
                bytes.len()
            ));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

// =====================================================================
// Strongly Typed Digests (ป้องกันการส่ง Hash สลับประเภทกัน)
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TransactionDigest(pub Digest);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjectDigest(pub Digest);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CheckpointDigest(pub Digest);

// Helper macros or functions can be added here to easily convert Vec<u8> to these types
