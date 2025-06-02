// Copyright (c) Kanari Network
// SPDX-License-Identifier: Apache-2.0

use lazy_static::lazy_static;
use log::debug;
use std::sync::RwLock;

// Gas fee collector address remains constant
pub const GAS_FEE_COLLECTOR: &str =
    "0x47621776628ba3a5b9baaab38e61f4c98e893e124204bc4dad52e702e2b24ea1";

// Gas fee configuration
pub const MIN_GAS_FEE: u64 = 20_000; // Minimum gas fee (0.00002 KA)
pub const BASE_GAS_FEE: u64 = 50_000; // Base gas fee (0.00005 KA)
pub const MAX_GAS_FEE: u64 = 3_000_000; // Maximum gas fee (0.003 KA)
pub const CONGESTION_MULTIPLIER: f64 = 1.5; // How much each pending tx affects fee

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

// Update network stats with pending transaction count
pub fn update_pending_transaction_count(count: usize) {
    if let Ok(mut stats) = NETWORK_STATS.write() {
        stats.pending_transactions = count;
    }
}

// Update network stats with last block time
pub fn update_last_block_time(timestamp: u64) {
    if let Ok(mut stats) = NETWORK_STATS.write() {
        stats.last_block_time = timestamp;
    }
}

// Update 24h transaction count
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

    debug!(
        "Calculated gas fee: {} (base: {}, tx_multiplier: {:.2}, volume_factor: {:.2}, priority: {}, pending txs: {})",
        gas_fee,
        BASE_GAS_FEE,
        tx_multiplier,
        volume_factor,
        priority,
        network_stats.pending_transactions
    );

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

/// Calculate deployment gas fee with special handling for new accounts
pub fn calculate_deployment_gas_fee(has_balance: bool) -> u64 {
    if has_balance {
        // Normal deployment gas for accounts with balance
        BASE_GAS_FEE
    } else {
        // Free deployment for new accounts (minimal gas)
        10_000 // 0.00001 KARI
    }
}

/// Check if an account qualifies for free deployment
pub fn qualifies_for_free_deployment(balance: u64) -> bool {
    balance == 0
}
