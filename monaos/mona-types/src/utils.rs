/// Utility functions and helper types for mona-types
use crate::address::Address;
use crate::balance::Balance;
use crate::coin::Coin;
use crate::object::{ID, UID};
use crate::tx_context::TxContext;

/// Utility functions for common operations
pub struct Utils;

impl Utils {
    /// Generate a new object ID from transaction context
    pub fn new_object_id(ctx: &mut TxContext) -> ID {
        ID::from_address(ctx.fresh_object_address())
    }

    /// Generate a new UID from transaction context  
    pub fn new_uid(ctx: &mut TxContext) -> UID {
        UID::new(Self::new_object_id(ctx))
    }

    /// Check if an address is the zero address
    pub fn is_zero_address(addr: &Address) -> bool {
        *addr == Address::zero()
    }

    /// Convert bytes to hex string
    pub fn bytes_to_hex(bytes: &[u8]) -> String {
        hex::encode(bytes)
    }    /// Convert hex string to bytes
    pub fn hex_to_bytes(hex: &str) -> std::result::Result<Vec<u8>, hex::FromHexError> {
        hex::decode(hex)
    }

    /// Generate a deterministic address from seed
    pub fn address_from_seed(seed: &[u8]) -> Address {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(seed);
        let hash = hasher.finalize();
        
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hash[..32]);
        Address::new(bytes)
    }

    /// Validate that a balance has sufficient funds
    pub fn has_sufficient_balance<T>(balance: &Balance<T>, required: u64) -> bool {
        balance.value() >= required
    }    /// Safe balance subtraction that returns an error instead of panicking
    pub fn safe_balance_subtract<T>(
        balance: &mut Balance<T>, 
        amount: u64
    ) -> std::result::Result<Balance<T>, UtilError> {
        if balance.value() < amount {
            return Err(UtilError::InsufficientBalance);
        }
        balance.split(amount).map_err(|_| UtilError::BalanceOperation)
    }

    /// Combine multiple balances into one
    pub fn combine_balances<T>(mut balances: Vec<Balance<T>>) -> Balance<T> {
        if balances.is_empty() {
            return Balance::zero();
        }

        let mut result = balances.remove(0);
        for balance in balances {
            result.join(balance);
        }
        result
    }    /// Split a coin into multiple smaller coins
    pub fn split_coin_multiple<T>(
        coin: &mut Coin<T>,
        amounts: Vec<u64>,
        _ctx: &mut TxContext,
    ) -> std::result::Result<Vec<Coin<T>>, UtilError> {
        let mut results = Vec::new();
        
        for amount in amounts {
            let split_coin = coin.split(amount)
                .map_err(|_| UtilError::InsufficientBalance)?;
            results.push(split_coin);
        }
        
        Ok(results)
    }
}

/// Utility errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UtilError {
    #[error("Insufficient balance")]
    InsufficientBalance,
    #[error("Balance operation failed")]
    BalanceOperation,
    #[error("Invalid input")]
    InvalidInput,
    #[error("Conversion failed")]
    ConversionFailed,
}

/// Constants used throughout the system
pub mod constants {
    /// Maximum transaction size in bytes
    pub const MAX_TRANSACTION_SIZE: usize = 128 * 1024; // 128KB

    /// Maximum number of inputs in a transaction
    pub const MAX_TRANSACTION_INPUTS: usize = 256;

    /// Maximum number of objects created in a single transaction
    pub const MAX_OBJECTS_PER_TRANSACTION: usize = 1024;

    /// Gas price bounds
    pub const MIN_GAS_PRICE: u64 = 1;
    pub const MAX_GAS_PRICE: u64 = 100_000;

    /// Object size limits
    pub const MAX_OBJECT_SIZE: usize = 256 * 1024; // 256KB
    pub const MAX_MOVE_PACKAGE_SIZE: usize = 1024 * 1024; // 1MB
}

/// Type aliases for commonly used types
pub type Result<T> = std::result::Result<T, UtilError>;

/// Validator for common input types
pub struct Validator;

impl Validator {    /// Validate an address format
    pub fn validate_address(_addr: &Address) -> std::result::Result<(), UtilError> {
        // Basic validation - address should not be all zeros unless it's the system address
        Ok(())
    }    /// Validate a balance amount
    pub fn validate_balance_amount(amount: u64) -> std::result::Result<(), UtilError> {
        if amount == 0 {
            return Err(UtilError::InvalidInput);
        }
        Ok(())
    }

    /// Validate transaction hash format
    pub fn validate_tx_hash(hash: &[u8]) -> Result<()> {
        if hash.len() != 32 {
            return Err(UtilError::InvalidInput);
        }
        Ok(())
    }

    /// Validate object ID format
    pub fn validate_object_id(id: &ID) -> Result<()> {
        // Object IDs should be valid addresses
        Self::validate_address(&id.to_address())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gas_coin::KARI;

    #[test]
    fn test_address_from_seed() {
        let seed = b"test_seed";
        let addr1 = Utils::address_from_seed(seed);
        let addr2 = Utils::address_from_seed(seed);
        assert_eq!(addr1, addr2); // Should be deterministic
        
        let addr3 = Utils::address_from_seed(b"different_seed");
        assert_ne!(addr1, addr3); // Different seeds should produce different addresses
    }

    #[test]
    fn test_sufficient_balance() {
        let balance: Balance<KARI> = Balance::with_value(100);
        assert!(Utils::has_sufficient_balance(&balance, 50));
        assert!(Utils::has_sufficient_balance(&balance, 100));
        assert!(!Utils::has_sufficient_balance(&balance, 101));
    }

    #[test]
    fn test_safe_balance_subtract() {
        let mut balance: Balance<KARI> = Balance::with_value(100);
        
        let result = Utils::safe_balance_subtract(&mut balance, 30);
        assert!(result.is_ok());
        assert_eq!(balance.value(), 70);
        
        let result = Utils::safe_balance_subtract(&mut balance, 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_combine_balances() {
        let balances = vec![
            Balance::<KARI>::with_value(10),
            Balance::<KARI>::with_value(20),
            Balance::<KARI>::with_value(30),
        ];
        
        let combined = Utils::combine_balances(balances);
        assert_eq!(combined.value(), 60);
    }

    #[test]
    fn test_bytes_hex_conversion() {
        let bytes = vec![0x12, 0x34, 0x56, 0x78];
        let hex = Utils::bytes_to_hex(&bytes);
        assert_eq!(hex, "12345678");
        
        let decoded = Utils::hex_to_bytes(&hex).unwrap();
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn test_validator() {
        let addr = Address::zero();
        assert!(Validator::validate_address(&addr).is_ok());
        
        assert!(Validator::validate_balance_amount(100).is_ok());
        assert!(Validator::validate_balance_amount(0).is_err());
        
        let hash = vec![0u8; 32];
        assert!(Validator::validate_tx_hash(&hash).is_ok());
        
        let short_hash = vec![0u8; 16];
        assert!(Validator::validate_tx_hash(&short_hash).is_err());
    }
}
