use hex::FromHex;
use std::convert::TryFrom;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize}; // Import Serialize and Deserialize
                                     // Add this at the top with other imports
use move_core_types::account_address::AccountAddress;

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

// Add conversion back to Move AccountAddress
impl From<Address> for AccountAddress {
    fn from(addr: Address) -> Self {
        // Find the first non-zero byte to trim leading zeros
        let start_idx = addr.0.iter().position(|&b| b != 0).unwrap_or(addr.0.len() - 1);
        let trimmed_bytes = &addr.0[start_idx..];

        // AccountAddress expects exactly the right number of bytes
        AccountAddress::from_bytes(trimmed_bytes).unwrap_or_else(|_| {
            // Fallback: use the full 32 bytes
            AccountAddress::from_bytes(&addr.0).expect("Failed to convert Address to AccountAddress")
        })
    }
}

/// Represents an address in the system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address([u8; Address::LENGTH]);

impl Address {
    /// The number of bytes in an address
    pub const LENGTH: usize = 32;

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
                Ok(if idx < bytes.len() - Self::LENGTH {
                    return Err(AddressParseError::InvalidPadding);
                })
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

        let bytes = <[u8; Self::LENGTH]>::from_hex(hex)
            .map_err(|_| AddressParseError::InvalidHexString)?;

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
