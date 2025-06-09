use std::time::{SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use log::debug;
use std::sync::RwLock;
use lazy_static::lazy_static;

use crate::balance::Balance;
use crate::coin::{Coin, TreasuryCap, CoinMetadata};
use crate::address::Address;

// Gas fee collector address remains constant
pub const GAS_FEE_COLLECTOR: &str = "0x47621776628ba3a5b9baaab38e61f4c98e893e124204bc4dad52e702e2b24ea1";

// Gas fee configuration
pub const MIN_GAS_FEE: u64 = 20_000;     // Minimum gas fee (0.00002 KA)
pub const BASE_GAS_FEE: u64 = 50_000;    // Base gas fee (0.00005 KA)
pub const MAX_GAS_FEE: u64 = 3_000_000;  // Maximum gas fee (0.003 KA)
pub const CONGESTION_MULTIPLIER: f64 = 1.5; // How much each pending tx affects fee

// Constants for KARI token
/// The amount of KA per Kari token based on the the fact that KA is
/// 10^-9 of a Kari token
pub const KA_PER_KARI: u64 = 1_000_000_000;

/// The total supply of Kari denominated in whole Kari tokens (100 Million)
pub const TOTAL_SUPPLY_KARI: u64 = 100_000_000;

/// The total supply of Kari denominated in KA (100 Million * 10^9)
pub const TOTAL_SUPPLY_KA: u64 = 100_000_000_000_000_000;

// Store network statistics for gas calculation
lazy_static! {
    pub static ref NETWORK_STATS: RwLock<NetworkStats> = RwLock::new(NetworkStats::default());
}

pub struct NetworkStats {
    pub pending_transactions: usize,
    pub last_block_time: u64,
    pub transaction_count_24h: usize,
}

impl Default for NetworkStats {
    fn default() -> Self {
        NetworkStats {
            pending_transactions: 0,
            last_block_time: 0,
            transaction_count_24h: 0,
        }
    }
}

// Make NetworkStats cloneable
impl Clone for NetworkStats {
    fn clone(&self) -> Self {
        Self {
            pending_transactions: self.pending_transactions,
            last_block_time: self.last_block_time,
            transaction_count_24h: self.transaction_count_24h,
        }
    }
}

/// Name of the KARI coin type - corresponds to the Move struct KARI
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KARI;

/// Type alias for KARI coin
pub type KariCoin = Coin<KARI>;

/// Type alias for KARI balance
pub type KariBalance = Balance<KARI>;

/// Type alias for KARI treasury capability
pub type KariTreasuryCap = TreasuryCap<KARI>;

/// Type alias for KARI metadata
pub type KariMetadata = CoinMetadata<KARI>;

/// KARI token operations
pub struct KariOps;

impl KariOps {
    /// Create the initial KARI supply (should only be called during genesis)
    pub fn create_genesis_supply(genesis_address: Address) -> Result<KariBalance, KariError> {
        if genesis_address != Address::zero() {
            return Err(KariError::NotSystemAddress);
        }
        
        Ok(Balance::with_value(TOTAL_SUPPLY_KA))
    }

    /// Get KARI metadata
    pub fn get_metadata() -> KariMetadata {
        CoinMetadata::new(
            crate::coin::ObjectId::new(Address::zero()), // Placeholder ID
            9, // 9 decimals
            "Kanara Network Coin".to_string(),
            "KARI".to_string(),
            "The native token of the Kanara Network".to_string(),
            None, // No icon URL yet
        )
    }

    /// Convert KA (smallest unit) to KARI
    pub fn ka_to_kari(ka_amount: u64) -> f64 {
        ka_amount as f64 / KA_PER_KARI as f64
    }

    /// Convert KARI to KA (smallest unit)
    pub fn kari_to_ka(kari_amount: f64) -> u64 {
        (kari_amount * KA_PER_KARI as f64) as u64
    }

    /// Format a KA amount as a human-readable KARI string
    pub fn format_kari_amount(ka_amount: u64) -> String {
        let kari_amount = Self::ka_to_kari(ka_amount);
        format!("{:.9} KARI", kari_amount)
    }

    /// Parse a KARI amount string to KA
    pub fn parse_kari_amount(kari_str: &str) -> Result<u64, KariError> {
        let kari_str = kari_str.trim().replace("KARI", "").trim().to_string();
        let kari_amount: f64 = kari_str.parse()
            .map_err(|_| KariError::InvalidAmount)?;
        
        if kari_amount < 0.0 {
            return Err(KariError::InvalidAmount);
        }
        
        Ok(Self::kari_to_ka(kari_amount))
    }

    /// Check if an amount is valid (not exceeding total supply)
    pub fn is_valid_amount(ka_amount: u64) -> bool {
        ka_amount <= TOTAL_SUPPLY_KA
    }

    /// Get current timestamp in milliseconds
    pub fn current_timestamp_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

/// KARI-specific errors
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KariError {
    #[error("Already minted")]
    AlreadyMinted,
    #[error("Not system address")]
    NotSystemAddress,
    #[error("Invalid amount")]
    InvalidAmount,
    #[error("Insufficient balance")]
    InsufficientBalance,
}

/// KARI error constants matching Move constants
pub mod error_constants {
    pub const E_ALREADY_MINTED: u64 = 0;
    pub const E_NOT_SYSTEM_ADDRESS: u64 = 1;
}

/// KARI transaction types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KariTransfer {
    pub from: Address,
    pub to: Address,
    pub amount: u64, // in KA
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KariBurn {
    pub burner: Address,
    pub amount: u64, // in KA
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KariMint {
    pub minter: Address,
    pub recipient: Address,
    pub amount: u64, // in KA
    pub timestamp: u64,
}

impl KariTransfer {
    pub fn new(from: Address, to: Address, amount: u64) -> Self {
        Self {
            from,
            to,
            amount,
            timestamp: KariOps::current_timestamp_ms(),
        }
    }

    pub fn amount_in_kari(&self) -> f64 {
        KariOps::ka_to_kari(self.amount)
    }
}

impl KariBurn {
    pub fn new(burner: Address, amount: u64) -> Self {
        Self {
            burner,
            amount,
            timestamp: KariOps::current_timestamp_ms(),
        }
    }

    pub fn amount_in_kari(&self) -> f64 {
        KariOps::ka_to_kari(self.amount)
    }
}

impl KariMint {
    pub fn new(minter: Address, recipient: Address, amount: u64) -> Self {
        Self {
            minter,
            recipient,
            amount,
            timestamp: KariOps::current_timestamp_ms(),
        }
    }

    pub fn amount_in_kari(&self) -> f64 {
        KariOps::ka_to_kari(self.amount)
    }
}

// Gas fee management functions
/// Update network stats with pending transaction count
pub fn update_pending_transaction_count(count: usize) {
    if let Ok(mut stats) = NETWORK_STATS.write() {
        stats.pending_transactions = count;
    }
}

/// Update network stats with last block time
pub fn update_last_block_time(timestamp: u64) {
    if let Ok(mut stats) = NETWORK_STATS.write() {
        stats.last_block_time = timestamp;
    }
}

/// Update 24h transaction count
pub fn update_transaction_count_24h(count: usize) {
    if let Ok(mut stats) = NETWORK_STATS.write() {
        stats.transaction_count_24h = count;
    }
}

/// Calculate gas fee dynamically based on network conditions
/// 
/// # Parameters
/// * `priority_boost`: Optional priority boost in gas units. Higher values will result in faster processing.
///
/// # Returns
/// The calculated gas fee in KA units, bounded between MIN_GAS_FEE and MAX_GAS_FEE.
/// 
/// # Algorithm
/// The gas calculation uses several factors:
/// 1. Base gas fee (constant)
/// 2. Network congestion multiplier based on pending transactions
/// 3. Transaction volume adjustment for network sustainability
/// 4. User-provided priority boost for urgent transactions
pub fn calculate_gas_fee(priority_boost: Option<u64>) -> u64 {
    let network_stats = match NETWORK_STATS.read() {
        Ok(stats) => stats,
        Err(_) => return BASE_GAS_FEE, // Default to base fee if can't read stats
    };
    
    // Calculate congestion component based on pending transactions with exponential scaling
    // This creates a more responsive fee market that scales with network demand
    let pending_tx_count = network_stats.pending_transactions as f64;
    let tx_multiplier = if pending_tx_count > 0.0 {
        // Log-based scaling to handle both small and large transaction volumes
        (1.0 + (pending_tx_count / 10.0).ln_1p()) * CONGESTION_MULTIPLIER
    } else {
        1.0
    };
    
    // Factor in 24h transaction volume for longer-term fee adjustment
    let volume_factor = if network_stats.transaction_count_24h > 1000 {
        // Slight increase based on 24h volume
        1.0 + (network_stats.transaction_count_24h as f64 / 10000.0).min(0.5)
    } else {
        1.0
    };
    
    // Apply user's priority boost if provided
    let priority = priority_boost.unwrap_or(0);
    
    // Calculate total gas fee: start with BASE_FEE and apply multipliers
    let gas_fee = ((BASE_GAS_FEE as f64) * tx_multiplier * volume_factor) as u64 + priority;
    
    // Ensure gas fee is within allowed range
    let gas_fee = gas_fee.clamp(MIN_GAS_FEE, MAX_GAS_FEE);
    
    debug!("Calculated gas fee: {} (base: {}, tx_multiplier: {:.2}, volume_factor: {:.2}, priority: {}, pending txs: {})",
           gas_fee, BASE_GAS_FEE, tx_multiplier, volume_factor, priority, network_stats.pending_transactions);
    
    gas_fee
}

/// Format gas fee for display with appropriate precision
pub fn format_gas_fee_display(fee: u64) -> String {
    const KA_PER_KARI: f64 = 1_000_000_000.0;
    let fee_in_kari = fee as f64 / KA_PER_KARI;
    format!("{:.9} KARI", fee_in_kari)
}

/// Calculate total amount needed for a transaction including gas
pub fn calculate_total_transaction_cost(amount: u64, gas_fee: u64) -> u64 {
    amount + gas_fee
}

/// Validate that the user has enough funds for amount + gas fee
pub fn validate_transaction_funds(balance: u64, amount: u64, gas_fee: u64) -> bool {
    balance >= calculate_total_transaction_cost(amount, gas_fee)
}

/// Get current network statistics for gas fee calculation
pub fn get_network_stats() -> NetworkStats {
    match NETWORK_STATS.read() {
        Ok(stats) => stats.clone(),
        Err(_) => NetworkStats::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kari_conversion() {
        assert_eq!(KariOps::kari_to_ka(1.0), KA_PER_KARI);
        assert_eq!(KariOps::ka_to_kari(KA_PER_KARI), 1.0);
        assert_eq!(KariOps::kari_to_ka(0.000000001), 1);
    }

    #[test]
    fn test_format_kari_amount() {
        let formatted = KariOps::format_kari_amount(KA_PER_KARI);
        assert_eq!(formatted, "1.000000000 KARI");
        
        let formatted = KariOps::format_kari_amount(1);
        assert_eq!(formatted, "0.000000001 KARI");
    }

    #[test]
    fn test_parse_kari_amount() {
        let ka = KariOps::parse_kari_amount("1.0 KARI").unwrap();
        assert_eq!(ka, KA_PER_KARI);
        
        let ka = KariOps::parse_kari_amount("0.000000001").unwrap();
        assert_eq!(ka, 1);
        
        assert!(KariOps::parse_kari_amount("invalid").is_err());
        assert!(KariOps::parse_kari_amount("-1.0").is_err());
    }

    #[test]
    fn test_valid_amount() {
        assert!(KariOps::is_valid_amount(TOTAL_SUPPLY_KA));
        assert!(KariOps::is_valid_amount(0));
        assert!(!KariOps::is_valid_amount(TOTAL_SUPPLY_KA + 1));
    }

    #[test]
    fn test_kari_transfer() {
        let from = Address::zero();
        let to = Address::zero();
        let amount = KA_PER_KARI; // 1 KARI
        
        let transfer = KariTransfer::new(from, to, amount);
        assert_eq!(transfer.from, from);
        assert_eq!(transfer.to, to);
        assert_eq!(transfer.amount, amount);
        assert_eq!(transfer.amount_in_kari(), 1.0);
    }

    #[test]
    fn test_gas_fee_calculation() {
        // Test basic gas fee calculation
        let base_fee = calculate_gas_fee(None);
        assert!(base_fee >= MIN_GAS_FEE);
        assert!(base_fee <= MAX_GAS_FEE);
        
        // Test with priority boost
        let priority_fee = calculate_gas_fee(Some(10_000));
        assert!(priority_fee >= base_fee);
    }

    #[test]
    fn test_transaction_cost_calculation() {
        let amount = 1_000_000;
        let gas_fee = 50_000;
        let total = calculate_total_transaction_cost(amount, gas_fee);
        assert_eq!(total, amount + gas_fee);
    }

    #[test]
    fn test_funds_validation() {
        let balance = 2_000_000;
        let amount = 1_000_000;
        let gas_fee = 50_000;
        
        assert!(validate_transaction_funds(balance, amount, gas_fee));
        assert!(!validate_transaction_funds(500_000, amount, gas_fee));
    }

    #[test]
    fn test_format_gas_fee_display() {
        let fee = KA_PER_KARI; // 1 KARI
        let formatted = format_gas_fee_display(fee);
        assert_eq!(formatted, "1.000000000 KARI");
    }
}
