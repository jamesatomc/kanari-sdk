// Copyright (c) KanariNetwork, Inc.
// SPDX-License-Identifier: Apache-2.0

use hex::FromHex;
use lazy_static::lazy_static;
use std::convert::TryFrom;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize}; // Import Serialize and Deserialize
// Add this at the top with other imports
use move_core_types::account_address::AccountAddress;

// Lazy-initialized account addresses for system constants
// These are created once and cached, avoiding runtime parsing overhead
lazy_static! {
    /// Standard library address (0x1)
    static ref STD_ACCOUNT_ADDRESS: AccountAddress =
        AccountAddress::from_hex_literal(Address::STD_ADDRESS)
            .expect("STD_ADDRESS constant is invalid");

    /// Kanari System address (0x2)
    static ref KANARI_SYSTEM_ACCOUNT_ADDRESS: AccountAddress =
        AccountAddress::from_hex_literal(Address::KANARI_SYSTEM_ADDRESS)
            .expect("KANARI_SYSTEM_ADDRESS constant is invalid");

    /// Genesis/System address (0x0)
    static ref GENESIS_ACCOUNT_ADDRESS: AccountAddress =
        AccountAddress::from_hex_literal(Address::GENESIS_ADDRESS)
            .expect("GENESIS_ADDRESS constant is invalid");

    /// DAO address
    static ref DAO_ACCOUNT_ADDRESS: AccountAddress =
        AccountAddress::from_hex_literal(Address::DAO_ADDRESS)
            .expect("DAO_ADDRESS constant is invalid");

    /// Dev wallet address
    static ref DEV_ACCOUNT_ADDRESS: AccountAddress =
        AccountAddress::from_hex_literal(Address::DEV_ADDRESS)
            .expect("DEV_ADDRESS constant is invalid");
}

// Add this implementation at the end of the file, right before or after the std::error::Error impl
impl From<AccountAddress> for Address {
    fn from(addr: AccountAddress) -> Self {
        // Get the bytes from AccountAddress
        let move_bytes = addr.to_vec();

        // Create a 32-byte array filled with zeros
        let mut bytes = [0u8; Self::LENGTH];

        // Handle the size difference between Move's AccountAddress and our Address
        // Copy the bytes from AccountAddress to our Address, right-aligned
        // This preserves the significant bytes if the sizes differ
        let start_idx = Self::LENGTH.saturating_sub(move_bytes.len());
        let copy_len = std::cmp::min(move_bytes.len(), Self::LENGTH);
        bytes[start_idx..start_idx + copy_len].copy_from_slice(&move_bytes[..copy_len]);

        Address::new(bytes)
    }
}

/// Represents an address in the system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address([u8; Address::LENGTH]);

impl Address {
    /// The number of bytes in an address
    pub const LENGTH: usize = 32;

    /// Move Standard Library address (0x1)
    pub const STD_ADDRESS: &'static str = "0x1";

    /// Kanari System address (0x2)
    pub const KANARI_SYSTEM_ADDRESS: &'static str = "0x2";

    /// Genesis/System address (0x0)
    pub const GENESIS_ADDRESS: &'static str = "0x0";

    /// Dev wallet address that receives initial token supply
    /// This matches the address in kanari.move genesis allocation
    pub const DEV_ADDRESS: &'static str =
        "0x3ba63b92aac5f2bff87e580e820b61faf1c5fe9ae12f0bc8addd931a340b3146";

    /// DAO address that receives all gas fees
    pub const DAO_ADDRESS: &'static str =
        "0x7e669cf309e9761beb2083d12564895ccea1389f2851a479edcc0d1ad19e4350";

    /// Helper to get AccountAddress for STD (0x1) - cached at first use
    /// Uses lazy_static for zero-cost after initialization
    pub fn std_account_address() -> AccountAddress {
        *STD_ACCOUNT_ADDRESS
    }

    /// Helper to get AccountAddress for Kanari System (0x2) - cached at first use
    /// Uses lazy_static for zero-cost after initialization
    pub fn kanari_system_account_address() -> AccountAddress {
        *KANARI_SYSTEM_ACCOUNT_ADDRESS
    }

    /// Helper to get AccountAddress for Genesis (0x0) - cached at first use
    /// Uses lazy_static for zero-cost after initialization
    pub fn genesis_account_address() -> AccountAddress {
        *GENESIS_ACCOUNT_ADDRESS
    }

    /// Helper to get AccountAddress for DAO - cached at first use
    /// Uses lazy_static for zero-cost after initialization
    pub fn dao_account_address() -> AccountAddress {
        *DAO_ACCOUNT_ADDRESS
    }

    /// Helper to get AccountAddress for Dev wallet - cached at first use
    /// Uses lazy_static for zero-cost after initialization
    pub fn dev_account_address() -> AccountAddress {
        *DEV_ACCOUNT_ADDRESS
    }

    /// Creates a new Address from raw bytes
    pub const fn new(bytes: [u8; Self::LENGTH]) -> Self {
        Address(bytes)
    }

    /// Zero address constant
    pub const ZERO: Self = Self([0u8; Self::LENGTH]);

    /// Returns the underlying bytes
    pub fn to_bytes(&self) -> &[u8; Self::LENGTH] {
        &self.0
    }

    /// Convert address to vector of bytes
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// Consume address and return the raw bytes
    pub fn into_bytes(self) -> [u8; Self::LENGTH] {
        self.0
    }

    /// Validates that the address is not zero when required
    fn validate_non_zero(&self) -> Result<(), AddressParseError> {
        if self.0.iter().all(|&b| b == 0) {
            return Err(AddressParseError::ZeroAddress);
        }
        Ok(())
    }

    /// Validates padding bytes are zeros
    fn validate_padding(bytes: &[u8]) -> Result<(), AddressParseError> {
        let first_non_zero = bytes.iter().position(|&b| b != 0);
        match first_non_zero {
            Some(idx) => {
                if idx < bytes.len() - Self::LENGTH {
                    return Err(AddressParseError::InvalidPadding);
                }
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Enhanced hex validation
    fn validate_hex(hex: &str) -> Result<(), AddressParseError> {
        if hex.is_empty() {
            return Err(AddressParseError::EmptyString);
        }

        // Check for valid hex characters and length
        if hex.len() > Self::LENGTH * 2 {
            return Err(AddressParseError::Overflow);
        }

        if let Some(invalid_char) = hex.chars().find(|c| !c.is_ascii_hexdigit()) {
            return Err(AddressParseError::InvalidCharacter(invalid_char));
        }

        Ok(())
    }

    /// Create an address from a hex string with validation
    pub fn from_hex<T: AsRef<[u8]>>(hex: T) -> Result<Self, AddressParseError> {
        let hex = hex.as_ref();
        let hex_str = std::str::from_utf8(hex).map_err(|_| AddressParseError::InvalidUtf8)?;

        Self::validate_hex(hex_str)?;

        let bytes =
            <[u8; Self::LENGTH]>::from_hex(hex).map_err(|_| AddressParseError::InvalidHexString)?;

        // Optional: Uncomment if zero addresses should be rejected
        Self::validate_non_zero(&Self(bytes))?;

        Ok(Self(bytes))
    }

    /// Convert address to hex string without 0x prefix
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Convert address to hex string with 0x prefix
    pub fn to_hex_literal(&self) -> String {
        format!("0x{}", self.to_hex())
    }

    /// Parse a hex literal (with 0x prefix) with enhanced validation
    pub fn from_hex_literal(literal: &str) -> Result<Self, AddressParseError> {
        if literal.is_empty() {
            return Err(AddressParseError::EmptyString);
        }

        // Handle tagged addresses (e.g. "Dilithium3:HEX_PUB" or "Dilithium3:0xADDRESS")
        if let Some((tag, addr_or_pub)) = literal.split_once(':') {
            // If it's a tagged address where the part after colon is a hex literal (0x...),
            // then it's just an address with a hint.
            if addr_or_pub.starts_with("0x") {
                return Self::from_hex_literal(addr_or_pub);
            }

            // Otherwise, it's a Public Key (tagged) that needs to be hashed to get the address.
            use sha3::{Digest, Sha3_256};
            let mut hasher = Sha3_256::new();

            // Determine hashing strategy based on curve type:
            // - Pure PQC (Dilithium*, SphincsPlus*): Hash the DECODED bytes (matching key generation)
            // - Hybrid (Ed25519Dilithium3, K256Dilithium3): Hash the hex STRING (matching key generation)
            let tag_upper = tag.to_uppercase();
            let is_hybrid = tag_upper.contains("HYBRID")
                || tag_upper.contains("ED25519")
                || tag_upper.contains("K256");

            if is_hybrid {
                // Hybrid keys: hash the hex string representation (contains colons between parts)
                hasher.update(addr_or_pub.as_bytes());
            } else {
                // Pure PQC keys: decode hex to bytes first, then hash
                let pubkey_bytes =
                    hex::decode(addr_or_pub).map_err(|_| AddressParseError::InvalidHexString)?;
                hasher.update(&pubkey_bytes);
            }

            let hash_result = hasher.finalize();
            let mut bytes = [0u8; Self::LENGTH];
            bytes.copy_from_slice(&hash_result[..]);
            return Ok(Self(bytes));
        }

        if !literal.starts_with("0x") {
            return Err(AddressParseError::InvalidHexPrefix);
        }

        let hex_str = &literal[2..];
        let hex_len = hex_str.len();

        // Validate hex string length
        if hex_len > Self::LENGTH * 2 {
            return Err(AddressParseError::Overflow);
        }

        Self::validate_hex(hex_str)?;

        // Pad if too short, but only with leading zeros
        let hex_str = if hex_len < Self::LENGTH * 2 {
            format!("{:0>width$}", hex_str, width = Self::LENGTH * 2)
        } else {
            hex_str.to_string()
        };

        Self::from_hex(&hex_str)
    }

    /// Convert to Move's AccountAddress
    pub fn to_account_address(&self) -> AccountAddress {
        AccountAddress::new(self.0)
    }

    /// Parse a string (potentially tagged) into a Move AccountAddress
    pub fn parse_to_account_address(s: &str) -> Result<AccountAddress, AddressParseError> {
        // If it already looks like a hex literal, use our enhanced parser
        if s.starts_with("0x") || s.contains(':') {
            Ok(Self::from_hex_literal(s)?.to_account_address())
        } else {
            // Otherwise, try to parse it as a raw hex string and then convert
            let hex_str = s.trim();
            // If it's a 32-byte hex string (64 chars), it's probably a raw address
            if hex_str.len() == Self::LENGTH * 2 {
                let addr = Self::from_hex(hex_str)?;
                Ok(addr.to_account_address())
            } else {
                // Try prepending 0x and parsing
                Ok(Self::from_hex_literal(&format!("0x{}", hex_str))?.to_account_address())
            }
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl fmt::LowerHex for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "0x")?;
        }
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; Address::LENGTH]> for Address {
    fn from(bytes: [u8; Address::LENGTH]) -> Self {
        Self(bytes)
    }
}

impl TryFrom<&[u8]> for Address {
    type Error = AddressParseError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.is_empty() {
            return Err(AddressParseError::EmptyString);
        }

        if bytes.len() > Self::LENGTH {
            return Err(AddressParseError::Overflow);
        }

        if bytes.len() < Self::LENGTH {
            return Err(AddressParseError::InvalidLength(bytes.len()));
        }

        // Validate padding if present
        Self::validate_padding(bytes)?;

        let mut address = [0u8; Self::LENGTH];
        address.copy_from_slice(bytes);

        // Optional: Uncomment if zero addresses should be rejected
        // Self::validate_non_zero(&Self(address))?;

        Ok(Self(address))
    }
}

impl FromStr for Address {
    type Err = AddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(address) = Address::from_hex_literal(s) {
            Ok(address)
        } else {
            Self::from_hex(s)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_hex_literal_tagged_address() {
        // Tagged address (0x prefixed)
        let addr_str =
            "Dilithium3:0xd897adffc6bd9c4334506147d8ff3c29cbf986599876ecfd930889f856c0d9c5";
        let addr = Address::from_hex_literal(addr_str).unwrap();
        assert_eq!(
            addr.to_hex_literal(),
            "0xd897adffc6bd9c4334506147d8ff3c29cbf986599876ecfd930889f856c0d9c5"
        );
    }

    #[test]
    fn test_from_hex_literal_tagged_pubkey() {
        // Tagged public key (non-0x prefixed) - Pure PQC keys hash decoded bytes
        let pubkey_hex = "d897adffc6bd9c4334506147d8ff3c29cbf986599876ecfd930889f856c0d9c5";
        let tagged_pubkey = format!("Dilithium3:{}", pubkey_hex);
        let addr = Address::from_hex_literal(&tagged_pubkey).unwrap();

        // Should hash the DECODED bytes (matching pure PQC key generation behavior)
        use sha3::{Digest, Sha3_256};
        let mut hasher = Sha3_256::new();

        // Decode hex to bytes first, then hash (matching key generation)
        let pubkey_bytes = hex::decode(pubkey_hex).expect("valid hex");
        hasher.update(&pubkey_bytes);

        let expected_hash = hasher.finalize();
        let mut expected_bytes = [0u8; Address::LENGTH];
        expected_bytes.copy_from_slice(&expected_hash[..]);
        let expected_addr = Address::from(expected_bytes);

        assert_eq!(addr, expected_addr);
    }

    #[test]
    fn test_from_hex_literal_dilithium5_key() {
        // Test Dilithium5 specifically - should match generate_dilithium5_keypair behavior
        // This is a sample Dilithium5 public key hex (1568 bytes when decoded)
        let pubkey_hex = "aabbccdd11223344556677889900aabbccddeeff00112233445566778899aabb";
        let tagged_pubkey = format!("Dilithium5:{}", pubkey_hex);
        let addr = Address::from_hex_literal(&tagged_pubkey).unwrap();

        // Should hash the DECODED bytes (matching Dilithium5 key generation)
        use sha3::{Digest, Sha3_256};
        let mut hasher = Sha3_256::new();
        let pubkey_bytes = hex::decode(pubkey_hex).expect("valid hex");
        hasher.update(&pubkey_bytes);

        let expected_hash = hasher.finalize();
        let mut expected_bytes = [0u8; Address::LENGTH];
        expected_bytes.copy_from_slice(&expected_hash[..]);
        let expected_addr = Address::from(expected_bytes);

        assert_eq!(addr, expected_addr);
    }

    #[test]
    fn test_from_hex_literal_hybrid_pubkey() {
        // Hybrid public key (tagged, two hex parts separated by colon)
        // Hybrid keys hash the STRING representation (not decoded bytes)
        let k256_part = "aabbccdd11223344";
        let dilithium_part = "5566778899aabbcc";
        let pubkey_hex = format!("{}:{}", k256_part, dilithium_part);
        let tagged_pubkey = format!("Hybrid:{}", pubkey_hex);
        let addr = Address::from_hex_literal(&tagged_pubkey).unwrap();

        // Should hash the HEX STRING (matching hybrid key generation behavior)
        use sha3::{Digest, Sha3_256};
        let mut hasher = Sha3_256::new();
        hasher.update(pubkey_hex.as_bytes());

        let expected_hash = hasher.finalize();
        let mut expected_bytes = [0u8; Address::LENGTH];
        expected_bytes.copy_from_slice(&expected_hash[..]);
        let expected_addr = Address::from(expected_bytes);

        assert_eq!(addr, expected_addr);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum AddressParseError {
    InvalidLength(usize),
    InvalidHexPrefix,
    InvalidHexString,
    InvalidCharacter(char),
    EmptyString,
    Overflow,
    InvalidUtf8,
    ZeroAddress,
    InvalidPadding,
}

impl fmt::Display for AddressParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::InvalidLength(len) => write!(
                f,
                "Invalid address length: {} (expected {})",
                len,
                Address::LENGTH
            ),
            Self::InvalidHexPrefix => write!(f, "Invalid hex literal prefix (expected '0x')"),
            Self::InvalidHexString => write!(f, "Invalid hex string"),
            Self::InvalidCharacter(c) => write!(f, "Invalid character in hex string: '{}'", c),
            Self::EmptyString => write!(f, "Empty address string"),
            Self::Overflow => write!(f, "Address value overflow"),
            Self::InvalidUtf8 => write!(f, "Invalid UTF-8 in address string"),
            Self::ZeroAddress => write!(f, "Zero address not allowed in this context"),
            Self::InvalidPadding => write!(f, "Invalid padding in address bytes"),
        }
    }
}

impl std::error::Error for AddressParseError {}
